use serde::{Deserialize, Deserializer};

// ZK compression RPC returns amounts as either JSON numbers or strings.
fn deserialize_flexible_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    match &value {
        serde_json::Value::Number(n) => {
            n.as_u64().ok_or_else(|| serde::de::Error::custom("number out of u64 range"))
        }
        serde_json::Value::String(s) => s
            .parse::<u64>()
            .map_err(|e| serde::de::Error::custom(format!("invalid u64 string: {e}"))),
        _ => Err(serde::de::Error::custom(format!("expected number or string, got {value}"))),
    }
}

/// Compressed token account as returned by the ZK compression RPC
#[derive(Debug, Clone, Deserialize)]
pub struct CompressedTokenAccount {
    pub account: CompressedAccount,
    #[serde(rename = "tokenData")]
    pub token_data: TokenData,
}

/// Compressed account metadata as returned by Helius/Photon RPC
#[derive(Debug, Clone, Deserialize)]
pub struct CompressedAccount {
    /// The hash of this compressed account (BN254 encoded string)
    pub hash: String,
    /// Lamports held by this compressed account
    #[serde(deserialize_with = "deserialize_flexible_u64")]
    pub lamports: u64,
    /// Owner program of the account
    pub owner: String,
    /// Leaf index in the Merkle tree
    #[serde(rename = "leafIndex")]
    pub leaf_index: u32,
    /// Merkle tree public key (flat field from Helius response)
    pub tree: String,
    /// Additional fields returned by Helius (ignored but must be accepted)
    #[serde(default)]
    pub address: Option<serde_json::Value>,
    /// Account data containing discriminator (token data version)
    #[serde(default)]
    pub data: Option<CompressedAccountData>,
    #[serde(default)]
    pub seq: Option<serde_json::Value>,
    #[serde(rename = "slotCreated", default)]
    pub slot_created: Option<u64>,
}

/// Account data from the compressed account response
#[derive(Debug, Clone, Deserialize)]
pub struct CompressedAccountData {
    /// Token data version discriminator (2 = V1, 3 = V2, 4 = ShaFlat)
    pub discriminator: u8,
    /// Base64-encoded account data
    #[serde(default)]
    pub data: Option<String>,
    /// Data hash
    #[serde(rename = "dataHash", default)]
    pub data_hash: Option<String>,
}

impl CompressedAccount {
    /// Get the token data version from the account data discriminator.
    /// Discriminator 2 → V1 (version 1), 3 → V2 (version 2), 4 → ShaFlat (version 3).
    /// Defaults to 1 (V1) if no data field is present.
    pub fn token_data_version(&self) -> u8 {
        match &self.data {
            Some(d) => match d.discriminator {
                2 => 1, // V1
                3 => 2, // V2
                4 => 3, // ShaFlat
                _ => 1, // default to V1
            },
            None => 1,
        }
    }
}

/// Token data within a compressed token account
#[derive(Debug, Clone, Deserialize)]
pub struct TokenData {
    /// Token mint public key
    pub mint: String,
    /// Token owner public key
    pub owner: String,
    /// Token amount
    #[serde(deserialize_with = "deserialize_flexible_u64")]
    pub amount: u64,
    /// Delegate public key (optional)
    pub delegate: Option<String>,
    /// Account state
    pub state: String,
}

/// Validity proof V2 response from the ZK compression RPC
#[derive(Debug, Clone, Deserialize)]
pub struct ValidityProofResponse {
    /// Compressed ZK proof (null when all accounts use proveByIndex)
    #[serde(rename = "compressedProof")]
    pub compressed_proof: Option<CompressedProof>,
    /// Per-account proof metadata
    pub accounts: Vec<ProofAccount>,
    /// Per-address proof metadata (empty for token transfers)
    #[serde(default)]
    pub addresses: Vec<serde_json::Value>,
}

/// Per-account data in the V2 validity proof response
#[derive(Debug, Clone, Deserialize)]
pub struct ProofAccount {
    pub hash: String,
    pub root: String,
    #[serde(rename = "rootIndex")]
    pub root_index: RootIndex,
    #[serde(rename = "leafIndex")]
    pub leaf_index: u32,
    #[serde(rename = "merkleContext")]
    pub merkle_context: ProofMerkleContext,
}

/// Root index with proveByIndex flag
#[derive(Debug, Clone, Deserialize)]
pub struct RootIndex {
    #[serde(rename = "rootIndex")]
    pub root_index: u32,
    #[serde(rename = "proveByIndex")]
    pub prove_by_index: bool,
}

/// Merkle context from the V2 validity proof response
#[derive(Debug, Clone, Deserialize)]
pub struct ProofMerkleContext {
    pub tree: String,
    pub queue: String,
    #[serde(rename = "treeType", default)]
    pub tree_type: Option<u8>,
}

/// Compressed ZK proof (Groth16)
#[derive(Debug, Clone, Deserialize)]
pub struct CompressedProof {
    /// Proof element a (32 bytes)
    pub a: Vec<u8>,
    /// Proof element b (64 bytes)
    pub b: Vec<u8>,
    /// Proof element c (32 bytes)
    pub c: Vec<u8>,
}

/// A hash paired with its tree and queue, used for validity proof requests
#[derive(Debug, Clone)]
pub struct HashWithTree {
    pub hash: String,
    pub tree: String,
    pub queue: String,
}

/// Generic JSON-RPC response wrapper
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse<T> {
    pub result: Option<RpcResult<T>>,
    pub error: Option<JsonRpcError>,
}

/// RPC result with context
#[derive(Debug, Clone, Deserialize)]
pub struct RpcResult<T> {
    pub value: T,
}

/// Items with cursor for paginated responses
#[derive(Debug, Clone, Deserialize)]
pub struct ItemsWithCursor<T> {
    pub items: Vec<T>,
    pub cursor: Option<String>,
}

/// JSON-RPC error
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}
