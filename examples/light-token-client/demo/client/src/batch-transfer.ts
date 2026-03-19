/**
 * Batch gasless transfer: pay multiple recipients in one transaction via Kora.
 *
 * Builds transfer instructions for each recipient, deduplicates ComputeBudget
 * instructions, prepends ATA creation, and sends a single V0 transaction to
 * Kora for fee-payer co-signing.
 *
 * Adapted from examples-light-token/toolkits/payments/send/batch-send.ts
 */
import { KoraClient } from "@solana/kora";
import { PublicKey, Keypair, Connection, VersionedTransaction } from "@solana/web3.js";
import { createRpc } from "@lightprotocol/stateless.js";
import {
  getAssociatedTokenAddressInterface,
  createAtaInterfaceIdempotentInstruction,
  createTransferInterfaceInstructions,
} from "@lightprotocol/compressed-token/unified";
import dotenv from "dotenv";
import path from "path";
import { getEnvOrThrow, keypairFromEnv, buildV0Transaction, LIGHT_TOKEN_PROGRAM_ID } from "./helpers.js";

dotenv.config({ path: path.join(process.cwd(), "..", ".env") });

const COMPUTE_BUDGET_ID = "ComputeBudget111111111111111111111111111111";

async function main(): Promise<void> {
  const zkRpcUrl = getEnvOrThrow("ZK_COMPRESSION_RPC_URL");
  const koraRpcUrl = process.env.KORA_RPC_URL || "http://localhost:8081/";

  const kora = new KoraClient({ rpcUrl: koraRpcUrl });
  const rpc = createRpc(zkRpcUrl);
  const connection = new Connection(zkRpcUrl, "confirmed");

  const sender = keypairFromEnv("TEST_SENDER_KEYPAIR");
  const mint = new PublicKey(getEnvOrThrow("LIGHT_TOKEN_MINT"));

  const { signer_address } = await kora.getPayerSigner();
  const koraFeePayer = new PublicKey(signer_address);

  // Generate 3 recipients with different amounts
  const recipients = [
    { address: Keypair.generate().publicKey, amount: 100_000 },
    { address: Keypair.generate().publicKey, amount: 200_000 },
    { address: Keypair.generate().publicKey, amount: 300_000 },
  ];

  console.log("Sender:", sender.publicKey.toBase58());
  console.log("Kora fee payer:", koraFeePayer.toBase58());
  console.log("Recipients:", recipients.length);

  // Build transfer instructions for each recipient, deduplicate ComputeBudget
  const allInstructions = [];
  let isFirst = true;

  for (const { address, amount } of recipients) {
    const destinationAta = getAssociatedTokenAddressInterface(mint, address);

    // Prepend ATA creation for each recipient
    allInstructions.push(
      createAtaInterfaceIdempotentInstruction(
        koraFeePayer, destinationAta, address, mint, LIGHT_TOKEN_PROGRAM_ID,
      ),
    );

    const ixBatches = await createTransferInterfaceInstructions(
      rpc, koraFeePayer, mint, amount, sender.publicKey, destinationAta,
    );

    // Deduplicate ComputeBudget instructions across transfers
    for (const ix of ixBatches[0]) {
      if (!isFirst && ix.programId.toBase58() === COMPUTE_BUDGET_ID) continue;
      allInstructions.push(ix);
    }
    isFirst = false;
  }

  // Build single V0 transaction, sign, send to Kora
  const { blockhash } = await connection.getLatestBlockhash("confirmed");
  const tx = buildV0Transaction({ instructions: allInstructions, feePayer: koraFeePayer, blockhash });
  tx.sign([sender]);

  const { signed_transaction } = await kora.signTransaction({
    transaction: Buffer.from(tx.serialize()).toString("base64"),
  });

  const finalTx = VersionedTransaction.deserialize(Buffer.from(signed_transaction, "base64"));
  const signature = await connection.sendRawTransaction(finalTx.serialize());
  const confirmation = await connection.confirmTransaction(signature, "confirmed");
  if (confirmation.value.err) {
    throw new Error(`Transaction failed: ${JSON.stringify(confirmation.value.err)}`);
  }

  console.log("Batch tx:", signature);
  for (const { address, amount } of recipients) {
    console.log(`  ${address.toBase58()}: ${amount}`);
  }
}

main().catch((e) => {
  console.error("Error:", e.message || e);
  process.exit(1);
});
