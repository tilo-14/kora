/**
 * E2E test for client-side Light Token transfers with Kora fee sponsorship.
 *
 * Tests all three transfer paths against a running Kora server on devnet:
 *   1. HOT  — sufficient on-chain Light Token ATA balance → TransferChecked (disc 12)
 *   2. MIXED — hot < amount but hot+cold >= amount → decompress + TransferChecked
 *   3. COLD  — zero hot balance → Transfer2 (disc 101) with validity proof
 *
 * Unlike the server-side test (which uses kora.transferTransaction({ light_token: true })),
 * this test builds instructions client-side via createTransferInterfaceInstructions(),
 * builds a V0 transaction, signs with the sender, then sends to Kora for co-signing.
 *
 * The tests run sequentially and each one changes the sender's balance,
 * so the order matters.
 *
 * Prerequisites:
 * - Run devnet-setup.ts first to create mint + fund sender
 * - Start Kora server with examples/light-token-client/demo/server/kora.toml
 * - Kora's fee payer must have SOL on devnet
 */
import { KoraClient } from "@solana/kora";
import {
  Keypair,
  PublicKey,
  Connection,
  VersionedTransaction,
} from "@solana/web3.js";
import { createRpc, Rpc } from "@lightprotocol/stateless.js";
import { createTransferInterfaceInstructions } from "@lightprotocol/compressed-token";
import {
  getAssociatedTokenAddressInterface,
  getAtaInterface,
  createAtaInterfaceIdempotentInstruction,
} from "@lightprotocol/compressed-token/unified";
import dotenv from "dotenv";
import path from "path";
import { buildV0Transaction, getEnvOrThrow, keypairFromEnv } from "./helpers.js";

dotenv.config({ path: path.join(process.cwd(), "..", ".env") });

const KORA_RPC_URL = process.env.KORA_RPC_URL || "http://localhost:8081/";
const ZK_RPC_URL = getEnvOrThrow("ZK_COMPRESSION_RPC_URL");
const SOLANA_RPC_URL = process.env.SOLANA_RPC_URL || ZK_RPC_URL;

const LIGHT_TOKEN_PROGRAM_ID = new PublicKey(
  "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m"
);
const DECIMALS = 6;

const TRANSFER_CHECKED_DISCRIMINATOR = 12;
const TRANSFER2_DISCRIMINATOR = 101; // Also used by decompress

// --- Helpers ---

function findDiscriminators(base64Tx: string): number[] {
  const tx = VersionedTransaction.deserialize(
    Buffer.from(base64Tx, "base64")
  );
  return tx.message.compiledInstructions
    .filter((ix) => ix.data.length > 0)
    .map((ix) => ix.data[0]);
}

async function getHotBalance(
  rpc: Rpc,
  owner: PublicKey,
  mint: PublicKey
): Promise<number> {
  const ata = getAssociatedTokenAddressInterface(mint, owner);
  try {
    const account = await getAtaInterface(rpc, ata, owner, mint);
    return Number(account.parsed.amount);
  } catch {
    return 0;
  }
}

async function getColdBalance(
  rpc: Rpc,
  owner: PublicKey,
  mint: PublicKey
): Promise<number> {
  const accounts = await rpc.getCompressedTokenAccountsByOwner(owner, {
    mint,
  });
  return accounts.items.reduce(
    (sum: number, a: any) => sum + Number(a.parsed.amount),
    0
  );
}

function formatTokens(amount: number, decimals = 6): string {
  return `${amount} (${(amount / 10 ** decimals).toFixed(decimals)} tokens)`;
}

// --- Test runner ---

interface TestResult {
  name: string;
  passed: boolean;
  signature?: string;
  error?: string;
}

/**
 * Build transfer instructions client-side, sign, send to Kora for co-signing,
 * broadcast, and confirm. Returns the last batch's signature and all discriminators.
 */
async function transferAndConfirm(
  kora: KoraClient,
  rpc: Rpc,
  connection: Connection,
  koraFeePayer: PublicKey,
  sender: Keypair,
  destination: Keypair,
  mint: PublicKey,
  amount: number
): Promise<{ signature: string; discriminators: number[] }> {
  const destinationAta = getAssociatedTokenAddressInterface(
    mint,
    destination.publicKey
  );

  const instructionBatches = await createTransferInterfaceInstructions(
    rpc,
    koraFeePayer,
    mint,
    amount,
    sender.publicKey,
    destinationAta,
    DECIMALS
  );

  // Prepend idempotent ATA creation to the first batch
  const createDestAtaIx = createAtaInterfaceIdempotentInstruction(
    koraFeePayer,
    destinationAta,
    destination.publicKey,
    mint,
    LIGHT_TOKEN_PROGRAM_ID
  );
  instructionBatches[0] = [createDestAtaIx, ...instructionBatches[0]];

  let lastSignature = "";
  const allDiscriminators: number[] = [];

  for (let i = 0; i < instructionBatches.length; i++) {
    const instructions = instructionBatches[i];
    const { blockhash } = await connection.getLatestBlockhash("confirmed");
    const tx = buildV0Transaction({
      instructions,
      feePayer: koraFeePayer,
      blockhash,
    });
    tx.sign([sender]);

    // Check discriminators before sending to Kora
    const batchDiscriminators = findDiscriminators(
      Buffer.from(tx.serialize()).toString("base64")
    );
    allDiscriminators.push(...batchDiscriminators);

    const { signed_transaction } = await kora.signTransaction({
      transaction: Buffer.from(tx.serialize()).toString("base64"),
    });

    const finalTx = VersionedTransaction.deserialize(
      Buffer.from(signed_transaction, "base64")
    );
    const signature = await connection.sendRawTransaction(finalTx.serialize());
    const confirmation = await connection.confirmTransaction(
      signature,
      "confirmed"
    );

    if (confirmation.value.err) {
      throw new Error(
        `Transaction failed on-chain: ${JSON.stringify(confirmation.value.err)}`
      );
    }

    lastSignature = signature;
  }

  return { signature: lastSignature, discriminators: allDiscriminators };
}

// --- Main ---

async function main() {
  console.log("============================================");
  console.log("  Client-Side Light-Token Transfer E2E Test");
  console.log("  Tests: HOT, MIXED, COLD paths");
  console.log("============================================");
  console.log("\nKora RPC:", KORA_RPC_URL);
  console.log("ZK RPC:", ZK_RPC_URL);

  const connection = new Connection(SOLANA_RPC_URL, "confirmed");
  const rpc = createRpc(ZK_RPC_URL);

  const kora = new KoraClient({ rpcUrl: KORA_RPC_URL });

  const sender = keypairFromEnv("TEST_SENDER_KEYPAIR");
  const destination = keypairFromEnv("DESTINATION_KEYPAIR");
  const mint = new PublicKey(getEnvOrThrow("LIGHT_TOKEN_MINT"));

  console.log("\nSender:", sender.publicKey.toBase58());
  console.log("Destination:", destination.publicKey.toBase58());
  console.log("Mint:", mint.toBase58());

  // Get Kora's fee payer
  const { signer_address } = await kora.getPayerSigner();
  const koraFeePayer = new PublicKey(signer_address);
  console.log("Kora fee payer:", koraFeePayer.toBase58());

  // Verify Kora is running
  try {
    await kora.getConfig();
    console.log("Kora server: OK");
  } catch {
    console.error("Kora server not reachable at", KORA_RPC_URL);
    process.exit(1);
  }

  // Check starting balances
  let hot = await getHotBalance(rpc, sender.publicKey, mint);
  let cold = await getColdBalance(rpc, sender.publicKey, mint);
  console.log("\n--- Starting balances ---");
  console.log("  Hot:", formatTokens(hot));
  console.log("  Cold:", formatTokens(cold));
  console.log("  Total:", formatTokens(hot + cold));

  if (hot === 0 && cold === 0) {
    console.error("No balance. Run devnet-setup.ts first.");
    process.exit(1);
  }

  const results: TestResult[] = [];

  // ---------------------------------------------------------------
  // TEST 1: HOT PATH
  // Transfer 1 token. Hot balance should be sufficient.
  // Expected: TransferChecked (disc 12), no decompress.
  // ---------------------------------------------------------------
  console.log("\n=== TEST 1: HOT PATH ===");
  const hotAmount = 1_000_000; // 1 token
  if (hot < hotAmount) {
    console.log(`  SKIP: hot balance ${hot} < ${hotAmount}`);
    results.push({ name: "HOT", passed: false, error: "insufficient hot" });
  } else {
    try {
      console.log(`  Transferring ${formatTokens(hotAmount)}...`);
      const { signature, discriminators } = await transferAndConfirm(
        kora, rpc, connection, koraFeePayer,
        sender, destination, mint, hotAmount
      );
      console.log("  Discriminators:", discriminators);

      const hasTransferChecked = discriminators.includes(TRANSFER_CHECKED_DISCRIMINATOR);
      const hasTransfer2 = discriminators.includes(TRANSFER2_DISCRIMINATOR);

      if (!hasTransferChecked) throw new Error("Missing TransferChecked (disc 12)");
      if (hasTransfer2) throw new Error("Unexpected Transfer2 in hot path");

      console.log("  PASS: Hot path confirmed. Sig:", signature.slice(0, 30) + "...");
      results.push({ name: "HOT", passed: true, signature });

      // Wait for balance to settle
      await new Promise((r) => setTimeout(r, 2000));
      hot = await getHotBalance(rpc, sender.publicKey, mint);
      cold = await getColdBalance(rpc, sender.publicKey, mint);
      console.log("  Post-transfer: hot=" + formatTokens(hot) + " cold=" + formatTokens(cold));
    } catch (e: any) {
      console.error("  FAIL:", e.message);
      results.push({ name: "HOT", passed: false, error: e.message });
    }
  }

  // ---------------------------------------------------------------
  // TEST 2: MIXED PATH
  // Transfer more than hot balance but less than hot + cold.
  // Expected: decompress (disc 101) + TransferChecked (disc 12).
  // ---------------------------------------------------------------
  console.log("\n=== TEST 2: MIXED PATH ===");
  // Refresh balances
  hot = await getHotBalance(rpc, sender.publicKey, mint);
  cold = await getColdBalance(rpc, sender.publicKey, mint);
  console.log("  Current: hot=" + formatTokens(hot) + " cold=" + formatTokens(cold));

  // We need: hot < mixedAmount <= hot + cold
  const mixedAmount = hot + Math.min(cold, 1_000_000);
  if (hot === 0 || cold === 0 || mixedAmount <= hot) {
    console.log("  SKIP: Need both hot > 0 and cold > 0 for mixed path");
    results.push({ name: "MIXED", passed: false, error: "can't trigger mixed" });
  } else {
    try {
      console.log(`  Transferring ${formatTokens(mixedAmount)} (hot=${formatTokens(hot)}, need ${formatTokens(mixedAmount - hot)} from cold)...`);
      const { signature, discriminators } = await transferAndConfirm(
        kora, rpc, connection, koraFeePayer,
        sender, destination, mint, mixedAmount
      );
      console.log("  Discriminators:", discriminators);

      const hasTransferChecked = discriminators.includes(TRANSFER_CHECKED_DISCRIMINATOR);
      const hasTransfer2 = discriminators.includes(TRANSFER2_DISCRIMINATOR);

      if (!hasTransferChecked) throw new Error("Missing TransferChecked (disc 12)");
      if (!hasTransfer2) throw new Error("Missing Transfer2/decompress (disc 101) — mixed path should decompress cold into hot");

      console.log("  PASS: Mixed path confirmed. Sig:", signature.slice(0, 30) + "...");
      results.push({ name: "MIXED", passed: true, signature });

      await new Promise((r) => setTimeout(r, 2000));
      hot = await getHotBalance(rpc, sender.publicKey, mint);
      cold = await getColdBalance(rpc, sender.publicKey, mint);
      console.log("  Post-transfer: hot=" + formatTokens(hot) + " cold=" + formatTokens(cold));
    } catch (e: any) {
      console.error("  FAIL:", e.message);
      results.push({ name: "MIXED", passed: false, error: e.message });
    }
  }

  // ---------------------------------------------------------------
  // TEST 3: COLD PATH
  // Hot balance should be 0 after mixed drained it. Transfer from cold only.
  // Expected: Transfer2 (disc 101), no TransferChecked.
  // ---------------------------------------------------------------
  console.log("\n=== TEST 3: COLD PATH ===");
  hot = await getHotBalance(rpc, sender.publicKey, mint);
  cold = await getColdBalance(rpc, sender.publicKey, mint);
  console.log("  Current: hot=" + formatTokens(hot) + " cold=" + formatTokens(cold));

  if (hot > 0) {
    console.log("  SKIP: Hot balance not zero — mixed path didn't drain it. Cold path can't be tested.");
    results.push({ name: "COLD", passed: false, error: "hot != 0" });
  } else if (cold < 1_000_000) {
    console.log("  SKIP: Insufficient cold balance for test");
    results.push({ name: "COLD", passed: false, error: "insufficient cold" });
  } else {
    try {
      const coldAmount = 1_000_000;
      console.log(`  Transferring ${formatTokens(coldAmount)} from cold only...`);
      const { signature, discriminators } = await transferAndConfirm(
        kora, rpc, connection, koraFeePayer,
        sender, destination, mint, coldAmount
      );
      console.log("  Discriminators:", discriminators);

      const hasTransfer2 = discriminators.includes(TRANSFER2_DISCRIMINATOR);
      const hasTransferChecked = discriminators.includes(TRANSFER_CHECKED_DISCRIMINATOR);

      if (!hasTransfer2) throw new Error("Missing Transfer2 (disc 101) — cold path should use compressed transfer");
      if (hasTransferChecked) throw new Error("Unexpected TransferChecked in cold path");

      console.log("  PASS: Cold path confirmed. Sig:", signature.slice(0, 30) + "...");
      results.push({ name: "COLD", passed: true, signature });

      await new Promise((r) => setTimeout(r, 2000));
      cold = await getColdBalance(rpc, sender.publicKey, mint);
      console.log("  Post-transfer: cold=" + formatTokens(cold));
    } catch (e: any) {
      console.error("  FAIL:", e.message);
      results.push({ name: "COLD", passed: false, error: e.message });
    }
  }

  // --- Summary ---
  console.log("\n============================================");
  console.log("  RESULTS");
  console.log("============================================");
  for (const r of results) {
    const status = r.passed ? "PASS" : "FAIL";
    const detail = r.signature
      ? ` (${r.signature.slice(0, 20)}...)`
      : r.error
      ? ` — ${r.error}`
      : "";
    console.log(`  ${status}: ${r.name}${detail}`);
  }

  const allPassed = results.every((r) => r.passed);
  console.log(
    allPassed
      ? "\nAll tests passed."
      : "\nSome tests failed."
  );
  process.exit(allPassed ? 0 : 1);
}

main().catch((e) => {
  console.error("Test runner failed:", e);
  process.exit(1);
});
