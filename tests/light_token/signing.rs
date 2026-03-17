use crate::common::*;
use jsonrpsee::rpc_params;
use kora_lib::transaction::TransactionUtil;
use solana_sdk::{
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};

fn get_light_token_mint() -> String {
    std::env::var("LIGHT_TOKEN_MINT").expect("LIGHT_TOKEN_MINT env var not set")
}

/// Sign a Light Token transfer transaction built by Kora.
/// Flow: transferTransaction → sender signs → signTransaction
#[tokio::test]
async fn test_sign_light_token_transaction() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let recipient = RecipientTestHelper::get_recipient_pubkey();
    let mint = get_light_token_mint();

    // 1. Get the transfer transaction from Kora
    let transfer_params = serde_json::json!({
        "amount": 500_000, // 0.5 token
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

    // 2. Sender signs the transaction
    let tx_base64 = transfer_response["transaction"].as_str().unwrap();
    let mut tx =
        TransactionUtil::decode_b64_transaction(tx_base64).expect("Failed to decode transaction");
    tx.try_partial_sign(&[&sender], *tx.message.recent_blockhash()).expect("Failed to sign");
    let signed_base64 =
        TransactionUtil::encode_versioned_transaction(&tx).expect("Failed to encode");

    // 3. Kora co-signs
    let sign_params = serde_json::json!({
        "transaction": signed_base64
    });
    let sign_response: serde_json::Value = ctx
        .rpc_call("signTransaction", rpc_params![sign_params])
        .await
        .expect("signTransaction failed");

    assert!(
        sign_response["signed_transaction"].as_str().is_some(),
        "Expected signed_transaction in response"
    );
    assert!(
        sign_response["signer_pubkey"].as_str().is_some(),
        "Expected signer_pubkey in response"
    );
}

/// Sign and send a Light Token transfer in one step.
/// Flow: transferTransaction → sender signs → signAndSendTransaction
#[tokio::test]
async fn test_sign_and_send_light_token_transaction() {
    let ctx = TestContext::new().await.expect("Failed to create test context");
    let sender = SenderTestHelper::get_test_sender_keypair();
    let recipient = RecipientTestHelper::get_recipient_pubkey();
    let mint = get_light_token_mint();

    // 1. Get the transfer transaction
    let transfer_params = serde_json::json!({
        "amount": 500_000, // 0.5 token
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

    // 2. Sender signs
    let tx_base64 = transfer_response["transaction"].as_str().unwrap();
    let mut tx =
        TransactionUtil::decode_b64_transaction(tx_base64).expect("Failed to decode transaction");
    tx.try_partial_sign(&[&sender], *tx.message.recent_blockhash()).expect("Failed to sign");
    let signed_base64 =
        TransactionUtil::encode_versioned_transaction(&tx).expect("Failed to encode");

    // 3. Kora co-signs and sends
    let send_params = serde_json::json!({
        "transaction": signed_base64
    });
    let send_response: serde_json::Value = ctx
        .rpc_call("signAndSendTransaction", rpc_params![send_params])
        .await
        .expect("signAndSendTransaction failed");

    assert!(
        send_response["signature"].as_str().is_some()
            || send_response["signed_transaction"].as_str().is_some(),
        "Expected signature or signed_transaction in response"
    );
}
