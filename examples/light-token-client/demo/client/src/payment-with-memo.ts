/**
 * Gasless transfer with memo: attach an invoice ID to a Light Token payment.
 *
 * Builds transfer instructions, appends a Memo program instruction with a
 * reference string, and sends to Kora for fee-payer co-signing. After
 * confirmation, reads the memo back from transaction logs.
 *
 * Adapted from examples-light-token/toolkits/payments/send/payment-with-memo.ts
 */
import { KoraClient } from "@solana/kora";
import {
  PublicKey,
  Connection,
  VersionedTransaction,
  TransactionInstruction,
} from "@solana/web3.js";
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

const MEMO_PROGRAM_ID = new PublicKey(
  "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
);

async function main(): Promise<void> {
  const zkRpcUrl = getEnvOrThrow("ZK_COMPRESSION_RPC_URL");
  const koraRpcUrl = process.env.KORA_RPC_URL || "http://localhost:8081/";

  const kora = new KoraClient({ rpcUrl: koraRpcUrl });
  const rpc = createRpc(zkRpcUrl);
  const connection = new Connection(zkRpcUrl, "confirmed");

  const sender = keypairFromEnv("TEST_SENDER_KEYPAIR");
  const destination = keypairFromEnv("DESTINATION_KEYPAIR");
  const mint = new PublicKey(getEnvOrThrow("LIGHT_TOKEN_MINT"));

  const { signer_address } = await kora.getPayerSigner();
  const koraFeePayer = new PublicKey(signer_address);

  const amount = Number(process.env.TRANSFER_AMOUNT || "1000000");
  const invoiceId = process.env.MEMO || "INV-2024-001";

  const destinationAta = getAssociatedTokenAddressInterface(
    mint,
    destination.publicKey,
  );

  const instructionBatches = await createTransferInterfaceInstructions(
    rpc, koraFeePayer, mint, amount, sender.publicKey, destinationAta,
  );

  // Prepend ATA creation, append memo to the transfer batch
  const createDestAtaIx = createAtaInterfaceIdempotentInstruction(
    koraFeePayer, destinationAta, destination.publicKey, mint, LIGHT_TOKEN_PROGRAM_ID,
  );
  const memoIx = new TransactionInstruction({
    keys: [],
    programId: MEMO_PROGRAM_ID,
    data: Buffer.from(invoiceId),
  });

  const instructions = [createDestAtaIx, ...instructionBatches[0], memoIx];

  const { blockhash } = await connection.getLatestBlockhash("confirmed");
  const tx = buildV0Transaction({ instructions, feePayer: koraFeePayer, blockhash });
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

  console.log("Tx with memo:", signature);

  // Read memo back from transaction logs
  const txDetails = await connection.getTransaction(signature, {
    maxSupportedTransactionVersion: 0,
  });
  const logs = txDetails?.meta?.logMessages || [];
  const memoLogs = logs.filter((log: string) =>
    log.includes("Program MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"),
  );
  console.log("Memo logs:", memoLogs);
}

main().catch((e) => {
  console.error("Error:", e.message || e);
  process.exit(1);
});
