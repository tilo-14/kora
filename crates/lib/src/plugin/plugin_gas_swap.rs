use async_trait::async_trait;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_message::compiled_instruction::CompiledInstruction;
use solana_sdk::pubkey::Pubkey;
use solana_system_interface::program::ID as SYSTEM_PROGRAM_ID;
use spl_token_2022_interface::ID as TOKEN_2022_PROGRAM_ID;
use spl_token_interface::ID as SPL_TOKEN_PROGRAM_ID;

use crate::{
    config::Config,
    error::KoraError,
    transaction::{
        ParsedSPLInstructionData, ParsedSPLInstructionType, ParsedSystemInstructionData,
        ParsedSystemInstructionType, VersionedTransactionResolved,
    },
};

use super::{PluginExecutionContext, TransactionPlugin};

pub(super) struct GasSwapPlugin;

impl GasSwapPlugin {
    fn outer_instruction_program_id(
        instruction: &CompiledInstruction,
        all_account_keys: &[Pubkey],
        context: PluginExecutionContext,
    ) -> Result<Pubkey, KoraError> {
        all_account_keys.get(instruction.program_id_index as usize).copied().ok_or_else(|| {
            KoraError::InvalidTransaction(format!(
                "Plugin gas_swap missing program_id at index {} in {}",
                instruction.program_id_index,
                context.method_name()
            ))
        })
    }

    fn validate_outer_program_shape(
        transaction: &VersionedTransactionResolved,
        context: PluginExecutionContext,
    ) -> Result<(), KoraError> {
        let outer_instructions = transaction.message.instructions();
        let all_account_keys = &transaction.all_account_keys;

        if outer_instructions.len() != 2 {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires exactly two outer instructions (1 System + 1 SPL/Token2022 transfer), found {} in {}",
                outer_instructions.len(),
                context.method_name()
            )));
        }

        let mut outer_system_count = 0usize;
        let mut outer_token_count = 0usize;

        for instruction in outer_instructions {
            let program_id =
                Self::outer_instruction_program_id(instruction, all_account_keys, context)?;
            if program_id == SYSTEM_PROGRAM_ID {
                outer_system_count += 1;
            } else if program_id == SPL_TOKEN_PROGRAM_ID || program_id == TOKEN_2022_PROGRAM_ID {
                outer_token_count += 1;
            } else {
                return Err(KoraError::InvalidTransaction(format!(
                    "Plugin gas_swap rejected outer program {} in {}. Only System and SPL/Token2022 transfer instructions are allowed.",
                    program_id,
                    context.method_name()
                )));
            }
        }

        if outer_system_count != 1 {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires exactly one outer System instruction, found {} in {}",
                outer_system_count,
                context.method_name()
            )));
        }

        if outer_token_count != 1 {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires exactly one outer SPL/Token2022 instruction, found {} in {}",
                outer_token_count,
                context.method_name()
            )));
        }

        Ok(())
    }

    fn validate_parsed_system_transfer(
        transaction: &mut VersionedTransactionResolved,
        fee_payer: &Pubkey,
        context: PluginExecutionContext,
    ) -> Result<(), KoraError> {
        let system_instructions = transaction.get_or_parse_system_instructions()?;

        if system_instructions.len() != 1
            || !system_instructions.contains_key(&ParsedSystemInstructionType::SystemTransfer)
        {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires only SystemTransfer instruction type in {}",
                context.method_name()
            )));
        }

        let transfers = system_instructions
            .get(&ParsedSystemInstructionType::SystemTransfer)
            .ok_or_else(|| {
                KoraError::InvalidTransaction(format!(
                    "Plugin gas_swap requires exactly one SystemTransfer in {}",
                    context.method_name()
                ))
            })?;

        if transfers.len() != 1 {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires exactly one SystemTransfer, found {} in {}",
                transfers.len(),
                context.method_name()
            )));
        }

        let ParsedSystemInstructionData::SystemTransfer { lamports, sender, .. } = &transfers[0]
        else {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires parsed SystemTransfer in {}",
                context.method_name()
            )));
        };

        if *lamports == 0 {
            return Err(KoraError::InvalidTransaction(
                "Plugin gas_swap requires non-zero SOL transfer".to_string(),
            ));
        }

        if *sender != *fee_payer {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires fee payer {} to be SOL transfer sender, got {}",
                fee_payer, sender
            )));
        }

        Ok(())
    }

    fn validate_parsed_token_transfer(
        transaction: &mut VersionedTransactionResolved,
        fee_payer: &Pubkey,
        context: PluginExecutionContext,
    ) -> Result<(), KoraError> {
        let spl_instructions = transaction.get_or_parse_spl_instructions()?;

        if spl_instructions.len() != 1
            || !spl_instructions.contains_key(&ParsedSPLInstructionType::SplTokenTransfer)
        {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires only SplTokenTransfer instruction type in {}",
                context.method_name()
            )));
        }

        let transfers =
            spl_instructions.get(&ParsedSPLInstructionType::SplTokenTransfer).ok_or_else(|| {
                KoraError::InvalidTransaction(format!(
                    "Plugin gas_swap requires exactly one SplTokenTransfer in {}",
                    context.method_name()
                ))
            })?;

        if transfers.len() != 1 {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires exactly one SplTokenTransfer, found {} in {}",
                transfers.len(),
                context.method_name()
            )));
        }

        let ParsedSPLInstructionData::SplTokenTransfer { amount, owner, .. } = &transfers[0] else {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires parsed SplTokenTransfer in {}",
                context.method_name()
            )));
        };

        if *amount == 0 {
            return Err(KoraError::InvalidTransaction(
                "Plugin gas_swap requires non-zero token transfer".to_string(),
            ));
        }

        if *owner == *fee_payer {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires token payer to differ from fee payer {}",
                fee_payer
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl TransactionPlugin for GasSwapPlugin {
    async fn validate(
        &self,
        transaction: &mut VersionedTransactionResolved,
        _config: &Config,
        _rpc_client: &RpcClient,
        fee_payer: &Pubkey,
        context: PluginExecutionContext,
    ) -> Result<(), KoraError> {
        Self::validate_outer_program_shape(transaction, context)?;
        Self::validate_parsed_system_transfer(transaction, fee_payer, context)?;
        Self::validate_parsed_token_transfer(transaction, fee_payer, context)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{super::TransactionPluginRunner, *};
    use crate::{
        config::TransactionPluginType,
        tests::{common::RpcMockBuilder, config_mock::ConfigMockBuilder},
        transaction::TransactionUtil,
    };
    use solana_compute_budget_interface::ComputeBudgetInstruction;
    use solana_message::{Message, VersionedMessage};
    use solana_sdk::{instruction::Instruction, pubkey::Pubkey};
    use solana_system_interface::instruction::{assign, transfer};
    use std::sync::Arc;

    fn enable_gas_swap_plugin(config: &mut Config) {
        config.kora.plugins.enabled = vec![TransactionPluginType::GasSwap];
    }

    fn build_runner() -> (Config, Arc<RpcClient>) {
        let mut config = ConfigMockBuilder::new().build();
        enable_gas_swap_plugin(&mut config);
        let rpc_client = RpcMockBuilder::new().build();
        (config, rpc_client)
    }

    #[tokio::test]
    async fn gas_swap_accepts_valid_top_level_swap_shape() {
        let (config, rpc_client) = build_runner();

        let fee_payer = Pubkey::new_unique();
        let source_wallet = Pubkey::new_unique();
        let source_token_account = Pubkey::new_unique();
        let destination_token_account = Pubkey::new_unique();
        let destination_wallet = Pubkey::new_unique();

        let token_ix = spl_token_interface::instruction::transfer(
            &spl_token_interface::id(),
            &source_token_account,
            &destination_token_account,
            &source_wallet,
            &[],
            1_500,
        )
        .unwrap();
        let sol_ix = transfer(&fee_payer, &destination_wallet, 20_000);

        let tx = TransactionUtil::new_unsigned_versioned_transaction(VersionedMessage::Legacy(
            Message::new(&[token_ix, sol_ix], Some(&fee_payer)),
        ));
        let mut resolved = VersionedTransactionResolved::from_kora_built_transaction(&tx).unwrap();

        let runner = TransactionPluginRunner::from_config(&config);
        let result = runner
            .run(
                &mut resolved,
                &config,
                rpc_client.as_ref(),
                &fee_payer,
                PluginExecutionContext::SignTransaction,
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn gas_swap_rejects_non_swap_programs() {
        let (config, rpc_client) = build_runner();

        let fee_payer = Pubkey::new_unique();
        let source_wallet = Pubkey::new_unique();
        let source_token_account = Pubkey::new_unique();
        let destination_token_account = Pubkey::new_unique();
        let destination_wallet = Pubkey::new_unique();

        let token_ix = spl_token_interface::instruction::transfer(
            &spl_token_interface::id(),
            &source_token_account,
            &destination_token_account,
            &source_wallet,
            &[],
            1_500,
        )
        .unwrap();
        let custom_ix =
            Instruction { program_id: Pubkey::new_unique(), accounts: vec![], data: vec![1, 2, 3] };
        let sol_ix = transfer(&fee_payer, &destination_wallet, 20_000);

        let tx = TransactionUtil::new_unsigned_versioned_transaction(VersionedMessage::Legacy(
            Message::new(&[token_ix, custom_ix, sol_ix], Some(&fee_payer)),
        ));
        let mut resolved = VersionedTransactionResolved::from_kora_built_transaction(&tx).unwrap();

        let runner = TransactionPluginRunner::from_config(&config);
        let result = runner
            .run(
                &mut resolved,
                &config,
                rpc_client.as_ref(),
                &fee_payer,
                PluginExecutionContext::SignAndSendTransaction,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KoraError::InvalidTransaction(_)));
    }

    #[tokio::test]
    async fn gas_swap_rejects_non_transfer_system_instruction() {
        let (config, rpc_client) = build_runner();

        let fee_payer = Pubkey::new_unique();
        let source_wallet = Pubkey::new_unique();
        let source_token_account = Pubkey::new_unique();
        let destination_token_account = Pubkey::new_unique();
        let token_ix = spl_token_interface::instruction::transfer(
            &spl_token_interface::id(),
            &source_token_account,
            &destination_token_account,
            &source_wallet,
            &[],
            1_500,
        )
        .unwrap();
        let assign_ix = assign(&fee_payer, &Pubkey::new_unique());

        let tx = TransactionUtil::new_unsigned_versioned_transaction(VersionedMessage::Legacy(
            Message::new(&[token_ix, assign_ix], Some(&fee_payer)),
        ));
        let mut resolved = VersionedTransactionResolved::from_kora_built_transaction(&tx).unwrap();

        let runner = TransactionPluginRunner::from_config(&config);
        let result = runner
            .run(
                &mut resolved,
                &config,
                rpc_client.as_ref(),
                &fee_payer,
                PluginExecutionContext::SignTransaction,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KoraError::InvalidTransaction(_)));
    }

    #[tokio::test]
    async fn gas_swap_rejects_when_no_token_transfer() {
        let (config, rpc_client) = build_runner();

        let fee_payer = Pubkey::new_unique();
        let destination_wallet = Pubkey::new_unique();
        let sol_ix = transfer(&fee_payer, &destination_wallet, 20_000);

        let tx = TransactionUtil::new_unsigned_versioned_transaction(VersionedMessage::Legacy(
            Message::new(&[sol_ix], Some(&fee_payer)),
        ));
        let mut resolved = VersionedTransactionResolved::from_kora_built_transaction(&tx).unwrap();

        let runner = TransactionPluginRunner::from_config(&config);
        let result = runner
            .run(
                &mut resolved,
                &config,
                rpc_client.as_ref(),
                &fee_payer,
                PluginExecutionContext::SignAndSendTransaction,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KoraError::InvalidTransaction(_)));
    }

    #[tokio::test]
    async fn gas_swap_rejects_extra_outer_instruction() {
        let (config, rpc_client) = build_runner();

        let fee_payer = Pubkey::new_unique();
        let source_wallet = Pubkey::new_unique();
        let source_token_account = Pubkey::new_unique();
        let destination_token_account = Pubkey::new_unique();
        let destination_wallet = Pubkey::new_unique();

        let token_ix = spl_token_interface::instruction::transfer(
            &spl_token_interface::id(),
            &source_token_account,
            &destination_token_account,
            &source_wallet,
            &[],
            1_500,
        )
        .unwrap();
        let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(200_000);
        let sol_ix = transfer(&fee_payer, &destination_wallet, 20_000);

        let tx = TransactionUtil::new_unsigned_versioned_transaction(VersionedMessage::Legacy(
            Message::new(&[token_ix, compute_budget_ix, sol_ix], Some(&fee_payer)),
        ));
        let mut resolved = VersionedTransactionResolved::from_kora_built_transaction(&tx).unwrap();

        let runner = TransactionPluginRunner::from_config(&config);
        let result = runner
            .run(
                &mut resolved,
                &config,
                rpc_client.as_ref(),
                &fee_payer,
                PluginExecutionContext::SignTransaction,
            )
            .await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KoraError::InvalidTransaction(_)));
    }
}
