/**
 * Devnet setup for client-side Light Token transfer testing.
 *
 * Creates a standard SPL mint, registers it with the Light Token Program,
 * wraps tokens into Light Token ATAs (hot state), and compresses some (cold state).
 *
 * Requirements:
 * - ZK compression-enabled RPC (Helius devnet)
 */
import {
  Connection,
  Keypair,
  PublicKey,
} from "@solana/web3.js";
import { createRpc } from "@lightprotocol/stateless.js";
import {
  createAtaInterfaceIdempotent,
  createSplInterface,
  wrap,
} from "@lightprotocol/compressed-token/unified";
import { compress } from "@lightprotocol/compressed-token";
import {
  createAssociatedTokenAccount,
  createMint,
  getAssociatedTokenAddressSync,
  mintTo,
} from "@solana/spl-token";
import bs58 from "bs58";
import dotenv from "dotenv";
import path from "path";
import { appendFile, readFile, writeFile } from "fs/promises";
import { getEnvOrThrow } from "./helpers.js";

dotenv.config({ path: path.join(process.cwd(), "..", ".env") });

const ENV_PATH = path.join(process.cwd(), "..", ".env");

const DECIMALS = 6;
const HOT_MINT_AMOUNT = 10_000_000; // 10 tokens (6 decimals) for hot path
const COLD_MINT_AMOUNT = 5_000_000; // 5 tokens (6 decimals) for cold path

function keypairFromEnvOrGenerate(key: string): Keypair {
  if (process.env[key]) {
    return Keypair.fromSecretKey(bs58.decode(process.env[key]!));
  }
  return Keypair.generate();
}

async function appendEnvVar(
  name: string,
  value: string,
  comment?: string,
): Promise<void> {
  const line = comment
    ? `\n# ${comment}\n${name}=${value}\n`
    : `\n${name}=${value}\n`;
  await appendFile(ENV_PATH, line);
  process.env[name] = value;
}

async function checkBalance(
  connection: Connection,
  pubkey: PublicKey,
  label: string,
  minLamports = 10_000_000,
): Promise<void> {
  const balance = await connection.getBalance(pubkey);
  if (balance < minLamports) {
    throw new Error(
      `${label} has insufficient balance (${balance} lamports).\n` +
        `  Fund manually: solana transfer --url devnet ${pubkey.toBase58()} 0.1 --allow-unfunded-recipient`,
    );
  }
}

async function persistKeypair(
  envKey: string,
  keypair: Keypair,
  label: string,
): Promise<void> {
  if (!process.env[envKey]) {
    await appendEnvVar(
      envKey,
      bs58.encode(keypair.secretKey),
      `${label}: ${keypair.publicKey.toBase58()}`,
    );
  }
}

async function main(): Promise<void> {
  const zkRpcUrl = getEnvOrThrow("ZK_COMPRESSION_RPC_URL");
  const solanaRpcUrl = process.env.SOLANA_RPC_URL || zkRpcUrl;

  const connection = new Connection(solanaRpcUrl, "confirmed");
  const rpc = createRpc(zkRpcUrl);

  // 1. Load or generate keypairs
  const payer = keypairFromEnvOrGenerate("SETUP_PAYER_KEYPAIR");
  const sender = keypairFromEnvOrGenerate("TEST_SENDER_KEYPAIR");
  const destination = keypairFromEnvOrGenerate("DESTINATION_KEYPAIR");
  const mintAuthority = keypairFromEnvOrGenerate("MINT_AUTHORITY");
  const koraFeePayer = keypairFromEnvOrGenerate("KORA_PRIVATE_KEY");

  await persistKeypair("SETUP_PAYER_KEYPAIR", payer, "Setup payer");
  await persistKeypair("TEST_SENDER_KEYPAIR", sender, "Test sender");
  await persistKeypair("DESTINATION_KEYPAIR", destination, "Destination");
  await persistKeypair("MINT_AUTHORITY", mintAuthority, "Mint authority");
  await persistKeypair("KORA_PRIVATE_KEY", koraFeePayer, "Kora fee payer");

  // 2. Check balances
  await checkBalance(connection, payer.publicKey, "Payer");
  await checkBalance(connection, koraFeePayer.publicKey, "Kora fee payer");

  // 3. Create or reuse SPL mint
  let mintAddress: PublicKey;
  if (process.env.LIGHT_TOKEN_MINT) {
    mintAddress = new PublicKey(process.env.LIGHT_TOKEN_MINT);
  } else {
    mintAddress = await createMint(
      connection,
      payer,
      mintAuthority.publicKey,
      null,
      DECIMALS,
    );
    await appendEnvVar(
      "LIGHT_TOKEN_MINT",
      mintAddress.toBase58(),
      `SPL mint registered with Light Token Program (${DECIMALS} decimals)`,
    );
  }
  console.log("Mint:", mintAddress.toBase58());

  // 4. Create sender SPL ATA and mint tokens
  let senderSplAta: PublicKey;
  try {
    senderSplAta = await createAssociatedTokenAccount(
      connection,
      payer,
      mintAddress,
      sender.publicKey,
    );
  } catch (e: any) {
    // Only fall back to derived address if account already exists
    if (e.message?.includes("already in use") || e.message?.includes("0x0")) {
      senderSplAta = getAssociatedTokenAddressSync(mintAddress, sender.publicKey);
    } else {
      throw e;
    }
  }

  const totalSplAmount = HOT_MINT_AMOUNT + COLD_MINT_AMOUNT;
  await mintTo(
    connection,
    payer,
    mintAddress,
    senderSplAta,
    mintAuthority,
    totalSplAmount,
  );

  // 5. Register SPL mint with Light Token Program
  try {
    await createSplInterface(rpc, payer, mintAddress);
  } catch (e: any) {
    if (
      e.message?.includes("already in use") ||
      e.message?.includes("0x0")
    ) {
      // Already registered, skip
    } else {
      throw e;
    }
  }

  // 6. Create Light Token ATAs
  const senderLightAta = await createAtaInterfaceIdempotent(
    rpc,
    payer,
    mintAddress,
    sender.publicKey,
  );
  const destLightAta = await createAtaInterfaceIdempotent(
    rpc,
    payer,
    mintAddress,
    destination.publicKey,
  );
  console.log("Sender Light Token ATA:", senderLightAta.toBase58());
  console.log("Destination Light Token ATA:", destLightAta.toBase58());

  // 7. Wrap SPL tokens to Light Token ATA (hot state)
  await wrap(
    rpc,
    payer,
    senderSplAta,
    senderLightAta,
    sender,
    mintAddress,
    BigInt(HOT_MINT_AMOUNT),
  );
  console.log(`Wrapped ${HOT_MINT_AMOUNT / 10 ** DECIMALS} tokens (hot)`);

  // 8. Compress SPL tokens (cold state)
  await compress(
    rpc,
    payer,
    mintAddress,
    COLD_MINT_AMOUNT,
    sender,
    senderSplAta,
    sender.publicKey,
  );
  console.log(`Compressed ${COLD_MINT_AMOUNT / 10 ** DECIMALS} tokens (cold)`);

  // 9. Update kora.toml with actual mint address
  const koraTomlPath = path.join(process.cwd(), "..", "server", "kora.toml");
  try {
    let tomlContent = await readFile(koraTomlPath, "utf-8");
    tomlContent = tomlContent.replace(
      /allowed_tokens\s*=\s*\[[\s\S]*?\]/,
      `allowed_tokens = [\n    "${mintAddress.toBase58()}", # Light Token mint\n]`,
    );
    tomlContent = tomlContent.replace(
      /allowed_spl_paid_tokens\s*=\s*\[[\s\S]*?\]/,
      `allowed_spl_paid_tokens = [\n    "${mintAddress.toBase58()}", # Light Token mint\n]`,
    );
    await writeFile(koraTomlPath, tomlContent);
    console.log("Updated kora.toml with mint:", mintAddress.toBase58());
  } catch (e) {
    console.warn("Could not update kora.toml:", e);
    console.warn("Manually set allowed_tokens to:", mintAddress.toBase58());
  }

  // Summary
  console.log("\nSetup complete:");
  console.log(`  Mint: ${mintAddress.toBase58()}`);
  console.log(`  Sender: ${sender.publicKey.toBase58()}`);
  console.log(`  Destination: ${destination.publicKey.toBase58()}`);
  console.log(`  Kora fee payer: ${koraFeePayer.publicKey.toBase58()}`);
  console.log(`  Hot balance: ${HOT_MINT_AMOUNT / 10 ** DECIMALS} tokens`);
  console.log(`  Cold balance: ${COLD_MINT_AMOUNT / 10 ** DECIMALS} tokens`);
  console.log(`\nKeypairs saved to ${ENV_PATH}`);
}

main().catch((e) => {
  console.error("Setup failed:", e);
  process.exit(1);
});
