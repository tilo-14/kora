// Light Token Integration Tests
//
// CONFIG: Uses tests/src/common/fixtures/light-token-test.toml
// REQUIRES: `light test-validator` running (Light Protocol programs + indexer)
// RUN: make integration-test-light
//
// TESTS: Light Token transfers via transferTransaction with light_token: true
//        - Hot path (on-chain balance sufficient)
//        - Cold path (only compressed balance)
//        - Mixed path (hot + cold)
//        - ATA creation for new destinations
//        - Error cases (insufficient balance, missing config, invalid mint)
//        - Sign and send flows
//        - Fee estimation

mod fee_estimation;
mod signing;
mod transfers;

// Make common utilities available
#[path = "../src/common/mod.rs"]
mod common;
