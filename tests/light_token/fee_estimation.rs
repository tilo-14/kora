use crate::common::*;
use jsonrpsee::rpc_params;
use kora_lib::transaction::TransactionUtil;
use solana_sdk::signature::Signer;

fn get_light_token_mint() -> String {
    std::env::var("LIGHT_TOKEN_MINT").expect("LIGHT_TOKEN_MINT env var not set")
}

/// Estimate fee for a Light Token transfer (V0 transaction with LUT).
#[tokio::test]
async fn test_estimate_light_token_transfer_fee() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let recipient = RecipientTestHelper::get_recipient_pubkey();
    let mint = get_light_token_mint();

    // 1. Build a Light Token transfer transaction
    let transfer_params = serde_json::json!({
        "amount": 1_000_000,
        "token": mint,
        "source": sender.pubkey().to_string(),
        "destination": recipient.to_string(),
        "light_token": true
    });
    let transfer_response: serde_json::Value = ctx
        .rpc_call("transferTransaction", rpc_params![transfer_params])
        .await
        .expect("transferTransaction failed");
    transfer_response.assert_success();

    let tx_base64 = transfer_response["transaction"].as_str().unwrap();

    // 2. Estimate the fee
    let fee_params = serde_json::json!({
        "transaction": tx_base64,
        "fee_token": mint
    });
    let fee_response: serde_json::Value = ctx
        .rpc_call("estimateTransactionFee", rpc_params![fee_params])
        .await
        .expect("estimateTransactionFee failed");

    assert!(fee_response["fee_in_lamports"].as_u64().is_some(), "Expected fee_in_lamports");
    assert!(fee_response["signer_pubkey"].as_str().is_some(), "Expected signer_pubkey");
}
