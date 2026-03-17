#!/usr/bin/env bash
# Light Token integration test orchestrator.
#
# Starts `light test-validator`, provisions Light Token state,
# starts Kora, runs Rust integration tests, then cleans up.
#
# Usage: make integration-test-light
# Requires: light CLI (npm i -g @lightprotocol/zk-compression-cli)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
KORA_PORT=8089
VALIDATOR_RPC_PORT=8899
INDEXER_PORT=8784
KORA_CONFIG="$PROJECT_ROOT/tests/src/common/fixtures/light-token-test.toml"
SIGNERS_CONFIG="$PROJECT_ROOT/tests/src/common/fixtures/signers.toml"
LOCAL_KEYS_DIR="$PROJECT_ROOT/tests/src/common/local-keys"

# PIDs for cleanup
LIGHT_VALIDATOR_PID=""
KORA_PID=""

cleanup() {
    echo "Cleaning up..."
    [ -n "$KORA_PID" ] && kill "$KORA_PID" 2>/dev/null || true
    [ -n "$LIGHT_VALIDATOR_PID" ] && kill "$LIGHT_VALIDATOR_PID" 2>/dev/null || true
    # Kill any orphaned processes on our ports
    lsof -ti:$KORA_PORT 2>/dev/null | xargs kill 2>/dev/null || true
    lsof -ti:$VALIDATOR_RPC_PORT 2>/dev/null | xargs kill 2>/dev/null || true
    wait 2>/dev/null || true
}
trap cleanup EXIT

echo "=== Light Token Integration Tests ==="

# 1. Check prerequisites
if ! command -v light &>/dev/null; then
    echo "ERROR: 'light' CLI not found. Install with: npm i -g @lightprotocol/zk-compression-cli"
    exit 1
fi

# 2. Start Light Protocol test validator
echo "Starting light test-validator..."
light test-validator \
    --rpc-port $VALIDATOR_RPC_PORT \
    --skip-prover \
    --limit-ledger-size 10000 &
LIGHT_VALIDATOR_PID=$!

# Wait for validator to be ready
echo "Waiting for validator..."
for i in $(seq 1 30); do
    if curl -s http://127.0.0.1:$VALIDATOR_RPC_PORT -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' 2>/dev/null | grep -q '"result"'; then
        echo "Validator ready (attempt $i)"
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "ERROR: Validator did not start within 30s"
        exit 1
    fi
    sleep 1
done

# 3. Airdrop SOL to test accounts
echo "Airdropping SOL..."
FEE_PAYER_PUBKEY=$(solana-keygen pubkey "$LOCAL_KEYS_DIR/fee-payer-local.json" 2>/dev/null || echo "")
SENDER_PUBKEY=$(solana-keygen pubkey "$LOCAL_KEYS_DIR/sender-local.json" 2>/dev/null || echo "")

if [ -n "$FEE_PAYER_PUBKEY" ]; then
    solana airdrop 10 "$FEE_PAYER_PUBKEY" --url http://127.0.0.1:$VALIDATOR_RPC_PORT 2>/dev/null || true
fi
if [ -n "$SENDER_PUBKEY" ]; then
    solana airdrop 10 "$SENDER_PUBKEY" --url http://127.0.0.1:$VALIDATOR_RPC_PORT 2>/dev/null || true
fi

# 4. Provision Light Token state
echo "Provisioning Light Token state..."
cd "$SCRIPT_DIR"
if [ ! -d "node_modules" ]; then
    pnpm install --frozen-lockfile 2>/dev/null || pnpm install
fi
LIGHT_SETUP_OUTPUT=$(npx tsx light-setup.ts 2>&1)
echo "$LIGHT_SETUP_OUTPUT"

# Extract env vars from setup output (lines matching KEY=VALUE)
eval "$(echo "$LIGHT_SETUP_OUTPUT" | grep -E '^[A-Z_]+=.+$')"

# 5. Start Kora
echo "Starting Kora on port $KORA_PORT..."
cd "$PROJECT_ROOT"
cargo run -p kora --bin kora -- \
    --config "$KORA_CONFIG" \
    --rpc-url "http://127.0.0.1:$VALIDATOR_RPC_PORT" \
    rpc start \
    --signers-config "$SIGNERS_CONFIG" \
    --port "$KORA_PORT" &
KORA_PID=$!

# Wait for Kora to be ready
echo "Waiting for Kora..."
for i in $(seq 1 30); do
    if curl -s "http://127.0.0.1:$KORA_PORT" -X POST \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","id":1,"method":"getConfig","params":{}}' 2>/dev/null | grep -q '"result"'; then
        echo "Kora ready (attempt $i)"
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "ERROR: Kora did not start within 30s"
        exit 1
    fi
    sleep 1
done

# 6. Run tests
echo "Running Light Token integration tests..."
TEST_SERVER_URL="http://127.0.0.1:$KORA_PORT" \
    RPC_URL="http://127.0.0.1:$VALIDATOR_RPC_PORT" \
    LIGHT_TOKEN_MINT="${LIGHT_TOKEN_MINT:-}" \
    cargo test -p tests --test light_token -- --nocapture

TEST_EXIT=$?

echo ""
if [ $TEST_EXIT -eq 0 ]; then
    echo "=== All Light Token tests passed ==="
else
    echo "=== Light Token tests FAILED ==="
fi

exit $TEST_EXIT
