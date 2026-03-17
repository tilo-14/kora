use crate::common::*;
use jsonrpsee::rpc_params;
use kora_lib::transaction::TransactionUtil;
use solana_sdk::transaction::VersionedTransaction;

/// Helper: get the Light Token mint from env (set by light-setup.ts)
fn get_light_token_mint() -> String {
    std::env::var("LIGHT_TOKEN_MINT").expect("LIGHT_TOKEN_MINT env var not set")
}

/// Helper: call transferTransaction with light_token: true
async fn light_transfer(
    ctx: &TestContext,
    amount: u64,
    source: &str,
    destination: &str,
) -> serde_json::Value {
    let mint = get_light_token_mint();
    // Pass as a single JSON object with named fields
    let params = serde_json::json!({
        "amount": amount,
        "token": mint,
        "source": source,
        "destination": destination,
        "light_token": true
    });
    ctx.rpc_call("transferTransaction", rpc_params![params])
        .await
        .expect("transferTransaction failed")
}

/// Helper: extract discriminators from a base64 V0 transaction
fn extract_discriminators(base64_tx: &str) -> Vec<u8> {
    let tx =
        TransactionUtil::decode_b64_transaction(base64_tx).expect("Failed to decode transaction");
    tx.message.instructions().iter().filter(|ix| !ix.data.is_empty()).map(|ix| ix.data[0]).collect()
}

const TRANSFER_CHECKED_DISC: u8 = 12;
const TRANSFER2_DISC: u8 = 101;

// --- Transfer path tests ---

/// Hot path: sender has sufficient on-chain Light Token ATA balance.
/// Expected: TransferChecked (disc 12), no Transfer2.
#[tokio::test]
async fn test_transfer_light_token_hot() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let recipient = RecipientTestHelper::get_recipient_pubkey();

    // Transfer 1 token (hot balance should be 10 tokens from setup)
    let response =
        light_transfer(&ctx, 1_000_000, &sender.pubkey().to_string(), &recipient.to_string()).await;
    response.assert_success();

    let tx_str = response["transaction"].as_str().expect("Expected transaction");
    assert!(response["message"].as_str().is_some(), "Expected message");
    assert!(response["blockhash"].as_str().is_some(), "Expected blockhash");

    let discs = extract_discriminators(tx_str);
    assert!(
        discs.contains(&TRANSFER_CHECKED_DISC),
        "Hot path should use TransferChecked (disc 12)"
    );
    assert!(!discs.contains(&TRANSFER2_DISC), "Hot path should not use Transfer2 (disc 101)");
}

/// Cold path: transfer more than hot balance so Kora uses compressed accounts.
/// This test assumes hot balance has been partially depleted by previous tests,
/// or the amount exceeds hot balance.
/// Expected: Transfer2 (disc 101).
#[tokio::test]
async fn test_transfer_light_token_cold() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let recipient = RecipientTestHelper::get_recipient_pubkey();

    // Transfer amount larger than hot balance to force cold path
    // Setup provisions 10 tokens hot + 5 tokens cold = 15 total
    // If we transfer 14 tokens, mixed path kicks in (hot insufficient alone)
    // For pure cold, we'd need hot=0 — this test may need sequential execution
    // For now, test that a large transfer succeeds and uses Transfer2
    let response = light_transfer(
        &ctx,
        14_000_000, // 14 tokens — exceeds 10 hot, needs cold
        &sender.pubkey().to_string(),
        &recipient.to_string(),
    )
    .await;
    response.assert_success();

    let tx_str = response["transaction"].as_str().expect("Expected transaction");
    let discs = extract_discriminators(tx_str);
    // Should have Transfer2 (disc 101) for the decompress/cold transfer portion
    assert!(
        discs.contains(&TRANSFER2_DISC),
        "Transfer exceeding hot balance should use Transfer2 (disc 101), got: {:?}",
        discs
    );
}

/// Test transfer with automatic ATA creation for a new destination.
#[tokio::test]
async fn test_transfer_light_token_with_ata_creation() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let random_dest = solana_sdk::signature::Keypair::new().pubkey();

    let response = light_transfer(
        &ctx,
        100_000, // 0.1 token
        &sender.pubkey().to_string(),
        &random_dest.to_string(),
    )
    .await;
    response.assert_success();

    let tx_str = response["transaction"].as_str().expect("Expected transaction");
    // Transaction should be decodable and simulatable
    let _tx =
        TransactionUtil::decode_b64_transaction(tx_str).expect("Failed to decode transaction");
}

// --- Error case tests ---

/// Insufficient balance: request more than total (hot + cold).
#[tokio::test]
async fn test_transfer_light_token_insufficient_balance() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let recipient = RecipientTestHelper::get_recipient_pubkey();
    let mint = get_light_token_mint();

    // Request way more than available (setup has 15 tokens total)
    let params = serde_json::json!({
        "amount": 100_000_000_000u64, // 100,000 tokens
        "token": mint,
        "source": sender.pubkey().to_string(),
        "destination": recipient.to_string(),
        "light_token": true
    });

    let result: Result<serde_json::Value, _> =
        ctx.rpc_call("transferTransaction", rpc_params![params]).await;

    assert!(result.is_err(), "Should fail with insufficient balance");
}

/// Invalid mint address.
#[tokio::test]
async fn test_transfer_light_token_invalid_mint() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let recipient = RecipientTestHelper::get_recipient_pubkey();

    let params = serde_json::json!({
        "amount": 1_000_000,
        "token": "invalid_mint_address",
        "source": sender.pubkey().to_string(),
        "destination": recipient.to_string(),
        "light_token": true
    });

    let result: Result<serde_json::Value, _> =
        ctx.rpc_call("transferTransaction", rpc_params![params]).await;

    assert!(result.is_err(), "Should fail with invalid mint address");
}

/// Verify that light_token: false uses the SPL path (not Light Token).
#[tokio::test]
async fn test_transfer_light_token_false_uses_spl_path() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let recipient = RecipientTestHelper::get_recipient_pubkey();
    let mint = get_light_token_mint();

    let params = serde_json::json!({
        "amount": 1_000_000,
        "token": mint,
        "source": sender.pubkey().to_string(),
        "destination": recipient.to_string(),
        "light_token": false
    });

    // This may fail (no SPL ATA for sender) but should NOT fail with
    // "zk_compression_rpc_url" error — proving it hit the SPL path
    let result: Result<serde_json::Value, _> =
        ctx.rpc_call("transferTransaction", rpc_params![params]).await;

    if let Err(e) = &result {
        let err_msg = format!("{e}");
        assert!(
            !err_msg.contains("zk_compression_rpc_url"),
            "light_token: false should not enter Light Token path"
        );
    }
}
