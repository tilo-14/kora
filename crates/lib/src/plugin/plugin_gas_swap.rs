use async_trait::async_trait;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_message::compiled_instruction::CompiledInstruction;
use solana_sdk::pubkey::Pubkey;
use solana_system_interface::{instruction::SystemInstruction, program::ID as SYSTEM_PROGRAM_ID};
use spl_token_2022_interface::{
    instruction::TokenInstruction as Token2022Instruction, ID as TOKEN_2022_PROGRAM_ID,
};
use spl_token_interface::{
    instruction::TokenInstruction as SplTokenInstruction, ID as SPL_TOKEN_PROGRAM_ID,
};

use crate::{
    config::Config, constant::instruction_indexes, error::KoraError,
    transaction::VersionedTransactionResolved,
};

use super::{PluginExecutionContext, TransactionPlugin};

pub(super) struct GasSwapPlugin;

impl GasSwapPlugin {
    fn require_program_id(
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

    fn require_account_pubkey(
        instruction: &CompiledInstruction,
        all_account_keys: &[Pubkey],
        index: usize,
        field: &str,
        context: PluginExecutionContext,
    ) -> Result<Pubkey, KoraError> {
        let key_index = instruction.accounts.get(index).copied().ok_or_else(|| {
            KoraError::InvalidTransaction(format!(
                "Plugin gas_swap missing account {} at index {} in {}",
                field,
                index,
                context.method_name()
            ))
        })?;

        all_account_keys.get(key_index as usize).copied().ok_or_else(|| {
            KoraError::InvalidTransaction(format!(
                "Plugin gas_swap missing account {} pubkey at key index {} in {}",
                field,
                key_index,
                context.method_name()
            ))
        })
    }

    fn validate_outer_system_instruction(
        instruction: &CompiledInstruction,
        all_account_keys: &[Pubkey],
        fee_payer: &Pubkey,
        context: PluginExecutionContext,
    ) -> Result<(), KoraError> {
        let system_ix =
            bincode::deserialize::<SystemInstruction>(&instruction.data).map_err(|e| {
                KoraError::InvalidTransaction(format!(
                    "Plugin gas_swap failed to decode system instruction in {}: {}",
                    context.method_name(),
                    e
                ))
            })?;

        match system_ix {
            SystemInstruction::Transfer { lamports } => Self::validate_fee_payer_sol_sender(
                instruction,
                all_account_keys,
                instruction_indexes::system_transfer::SENDER_INDEX,
                lamports,
                fee_payer,
                context,
            ),
            SystemInstruction::TransferWithSeed { lamports, .. } => {
                Self::validate_fee_payer_sol_sender(
                    instruction,
                    all_account_keys,
                    instruction_indexes::system_transfer_with_seed::SENDER_INDEX,
                    lamports,
                    fee_payer,
                    context,
                )
            }
            other => Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap rejected system instruction {:?} in {}",
                other,
                context.method_name()
            ))),
        }
    }

    fn validate_fee_payer_sol_sender(
        instruction: &CompiledInstruction,
        all_account_keys: &[Pubkey],
        sender_index: usize,
        lamports: u64,
        fee_payer: &Pubkey,
        context: PluginExecutionContext,
    ) -> Result<(), KoraError> {
        if lamports == 0 {
            return Err(KoraError::InvalidTransaction(
                "Plugin gas_swap requires non-zero SOL transfer".to_string(),
            ));
        }

        let sender = Self::require_account_pubkey(
            instruction,
            all_account_keys,
            sender_index,
            "sender",
            context,
        )?;
        if sender != *fee_payer {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap requires fee payer {} to be SOL transfer sender, got {}",
                fee_payer, sender
            )));
        }

        Ok(())
    }

    fn validate_outer_token_instruction(
        program_id: &Pubkey,
        instruction: &CompiledInstruction,
        all_account_keys: &[Pubkey],
        fee_payer: &Pubkey,
        context: PluginExecutionContext,
    ) -> Result<(), KoraError> {
        let (owner_index, amount) = if *program_id == SPL_TOKEN_PROGRAM_ID {
            match SplTokenInstruction::unpack(&instruction.data).map_err(|e| {
                KoraError::InvalidTransaction(format!(
                    "Plugin gas_swap failed to decode SPL token instruction in {}: {}",
                    context.method_name(),
                    e
                ))
            })? {
                SplTokenInstruction::Transfer { amount } => {
                    (instruction_indexes::spl_token_transfer::OWNER_INDEX, amount)
                }
                SplTokenInstruction::TransferChecked { amount, .. } => {
                    (instruction_indexes::spl_token_transfer_checked::OWNER_INDEX, amount)
                }
                other => {
                    return Err(KoraError::InvalidTransaction(format!(
                        "Plugin gas_swap rejected SPL instruction {:?} in {}",
                        other,
                        context.method_name()
                    )));
                }
            }
        } else if *program_id == TOKEN_2022_PROGRAM_ID {
            #[allow(deprecated)]
            match Token2022Instruction::unpack(&instruction.data).map_err(|e| {
                KoraError::InvalidTransaction(format!(
                    "Plugin gas_swap failed to decode Token2022 instruction in {}: {}",
                    context.method_name(),
                    e
                ))
            })? {
                Token2022Instruction::Transfer { amount } => {
                    (instruction_indexes::spl_token_transfer::OWNER_INDEX, amount)
                }
                Token2022Instruction::TransferChecked { amount, .. } => {
                    (instruction_indexes::spl_token_transfer_checked::OWNER_INDEX, amount)
                }
                other => {
                    return Err(KoraError::InvalidTransaction(format!(
                        "Plugin gas_swap rejected Token2022 instruction {:?} in {}",
                        other,
                        context.method_name()
                    )));
                }
            }
        } else {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin gas_swap only supports token instructions from {} or {}, got {} in {}",
                SPL_TOKEN_PROGRAM_ID,
                TOKEN_2022_PROGRAM_ID,
                program_id,
                context.method_name()
            )));
        };

        if amount == 0 {
            return Err(KoraError::InvalidTransaction(
                "Plugin gas_swap requires non-zero token transfer".to_string(),
            ));
        }

        let owner = Self::require_account_pubkey(
            instruction,
            all_account_keys,
            owner_index,
            "owner",
            context,
        )?;
        if owner == *fee_payer {
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
    fn name(&self) -> &'static str {
        "gas_swap"
    }

    async fn validate(
        &self,
        transaction: &mut VersionedTransactionResolved,
        _config: &Config,
        _rpc_client: &RpcClient,
        fee_payer: &Pubkey,
        context: PluginExecutionContext,
    ) -> Result<(), KoraError> {
        let outer_instructions = transaction.message.instructions();
        let all_account_keys = &transaction.all_account_keys;

        let mut system_transfer_count = 0usize;
        let mut token_transfer_count = 0usize;

        for instruction in outer_instructions {
            let program_id = Self::require_program_id(instruction, all_account_keys, context)?;

            if program_id == SYSTEM_PROGRAM_ID {
                Self::validate_outer_system_instruction(
                    instruction,
                    all_account_keys,
                    fee_payer,
                    context,
                )?;
                system_transfer_count += 1;
            } else if program_id == SPL_TOKEN_PROGRAM_ID || program_id == TOKEN_2022_PROGRAM_ID {
                Self::validate_outer_token_instruction(
                    &program_id,
                    instruction,
                    all_account_keys,
                    fee_payer,
                    context,
                )?;
                token_transfer_count += 1;
            } else {
                return Err(KoraError::InvalidTransaction(format!(
                    "Plugin {} rejected outer program {} in {}. Only System transfer and SPL transfer instructions are allowed.",
                    self.name(),
                    program_id,
                    context.method_name()
                )));
            }
        }

        if system_transfer_count != 1 {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin {} requires exactly one top-level SOL transfer from fee payer, found {}",
                self.name(),
                system_transfer_count
            )));
        }

        if token_transfer_count != 1 {
            return Err(KoraError::InvalidTransaction(format!(
                "Plugin {} requires exactly one top-level SPL token transfer, found {}",
                self.name(),
                token_transfer_count
            )));
        }

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
