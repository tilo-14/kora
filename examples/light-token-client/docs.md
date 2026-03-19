# Client-side Light Token transfers with Kora: technical reference

This example demonstrates gasless Light Token transfers where the client builds transactions using `@lightprotocol/compressed-token` and Kora sponsors the fees via `signTransaction`.

The server requires config changes only — add Light Protocol programs to `allowed_programs`. The client talks to ZK compression RPC directly for compressed account state and validity proofs.

## What was added

### Server configuration

**`demo/server/kora.toml`** — Kora config with three Light Protocol programs added to `allowed_programs` (required for Light Token transactions):

```
cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m   Light Token Program
SySTEM1eSU2p4BGQfQpimFEWWSC1XDFeun3Nqzz3rT7   Light System Program
compr6CUsB5m2jS4Y3831ztGSTnDpnKJTKS95d64XVq   Account Compression Program
```

No `zk_compression_rpc_url` or `light_lut_address` — the server does not call ZK compression RPC.

Fee payer policy is restricted to minimum required permissions:
- `system.allow_transfer = true` — needed for Light Token rent top-ups
- `system.allow_create_account = true` — needed for ATA creation
- `spl_token.allow_initialize_account = true` — needed for ATA initialization
- All other permissions set to `false`

**`demo/server/signers.toml`** — Memory signer using `KORA_PRIVATE_KEY` env var.

### Client code

**`demo/client/src/basic-transfer.ts`** — Main transfer flow:

1. `kora.getPayerSigner()` — get Kora's fee payer pubkey
2. `getAssociatedTokenAddressInterface(mint, destination)` — derive destination Light Token ATA
3. `createTransferInterfaceInstructions(rpc, koraFeePayer, mint, amount, sender, destinationAta, decimals)` — Light Protocol SDK builds transfer instructions client-side. Accepts `payer: PublicKey` (not `Signer`), so Kora's pubkey works directly. The SDK handles compressed account fetching, validity proof retrieval, and account selection internally.
4. Prepend `createAtaInterfaceIdempotentInstruction(koraFeePayer, destinationAta, destination, mint, LIGHT_TOKEN_PROGRAM_ID)` to ensure destination ATA exists (no-op if already created)
5. Loop through all instruction batches (each batch = one V0 transaction). Almost always one batch, but handles fragmented balances:
   - `buildV0Transaction()` — compile V0 message (no lookup table needed; v3 packed-account layout fits in standard messages)
   - `tx.sign([sender])` — partial sign with sender keypair
   - `kora.signTransaction()` — Kora validates allowed programs + signs as fee payer
   - `connection.sendRawTransaction()` + `confirmTransaction()` — broadcast

**`demo/client/src/helpers.ts`** — Shared utilities: `getEnvOrThrow()`, `keypairFromEnv()`, and `buildV0Transaction()`.

**`demo/client/src/devnet-setup.ts`** — Provisions test state on devnet:
- Creates SPL mint, registers with Light Token Program via `createSplInterface()`
- Creates sender + destination Light Token ATAs via `createAtaInterfaceIdempotent()`
- Wraps 10 tokens to sender ATA (hot state) via `wrap()`
- Compresses 5 tokens to sender (cold state) via `compress()`
- Updates `kora.toml` with actual mint address
- Saves keypairs to `.env`

### Dependencies

```
@lightprotocol/compressed-token    0.23.0-beta.10   Transfer instructions, ATA creation
@lightprotocol/stateless.js        0.23.0-beta.10   ZK compression RPC client
@solana/kora                       link:../../../../sdks/ts   Kora RPC client
@solana/spl-token                  ^0.4.14          SPL mint creation (createMint, mintTo, ATA helpers) in devnet-setup
@solana/web3.js                    ^1.98.0          Connection, VersionedTransaction
```

## What the fee payer actually pays

| Cost | Amount | Mechanism |
| ---- | ------ | --------- |
| Transaction fee | ~5,000 lamports | Solana network fee |
| ATA creation (if needed) | ~17,400 lamports | CPI from Light Token Program; `RENT_SPONSOR_V1` PDA covers rent-exemption, payer covers compression incentive |
| Per-write top-up | ~766 lamports | Bumps virtual rent balance to keep account active |

Total per transaction: ~6,000 lamports (transfer only) to ~23,000 lamports (with ATA creation).

`max_allowed_lamports` in `kora.toml` only validates System/SPL/Token-2022 instruction outflows. Light Token CPI outflows bypass this check because the validator doesn't parse unknown program instructions.

## Validation flow (signTransaction)

When a client sends a V0 transaction to `signTransaction`:

1. **Decode** — base64 → `VersionedTransaction`
2. **Resolve** — V0 lookup table addresses resolved (if any)
3. **Validate programs** — every instruction's `program_id` checked against `allowed_programs`
4. **Validate fee payer policy** — fee payer usage in System/SPL/Token-2022 checked against policy
5. **Sign** — Kora adds fee payer signature
6. **Return** — fully signed transaction

Light Token instructions pass step 3 because the three program IDs are in `allowed_programs`. Step 4 only checks System/SPL/Token-2022 instructions — Light Token instructions are not inspected beyond the program ID allowlist.

## Security model

Kora validates Light Token transactions through two mechanisms:

1. **Program allowlist** — every instruction's `program_id` is checked against `allowed_programs`. Only the three Light Protocol program IDs listed above pass.
2. **Fee payer policy** — System and SPL Token instructions are checked against granular policy flags. The example config permits only `CreateAccount` and `InitializeAccount` (required for ATA creation).

Light Token CPI outflows (~17,400 lamports for ATA creation, ~766 per write) are not inspected by Kora's `max_allowed_lamports` validator, which only tracks System/SPL/Token-2022 instruction outflows. Operators should set `max_allowed_lamports` high enough to cover these costs and monitor fee payer balance.

## Multi-batch handling

`createTransferInterfaceInstructions` returns `TransactionInstruction[][]`. Each inner array is one transaction. The example loops through all batches, sending each as a separate V0 transaction to Kora. In practice, almost always returns one batch. Multiple batches occur when the sender's compressed balance is fragmented across many accounts.

Pattern follows the [Light Token integration guide](https://github.com/Lightprotocol/examples-light-token).

## V0 without lookup table

Light Protocol v3 uses a packed-account layout in instruction data that reduces the number of unique accounts per instruction. V0 messages compile without an address lookup table. The `9NYFyEqPeWQHiS8Jv4VjZcjKBMPRCJ3KbEbaBcy4Mza` LUT referenced in `kora-light-client` is not needed for v3 instructions, which use a packed-account layout that fits in standard V0 messages.

## Verified on devnet

Confirmed transactions:
- `4LeH7xe8Z74PTHCVY13SWoVqtLmFDxCQnMKupckSWKqD28iKZhQHPQ6ecCezJPBTRqoC2rch6CshWtvqYyDD7HhC` (1 token, initial test)
- `4ugP4aDRWgY3wtLT7dFXKbFpgfXbGso4b5oBmTb2VcfEgVS8sdSsHE3XS8VmXKHNMkzyQdZ5u3JhV2LHm4DdupN5` (1 token, with idempotent ATA + multi-batch support)
- `58dsvdeMALoaiQrFq3fpvzQdPbE9fR7Qk9tVornbGzx7EgzRc838w4RueBTKKcap4jHaRDtbdWrHHcdayjmX2euC` (2 tokens, configurable amount)
