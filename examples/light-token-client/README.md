# Kora + Light Token (client-side)

Kora handles fee sponsorship. You build transactions with Light Token instructions client-side and Kora signs them as fee payer:

1. Build Light Token transfer instructions using `@lightprotocol/compressed-token`
2. Assemble V0 transaction with Kora's fee payer address
3. Sign with sender keypair
4. Send to Kora for fee-payer co-signing
5. Broadcast to Solana

Kora does not need Light Protocol SDKs or a ZK compression RPC endpoint.
The client talks to ZK compression RPC directly.

## What you will implement

| | Server (Kora) | Client |
| --- | --- | --- |
| **Configuration** | Add Light Protocol programs to `allowed_programs` | Set `ZK_COMPRESSION_RPC_URL` |
| **Build transfer** | — | `createTransferInterfaceInstructions()` |
| **Create ATA** | — | `createAtaInterfaceIdempotentInstruction()` |
| **Sign as fee payer** | `signTransaction` | — |
| **Broadcast** | — | `sendRawTransaction()` |

### Source files

- **[quick-start.ts](demo/client/src/quick-start.ts)** — Build a Light Token transfer client-side, send to Kora for fee sponsorship. Handles multi-batch transfers.
- **[helpers.ts](demo/client/src/helpers.ts)** — Shared utilities: env helpers, keypair loading, V0 transaction assembly.
- **[devnet-setup.ts](demo/client/src/devnet-setup.ts)** — Create SPL mint, register with Light Token Program, wrap and compress tokens on devnet.
- **[test-transfer.ts](demo/client/src/test-transfer.ts)** — E2E test covering hot, cold, and mixed transfer paths.

## Server configuration

Add Light Protocol programs to `allowed_programs` in `kora.toml`:

```toml
allowed_programs = [
    # ... existing programs ...
    "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m",  # Light Token Program
    "SySTEM1eSU2p4BGQfQpimFEWWSC1XDFeun3Nqzz3rT7",  # Light System Program
    "compr6CUsB5m2jS4Y3831ztGSTnDpnKJTKS95d64XVq",  # Account Compression Program
]
```

No `zk_compression_rpc_url` or `light_lut_address` required on the server.

### What the Kora operator needs

1. **Add Light Protocol programs to `allowed_programs`** in `kora.toml`:
   ```toml
   allowed_programs = [
       # ... existing programs ...
       "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m",  # Light Token Program
       "SySTEM1eSU2p4BGQfQpimFEWWSC1XDFeun3Nqzz3rT7",  # Light System Program
       "compr6CUsB5m2jS4Y3831ztGSTnDpnKJTKS95d64XVq",  # Account Compression Program
   ]
   ```

2. **Set fee payer policy** — enable only the two permissions Light Token transactions require:
   ```toml
   [validation.fee_payer_policy.system]
   allow_create_account = true    # ATA creation

   [validation.fee_payer_policy.spl_token]
   allow_initialize_account = true    # ATA initialization
   ```
   All other fee payer policy permissions should remain `false`.

3. **Fund the fee payer** — each transfer costs ~6,000 lamports (transfer only) to ~23,000 lamports (with ATA creation). Light Token CPI outflows (~17,400 lamports for ATA creation, ~766 per write) are not tracked by `max_allowed_lamports`.

4. **No Light Protocol SDKs or ZK compression RPC needed on the server** — the client talks to ZK compression RPC directly.

## Setup

```bash
cd demo/client
pnpm install
cp ../.env.example ../.env
# Fill in ZK_COMPRESSION_RPC_URL (Helius devnet endpoint)
```

### Environment variables

| Variable | Description |
| -------- | ----------- |
| `ZK_COMPRESSION_RPC_URL` | Helius or Triton RPC endpoint with ZK compression support. |
| `SOLANA_RPC_URL` | Solana RPC (defaults to `ZK_COMPRESSION_RPC_URL`). |
| `KORA_RPC_URL` | Kora JSON-RPC endpoint (default `http://localhost:8081/`). |
| `TEST_SENDER_KEYPAIR` | Base58-encoded sender keypair. Created by `devnet-setup`. |
| `DESTINATION_KEYPAIR` | Base58-encoded destination keypair. Created by `devnet-setup`. |
| `KORA_PRIVATE_KEY` | Fee payer keypair used by Kora server. Created by `devnet-setup`. |
| `LIGHT_TOKEN_MINT` | SPL mint address registered with Light Token Program. Created by `devnet-setup`. |

## Quick start

```bash
# 1. Create test mint, Light Token accounts, wrap + compress tokens
pnpm run devnet-setup

# 2. Start Kora server (in separate terminal)
cd ../../.. && cargo run -p kora-cli --bin kora -- \
  --config examples/light-token-client/demo/server/kora.toml \
  --rpc-url "$SOLANA_RPC_URL" \
  rpc start --signers-config examples/light-token-client/demo/server/signers.toml --port 8081

# 3. Transfer 1 token (default)
pnpm start

# 4. Transfer custom amount
TRANSFER_AMOUNT=5000000 pnpm start

# 5. Run E2E test (hot/cold/mixed paths)
pnpm run test-transfer
```

### Notes

- Validity proofs are slot-bounded. Submit transactions to Kora promptly after building.
- `createTransferInterfaceInstructions` returns `TransactionInstruction[][]`. Each inner array is one transaction. Almost always returns just one. The example handles multi-batch automatically.
