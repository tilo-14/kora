use solana_sdk::pubkey::Pubkey;

use crate::error::KoraError;

use super::types::{
    CompressedTokenAccount, HashWithTree, ItemsWithCursor, JsonRpcResponse, RpcResult,
    ValidityProofResponse,
};

/// HTTP JSON-RPC client for ZK compression methods.
///
/// Communicates with a ZK compression RPC endpoint (e.g., Photon indexer)
/// to fetch compressed token accounts and validity proofs.
pub struct LightRpcClient {
    client: reqwest::Client,
    endpoint: String,
}

impl LightRpcClient {
    /// Create a new LightRpcClient targeting the given ZK compression RPC endpoint
    pub fn new(endpoint: &str) -> Self {
        Self { client: reqwest::Client::new(), endpoint: endpoint.to_string() }
    }

    /// Fetch compressed token accounts owned by `owner` for the given `mint`
    pub async fn get_compressed_token_accounts_by_owner(
        &self,
        owner: &Pubkey,
        mint: &Pubkey,
    ) -> Result<Vec<CompressedTokenAccount>, KoraError> {
        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getCompressedTokenAccountsByOwner",
            "params": {
                "owner": owner.to_string(),
                "mint": mint.to_string()
            }
        });

        let response =
            self.client.post(&self.endpoint).json(&request_body).send().await.map_err(|e| {
                KoraError::RpcError(format!(
                    "Failed to call getCompressedTokenAccountsByOwner: {e}"
                ))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(KoraError::RpcError(format!(
                "ZK compression RPC returned status {status}: {body}"
            )));
        }

        let rpc_response: JsonRpcResponse<ItemsWithCursor<CompressedTokenAccount>> =
            response.json().await.map_err(|e| {
                KoraError::RpcError(format!(
                    "Failed to parse getCompressedTokenAccountsByOwner response: {e}"
                ))
            })?;

        if let Some(error) = rpc_response.error {
            return Err(KoraError::RpcError(format!(
                "ZK compression RPC error ({}): {}",
                error.code, error.message
            )));
        }

        match rpc_response.result {
            Some(RpcResult { value: items_cursor }) => Ok(items_cursor.items),
            None => Err(KoraError::RpcError(
                "Empty result from getCompressedTokenAccountsByOwner".to_string(),
            )),
        }
    }

    /// Fetch a validity proof for the given compressed account hashes
    pub async fn get_validity_proof(
        &self,
        hashes: &[HashWithTree],
    ) -> Result<ValidityProofResponse, KoraError> {
        // Use getValidityProof (V0 is deprecated on Helius).
        // The new API takes hashes as plain base58 strings, not objects.
        let hash_strings: Vec<&str> = hashes.iter().map(|h| h.hash.as_str()).collect();

        let request_body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getValidityProofV2",
            "params": {
                "hashes": hash_strings,
                "newAddressesWithTrees": []
            }
        });

        let response =
            self.client.post(&self.endpoint).json(&request_body).send().await.map_err(|e| {
                KoraError::RpcError(format!("Failed to call getValidityProof: {e}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(KoraError::RpcError(format!(
                "ZK compression RPC returned status {status}: {body}"
            )));
        }

        let rpc_response: JsonRpcResponse<ValidityProofResponse> =
            response.json().await.map_err(|e| {
                KoraError::RpcError(format!("Failed to parse getValidityProof response: {e}"))
            })?;

        if let Some(error) = rpc_response.error {
            return Err(KoraError::RpcError(format!(
                "ZK compression RPC error ({}): {}",
                error.code, error.message
            )));
        }

        match rpc_response.result {
            Some(RpcResult { value: proof }) => Ok(proof),
            None => Err(KoraError::RpcError("Empty result from getValidityProof".to_string())),
        }
    }
}
