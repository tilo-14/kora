/**
 * Gasless Light Token (compressed token) transfer via Kora with payment.
 *
 * This demo mirrors the SPL getting-started/full-demo.ts pattern:
 *   1. Initialize clients
 *   2. Setup keypairs
 *   3. Create light token transfer instructions via Kora
 *   4. Estimate fee and get payment instruction
 *   5. Build final V0 transaction with Light Protocol lookup table
 *   6. Sign and submit to Kora for co-signing and broadcast
 *
 * Key difference from SPL transfers:
 *   Light token transfers require the Light Protocol Address Lookup Table
 *   to fit compressed transfer instructions within Solana's 1232-byte limit.
 *   Kora handles validity proofs and Merkle tree resolution server-side.
 *
 * Prerequisites:
 * - Running Kora server with light_token config (see server/kora.toml)
 * - Sender holds compressed tokens (run devnet-setup.ts first)
 * - Environment variables in ../.env
 */
import { KoraClient } from "@solana/kora";
import {
  Keypair,
  PublicKey,
  Connection,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
} from "@solana/web3.js";
import bs58 from "bs58";
import dotenv from "dotenv";
import path from "path";

dotenv.config({ path: path.join(process.cwd(), "..", ".env") });

const CONFIG = {
  koraRpcUrl: process.env.KORA_RPC_URL || "http://localhost:8080/",
  solanaRpcUrl:
    process.env.SOLANA_RPC_URL || process.env.ZK_COMPRESSION_RPC_URL!,
  usdcMint: process.env.LIGHT_TOKEN_MINT!,

  // Light Protocol Address Lookup Table
  // Mainnet: 9NYFyEqPkyXUhkerbGHXUXkvb4qpzeEdHuGpgbgpH1NJ
  // Devnet:  qAJZMgnQJ8G6vA3WRcjD9Jan1wtKkaCFWLWskxJrR5V
  lightLookupTable: new PublicKey(
    process.env.LIGHT_LUT_ADDRESS ||
      "qAJZMgnQJ8G6vA3WRcjD9Jan1wtKkaCFWLWskxJrR5V",
  ),
};

/**
 * Convert a @solana/kit Instruction (from Kora SDK) to @solana/web3.js
 * TransactionInstruction.
 *
 * The Kora SDK returns @solana/kit Instruction objects with:
 *   programAddress (string) — not programId (PublicKey)
 *   accounts[].address + role — not pubkey + isSigner + isWritable
 *   data as Uint8Array — not Buffer
 *
 * AccountRole enum: 0=READONLY, 1=WRITABLE, 2=READONLY_SIGNER, 3=WRITABLE_SIGNER
 */
function toTransactionInstruction(ix: {
  programAddress: string;
  accounts?: ReadonlyArray<{ address: string; role: number }>;
  data?: Uint8Array;
}): TransactionInstruction {
  return new TransactionInstruction({
    programId: new PublicKey(ix.programAddress),
    keys: (ix.accounts || []).map((a) => ({
      pubkey: new PublicKey(a.address),
      isSigner: a.role >= 2,
      isWritable: a.role === 1 || a.role === 3,
    })),
    data: Buffer.from(ix.data || new Uint8Array()),
  });
}

function getEnvKeypair(envKey: string): Keypair {
  const secret = process.env[envKey];
  if (!secret) throw new Error(`Missing env var: ${envKey}`);
  return Keypair.fromSecretKey(bs58.decode(secret));
}

// ---------------------------------------------------------------------------
// Step 1: Initialize clients
// ---------------------------------------------------------------------------
async function initializeClients() {
  console.log("\n[1/6] Initializing clients");
  console.log("  -> Kora RPC:", CONFIG.koraRpcUrl);
  console.log("  -> Solana RPC:", CONFIG.solanaRpcUrl);

  const kora = new KoraClient({
    rpcUrl: CONFIG.koraRpcUrl,
    // apiKey: process.env.KORA_API_KEY,
    // hmacSecret: process.env.KORA_HMAC_SECRET,
  });

  const connection = new Connection(CONFIG.solanaRpcUrl, "confirmed");

  return { kora, connection };
}

// ---------------------------------------------------------------------------
// Step 2: Setup keypairs and get Kora's fee payer
// ---------------------------------------------------------------------------
async function setupKeys(kora: KoraClient) {
  console.log("\n[2/6] Setting up keypairs");

  const sender = getEnvKeypair("TEST_SENDER_KEYPAIR");
  const destination = getEnvKeypair("DESTINATION_KEYPAIR");

  const { signer_address } = await kora.getPayerSigner();
  const feePayer = new PublicKey(signer_address);

  console.log("  -> Sender:", sender.publicKey.toBase58());
  console.log("  -> Destination:", destination.publicKey.toBase58());
  console.log("  -> Kora fee payer:", feePayer.toBase58());

  return { sender, destination, feePayer };
}

// ---------------------------------------------------------------------------
// Step 3: Create light token transfer instructions
// ---------------------------------------------------------------------------
async function createTransferInstructions(
  kora: KoraClient,
  sender: Keypair,
  destination: Keypair,
) {
  console.log("\n[3/6] Creating light token transfer instructions");

  const amount = 1_000_000; // 1 token (6 decimals)
  console.log("  -> Token mint:", CONFIG.usdcMint);
  console.log("  -> Amount:", amount);

  // Kora builds the compressed transfer server-side:
  //   - Fetches sender's compressed token accounts
  //   - Gets validity proofs from ZK compression RPC
  //   - Constructs Light Protocol instructions
  const transferRequest = {
    amount,
    token: CONFIG.usdcMint,
    source: sender.publicKey.toBase58(),
    destination: destination.publicKey.toBase58(),
    light_token: true,
  };
  const result = await kora.transferTransaction(transferRequest);

  // Convert @solana/kit instructions to @solana/web3.js format
  const instructions = result.instructions.map((ix) =>
    toTransactionInstruction(ix as unknown as Parameters<typeof toTransactionInstruction>[0]),
  );
  console.log("  -> Got", instructions.length, "transfer instruction(s)");

  return { instructions, baseTransaction: result.transaction };
}

// ---------------------------------------------------------------------------
// Step 4: Get payment instruction from Kora
// ---------------------------------------------------------------------------
async function getPaymentInstruction(
  kora: KoraClient,
  baseTransaction: string,
  sender: Keypair,
) {
  console.log("\n[4/6] Estimating fee and getting payment instruction");

  const koraConfig = await kora.getConfig();
  const feeToken = koraConfig.validation_config.allowed_spl_paid_tokens[0];

  if (!feeToken) {
    console.log(
      "  -> No paid tokens configured — skipping payment (free mode)",
    );
    return { paymentInstruction: null };
  }

  console.log("  -> Fee token:", feeToken);

  const paymentInfo = await kora.getPaymentInstruction({
    transaction: baseTransaction,
    fee_token: feeToken,
    source_wallet: sender.publicKey.toBase58(),
  });

  // Convert payment instruction to web3.js format
  const paymentInstruction = toTransactionInstruction(
    paymentInfo.payment_instruction as unknown as Parameters<typeof toTransactionInstruction>[0],
  );

  console.log("  -> Payment amount:", paymentInfo.payment_amount);
  console.log("  -> Payment address:", paymentInfo.payment_address);

  return { paymentInstruction };
}

// ---------------------------------------------------------------------------
// Step 5: Build and sign final V0 transaction
// ---------------------------------------------------------------------------
async function buildAndSignTransaction(
  kora: KoraClient,
  connection: Connection,
  instructions: TransactionInstruction[],
  paymentInstruction: TransactionInstruction | null,
  sender: Keypair,
  feePayer: PublicKey,
) {
  console.log("\n[5/6] Building final V0 transaction");

  // Fetch the Light Protocol Address Lookup Table.
  // This table contains frequently used Light Protocol addresses,
  // reducing transaction size to fit within Solana's 1232-byte limit.
  const lutAccount = await connection.getAddressLookupTable(
    CONFIG.lightLookupTable,
  );
  if (!lutAccount.value) {
    throw new Error(
      "Could not fetch Light Protocol lookup table at " +
        CONFIG.lightLookupTable.toBase58(),
    );
  }
  console.log(
    "  -> Light LUT loaded:",
    CONFIG.lightLookupTable.toBase58(),
  );

  // Combine all instructions
  const allInstructions = paymentInstruction
    ? [...instructions, paymentInstruction]
    : instructions;

  // Get fresh blockhash for the final transaction
  const { blockhash } = await kora.getBlockhash();

  // Build V0 message with the Light Protocol lookup table
  const messageV0 = new TransactionMessage({
    payerKey: feePayer,
    recentBlockhash: blockhash,
    instructions: allInstructions,
  }).compileToV0Message([lutAccount.value]);

  const transaction = new VersionedTransaction(messageV0);
  console.log(
    "  -> V0 transaction built with",
    allInstructions.length,
    "instructions",
  );

  // Sign with the sender keypair
  transaction.sign([sender]);
  console.log("  -> Signed by sender");

  return Buffer.from(transaction.serialize()).toString("base64");
}

// ---------------------------------------------------------------------------
// Step 6: Submit to Kora for co-signing and broadcast
// ---------------------------------------------------------------------------
async function submitTransaction(
  kora: KoraClient,
  connection: Connection,
  signedTransaction: string,
) {
  console.log("\n[6/6] Submitting to Kora for co-signing and broadcast");

  // Kora validates the transaction, co-signs with the fee payer keypair
  const { signed_transaction } = await kora.signTransaction({
    transaction: signedTransaction,
  });
  console.log("  -> Co-signed by Kora");

  // Send the fully-signed transaction to the network
  const finalTx = VersionedTransaction.deserialize(
    Buffer.from(signed_transaction, "base64"),
  );
  const signature = await connection.sendRawTransaction(finalTx.serialize());
  console.log("  -> Submitted to network");

  console.log("  -> Awaiting confirmation...");
  await connection.confirmTransaction(signature, "confirmed");

  return signature;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main() {
  console.log("\n========================================================");
  console.log("KORA GASLESS LIGHT TOKEN TRANSFER DEMO (WITH PAYMENT)");
  console.log("========================================================");
  console.log("\nThis demo builds a compressed token transfer via Kora,");
  console.log(
    "adds a payment instruction, and submits for gasless execution.\n",
  );

  try {
    // Step 1: Initialize clients
    const { kora, connection } = await initializeClients();

    // Step 2: Setup keys
    const { sender, destination, feePayer } = await setupKeys(kora);

    // Step 3: Create light token transfer instructions
    const { instructions, baseTransaction } =
      await createTransferInstructions(kora, sender, destination);

    // Step 4: Get payment instruction (skipped if pricing is free)
    const { paymentInstruction } = await getPaymentInstruction(
      kora,
      baseTransaction,
      sender,
    );

    // Step 5: Build and sign final transaction with Light LUT
    const signedTransaction = await buildAndSignTransaction(
      kora,
      connection,
      instructions,
      paymentInstruction,
      sender,
      feePayer,
    );

    // Step 6: Submit via Kora
    const signature = await submitTransaction(
      kora,
      connection,
      signedTransaction,
    );

    console.log("\n========================================================");
    console.log("SUCCESS: Light token transfer confirmed on Solana");
    console.log("========================================================");
    console.log("\nTransaction signature:");
    console.log(signature);
  } catch (error) {
    console.error("\n========================================================");
    console.error("ERROR: Demo failed");
    console.error("========================================================");
    console.error("\nDetails:", error);
    process.exit(1);
  }
}

main().catch((e) => console.error("Error:", e));
