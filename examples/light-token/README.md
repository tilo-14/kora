## Gasless Light Token transfers with Kora

This example shows how to send gasless compressed token (Light Token) transfers using Kora as a paymaster.

### How it works

Unlike SPL token transfers where Kora builds the transaction server-side via `transferTransaction()`, Light Token transfers use a **client-side construction** pattern:

1. Your app builds the compressed transfer transaction using the Light Protocol SDK
2. Your app signs the transaction with the user's keypair
3. Your app submits the pre-built transaction to Kora's `signAndSendTransaction`
4. Kora validates, co-signs (as fee payer), and broadcasts to the Solana network

### Prerequisites

- A running Kora server with Light Protocol programs in the allowlist (see `demo/server/kora.toml`)
- A ZK compression-enabled RPC endpoint (e.g., [Helius](https://helius.dev))
- The sender must hold compressed tokens for the target mint

### Quick start

```bash
cd demo/client
pnpm install
pnpm run start        # Minimal example
pnpm run full-demo    # Full 6-step walkthrough
```

### Kora configuration

Add these programs to `allowed_programs` in your `kora.toml`:

```toml
# Light Protocol programs
"cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m"   # Light Token Program
"SySTEM1eSU2p4BGQfQpimFEWWSC1XDFeun3Nqzz3rT7"   # Light System Program
"compr6CUsB5m2jS4Y3831ztGSTnDpnKJTKS95d64XVq"   # Account Compression Program
```

And allow the Light Token Program to reference the fee payer as writable (needed for rent top-ups):

```toml
[validation.fee_payer_policy]
allow_fee_payer_writable_in_programs = [
    "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m"
]
```
