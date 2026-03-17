/**
 * Devnet setup for light-token e2e testing.
 *
 * Creates a standard SPL mint, wraps tokens into light-token ATAs (hot state),
 * and mints compressed tokens (cold state) for testing both paths.
 *
 * Flow mirrors production: SPL mint → register with Light Token Program →
 * wrap SPL tokens into light-token ATAs.
 *
 * Requirements:
 * - Funded devnet keypairs (airdrop SOL first if needed)
 * - ZK compression-enabled RPC (Helius devnet)
 */
import {
  Keypair,
  PublicKey,
  Connection,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import { Rpc } from "@lightprotocol/stateless.js";
import {
  createSplInterface,
  createAtaInterfaceIdempotent,
  mintToInterface,
  getAssociatedTokenAddressInterface,
  wrap,
} from "@lightprotocol/compressed-token/unified";
import { compress } from "@lightprotocol/compressed-token";
import {
  createMint,
  mintTo,
  createAssociatedTokenAccount,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import bs58 from "bs58";
import dotenv from "dotenv";
import path from "path";
import { appendFile, readFile, writeFile } from "fs/promises";

dotenv.config({ path: path.join(process.cwd(), "..", ".env") });

const ENV_PATH = path.join(process.cwd(), "..", ".env");

const DECIMALS = 6;
const HOT_MINT_AMOUNT = 10_000_000; // 10 tokens (6 decimals) for hot path
const COLD_MINT_AMOUNT = 5_000_000; // 5 tokens (6 decimals) for cold path

function getEnvOrThrow(key: string): string {
  const val = process.env[key];
  if (!val) throw new Error(`Missing env var: ${key}`);
  return val;
}

function keypairFromEnvOrGenerate(key: string): Keypair {
  if (process.env[key]) {
    return Keypair.fromSecretKey(bs58.decode(process.env[key]!));
  }
  return Keypair.generate();
}

async function appendEnvVar(name: string, value: string, comment?: string) {
  const line = comment
    ? `\n# ${comment}\n${name}=${value}\n`
    : `\n${name}=${value}\n`;
  await appendFile(ENV_PATH, line);
  process.env[name] = value;
}

async function sleep(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

const AIRDROP_AMOUNT = 50_000_000; // 0.05 SOL — enough for setup txs + rent

async function airdropIfNeeded(
  connection: Connection,
  pubkey: PublicKey,
  label: string,
  minLamports = 10_000_000 // 0.01 SOL minimum
) {
  const balance = await connection.getBalance(pubkey);
  if (balance >= minLamports) {
    console.log(
      `  ${label}: ${balance / LAMPORTS_PER_SOL} SOL (sufficient)`
    );
    return;
  }

  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      console.log(
        `  Airdropping 0.05 SOL to ${label} (attempt ${attempt}/3)...`
      );
      const sig = await connection.requestAirdrop(pubkey, AIRDROP_AMOUNT);
      await connection.confirmTransaction(sig, "confirmed");
      console.log(`  Airdrop confirmed: ${sig}`);
      return;
    } catch (e: any) {
      if (attempt < 3) {
        const waitMs = attempt * 5000;
        console.log(`  Airdrop failed, retrying in ${waitMs / 1000}s...`);
        await sleep(waitMs);
      } else {
        throw new Error(
          `Failed to airdrop to ${label} after 3 attempts: ${e.message}\n` +
            `  Fund manually: solana airdrop 0.05 ${pubkey.toBase58()} --url devnet`
        );
      }
    }
  }
}

async function main() {
  console.log("=== Light-Token Devnet Setup (SPL + Wrap Flow) ===\n");

  const zkRpcUrl = getEnvOrThrow("ZK_COMPRESSION_RPC_URL");
  // Use Helius for everything (the public devnet endpoint rate-limits airdrops)
  const solanaRpcUrl = process.env.SOLANA_RPC_URL || zkRpcUrl;

  const connection = new Connection(solanaRpcUrl, "confirmed");
  const rpc = new Rpc(zkRpcUrl, zkRpcUrl, zkRpcUrl);

  console.log("RPC:", solanaRpcUrl);
  console.log("ZK RPC:", zkRpcUrl);

  // --- Keypairs ---
  const payer = keypairFromEnvOrGenerate("SETUP_PAYER_KEYPAIR");
  if (!process.env.SETUP_PAYER_KEYPAIR) {
    await appendEnvVar(
      "SETUP_PAYER_KEYPAIR",
      bs58.encode(payer.secretKey),
      `Setup payer: ${payer.publicKey.toBase58()}`
    );
  }

  const sender = keypairFromEnvOrGenerate("TEST_SENDER_KEYPAIR");
  if (!process.env.TEST_SENDER_KEYPAIR) {
    await appendEnvVar(
      "TEST_SENDER_KEYPAIR",
      bs58.encode(sender.secretKey),
      `Test sender: ${sender.publicKey.toBase58()}`
    );
  }

  const destination = keypairFromEnvOrGenerate("DESTINATION_KEYPAIR");
  if (!process.env.DESTINATION_KEYPAIR) {
    await appendEnvVar(
      "DESTINATION_KEYPAIR",
      bs58.encode(destination.secretKey),
      `Destination: ${destination.publicKey.toBase58()}`
    );
  }

  const mintAuthority = keypairFromEnvOrGenerate("MINT_AUTHORITY");
  if (!process.env.MINT_AUTHORITY) {
    await appendEnvVar(
      "MINT_AUTHORITY",
      bs58.encode(mintAuthority.secretKey),
      `Mint authority: ${mintAuthority.publicKey.toBase58()}`
    );
  }

  // Kora fee payer (used by the Kora server to pay SOL fees)
  const koraFeePayer = keypairFromEnvOrGenerate("KORA_PRIVATE_KEY");
  if (!process.env.KORA_PRIVATE_KEY) {
    await appendEnvVar(
      "KORA_PRIVATE_KEY",
      bs58.encode(koraFeePayer.secretKey),
      `Kora fee payer: ${koraFeePayer.publicKey.toBase58()}`
    );
  }

  console.log("\nKeypairs:");
  console.log("  Payer:", payer.publicKey.toBase58());
  console.log("  Sender:", sender.publicKey.toBase58());
  console.log("  Destination:", destination.publicKey.toBase58());
  console.log("  Mint authority:", mintAuthority.publicKey.toBase58());
  console.log("  Kora fee payer:", koraFeePayer.publicKey.toBase58());

  // --- Fund accounts ---
  // Only payer and kora fee payer need SOL. Sender just co-signs (Kora pays fees).
  console.log("\n--- Fund accounts ---");
  await airdropIfNeeded(connection, payer.publicKey, "Payer");
  await airdropIfNeeded(connection, koraFeePayer.publicKey, "Kora fee payer");

  // --- Step 1: Create standard SPL mint ---
  console.log("\n--- Step 1: Create SPL mint ---");
  let mintAddress: PublicKey;

  if (process.env.LIGHT_TOKEN_MINT) {
    mintAddress = new PublicKey(process.env.LIGHT_TOKEN_MINT);
    console.log("  Using existing mint:", mintAddress.toBase58());
  } else {
    console.log("  Creating new SPL mint...");
    mintAddress = await createMint(
      connection,
      payer,
      mintAuthority.publicKey,
      null, // no freeze authority
      DECIMALS
    );
    console.log("  SPL mint created:", mintAddress.toBase58());

    await appendEnvVar(
      "LIGHT_TOKEN_MINT",
      mintAddress.toBase58(),
      `SPL mint registered with Light Token Program (${DECIMALS} decimals)`
    );
  }

  // --- Step 2: Create sender's SPL ATA and mint SPL tokens ---
  console.log("\n--- Step 2: Mint SPL tokens to sender ---");
  const senderSplAta = await createAssociatedTokenAccount(
    connection,
    payer,
    mintAddress,
    sender.publicKey
  ).catch(async () => {
    // ATA may already exist
    return getAssociatedTokenAddressSync(mintAddress, sender.publicKey);
  });
  console.log("  Sender SPL ATA:", senderSplAta.toBase58());

  const totalSplAmount = HOT_MINT_AMOUNT + COLD_MINT_AMOUNT;
  const mintSig = await mintTo(
    connection,
    payer,
    mintAddress,
    senderSplAta,
    mintAuthority,
    totalSplAmount
  );
  console.log(
    `  Minted ${totalSplAmount / 10 ** DECIMALS} SPL tokens to sender`
  );
  console.log("  Tx:", mintSig);

  // --- Step 3: Register SPL mint with Light Token Program ---
  console.log("\n--- Step 3: Register SPL mint with Light Token Program ---");
  try {
    const registerSig = await createSplInterface(rpc, payer, mintAddress);
    console.log("  SPL interface registered, tx:", registerSig);
  } catch (e: any) {
    if (e.message?.includes("already in use") || e.message?.includes("0x0")) {
      console.log("  SPL interface already registered (skipping)");
    } else {
      throw e;
    }
  }

  // --- Step 4: Create light-token ATAs ---
  console.log("\n--- Step 4: Create light-token ATAs ---");

  console.log("  Creating sender light-token ATA...");
  const senderLightAta = await createAtaInterfaceIdempotent(
    rpc,
    payer,
    mintAddress,
    sender.publicKey
  );
  console.log("  Sender light-token ATA:", senderLightAta.toBase58());

  console.log("  Creating destination light-token ATA...");
  const destLightAta = await createAtaInterfaceIdempotent(
    rpc,
    payer,
    mintAddress,
    destination.publicKey
  );
  console.log("  Destination light-token ATA:", destLightAta.toBase58());

  // --- Step 5: Wrap SPL tokens → light-token ATA (hot state) ---
  console.log("\n--- Step 5: Wrap SPL tokens to light-token ATA (hot state) ---");
  const wrapSig = await wrap(
    rpc,
    payer,
    senderSplAta,
    senderLightAta,
    sender,
    mintAddress,
    BigInt(HOT_MINT_AMOUNT)
  );
  console.log(
    `  Wrapped ${HOT_MINT_AMOUNT / 10 ** DECIMALS} tokens to light-token ATA`
  );
  console.log("  Tx:", wrapSig);

  // --- Step 6: Compress SPL tokens (cold state) ---
  console.log("\n--- Step 6: Compress SPL tokens (cold state) ---");
  const compressSig = await compress(
    rpc,
    payer,
    mintAddress,
    COLD_MINT_AMOUNT,
    sender,
    senderSplAta,
    sender.publicKey
  );
  console.log(
    `  Compressed ${COLD_MINT_AMOUNT / 10 ** DECIMALS} tokens (cold/compressed)`
  );
  console.log("  Tx:", compressSig);

  // --- Step 7: Update kora.toml with actual mint address ---
  console.log("\n--- Step 7: Update kora.toml with mint address ---");
  const koraTomlPath = path.join(process.cwd(), "..", "server", "kora.toml");
  try {
    let tomlContent = await readFile(koraTomlPath, "utf-8");
    // Replace allowed_tokens and allowed_spl_paid_tokens mint addresses
    tomlContent = tomlContent.replace(
      /allowed_tokens\s*=\s*\[[\s\S]*?\]/,
      `allowed_tokens = [\n    "${mintAddress.toBase58()}", # SPL mint for light-token testing\n]`
    );
    tomlContent = tomlContent.replace(
      /allowed_spl_paid_tokens\s*=\s*\[[\s\S]*?\]/,
      `allowed_spl_paid_tokens = [\n    "${mintAddress.toBase58()}", # SPL mint for light-token testing\n]`
    );
    await writeFile(koraTomlPath, tomlContent);
    console.log("  Updated kora.toml with mint:", mintAddress.toBase58());
  } catch (e) {
    console.warn("  Could not update kora.toml:", e);
    console.warn("  Manually set allowed_tokens to:", mintAddress.toBase58());
  }

  // --- Summary ---
  console.log("\n=== Setup Complete ===");
  console.log(`  SPL Mint: ${mintAddress.toBase58()}`);
  console.log(`  Sender: ${sender.publicKey.toBase58()}`);
  console.log(`  Destination: ${destination.publicKey.toBase58()}`);
  console.log(`  Kora fee payer: ${koraFeePayer.publicKey.toBase58()}`);
  console.log(
    `  Hot balance (light-token ATA): ${HOT_MINT_AMOUNT / 10 ** DECIMALS} tokens`
  );
  console.log(
    `  Cold balance (compressed): ${COLD_MINT_AMOUNT / 10 ** DECIMALS} tokens`
  );
  console.log(`\nKeypairs saved to ${ENV_PATH}`);
  console.log(
    "Next: start Kora server with the demo kora.toml, then run test-server-transfer.ts"
  );
}

main().catch((e) => {
  console.error("Setup failed:", e);
  process.exit(1);
});
