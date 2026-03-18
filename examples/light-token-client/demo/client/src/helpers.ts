/**
 * Utility to build V0 transactions for Light Token transfers.
 *
 * Light Protocol v3 instructions use a packed-account layout that fits
 * in standard V0 messages without address lookup tables.
 */
import {
  PublicKey,
  VersionedTransaction,
  TransactionMessage,
  TransactionInstruction,
} from "@solana/web3.js";

/**
 * Build a V0 versioned transaction from Light Token instructions.
 *
 * The transaction is returned unsigned — the caller must sign with the
 * sender keypair and then send to Kora for fee-payer co-signing.
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
