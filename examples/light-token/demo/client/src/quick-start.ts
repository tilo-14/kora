/**
 * Minimal example: gasless light token (compressed token) transfer via Kora.
 *
 * Kora handles all compression complexity server-side when you pass
 * light_token: true to transferTransaction. The client just signs and submits.
 *
 * Prerequisites:
 * - Running Kora server with light_token config (see server/kora.toml)
 * - Sender holds compressed tokens (run devnet-setup.ts first)
 * - Environment variables in ../.env
 */
import { KoraClient } from "@solana/kora";
import {
  Keypair,
  Connection,
  VersionedTransaction,
} from "@solana/web3.js";
import bs58 from "bs58";
import dotenv from "dotenv";
import path from "path";

dotenv.config({ path: path.join(process.cwd(), "..", ".env") });

async function main() {
  const client = new KoraClient({
    rpcUrl: process.env.KORA_RPC_URL || "http://localhost:8080/",
  });
  const connection = new Connection(
    process.env.SOLANA_RPC_URL || process.env.ZK_COMPRESSION_RPC_URL!,
    "confirmed",
  );

  // Keypairs (in production, use a wallet adapter)
  const sender = Keypair.fromSecretKey(
    bs58.decode(process.env.TEST_SENDER_KEYPAIR!),
  );
  const destination = Keypair.fromSecretKey(
    bs58.decode(process.env.DESTINATION_KEYPAIR!),
  );
  const mint = process.env.LIGHT_TOKEN_MINT!;

  console.log("Sender:", sender.publicKey.toBase58());
  console.log("Destination:", destination.publicKey.toBase58());
  console.log("Mint:", mint);

  // 1. Kora builds the compressed token transfer transaction.
  //    With light_token: true, Kora fetches validity proofs, resolves
  //    Merkle trees, and includes the Light Protocol lookup table.
  const transferRequest = {
    amount: 1_000_000, // 1 token (6 decimals)
    token: mint,
    source: sender.publicKey.toBase58(),
    destination: destination.publicKey.toBase58(),
    light_token: true,
  };
  const { transaction } = await client.transferTransaction(transferRequest);
  console.log("Transaction built by Kora");

  // 2. Sender signs the transaction
  const tx = VersionedTransaction.deserialize(
    Buffer.from(transaction, "base64"),
  );
  tx.sign([sender]);
  console.log("Signed by sender");

  // 3. Kora co-signs as fee payer
  const signedBase64 = Buffer.from(tx.serialize()).toString("base64");
  const { signed_transaction } = await client.signTransaction({
    transaction: signedBase64,
  });
  console.log("Co-signed by Kora");

  // 4. Send to the network
  const finalTx = VersionedTransaction.deserialize(
    Buffer.from(signed_transaction, "base64"),
  );
  const signature = await connection.sendRawTransaction(finalTx.serialize());
  await connection.confirmTransaction(signature, "confirmed");

  console.log("\nLight token transfer confirmed!");
  console.log("Signature:", signature);
}

main().catch((e) => console.error("Error:", e));
