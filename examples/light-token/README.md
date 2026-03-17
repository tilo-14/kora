## Gasless Light Token transfers with Kora

Light Token reduces token account creation cost by 200x compared to SPL, with sponsored rent-exemption. The light-token API matches the SPL-token API almost entirely. Your users hold the same stablecoins, just stored more efficiently.

### How it works

Add `light_token: true` to Kora's `transferTransaction` request. Kora detects the sender's balance across hot (on-chain) and cold (auto-compressed) state, builds the transfer, and returns a signed V0 transaction. The client signs and submits as usual.

Light Token accounts auto-compress inactive balances. With `light_token: true`, Kora detects and loads cold balances server-side, so the client flow stays the same.

| Path | When | What Kora does |
|:-----|:-----|:---------------|
| **Hot** | On-chain balance covers the amount | `TransferChecked` from sender's Light Token associated token account |
| **Cold** | All balance is compressed | `Transfer2` with compressed inputs and validity proofs |
| **Mixed** | On-chain + compressed covers the amount | `Decompress` shortfall, then `TransferChecked` — single transaction |

All paths produce V0 transactions referencing the Light Protocol address lookup table.

### API change

One new optional field on `transferTransaction`:

```typescript
const { transaction } = await client.transferTransaction({
  amount: 1_000_000,
  token: mintAddress,
  source: senderPubkey,
  destination: destinationPubkey,
  light_token: true, // ← new
});
```

The response shape is unchanged. Kora returns a base64-encoded V0 transaction, message, blockhash, and signer pubkey — same as SPL transfers.

### Server configuration

Add to `kora.toml`:

```toml
[kora]
# Required: ZK compression-enabled RPC (e.g., Helius)
zk_compression_rpc_url = "https://devnet.helius-rpc.com?api-key=YOUR_KEY"

# Optional: override Light Protocol lookup table (auto-detected if omitted)
# light_lut_address = "9NYFyEqPeWQHiS8Jv4VjZcjKBMPRCJ3KbEbaBcy4Mza"
```

Add Light Protocol programs to `allowed_programs`:

```toml
[validation]
allowed_programs = [
    # ... existing programs ...
    "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m",  # Light Token Program
    "SySTEM1eSU2p4BGQfQpimFEWWSC1XDFeun3Nqzz3rT7",  # Light System Program
    "compr6CUsB5m2jS4Y3831ztGSTnDpnKJTKS95d64XVq",  # Account Compression Program
]
```

Allow the Light Token Program to reference the fee payer as writable (needed for rent top-ups):

```toml
[validation.fee_payer_policy]
allow_fee_payer_writable_in_programs = [
    "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m",  # Light Token Program
    "SySTEM1eSU2p4BGQfQpimFEWWSC1XDFeun3Nqzz3rT7",  # Light System Program
]
```

### Prerequisites

- Running Kora server with the configuration above
- A ZK compression-enabled RPC endpoint (e.g., [Helius](https://helius.dev))
- Sender holds Light Token balance (hot, cold, or both)

### Examples

```bash
cd demo/client
pnpm install
pnpm run start        # Minimal transfer example
pnpm run full-demo    # Full walkthrough with payment instruction
```

| File | Description |
|:-----|:------------|
| `demo/client/src/quick-start.ts` | Minimal transfer: build, sign, submit |
| `demo/client/src/full-demo.ts` | 6-step walkthrough with payment instruction |
| `demo/client/src/test-server-transfer.ts` | E2E test covering hot, cold, and mixed paths |
| `demo/client/src/devnet-setup.ts` | Create mint, Light Token accounts, and compressed tokens on devnet |
