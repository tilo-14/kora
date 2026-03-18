/**
 * Client-side Light Token transfer with Kora fee sponsorship.
 *
 * The client builds the Light Token transfer using the Light Protocol SDK,
 * then sends each transaction to Kora for fee-payer signing.
 * Kora pays the transaction fee and rent top-ups.
 *
 * Prerequisites:
 * - Running Kora server with Light Protocol programs allowlisted (see server/kora.toml)
 * - Sender holds Light Token balance (run devnet-setup.ts first)
 * - Environment variables in ../.env
 */
import { KoraClient } from "@solana/kora";
import { PublicKey, Connection, VersionedTransaction } from "@solana/web3.js";
import { createRpc } from "@lightprotocol/stateless.js";
import { createTransferInterfaceInstructions } from "@lightprotocol/compressed-token";
import {
  getAssociatedTokenAddressInterface,
  createAtaInterfaceIdempotentInstruction,
} from "@lightprotocol/compressed-token/unified";
import dotenv from "dotenv";
import path from "path";
import { getEnvOrThrow, keypairFromEnv, buildV0Transaction, LIGHT_TOKEN_PROGRAM_ID } from "./helpers.js";

dotenv.config({ path: path.join(process.cwd(), "..", ".env") });

async function main(): Promise<void> {
  const zkRpcUrl = getEnvOrThrow("ZK_COMPRESSION_RPC_URL");
  const koraRpcUrl = process.env.KORA_RPC_URL || "http://localhost:8081/";

  const kora = new KoraClient({ rpcUrl: koraRpcUrl });
  const rpc = createRpc(zkRpcUrl);
  const connection = new Connection(zkRpcUrl, "confirmed");

  const sender = keypairFromEnv("TEST_SENDER_KEYPAIR");
  const destination = keypairFromEnv("DESTINATION_KEYPAIR");
  const mint = new PublicKey(getEnvOrThrow("LIGHT_TOKEN_MINT"));
  const decimals = 6;

  console.log("Sender:", sender.publicKey.toBase58());
  console.log("Destination:", destination.publicKey.toBase58());
  console.log("Mint:", mint.toBase58());

  const { signer_address } = await kora.getPayerSigner();
  const koraFeePayer = new PublicKey(signer_address);
  console.log("Kora fee payer:", koraFeePayer.toBase58());

  const amount = Number(
    process.env.TRANSFER_AMOUNT || process.argv[2] || "1000000",
  );

  const destinationAta = getAssociatedTokenAddressInterface(
    mint,
    destination.publicKey,
  );

  const instructionBatches = await createTransferInterfaceInstructions(
    rpc,
    koraFeePayer,
    mint,
    amount,
    sender.publicKey,
    destinationAta,
    decimals,
  );

  // Ensure destination ATA exists (idempotent -- no-op if already created)
  const createDestAtaIx = createAtaInterfaceIdempotentInstruction(
    koraFeePayer,
    destinationAta,
    destination.publicKey,
    mint,
    LIGHT_TOKEN_PROGRAM_ID,
  );
  instructionBatches[0] = [createDestAtaIx, ...instructionBatches[0]];

  for (const ixs of instructionBatches) {
    const { blockhash } = await connection.getLatestBlockhash("confirmed");
    const tx = buildV0Transaction({ instructions: ixs, feePayer: koraFeePayer, blockhash });
    tx.sign([sender]);

    const { signed_transaction } = await kora.signTransaction({
      transaction: Buffer.from(tx.serialize()).toString("base64"),
    });

    const finalTx = VersionedTransaction.deserialize(Buffer.from(signed_transaction, "base64"));
    const signature = await connection.sendRawTransaction(finalTx.serialize());
    await connection.confirmTransaction(signature, "confirmed");
    console.log("Tx:", signature);
  }
}

main().catch((e) => {
  console.error("Error:", e.message || e);
  process.exit(1);
});
