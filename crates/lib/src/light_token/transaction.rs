use solana_address_lookup_table_interface::state::AddressLookupTable;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_message::{v0, AddressLookupTableAccount, VersionedMessage};
use solana_sdk::{
    hash::Hash, instruction::Instruction, pubkey::Pubkey, transaction::VersionedTransaction,
};
use std::sync::Arc;

use crate::{error::KoraError, transaction::TransactionUtil, CacheUtil};
use std::str::FromStr;

use super::constants::{LIGHT_LUT_DEVNET, LIGHT_LUT_MAINNET};

/// Detect whether an RPC URL points to devnet.
///
/// Simple heuristic: if the URL contains "devnet", use devnet LUT;
/// otherwise default to mainnet.
fn detect_lut_address(rpc_url: &str) -> Pubkey {
    if rpc_url.contains("devnet") {
        LIGHT_LUT_DEVNET
    } else if rpc_url.contains("localhost") || rpc_url.contains("127.0.0.1") {
        // Local validator — use devnet LUT as a reasonable default
        LIGHT_LUT_DEVNET
    } else {
        LIGHT_LUT_MAINNET
    }
}

/// Build a V0 versioned transaction with Light Protocol's address lookup table.
///
/// This fetches the lookup table from the Solana RPC, compiles a V0 message
/// referencing it, and returns an unsigned VersionedTransaction.
///
/// If `lut_override` is provided, it is parsed as a Pubkey and used instead of
/// the automatic `detect_lut_address()` heuristic.
pub async fn build_light_token_v0_transaction(
    rpc_client: &Arc<RpcClient>,
    fee_payer: &Pubkey,
    instructions: &[Instruction],
    rpc_url: &str,
    lut_override: Option<&str>,
) -> Result<(VersionedTransaction, Hash), KoraError> {
    let lut_address = match lut_override {
        Some(addr) => Pubkey::from_str(addr).map_err(|e| {
            KoraError::InvalidRequest(format!("Invalid light_lut_address '{addr}': {e}"))
        })?,
        None => detect_lut_address(rpc_url),
    };
    log::info!("Light Token: using LUT address {lut_address}");

    // Fetch the lookup table account
    let lut_account =
        CacheUtil::get_account(rpc_client, &lut_address, false).await.map_err(|e| {
            KoraError::RpcError(format!(
                "Failed to fetch Light Protocol lookup table {lut_address}: {e}"
            ))
        })?;

    let lut = AddressLookupTable::deserialize(&lut_account.data).map_err(|e| {
        KoraError::InvalidTransaction(format!(
            "Failed to deserialize Light Protocol lookup table: {e}"
        ))
    })?;

    // Get recent blockhash
    let (blockhash, _) =
        rpc_client.get_latest_blockhash_with_commitment(CommitmentConfig::confirmed()).await?;

    // Use Solana SDK's try_compile which handles V0 message construction correctly
    let address_lookup_table_accounts =
        vec![AddressLookupTableAccount { key: lut_address, addresses: lut.addresses.to_vec() }];

    let v0_message = v0::Message::try_compile(
        fee_payer,
        instructions,
        &address_lookup_table_accounts,
        blockhash,
    )
    .map_err(|e| KoraError::InvalidTransaction(format!("Failed to compile V0 message: {e}")))?;

    let message = VersionedMessage::V0(v0_message);
    let transaction = TransactionUtil::new_unsigned_versioned_transaction(message);

    Ok((transaction, blockhash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_lut_devnet() {
        let lut = detect_lut_address("https://api.devnet.solana.com");
        assert_eq!(lut, LIGHT_LUT_DEVNET);
    }

    #[test]
    fn test_detect_lut_mainnet() {
        let lut = detect_lut_address("https://api.mainnet-beta.solana.com");
        assert_eq!(lut, LIGHT_LUT_MAINNET);
    }

    #[test]
    fn test_detect_lut_localhost() {
        let lut = detect_lut_address("http://127.0.0.1:8899");
        assert_eq!(lut, LIGHT_LUT_DEVNET);
    }
}
