/**
 * Shared utilities for client-side Light Token examples.
 */
import {
  Keypair,
  PublicKey,
  VersionedTransaction,
  TransactionMessage,
  TransactionInstruction,
} from "@solana/web3.js";
import bs58 from "bs58";

export function getEnvOrThrow(key: string): string {
  const val = process.env[key];
  if (!val) throw new Error(`Missing env var: ${key}`);
  return val;
}

export function keypairFromEnv(key: string): Keypair {
  return Keypair.fromSecretKey(bs58.decode(getEnvOrThrow(key)));
}

/**
 * Build a V0 versioned transaction from Light Token instructions.
 * Returns unsigned — caller must sign then send to Kora for co-signing.
 */
export function buildV0Transaction(params: {
  instructions: TransactionInstruction[];
  feePayer: PublicKey;
  blockhash: string;
}): VersionedTransaction {
  const messageV0 = new TransactionMessage({
    payerKey: params.feePayer,
    recentBlockhash: params.blockhash,
    instructions: params.instructions,
  }).compileToV0Message();

  return new VersionedTransaction(messageV0);
}
