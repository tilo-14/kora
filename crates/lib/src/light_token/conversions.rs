//! Type conversions between Kora RPC types and kora-light-client types.
//!
//! This module bridges the gap between Kora's JSON-RPC response types
//! (which use strings and serde_json::Value) and kora-light-client's
//! typed instruction builder inputs.

use kora_light_client::{CompressedProof as LightCompressedProof, CompressedTokenAccountInput};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::error::KoraError;

use super::types::{CompressedTokenAccount, HashWithTree, ValidityProofResponse};

/// Extract hash-with-tree metadata from compressed accounts for validity proof requests.
pub fn extract_hashes_with_tree(accounts: &[CompressedTokenAccount]) -> Vec<HashWithTree> {
    accounts
        .iter()
        .map(|acct| HashWithTree {
            hash: acct.account.hash.clone(),
            tree: acct.account.tree.clone(),
            // queue is not returned by getCompressedTokenAccountsByOwner; use tree as
            // placeholder. In to_inputs(), the actual queue comes from the validity proof
            // response's merkle_context — this fallback is only used if hash matching fails.
            queue: acct.account.tree.clone(),
        })
        .collect()
}

/// Parse a BN254 hash string (base58-encoded) into a 32-byte array.
pub fn parse_bn254_hash(hash_str: &str) -> Result<[u8; 32], KoraError> {
    let bytes = bs58::decode(hash_str).into_vec().map_err(|e| {
        KoraError::InvalidTransaction(format!("Invalid BN254 hash '{hash_str}': {e}"))
    })?;
    let arr: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
        KoraError::InvalidTransaction(format!(
            "BN254 hash has wrong length: expected 32, got {}",
            v.len()
        ))
    })?;
    Ok(arr)
}

/// Convert compressed accounts + V2 validity proof into kora-light-client inputs.
///
/// Uses the proof response's per-account merkle context for tree/queue (accurate),
/// falling back to the compressed account's tree field if no matching proof account.
pub fn to_inputs(
    accounts: &[CompressedTokenAccount],
    proof: &ValidityProofResponse,
) -> Result<Vec<CompressedTokenAccountInput>, KoraError> {
    accounts
        .iter()
        .enumerate()
        .map(|(i, acct)| {
            let hash = parse_bn254_hash(&acct.account.hash)?;

            // Match proof account by hash to get tree, queue, root_index
            let proof_acct = proof.accounts.iter().find(|pa| pa.hash == acct.account.hash);

            let (tree, queue, root_index, prove_by_index) = match proof_acct {
                Some(pa) => {
                    let tree = Pubkey::from_str(&pa.merkle_context.tree).map_err(|e| {
                        KoraError::InvalidTransaction(format!("Invalid tree pubkey: {e}"))
                    })?;
                    let queue = Pubkey::from_str(&pa.merkle_context.queue).map_err(|e| {
                        KoraError::InvalidTransaction(format!("Invalid queue pubkey: {e}"))
                    })?;
                    (tree, queue, pa.root_index.root_index as u16, pa.root_index.prove_by_index)
                }
                None => {
                    // Fallback: use tree from compressed account, no queue
                    let tree = Pubkey::from_str(&acct.account.tree).map_err(|e| {
                        KoraError::InvalidTransaction(format!("Invalid tree pubkey: {e}"))
                    })?;
                    log::warn!(
                        "No matching proof account for hash {} at index {i}, using fallback",
                        acct.account.hash
                    );
                    (tree, tree, 0u16, false)
                }
            };

            let owner = Pubkey::from_str(&acct.token_data.owner)
                .map_err(|e| KoraError::InvalidTransaction(format!("Invalid owner pubkey: {e}")))?;
            let mint = Pubkey::from_str(&acct.token_data.mint)
                .map_err(|e| KoraError::InvalidTransaction(format!("Invalid mint pubkey: {e}")))?;
            let delegate = acct
                .token_data
                .delegate
                .as_ref()
                .map(|d| Pubkey::from_str(d))
                .transpose()
                .map_err(|e| {
                    KoraError::InvalidTransaction(format!("Invalid delegate pubkey: {e}"))
                })?;

            Ok(CompressedTokenAccountInput {
                hash,
                tree,
                queue,
                amount: acct.token_data.amount,
                leaf_index: acct.account.leaf_index,
                prove_by_index,
                root_index,
                version: acct.account.token_data_version(),
                owner,
                mint,
                delegate,
            })
        })
        .collect()
}

/// Convert a V2 `ValidityProofResponse` to kora-light-client's `CompressedProof`.
///
/// When `compressedProof` is null (all accounts use proveByIndex), returns a zeroed proof.
pub fn to_proof(proof: &ValidityProofResponse) -> Result<LightCompressedProof, KoraError> {
    match &proof.compressed_proof {
        Some(cp) => {
            let a: [u8; 32] = cp.a.as_slice().try_into().map_err(|_| {
                KoraError::InvalidTransaction(format!(
                    "Proof element 'a' wrong length: expected 32, got {}",
                    cp.a.len()
                ))
            })?;
            let b: [u8; 64] = cp.b.as_slice().try_into().map_err(|_| {
                KoraError::InvalidTransaction(format!(
                    "Proof element 'b' wrong length: expected 64, got {}",
                    cp.b.len()
                ))
            })?;
            let c: [u8; 32] = cp.c.as_slice().try_into().map_err(|_| {
                KoraError::InvalidTransaction(format!(
                    "Proof element 'c' wrong length: expected 32, got {}",
                    cp.c.len()
                ))
            })?;
            Ok(LightCompressedProof { a, b, c })
        }
        None => {
            // All accounts use proveByIndex — no ZK proof needed
            Ok(LightCompressedProof { a: [0u8; 32], b: [0u8; 64], c: [0u8; 32] })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light_token::types::{
        CompressedAccount, CompressedProof, ProofAccount, ProofMerkleContext, RootIndex, TokenData,
    };

    fn make_rpc_account(amount: u64) -> CompressedTokenAccount {
        let tree = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let hash_bytes = [42u8; 32];
        let hash_str = bs58::encode(&hash_bytes).into_string();

        CompressedTokenAccount {
            account: CompressedAccount {
                hash: hash_str,
                lamports: 0,
                owner: "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m".to_string(),
                leaf_index: 7,
                tree: tree.to_string(),
                address: None,
                data: None,
                seq: None,
                slot_created: None,
            },
            token_data: TokenData {
                mint: mint.to_string(),
                owner: owner.to_string(),
                amount,
                delegate: None,
                state: "initialized".to_string(),
            },
        }
    }

    fn make_proof_response(accounts: &[CompressedTokenAccount]) -> ValidityProofResponse {
        let proof_accounts: Vec<ProofAccount> = accounts
            .iter()
            .map(|a| {
                let queue = Pubkey::new_unique();
                ProofAccount {
                    hash: a.account.hash.clone(),
                    root: "11111111111111111111111111111111".to_string(),
                    root_index: RootIndex { root_index: 5, prove_by_index: false },
                    leaf_index: a.account.leaf_index,
                    merkle_context: ProofMerkleContext {
                        tree: a.account.tree.clone(),
                        queue: queue.to_string(),
                        tree_type: Some(1),
                    },
                }
            })
            .collect();

        ValidityProofResponse {
            compressed_proof: Some(CompressedProof {
                a: vec![1u8; 32],
                b: vec![2u8; 64],
                c: vec![3u8; 32],
            }),
            accounts: proof_accounts,
            addresses: vec![],
        }
    }

    #[test]
    fn test_parse_bn254_hash_valid() {
        let bytes = [1u8; 32];
        let encoded = bs58::encode(&bytes).into_string();
        let result = parse_bn254_hash(&encoded).unwrap();
        assert_eq!(result, bytes);
    }

    #[test]
    fn test_parse_bn254_hash_wrong_length() {
        let bytes = [1u8; 16];
        let encoded = bs58::encode(&bytes).into_string();
        let result = parse_bn254_hash(&encoded);
        assert!(result.is_err());
    }

    #[test]
    fn test_to_inputs_basic() {
        let accounts = vec![make_rpc_account(1000)];
        let proof = make_proof_response(&accounts);
        let inputs = to_inputs(&accounts, &proof).unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].amount, 1000);
        assert_eq!(inputs[0].root_index, 5);
        assert_eq!(inputs[0].leaf_index, 7);
        assert!(inputs[0].delegate.is_none());
    }

    #[test]
    fn test_to_inputs_batch() {
        let accounts = vec![make_rpc_account(500), make_rpc_account(300)];
        let proof = make_proof_response(&accounts);
        let inputs = to_inputs(&accounts, &proof).unwrap();
        assert_eq!(inputs.len(), 2);
    }

    #[test]
    fn test_to_proof_valid() {
        let accounts = vec![make_rpc_account(100)];
        let proof_resp = make_proof_response(&accounts);
        let proof = to_proof(&proof_resp).unwrap();
        assert_eq!(proof.a, [1u8; 32]);
        assert_eq!(proof.b, [2u8; 64]);
        assert_eq!(proof.c, [3u8; 32]);
    }

    #[test]
    fn test_to_proof_null_prove_by_index() {
        let proof_resp =
            ValidityProofResponse { compressed_proof: None, accounts: vec![], addresses: vec![] };
        let proof = to_proof(&proof_resp).unwrap();
        assert_eq!(proof.a, [0u8; 32]);
    }

    fn root_indices(proof: &ValidityProofResponse) -> Vec<u16> {
        proof.accounts.iter().map(|a| a.root_index.root_index as u16).collect()
    }

    #[test]
    fn test_root_indices() {
        let accounts = vec![make_rpc_account(100)];
        let proof = make_proof_response(&accounts);
        let indices = root_indices(&proof);
        assert_eq!(indices, vec![5u16]);
    }
}
