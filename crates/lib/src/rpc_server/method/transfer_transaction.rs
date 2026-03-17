use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_keychain::SolanaSigner;
use solana_message::Message;
use solana_sdk::{message::VersionedMessage, pubkey::Pubkey, transaction::VersionedTransaction};
use solana_system_interface::instruction::transfer;
use std::{str::FromStr, sync::Arc};
use utoipa::ToSchema;

use crate::{
    constant::NATIVE_SOL,
    light_token::{
        constants::WSOL_MINT, conversions, instruction_builder::select_input_accounts,
        rpc_client::LightRpcClient, transaction::build_light_token_v0_transaction,
    },
    state::{get_config, get_request_signer_with_signer_key},
    token::token::TokenUtil,
    transaction::{
        TransactionUtil, VersionedMessageExt, VersionedTransactionOps, VersionedTransactionResolved,
    },
    validator::transaction_validator::TransactionValidator,
    CacheUtil, KoraError,
};

#[derive(Debug, Deserialize, ToSchema)]
pub struct TransferTransactionRequest {
    pub amount: u64,
    pub token: String,
    pub source: String,
    pub destination: String,
    /// Optional signer signer_key to ensure consistency across related RPC calls
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer_key: Option<String>,
    /// Use Light Token (compressed token) transfer instead of SPL.
    /// Requires `zk_compression_rpc_url` to be configured on the server.
    #[serde(default)]
    pub light_token: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TransferTransactionResponse {
    pub transaction: String,
    pub message: String,
    pub blockhash: String,
    /// Public key of the signer used (for client consistency)
    pub signer_pubkey: String,
}

pub async fn transfer_transaction(
    rpc_client: &Arc<RpcClient>,
    request: TransferTransactionRequest,
) -> Result<TransferTransactionResponse, KoraError> {
    let signer = get_request_signer_with_signer_key(request.signer_key.as_deref())?;
    let fee_payer = signer.pubkey();

    let validator = TransactionValidator::new(fee_payer)?;

    let source = Pubkey::from_str(&request.source)
        .map_err(|e| KoraError::ValidationError(format!("Invalid source address: {e}")))?;
    let destination = Pubkey::from_str(&request.destination)
        .map_err(|e| KoraError::ValidationError(format!("Invalid destination address: {e}")))?;
    let token_mint = Pubkey::from_str(&request.token)
        .map_err(|e| KoraError::ValidationError(format!("Invalid token address: {e}")))?;

    // Branch: Light Token (compressed token) transfer
    if request.light_token {
        return light_token_transfer(
            rpc_client,
            &signer,
            &fee_payer,
            &source,
            &destination,
            &token_mint,
            request.amount,
        )
        .await;
    }

    // manually check disallowed account because we're creating the message
    if validator.is_disallowed_account(&source) {
        return Err(KoraError::InvalidTransaction(format!(
            "Source account {source} is disallowed"
        )));
    }

    if validator.is_disallowed_account(&destination) {
        return Err(KoraError::InvalidTransaction(format!(
            "Destination account {destination} is disallowed"
        )));
    }

    let mut instructions = vec![];

    // Handle native SOL transfers
    if request.token == NATIVE_SOL {
        instructions.push(transfer(&source, &destination, request.amount));
    } else {
        // Handle wrapped SOL and other SPL tokens
        let token_mint = validator.fetch_and_validate_token_mint(&token_mint, rpc_client).await?;
        let token_program = token_mint.get_token_program();
        let decimals = token_mint.decimals();

        let source_ata = token_program.get_associated_token_address(&source, &token_mint.address());
        let dest_ata =
            token_program.get_associated_token_address(&destination, &token_mint.address());

        CacheUtil::get_account(rpc_client, &source_ata, false)
            .await
            .map_err(|_| KoraError::AccountNotFound(source_ata.to_string()))?;

        if CacheUtil::get_account(rpc_client, &dest_ata, false).await.is_err() {
            instructions.push(token_program.create_associated_token_account_instruction(
                &fee_payer,
                &destination,
                &token_mint.address(),
            ));
        }

        instructions.push(
            token_program
                .create_transfer_checked_instruction(
                    &source_ata,
                    &token_mint.address(),
                    &dest_ata,
                    &source,
                    request.amount,
                    decimals,
                )
                .map_err(|e| {
                    KoraError::InvalidTransaction(format!(
                        "Failed to create transfer instruction: {e}"
                    ))
                })?,
        );
    }

    let blockhash =
        rpc_client.get_latest_blockhash_with_commitment(CommitmentConfig::confirmed()).await?;

    let message = VersionedMessage::Legacy(Message::new_with_blockhash(
        &instructions,
        Some(&fee_payer),
        &blockhash.0,
    ));
    let transaction = TransactionUtil::new_unsigned_versioned_transaction(message);

    let mut resolved_transaction =
        VersionedTransactionResolved::from_kora_built_transaction(&transaction)?;

    // validate transaction before signing
    validator.validate_transaction(&mut resolved_transaction, rpc_client).await?;

    // Find the fee payer position in the account keys
    let fee_payer_position = resolved_transaction.find_signer_position(&fee_payer)?;

    let message_bytes = resolved_transaction.transaction.message.serialize();
    let signature = signer
        .sign_message(&message_bytes)
        .await
        .map_err(|e| KoraError::SigningError(e.to_string()))?;

    resolved_transaction.transaction.signatures[fee_payer_position] = signature;

    let encoded = resolved_transaction.encode_b64_transaction()?;
    let message_encoded = transaction.message.encode_b64_message()?;

    Ok(TransferTransactionResponse {
        transaction: encoded,
        message: message_encoded,
        blockhash: blockhash.0.to_string(),
        signer_pubkey: fee_payer.to_string(),
    })
}

// Handle a Light Token transfer with automatic hot/cold detection.
//
// Mirrors the SDK's `createTransferInterfaceInstructions` behavior:
// 1. Map NATIVE_SOL -> wSOL mint
// 2. Detect mint type (light-token vs SPL) for correct ATA derivation
// 3. Check hot (on-chain ATA) balance
// 4. If hot sufficient -> TransferChecked (light-token or SPL depending on mint)
// 5. Check cold (compressed) balance
// 6. If cold only -> Transfer2 with compressed inputs + proofs
// 7. If mixed -> decompress cold->hot, then transfer
async fn light_token_transfer(
    rpc_client: &Arc<RpcClient>,
    signer: &Arc<solana_keychain::Signer>,
    fee_payer: &Pubkey,
    source: &Pubkey,
    destination: &Pubkey,
    mint: &Pubkey,
    amount: u64,
) -> Result<TransferTransactionResponse, KoraError> {
    let config = get_config()?;
    let zk_rpc_url = config.kora.zk_compression_rpc_url.as_deref().ok_or_else(|| {
        KoraError::InvalidRequest(
            "zk_compression_rpc_url not configured — required for light_token transfers"
                .to_string(),
        )
    })?;
    let lut_override = config.kora.light_lut_address.as_deref();

    // 1. Map NATIVE_SOL -> wSOL mint
    let effective_mint = if *mint == Pubkey::from_str(NATIVE_SOL).unwrap_or_default() {
        Pubkey::from_str(WSOL_MINT).map_err(|e| {
            KoraError::InternalServerError(format!("Invalid wSOL mint constant: {e}"))
        })?
    } else {
        *mint
    };

    // 2. Get decimals from the SPL/Token-2022 mint (works for any standard mint)
    let decimals = TokenUtil::get_mint_decimals(rpc_client.as_ref(), &effective_mint).await?;

    // 3. Check hot balance using light-token ATA derivation (always when light_token: true)
    let source_ata = kora_light_client::get_associated_token_address(source, &effective_mint);
    let hot_balance = match CacheUtil::get_account(rpc_client, &source_ata, false).await {
        Ok(account) => {
            // Light-token ATAs are 272 bytes (not 165 like SPL), so Pack::unpack rejects them.
            // The token amount sits at offset 64 in the same SPL Account layout.
            if account.data.len() >= 72 {
                u64::from_le_bytes(account.data[64..72].try_into().unwrap_or_default())
            } else {
                0
            }
        }
        Err(_) => 0u64,
    };

    // 4. Hot balance sufficient -> transfer (fast path, no proofs needed)
    if hot_balance >= amount {
        log::debug!("Light Token hot path: hot_balance={hot_balance} >= amount={amount}");
        return hot_transfer(
            rpc_client,
            signer,
            fee_payer,
            source,
            destination,
            &effective_mint,
            amount,
            decimals,
            zk_rpc_url,
            lut_override,
        )
        .await;
    }

    // 5. Check cold (compressed) balance
    let light_rpc = LightRpcClient::new(zk_rpc_url);
    let compressed_accounts =
        light_rpc.get_compressed_token_accounts_by_owner(source, &effective_mint).await?;
    let cold_balance: u64 = compressed_accounts.iter().map(|a| a.token_data.amount).sum();
    let total = hot_balance.saturating_add(cold_balance);

    if total < amount {
        return Err(KoraError::InsufficientFunds(format!(
            "Insufficient Light Token balance: hot={hot_balance}, cold={cold_balance}, \
             total={total}, needed={amount}"
        )));
    }

    // 6. Pure cold -> Transfer2 with compressed inputs (no hot balance at all)
    if hot_balance == 0 {
        log::debug!("Light Token cold path: cold_balance={cold_balance}, using Transfer2");
        return cold_transfer(
            rpc_client,
            signer,
            fee_payer,
            source,
            destination,
            &effective_mint,
            amount,
            &compressed_accounts,
            &light_rpc,
            zk_rpc_url,
            lut_override,
        )
        .await;
    }

    // 7. Mixed: decompress cold->hot first, then transfer the full amount
    let shortfall = amount.saturating_sub(hot_balance);
    log::debug!(
        "Light Token mixed path: hot={hot_balance}, cold={cold_balance}, \
         loading {shortfall} from cold, then transfer of {amount}"
    );
    mixed_transfer(
        rpc_client,
        signer,
        fee_payer,
        source,
        destination,
        &effective_mint,
        amount,
        shortfall,
        &compressed_accounts,
        &light_rpc,
        zk_rpc_url,
        lut_override,
        decimals,
    )
    .await
}

// Hot path: source has sufficient balance in on-chain light-token ATA.
// Uses light-token TransferChecked (discriminator 12) with V0 + LUT.
#[allow(clippy::too_many_arguments)]
async fn hot_transfer(
    rpc_client: &Arc<RpcClient>,
    signer: &Arc<solana_keychain::Signer>,
    fee_payer: &Pubkey,
    source: &Pubkey,
    destination: &Pubkey,
    mint: &Pubkey,
    amount: u64,
    decimals: u8,
    zk_rpc_url: &str,
    lut_override: Option<&str>,
) -> Result<TransferTransactionResponse, KoraError> {
    let instructions = build_light_token_transfer_instructions(
        rpc_client,
        fee_payer,
        source,
        destination,
        mint,
        amount,
        decimals,
    )
    .await?;

    let (mut transaction, blockhash) = build_light_token_v0_transaction(
        rpc_client,
        fee_payer,
        &instructions,
        zk_rpc_url,
        lut_override,
    )
    .await?;

    sign_v0_and_build_response(signer, &mut transaction, fee_payer, blockhash).await
}

// Fetch validity proof and convert compressed accounts to kora-light-client types.
async fn fetch_and_convert_proof(
    light_rpc: &LightRpcClient,
    compressed_accounts: &[crate::light_token::types::CompressedTokenAccount],
    amount: u64,
) -> Result<
    (Vec<kora_light_client::CompressedTokenAccountInput>, kora_light_client::CompressedProof),
    KoraError,
> {
    let selected = select_input_accounts(compressed_accounts, amount)?;
    let hashes = conversions::extract_hashes_with_tree(&selected);
    let proof = light_rpc.get_validity_proof(&hashes).await?;
    let inputs = conversions::to_inputs(&selected, &proof)?;
    let compressed_proof = conversions::to_proof(&proof)?;
    Ok((inputs, compressed_proof))
}

// Build ATA-creation + Light Token TransferChecked instructions for a light-token mint.
// Uses kora-light-client's ATA derivation and TransferChecked (discriminator 12).
#[allow(clippy::too_many_arguments)]
async fn build_light_token_transfer_instructions(
    rpc_client: &Arc<RpcClient>,
    fee_payer: &Pubkey,
    source: &Pubkey,
    destination: &Pubkey,
    mint: &Pubkey,
    amount: u64,
    decimals: u8,
) -> Result<Vec<solana_sdk::instruction::Instruction>, KoraError> {
    let source_ata = kora_light_client::get_associated_token_address(source, mint);
    let dest_ata = kora_light_client::get_associated_token_address(destination, mint);

    let mut instructions = vec![];

    // Create destination ATA if needed (idempotent — no-op if it already exists)
    if CacheUtil::get_account(rpc_client, &dest_ata, false).await.is_err() {
        instructions.push(
            kora_light_client::create_ata_idempotent_instruction(fee_payer, destination, mint)
                .map_err(|e| {
                    KoraError::InvalidTransaction(format!(
                        "Failed to create light-token ATA instruction: {e}"
                    ))
                })?,
        );
    }

    // Light Token TransferChecked (discriminator 12)
    instructions.push(
        kora_light_client::create_transfer_checked_instruction(
            &source_ata,
            &dest_ata,
            mint,
            source,
            amount,
            decimals,
            fee_payer,
        )
        .map_err(|e| {
            KoraError::InvalidTransaction(format!(
                "Failed to create light-token transfer instruction: {e}"
            ))
        })?,
    );

    Ok(instructions)
}

// Sign a V0 transaction and build the response.
async fn sign_v0_and_build_response(
    signer: &Arc<solana_keychain::Signer>,
    transaction: &mut VersionedTransaction,
    fee_payer: &Pubkey,
    blockhash: solana_sdk::hash::Hash,
) -> Result<TransferTransactionResponse, KoraError> {
    let fee_payer_position = transaction
        .message
        .static_account_keys()
        .iter()
        .position(|k| k == fee_payer)
        .ok_or_else(|| {
            KoraError::InternalServerError("Fee payer not found in transaction keys".to_string())
        })?;

    let message_bytes = transaction.message.serialize();
    let signature = signer
        .sign_message(&message_bytes)
        .await
        .map_err(|e| KoraError::SigningError(e.to_string()))?;
    transaction.signatures[fee_payer_position] = signature;

    let encoded = TransactionUtil::encode_versioned_transaction(transaction)?;
    let message_encoded = transaction.message.encode_b64_message()?;

    Ok(TransferTransactionResponse {
        transaction: encoded,
        message: message_encoded,
        blockhash: blockhash.to_string(),
        signer_pubkey: fee_payer.to_string(),
    })
}

// Cold path: source has no hot balance, only compressed accounts.
// Uses Transfer2 via kora-light-client with compressed inputs and validity proofs.
#[allow(clippy::too_many_arguments)]
async fn cold_transfer(
    rpc_client: &Arc<RpcClient>,
    signer: &Arc<solana_keychain::Signer>,
    fee_payer: &Pubkey,
    source: &Pubkey,
    destination: &Pubkey,
    mint: &Pubkey,
    amount: u64,
    compressed_accounts: &[crate::light_token::types::CompressedTokenAccount],
    light_rpc: &LightRpcClient,
    zk_rpc_url: &str,
    lut_override: Option<&str>,
) -> Result<TransferTransactionResponse, KoraError> {
    let (inputs, compressed_proof) =
        fetch_and_convert_proof(light_rpc, compressed_accounts, amount).await?;

    let instruction = kora_light_client::create_transfer2_instruction(
        fee_payer,
        source,
        mint,
        &inputs,
        &compressed_proof,
        destination,
        amount,
    )
    .map_err(|e| KoraError::InvalidTransaction(format!("Light Token transfer error: {e}")))?;

    let (mut transaction, blockhash) = build_light_token_v0_transaction(
        rpc_client,
        fee_payer,
        &[instruction],
        zk_rpc_url,
        lut_override,
    )
    .await?;

    sign_v0_and_build_response(signer, &mut transaction, fee_payer, blockhash).await
}

// Mixed path: source has some hot balance + cold balance.
// Decompresses cold accounts into the source's light-token ATA, then
// transfers the full amount via light-token TransferChecked.
#[allow(clippy::too_many_arguments)]
async fn mixed_transfer(
    rpc_client: &Arc<RpcClient>,
    signer: &Arc<solana_keychain::Signer>,
    fee_payer: &Pubkey,
    source: &Pubkey,
    destination: &Pubkey,
    mint: &Pubkey,
    amount: u64,
    shortfall: u64,
    compressed_accounts: &[crate::light_token::types::CompressedTokenAccount],
    light_rpc: &LightRpcClient,
    zk_rpc_url: &str,
    lut_override: Option<&str>,
    decimals: u8,
) -> Result<TransferTransactionResponse, KoraError> {
    let (inputs, compressed_proof) =
        fetch_and_convert_proof(light_rpc, compressed_accounts, shortfall).await?;

    // Derive source ATA using light-token derivation
    let source_ata = kora_light_client::get_associated_token_address(source, mint);

    // Build decompress instruction (cold -> source light-token ATA)
    let decompress_ix = kora_light_client::create_decompress_instruction(
        fee_payer,
        source,
        mint,
        &inputs,
        &compressed_proof,
        &source_ata,
        shortfall,
        decimals,
        None,
    )
    .map_err(|e| KoraError::InvalidTransaction(format!("Decompress error: {e}")))?;

    // Build transfer instructions using light-token program
    let mut transfer_instructions = build_light_token_transfer_instructions(
        rpc_client,
        fee_payer,
        source,
        destination,
        mint,
        amount,
        decimals,
    )
    .await?;

    let mut instructions = vec![decompress_ix];
    instructions.append(&mut transfer_instructions);

    let (mut transaction, blockhash) = build_light_token_v0_transaction(
        rpc_client,
        fee_payer,
        &instructions,
        zk_rpc_url,
        lut_override,
    )
    .await?;

    sign_v0_and_build_response(signer, &mut transaction, fee_payer, blockhash).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        state::update_config,
        tests::{
            common::{setup_or_get_test_signer, RpcMockBuilder},
            config_mock::ConfigMockBuilder,
        },
    };

    #[tokio::test]
    async fn test_transfer_transaction_invalid_source() {
        let config = ConfigMockBuilder::new().build();
        update_config(config).unwrap();
        let _ = setup_or_get_test_signer();

        let rpc_client = Arc::new(RpcMockBuilder::new().with_mint_account(6).build());

        let request = TransferTransactionRequest {
            amount: 1000,
            token: Pubkey::new_unique().to_string(),
            source: "invalid".to_string(),
            destination: Pubkey::new_unique().to_string(),
            signer_key: None,
            light_token: false,
        };

        let result = transfer_transaction(&rpc_client, request).await;

        assert!(result.is_err(), "Should fail with invalid source address");
        let error = result.unwrap_err();
        assert!(matches!(error, KoraError::ValidationError(_)), "Should return ValidationError");
        match error {
            KoraError::ValidationError(error_message) => {
                assert!(error_message.contains("Invalid source address"));
            }
            _ => panic!("Should return ValidationError"),
        }
    }

    #[tokio::test]
    async fn test_transfer_transaction_invalid_destination() {
        let config = ConfigMockBuilder::new().build();
        update_config(config).unwrap();
        let _ = setup_or_get_test_signer();

        let rpc_client = Arc::new(RpcMockBuilder::new().with_mint_account(6).build());

        let request = TransferTransactionRequest {
            amount: 1000,
            token: Pubkey::new_unique().to_string(),
            source: Pubkey::new_unique().to_string(),
            destination: "invalid_pubkey".to_string(),
            signer_key: None,
            light_token: false,
        };

        let result = transfer_transaction(&rpc_client, request).await;

        assert!(result.is_err(), "Should fail with invalid destination address");
        let error = result.unwrap_err();
        match error {
            KoraError::ValidationError(error_message) => {
                assert!(error_message.contains("Invalid destination address"));
            }
            _ => panic!("Should return ValidationError"),
        }
    }

    #[tokio::test]
    async fn test_transfer_transaction_invalid_token() {
        let config = ConfigMockBuilder::new().build();
        update_config(config).unwrap();
        let _ = setup_or_get_test_signer();

        let rpc_client = Arc::new(RpcMockBuilder::new().with_mint_account(6).build());

        let request = TransferTransactionRequest {
            amount: 1000,
            token: "invalid_token_address".to_string(),
            source: Pubkey::new_unique().to_string(),
            destination: Pubkey::new_unique().to_string(),
            signer_key: None,
            light_token: false,
        };

        let result = transfer_transaction(&rpc_client, request).await;

        assert!(result.is_err(), "Should fail with invalid token address");
        let error = result.unwrap_err();
        match error {
            KoraError::ValidationError(error_message) => {
                assert!(error_message.contains("Invalid token address"));
            }
            _ => panic!("Should return ValidationError"),
        }
    }

    #[tokio::test]
    async fn test_light_token_without_zk_rpc_url() {
        // Config without zk_compression_rpc_url should fail with clear error
        let config = ConfigMockBuilder::new().build();
        update_config(config).unwrap();
        let _ = setup_or_get_test_signer();

        let rpc_client = Arc::new(RpcMockBuilder::new().with_mint_account(6).build());

        let request = TransferTransactionRequest {
            amount: 1000,
            token: Pubkey::new_unique().to_string(),
            source: Pubkey::new_unique().to_string(),
            destination: Pubkey::new_unique().to_string(),
            signer_key: None,
            light_token: true,
        };

        let result = transfer_transaction(&rpc_client, request).await;

        assert!(result.is_err(), "Should fail without zk_compression_rpc_url");
        let error = result.unwrap_err();
        match error {
            KoraError::InvalidRequest(msg) => {
                assert!(msg.contains("zk_compression_rpc_url"));
            }
            other => panic!("Expected InvalidRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_light_token_false_uses_spl_path() {
        // light_token=false should fall through to the normal SPL path,
        // not hit the light token code path. We verify by using an invalid
        // source address — the error should NOT be about zk_compression_rpc_url.
        let config = ConfigMockBuilder::new().build();
        update_config(config).unwrap();
        let _ = setup_or_get_test_signer();

        let rpc_client = Arc::new(RpcMockBuilder::new().with_mint_account(6).build());

        let request = TransferTransactionRequest {
            amount: 1000,
            token: Pubkey::new_unique().to_string(),
            source: "invalid".to_string(),
            destination: Pubkey::new_unique().to_string(),
            signer_key: None,
            light_token: false,
        };

        let result = transfer_transaction(&rpc_client, request).await;
        assert!(result.is_err());
        let error = result.unwrap_err();
        // Crucially, the error should NOT mention zk_compression_rpc_url,
        // proving we hit the SPL path, not the light token path.
        let error_msg = format!("{error}");
        assert!(
            !error_msg.contains("zk_compression_rpc_url"),
            "light_token=false should not enter the Light Token path, but got: {error_msg}"
        );
    }
}
