/**
 * Client-side Light Token transfer with Kora fee sponsorship.
 *
 * The client builds the compressed token transfer using the Light Protocol SDK,
 * then sends each transaction to Kora for fee-payer signing.
 * Kora pays the transaction fee and rent top-ups.
 *
 * Prerequisites:
 * - Running Kora server with Light Protocol programs allowlisted (see server/kora.toml)
 * - Sender holds Light Token balance (run devnet-setup.ts first)
 * - Environment variables in ../.env
 */
import { KoraClient } from "@solana/kora";
import {
  Keypair,
  PublicKey,
  Connection,
  VersionedTransaction,
} from "@solana/web3.js";
import { Rpc } from "@lightprotocol/stateless.js";
import { createTransferInterfaceInstructions } from "@lightprotocol/compressed-token";
import {
  getAssociatedTokenAddressInterface,
  createAtaInterfaceIdempotentInstruction,
} from "@lightprotocol/compressed-token/unified";

const LIGHT_TOKEN_PROGRAM_ID = new PublicKey(
  "cTokenmWW8bLPjZEBAUgYy3zKxQZW6VKi7bqNFEVv3m"
);
import { getMint } from "@solana/spl-token";
import bs58 from "bs58";
import dotenv from "dotenv";
import path from "path";
import { buildV0Transaction } from "./helpers.js";

dotenv.config({ path: path.join(process.cwd(), "..", ".env") });

function getEnvOrThrow(key: string): string {
  const val = process.env[key];
  if (!val) throw new Error(`Missing env var: ${key}`);
  return val;
}

async function main() {
  const zkRpcUrl = getEnvOrThrow("ZK_COMPRESSION_RPC_URL");
  const solanaRpcUrl = process.env.SOLANA_RPC_URL || zkRpcUrl;
  const koraRpcUrl = process.env.KORA_RPC_URL || "http://localhost:8081/";

  // 1. Initialize clients
  const kora = new KoraClient({ rpcUrl: koraRpcUrl });
  const rpc = new Rpc(zkRpcUrl, zkRpcUrl, zkRpcUrl);
  const connection = new Connection(solanaRpcUrl, "confirmed");

  // Keypairs (in production, use a wallet adapter)
  const sender = Keypair.fromSecretKey(
    bs58.decode(getEnvOrThrow("TEST_SENDER_KEYPAIR"))
  );
  const destination = Keypair.fromSecretKey(
    bs58.decode(getEnvOrThrow("DESTINATION_KEYPAIR"))
  );
  const mint = new PublicKey(getEnvOrThrow("LIGHT_TOKEN_MINT"));

  console.log("Sender:", sender.publicKey.toBase58());
  console.log("Destination:", destination.publicKey.toBase58());
  console.log("Mint:", mint.toBase58());

  // 2. Get Kora's fee payer pubkey
  const { signer_address } = await kora.getPayerSigner();
  const koraFeePayer = new PublicKey(signer_address);
  console.log("Kora fee payer:", koraFeePayer.toBase58());

  // 3. Check fee payer has enough SOL
  const feePayerBalance = await connection.getBalance(koraFeePayer);
  if (feePayerBalance < 5_000_000) {
    throw new Error(
      `Kora fee payer has insufficient SOL: ${feePayerBalance / 1e9} SOL. ` +
        `Fund it: solana transfer --url devnet ${koraFeePayer.toBase58()} 0.1 --allow-unfunded-recipient`
    );
  }

  // 4. Get mint decimals
  const mintAccount = await getMint(connection, mint);
  const decimals = mintAccount.decimals;
  console.log("Decimals:", decimals);

  // 5. Derive destination Light Token ATA
  const destinationAta = getAssociatedTokenAddressInterface(
    mint,
    destination.publicKey
  );
  console.log("Destination ATA:", destinationAta.toBase58());

  // 6. Build Light Token transfer instructions client-side.
  //    The Light Protocol SDK fetches compressed accounts, validity proofs,
  //    and builds instructions. Setting payer = Kora's pubkey so rent
  //    top-ups are sponsored.
  const amount = Number(
    process.env.TRANSFER_AMOUNT || process.argv[2] || "1000000"
  );
  console.log(`\nBuilding transfer of ${amount / 10 ** decimals} tokens...`);

  let instructionBatches;
  try {
    instructionBatches = await createTransferInterfaceInstructions(
      rpc,
      koraFeePayer,
      mint,
      amount,
      sender.publicKey,
      destinationAta,
      decimals
    );
  } catch (e: any) {
    throw new Error(
      `Failed to build transfer instructions: ${e.message}\n` +
        `Check ZK RPC connectivity and sender balance.`
    );
  }

  // Ensure destination ATA exists (idempotent — no-op if already created)
  const createDestAtaIx = createAtaInterfaceIdempotentInstruction(
    koraFeePayer,
    destinationAta,
    destination.publicKey,
    mint,
    LIGHT_TOKEN_PROGRAM_ID
  );
  instructionBatches[0] = [createDestAtaIx, ...instructionBatches[0]];

  console.log(`${instructionBatches.length} batch(es) to send`);

  // 7. Send each batch: build V0 tx → sign → Kora co-signs → broadcast.
  //    Validity proofs are slot-bounded. Submit each batch to Kora promptly
  //    after building. If too much time passes, proofs expire on-chain.
  for (let i = 0; i < instructionBatches.length; i++) {
    const instructions = instructionBatches[i];
    const isLast = i === instructionBatches.length - 1;
    console.log(
      `\nBatch ${i + 1}/${instructionBatches.length} ` +
        `(${instructions.length} instructions)` +
        `${isLast ? " [transfer]" : " [load]"}`
    );

    const { blockhash } = await connection.getLatestBlockhash("confirmed");
    const tx = buildV0Transaction({
      instructions,
      feePayer: koraFeePayer,
      blockhash,
    });
    tx.sign([sender]);

    let signed_transaction: string;
    try {
      const result = await kora.signTransaction({
        transaction: Buffer.from(tx.serialize()).toString("base64"),
      });
      signed_transaction = result.signed_transaction;
    } catch (e: any) {
      throw new Error(
        `Kora rejected batch ${i + 1}: ${e.message}\n` +
          `Check allowed_programs config and fee payer balance.`
      );
    }

    const finalTx = VersionedTransaction.deserialize(
      Buffer.from(signed_transaction, "base64")
    );

    let signature: string;
    try {
      signature = await connection.sendRawTransaction(finalTx.serialize());
      await connection.confirmTransaction(signature, "confirmed");
    } catch (e: any) {
      throw new Error(
        `Batch ${i + 1} failed on-chain: ${e.message}\n` +
          `Proofs may have expired — rebuild and retry.`
      );
    }

    console.log(`Confirmed: ${signature}`);
  }

  console.log("\nLight Token transfer complete.");
}

main().catch((e) => {
  console.error("Error:", e.message || e);
  process.exit(1);
});
