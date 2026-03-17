use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

// Re-export program IDs from kora-light-client (canonical source of truth)
pub use kora_light_client::program_ids::{
    ACCOUNT_COMPRESSION_AUTHORITY_PDA, ACCOUNT_COMPRESSION_PROGRAM_ID, CPI_AUTHORITY_PDA,
    LIGHT_SYSTEM_PROGRAM_ID, LIGHT_TOKEN_PROGRAM_ID, NOOP_PROGRAM_ID, REGISTERED_PROGRAM_PDA,
    TRANSFER2_DISCRIMINATOR,
};

/// Wrapped SOL mint address (string form for comparison in transfer routing)
pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// CPI authority seed used for PDA derivation from the Light Token Program
pub const CPI_AUTHORITY_SEED: &[u8] = b"cpi_authority";

/// Light Protocol lookup table address for mainnet
pub fn light_lut_mainnet() -> Pubkey {
    Pubkey::from_str("9NYFyEqPkyXUhkerbGHXUXkvb4qpzeEdHuGpgbgpH1NJ").unwrap()
}

/// Light Protocol lookup table address for devnet
pub fn light_lut_devnet() -> Pubkey {
    Pubkey::from_str("qAJZMgnQJ8G6vA3WRcjD9Jan1wtKkaCFWLWskxJrR5V").unwrap()
}
