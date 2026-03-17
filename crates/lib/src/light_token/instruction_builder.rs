use crate::error::KoraError;

use super::types::CompressedTokenAccount;

/// Maximum number of compressed accounts per instruction.
/// Matches kora-light-client's limit to prevent exceeding transaction account limits.
const MAX_INPUT_ACCOUNTS: usize = 8;

/// Select compressed token accounts to satisfy the requested transfer amount.
///
/// Uses a greedy algorithm: sorts by descending amount and picks up to
/// `MAX_INPUT_ACCOUNTS` (8) accounts until the cumulative sum meets or
/// exceeds `amount`.
pub fn select_input_accounts(
    accounts: &[CompressedTokenAccount],
    amount: u64,
) -> Result<Vec<CompressedTokenAccount>, KoraError> {
    if accounts.is_empty() {
        return Err(KoraError::InsufficientFunds(
            "No compressed token accounts found for this owner and mint".to_string(),
        ));
    }

    let mut with_amounts: Vec<(u64, &CompressedTokenAccount)> =
        accounts.iter().map(|acct| (acct.token_data.amount, acct)).collect();

    with_amounts.sort_by(|a, b| b.0.cmp(&a.0));

    let mut selected = vec![];
    let mut cumulative: u64 = 0;

    for (amt, acct) in with_amounts.iter().take(MAX_INPUT_ACCOUNTS) {
        selected.push((*acct).clone());
        cumulative = cumulative.saturating_add(*amt);
        if cumulative >= amount {
            return Ok(selected);
        }
    }

    Err(KoraError::InsufficientFunds(format!(
        "Insufficient compressed token balance: need {amount}, have {cumulative}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light_token::types::{CompressedAccount, TokenData};
    use solana_sdk::pubkey::Pubkey;

    fn make_test_account(amount: u64) -> CompressedTokenAccount {
        CompressedTokenAccount {
            account: CompressedAccount {
                hash: "test_hash".to_string(),
                lamports: 0,
                owner: "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m".to_string(),
                leaf_index: 0,
                tree: Pubkey::new_unique().to_string(),
                address: None,
                data: None,
                seq: None,
                slot_created: None,
            },
            token_data: TokenData {
                mint: Pubkey::new_unique().to_string(),
                owner: Pubkey::new_unique().to_string(),
                amount,
                delegate: None,
                state: "initialized".to_string(),
            },
        }
    }

    #[test]
    fn test_select_input_accounts_sufficient_single() {
        let accounts = vec![make_test_account(1000)];
        let selected = select_input_accounts(&accounts, 500).unwrap();
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn test_select_input_accounts_sufficient_multiple() {
        let accounts = vec![make_test_account(300), make_test_account(500), make_test_account(200)];
        let selected = select_input_accounts(&accounts, 700).unwrap();
        // Sorted descending: 500, 300, 200. Need 700, so 500 + 300 = 800 >= 700
        assert_eq!(selected.len(), 2);
    }

    #[test]
    fn test_select_input_accounts_insufficient() {
        let accounts = vec![make_test_account(100)];
        let result = select_input_accounts(&accounts, 500);
        assert!(result.is_err());
        match result.unwrap_err() {
            KoraError::InsufficientFunds(msg) => {
                assert!(msg.contains("need 500"));
                assert!(msg.contains("have 100"));
            }
            other => panic!("Expected InsufficientFunds, got {other:?}"),
        }
    }

    #[test]
    fn test_select_input_accounts_empty() {
        let result = select_input_accounts(&[], 100);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), KoraError::InsufficientFunds(_)));
    }

    #[test]
    fn test_select_input_accounts_max_cap() {
        // 10 accounts of 100 each = 1000 total, but MAX_INPUT_ACCOUNTS = 8
        let accounts: Vec<_> = (0..10).map(|_| make_test_account(100)).collect();
        // Requesting 800 should succeed (8 * 100 = 800)
        let selected = select_input_accounts(&accounts, 800).unwrap();
        assert_eq!(selected.len(), 8);

        // Requesting 900 should fail (only 8 accounts considered = 800 < 900)
        let result = select_input_accounts(&accounts, 900);
        assert!(result.is_err());
        match result.unwrap_err() {
            KoraError::InsufficientFunds(msg) => {
                assert!(msg.contains("need 900"));
                assert!(msg.contains("have 800"));
            }
            other => panic!("Expected InsufficientFunds, got {other:?}"),
        }
    }
}
