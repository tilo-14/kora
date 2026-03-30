/**
 * This file is used to setup the client for the Kora project.
 * It creates the necessary keypairs and the mint account.
 * It airdrops SOL to a Test Sender and Kora Private Key.
 * It initializes a fake/local USDC mint account.
 * It creates the associated token accounts for the Test Sender, Kora Private Key, and Destination KeyPair.
 * It mints 100,000 tokens to the Test Sender, Kora Private Key, and Destination KeyPair.
 */
import { assertKeyGenerationIsAvailable } from "@solana/assertions";
import { getCreateAccountInstruction } from "@solana-program/system";
import {
    findAssociatedTokenPda,
    getCreateAssociatedTokenIdempotentInstructionAsync,
    getInitializeMintInstruction,
    getMintSize,
    getMintToInstruction,
    TOKEN_PROGRAM_ADDRESS,
} from "@solana-program/token";
import {
    airdropFactory,
    createSolanaRpc,
    createSolanaRpcSubscriptions,
    lamports,
    sendAndConfirmTransactionFactory,
    pipe,
    createTransactionMessage,
    setTransactionMessageLifetimeUsingBlockhash,
    setTransactionMessageFeePayerSigner,
    appendTransactionMessageInstructions,
    TransactionSigner,
    SolanaRpcApi,
    RpcSubscriptions,
    Rpc,
    SolanaRpcSubscriptionsApi,
    MicroLamports,
    Commitment,
    Signature,
    signTransactionMessageWithSigners,
    getSignatureFromTransaction,
    Instruction,
    createKeyPairSignerFromBytes,
    getBase58Decoder,
    getBase58Encoder,
    KeyPairSigner,
    TransactionMessage,
    assertIsTransactionWithBlockhashLifetime,
    TransactionMessageWithSigners,
    TransactionMessageWithFeePayer,
} from "@solana/kit";
import {
    updateOrAppendSetComputeUnitLimitInstruction,
    updateOrAppendSetComputeUnitPriceInstruction,
    MAX_COMPUTE_UNIT_LIMIT,
    estimateComputeUnitLimitFactory
} from "@solana-program/compute-budget";
import {
    Keypair as Web3Keypair,
    PublicKey,
} from "@solana/web3.js";
import { createRpc, buildAndSignTx, sendAndConfirmTx } from "@lightprotocol/stateless.js";
import {
    createSplInterfaceInstruction,
    createAtaInstructions,
    getAtaAddress,
    createWrapInstruction,
    getSplInterfaces,
} from "@lightprotocol/token-interface";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { appendFile } from 'fs/promises';
import path from "path";
import dotenv from "dotenv";

dotenv.config({path: path.join(process.cwd(), '..', '.env')});

const LAMPORTS_PER_SOL = BigInt(1_000_000_000);
const DECIMALS = 6;
const DROP_AMOUNT = 100_000;

interface Client {
    rpc: Rpc<SolanaRpcApi>;
    rpcSubscriptions: RpcSubscriptions<SolanaRpcSubscriptionsApi>;
}

export const createDefaultTransaction = async (
    client: Client,
    feePayer: TransactionSigner,
    computeLimit: number = MAX_COMPUTE_UNIT_LIMIT,
    feeMicroLamports: MicroLamports = 1n as MicroLamports
): Promise<TransactionMessage & TransactionMessageWithFeePayer & TransactionMessageWithSigners> => {
    const { value: latestBlockhash } = await client.rpc
        .getLatestBlockhash()
        .send();
    return pipe(
        createTransactionMessage({ version: 0 }),
        (tx) => setTransactionMessageFeePayerSigner(feePayer, tx),
        (tx) => setTransactionMessageLifetimeUsingBlockhash(latestBlockhash, tx),
        (tx) => updateOrAppendSetComputeUnitPriceInstruction(feeMicroLamports, tx),
        (tx) => updateOrAppendSetComputeUnitLimitInstruction(computeLimit, tx),
    );
};

export const signAndSendTransaction = async (
    client: Client,
    transactionMessage: TransactionMessage & TransactionMessageWithFeePayer & TransactionMessageWithSigners,
    commitment: Commitment = 'confirmed'
) => {
    const signedTransaction =
        await signTransactionMessageWithSigners(transactionMessage);
    const signature = getSignatureFromTransaction(signedTransaction);
    assertIsTransactionWithBlockhashLifetime(signedTransaction);
    await sendAndConfirmTransactionFactory(client)(signedTransaction, {
        commitment,
    });
    return signature;
};


async function sendAndConfirmInstructions(
    client: Client,
    payer: TransactionSigner,
    instructions: Instruction[],
    description: string
): Promise<Signature> {
    try {
        const simulationTx = await pipe(
            await createDefaultTransaction(client, payer),
            (tx) => appendTransactionMessageInstructions(instructions, tx),
        );
        const estimateCompute = estimateComputeUnitLimitFactory({ rpc: client.rpc });
        const computeUnitLimit = await estimateCompute(simulationTx);
        const signature = await pipe(
            await createDefaultTransaction(client, payer, computeUnitLimit),
            (tx) => appendTransactionMessageInstructions(instructions, tx),
            (tx) => signAndSendTransaction(client, tx)
        );
        console.log(`    - ${description} - Signature: ${signature}`);

        return signature;
    } catch (error) {
        throw new Error(`Failed to ${description.toLowerCase()}: ${error instanceof Error ? error.message : 'Unknown error'}`);
    }
}

async function createB58SecretKey(): Promise<string> {
    await assertKeyGenerationIsAvailable();
    const base58Decoder = getBase58Decoder();
    // Create keypair with exportable private key
    // For demo purposes only
    const keyPair = await crypto.subtle.generateKey(
        "Ed25519",  // Algorithm. Native implementation status: https://github.com/WICG/webcrypto-secure-curves/issues/20
        true,       // Allows the private key to be exported (eg for saving it to a file) - public key is always extractable see https://wicg.github.io/webcrypto-secure-curves/#ed25519-operations
        ["sign", "verify"], // Allowed uses
    );

    // Get the raw 32-byte private key
    const pkcs8ArrayBuffer = await crypto.subtle.exportKey("pkcs8", keyPair.privateKey);
    const pkcs8Bytes = new Uint8Array(pkcs8ArrayBuffer);
    const rawPrivateKey = pkcs8Bytes.slice(-32);

    // Get the 32-byte public key
    const publicKeyArrayBuffer = await crypto.subtle.exportKey("raw", keyPair.publicKey);
    const publicKeyBytes = new Uint8Array(publicKeyArrayBuffer);

    // Create Solana-style 64-byte secret key (private + public)
    const solanaSecretKey = new Uint8Array(64);
    solanaSecretKey.set(rawPrivateKey, 0);     // First 32 bytes
    solanaSecretKey.set(publicKeyBytes, 32);   // Next 32 bytes

    const b58Secret = base58Decoder.decode(solanaSecretKey)

    return b58Secret;
}

const createKeyPairSignerFromB58Secret = async (b58Secret: string) => {
    const base58Encoder = getBase58Encoder();
    const b58SecretEncoded = base58Encoder.encode(b58Secret);
    return await createKeyPairSignerFromBytes(b58SecretEncoded);
}

const addKeypairToEnvFile = async (
    variableName: string,
    envPath: string = path.join(process.cwd(), '..'),
    envFileName: string = ".env",
    b58Secret?: string,
) => {

    if (!b58Secret) {
        b58Secret = await createB58SecretKey();
    }

    const keypairSigner = await createKeyPairSignerFromB58Secret(b58Secret);

    const fullPath = path.join(envPath, envFileName);
    try {
        await appendFile(
            fullPath,
            `\n# Solana Address: ${keypairSigner.address}\n${variableName}=${b58Secret}\n`,
        );
        console.log(`${variableName} added to env file successfully`);
        return keypairSigner;
    } catch (e) {
        throw e;
    }
};


async function initializeToken({
    client,
    mintAuthority,
    payer,
    owner,
    mint,
    dropAmount,
    decimals,
    otherAtaWallets,
}: {
    client: Client,
    mintAuthority: KeyPairSigner<string>,
    payer: KeyPairSigner<string>,
    owner: KeyPairSigner<string>,
    mint: KeyPairSigner<string>,
    dropAmount: number,
    decimals: number,
    otherAtaWallets?: KeyPairSigner<string>[],
}) {
    // Get Owner ATA
    const [ata] = await findAssociatedTokenPda({
        mint: mint.address,
        owner: owner.address,
        tokenProgram: TOKEN_PROGRAM_ADDRESS,
    });

    // Get Mint size & rent
    const mintSpace = BigInt(getMintSize());
    const mintRent = await client.rpc.getMinimumBalanceForRentExemption(mintSpace).send();

    // Create instructions for new token mint
    const baseInstructions = [
        // Create the Mint Account
        getCreateAccountInstruction({
            payer,
            newAccount: mint,
            lamports: mintRent,
            space: mintSpace,
            programAddress: TOKEN_PROGRAM_ADDRESS,
        }),
        // Initialize the Mint
        getInitializeMintInstruction({
            mint: mint.address,
            decimals,
            mintAuthority: mintAuthority.address
        }),
        // Create Associated Token Account
        await getCreateAssociatedTokenIdempotentInstructionAsync({
            mint: mint.address,
            payer,
            owner: owner.address,
        }),
        // Mint To the Destination Associated Token Account
        getMintToInstruction({
            mint: mint.address,
            token: ata,
            amount: BigInt(dropAmount * 10 ** decimals),
            mintAuthority,
        }),
    ];

    // Generate Create ATA instructions for other token accounts we wish to add
    const otherAtaInstructions = otherAtaWallets 
        ? await Promise.all(otherAtaWallets.map(async (wallet) => 
            await getCreateAssociatedTokenIdempotentInstructionAsync({
                mint: mint.address,
                payer,
                owner: wallet.address,
            })
        ))
        : [];

    const instructions = [...baseInstructions, ...otherAtaInstructions];

    await sendAndConfirmInstructions(client, payer, instructions, 'Mint account created and initialized');

    console.log(`Initialized token ${mint.address} / Dropped ${dropAmount} tokens to ${owner.address}`);

}

async function getOrCreateEnvKeyPair(envKey: string) {
    if (process.env[envKey]) {
        return await createKeyPairSignerFromB58Secret(process.env[envKey]);
    }
    return await addKeypairToEnvFile(envKey);
}

/** Convert a kit KeyPairSigner env var to a web3.js v1 Keypair. */
function getWeb3Keypair(envKey: string): Web3Keypair {
    const base58Encoder = getBase58Encoder();
    return Web3Keypair.fromSecretKey(new Uint8Array(base58Encoder.encode(process.env[envKey]!)));
}

async function main() {
    console.log('Starting setup...');
    // 1 - Create client
    const httpEndpoint = 'http://127.0.0.1:8899';
    const wsEndpoint = 'ws://127.0.0.1:8900';
    const rpc = createSolanaRpc(httpEndpoint);
    const rpcSubscriptions = createSolanaRpcSubscriptions(wsEndpoint);
    const airdrop = airdropFactory({ rpc, rpcSubscriptions });
    const client: Client = { rpc, rpcSubscriptions };

    // 2 - Get or create keypairs
    const USDC_LOCAL_KEY = await getOrCreateEnvKeyPair('USDC_LOCAL_KEY');
    const TEST_SENDER_KEYPAIR = await getOrCreateEnvKeyPair('TEST_SENDER_KEYPAIR');
    const KORA_PRIVATE_KEY = await getOrCreateEnvKeyPair('KORA_PRIVATE_KEY');
    const MINT_AUTHORITY = await getOrCreateEnvKeyPair('MINT_AUTHORITY');
    const DESTINATION_KEYPAIR = await getOrCreateEnvKeyPair('DESTINATION_KEYPAIR');

    // 3 - Airdrop SOL to test sender and kora wallets
    await Promise.all([
        airdrop({
            commitment: 'processed',
            lamports: lamports(LAMPORTS_PER_SOL),
            recipientAddress: KORA_PRIVATE_KEY.address
        }),
        airdrop({
            commitment: 'processed',
            lamports: lamports(LAMPORTS_PER_SOL),
            recipientAddress: TEST_SENDER_KEYPAIR.address
        }),
    ])
    
    // 4 - Execute initializeToken
    await initializeToken({
        client,
        mintAuthority: MINT_AUTHORITY,
        payer: KORA_PRIVATE_KEY,
        owner: TEST_SENDER_KEYPAIR,
        mint: USDC_LOCAL_KEY,
        dropAmount: DROP_AMOUNT,
        decimals: DECIMALS,
        otherAtaWallets: [TEST_SENDER_KEYPAIR, KORA_PRIVATE_KEY, DESTINATION_KEYPAIR],
    })

    // 5 - Register SPL mint with Light Token Program and wrap tokens
    // Must use an RPC provider that supports ZK compression, such as Helius or Triton
    const lightRpc = createRpc(httpEndpoint);
    const web3Payer = getWeb3Keypair('KORA_PRIVATE_KEY');
    const web3Sender = getWeb3Keypair('TEST_SENDER_KEYPAIR');
    const mintPubkey = new PublicKey(USDC_LOCAL_KEY.address);

    // Register existing SPL mint with Light Token Program (one-time)
    const registerIx = createSplInterfaceInstruction({
        feePayer: web3Payer.publicKey,
        mint: mintPubkey,
        index: 0,
        tokenProgramId: TOKEN_PROGRAM_ID,
    });
    const { blockhash: regBlockhash } = await lightRpc.getLatestBlockhash();
    const registerTx = buildAndSignTx([registerIx], web3Payer, regBlockhash);
    await sendAndConfirmTx(lightRpc, registerTx);
    console.log('  ✓ SPL mint registered with Light Token Program');

    // Create Light Token ATA for sender
    const ataIxs = await createAtaInstructions({
        owner: web3Sender.publicKey,
        mint: mintPubkey,
        payer: web3Payer.publicKey,
    });
    const senderLightAta = getAtaAddress({ owner: web3Sender.publicKey, mint: mintPubkey });
    const { blockhash: ataBlockhash } = await lightRpc.getLatestBlockhash();
    const ataTx = buildAndSignTx(ataIxs, web3Payer, ataBlockhash);
    await sendAndConfirmTx(lightRpc, ataTx);
    console.log('  ✓ Light Token ATA created for sender');

    // Wrap SPL tokens into Light Token
    const { findAssociatedTokenPda: findSplAta } = await import("@solana-program/token");
    const [senderSplAta] = await findSplAta({
        mint: USDC_LOCAL_KEY.address,
        owner: TEST_SENDER_KEYPAIR.address,
        tokenProgram: TOKEN_PROGRAM_ADDRESS,
    });

    const splInterfaces = await getSplInterfaces(lightRpc, mintPubkey);
    const activeInterface = splInterfaces.find(i => i.isInitialized);
    if (!activeInterface) throw new Error("No initialized SPL interface found");

    const wrapAmount = BigInt((DROP_AMOUNT / 2) * 10 ** DECIMALS);
    const wrapIx = createWrapInstruction({
        source: new PublicKey(senderSplAta),
        destination: senderLightAta,
        owner: web3Sender.publicKey,
        mint: mintPubkey,
        amount: wrapAmount,
        splInterface: activeInterface,
        decimals: DECIMALS,
        payer: web3Payer.publicKey,
    });
    const { blockhash: wrapBlockhash } = await lightRpc.getLatestBlockhash();
    const wrapTx = buildAndSignTx([wrapIx], web3Payer, wrapBlockhash, [web3Sender]);
    await sendAndConfirmTx(lightRpc, wrapTx);
    console.log(`  ✓ Wrapped ${DROP_AMOUNT / 2} tokens into Light Token`);
}
main().catch(e => console.error('Error:', e));
