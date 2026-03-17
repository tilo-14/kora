/**
 * Provision Light Token state for integration tests.
 *
 * Runs against a local `light test-validator`. Creates:
 * 1. SPL mint (6 decimals)
 * 2. Register with Light Token Program
 * 3. Sender + destination Light Token ATAs
 * 4. Wrap tokens to sender's ATA (10 tokens hot)
 * 5. Compress tokens for sender (5 tokens cold)
 *
 * Outputs env vars as KEY=VALUE lines to stdout for the shell
 * orchestrator to capture.
 */
import {
  Keypair,
  PublicKey,
  Connection,
} from "@solana/web3.js";
import { Rpc } from "@lightprotocol/stateless.js";
import {
  createSplInterface,
  createAtaInterfaceIdempotent,
  wrap,
} from "@lightprotocol/compressed-token/unified";
import { compress } from "@lightprotocol/compressed-token";
import {
  createMint,
  mintTo,
  createAssociatedTokenAccount,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";

const RPC_URL = process.env.RPC_URL || "http://127.0.0.1:8899";
const ZK_RPC_URL = process.env.ZK_COMPRESSION_RPC_URL || "http://127.0.0.1:8784";
const DECIMALS = 6;
const HOT_AMOUNT = 10_000_000; // 10 tokens
const COLD_AMOUNT = 5_000_000; // 5 tokens

const LOCAL_KEYS_DIR = path.join(__dirname, "..", "src", "common", "local-keys");

function loadKeypair(filename: string): Keypair {
  const keyPath = path.join(LOCAL_KEYS_DIR, filename);
  const keyData = JSON.parse(fs.readFileSync(keyPath, "utf-8"));
  return Keypair.fromSecretKey(Uint8Array.from(keyData));
}

async function main() {
  const connection = new Connection(RPC_URL, "confirmed");
  const rpc = new Rpc(ZK_RPC_URL, ZK_RPC_URL, ZK_RPC_URL);

  // Load test keypairs (same ones used by existing Kora tests)
  const payer = loadKeypair("fee-payer-local.json");
  const sender = loadKeypair("sender-local.json");

  // Generate fresh keypairs for light-token specific accounts
  const destination = Keypair.generate();
  const mintAuthority = Keypair.generate();

  console.error("Payer:", payer.publicKey.toBase58());
  console.error("Sender:", sender.publicKey.toBase58());
  console.error("Destination:", destination.publicKey.toBase58());

  // Step 1: Create SPL mint
  console.error("Creating SPL mint...");
  const mintAddress = await createMint(
    connection,
    payer,
    mintAuthority.publicKey,
    null,
    DECIMALS,
  );
  console.error("Mint:", mintAddress.toBase58());

  // Step 2: Create sender SPL ATA and mint tokens
  console.error("Minting SPL tokens to sender...");
  const senderSplAta = await createAssociatedTokenAccount(
    connection,
    payer,
    mintAddress,
    sender.publicKey,
  );
  await mintTo(
    connection,
    payer,
    mintAddress,
    senderSplAta,
    mintAuthority,
    HOT_AMOUNT + COLD_AMOUNT,
  );

  // Step 3: Register SPL mint with Light Token Program
  console.error("Registering with Light Token Program...");
  try {
    await createSplInterface(rpc, payer, mintAddress);
  } catch (e: any) {
    if (e.message?.includes("already in use") || e.message?.includes("0x0")) {
      console.error("Already registered (skipping)");
    } else {
      throw e;
    }
  }

  // Step 4: Create Light Token ATAs
  console.error("Creating Light Token ATAs...");
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
  console.error("Sender Light ATA:", senderLightAta.toBase58());
  console.error("Dest Light ATA:", destLightAta.toBase58());

  // Step 5: Wrap tokens to Light Token ATA (hot state)
  console.error(`Wrapping ${HOT_AMOUNT / 10 ** DECIMALS} tokens (hot)...`);
  await wrap(
    rpc,
    payer,
    senderSplAta,
    senderLightAta,
    sender,
    mintAddress,
    BigInt(HOT_AMOUNT),
  );

  // Step 6: Compress tokens (cold state)
  console.error(`Compressing ${COLD_AMOUNT / 10 ** DECIMALS} tokens (cold)...`);
  await compress(
    rpc,
    payer,
    mintAddress,
    COLD_AMOUNT,
    sender,
    senderSplAta,
    sender.publicKey,
  );

  // Output env vars to stdout (captured by run-light-tests.sh)
  console.log(`LIGHT_TOKEN_MINT=${mintAddress.toBase58()}`);
  console.log(`LIGHT_DESTINATION_PUBKEY=${destination.publicKey.toBase58()}`);

  console.error("Light Token setup complete.");
}

main().catch((e) => {
  console.error("Setup failed:", e);
  process.exit(1);
});
