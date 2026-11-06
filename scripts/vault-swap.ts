/**
 * Devnet client: vault swaps W11A -> W11B through the Raydium CLMM pool from week 11.
 *
 * Prereq: program deployed (`solana program deploy`, needs ~3.5 devnet SOL),
 *         IDL at target/idl/liquidity_vault.json.
 * Run:    npx tsx scripts/vault-swap.ts [amount=20] [slippageBps=50]
 *
 * Flow: init vault (if needed) -> deposit W11A -> set_pool -> init W11B account
 *       -> off-chain quote (Raydium SDK) -> min_amount_out = quote * (1 - slippage)
 *       -> swap_via_raydium (vault PDA signs the CPI)
 */
import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { PoolUtils, Raydium } from "@raydium-io/raydium-sdk-v2";
import {
  getAccount,
  getAssociatedTokenAddressSync,
  getOrCreateAssociatedTokenAccount,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { ComputeBudgetProgram, Connection, Keypair, PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import fs from "fs";
import os from "os";
import path from "path";

const RPC_URL = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const POOL_ID = new PublicKey("8HUDR4pgizvYtgRdAE82RyyK8jTEqVNLi9c9xNFbvMmH");
const MEMO_PROGRAM_ID = new PublicKey("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const AMOUNT = Number(process.argv[2] ?? 20);
const SLIPPAGE_BPS = Number(process.argv[3] ?? 50);

const wallet = Keypair.fromSecretKey(
  Uint8Array.from(JSON.parse(fs.readFileSync(process.env.KEYPAIR ?? path.join(os.homedir(), ".config/solana/id.json"), "utf8")))
);

async function main() {
  const connection = new Connection(RPC_URL, "confirmed");
  const provider = new anchor.AnchorProvider(connection, new anchor.Wallet(wallet), { commitment: "confirmed" });
  const idl = JSON.parse(fs.readFileSync(path.join(__dirname, "../idl/liquidity_vault.json"), "utf8"));
  const program = new anchor.Program(idl, provider);
  const pid = program.programId;

  const raydium = await Raydium.load({ connection, owner: wallet, cluster: "devnet", disableFeatureCheck: true, disableLoadToken: true });
  const { poolInfo, poolKeys, computePoolInfo, tickData } = await raydium.clmm.getPoolInfoFromRpc(POOL_ID.toBase58());
  const mintA = new PublicKey(poolInfo.mintA.address); // W11A — vault underlying
  const mintB = new PublicKey(poolInfo.mintB.address); // W11B
  const ONE = 10 ** poolInfo.mintA.decimals;

  const [config] = PublicKey.findProgramAddressSync([Buffer.from("config"), mintA.toBuffer()], pid);
  const [shareMint] = PublicKey.findProgramAddressSync([Buffer.from("shares"), config.toBuffer()], pid);
  const [vaultA] = PublicKey.findProgramAddressSync([Buffer.from("vault"), config.toBuffer()], pid);
  const vaultB = getAssociatedTokenAddressSync(mintB, config, true);
  const [userPosition] = PublicKey.findProgramAddressSync([Buffer.from("user"), config.toBuffer(), wallet.publicKey.toBuffer()], pid);

  // ---- one-time setup ----
  if (!(await connection.getAccountInfo(config))) {
    await program.methods.initializeVault(50, new BN(0))
      .accountsPartial({ admin: wallet.publicKey, underlyingMint: mintA, config, shareMint, vaultTokenAccount: vaultA, tokenProgram: TOKEN_PROGRAM_ID, systemProgram: SystemProgram.programId, rent: SYSVAR_RENT_PUBKEY })
      .rpc();
    console.log("vault initialized:", config.toBase58());
  }
  const userA = await getOrCreateAssociatedTokenAccount(connection, wallet, mintA, wallet.publicKey);
  if (Number((await getAccount(connection, vaultA)).amount) < AMOUNT * ONE) {
    await program.methods.deposit(new BN(100 * ONE), new BN(0))
      .accountsPartial({ owner: wallet.publicKey, config, underlyingMint: mintA, shareMint, vaultTokenAccount: vaultA, userPosition, userTokenAccount: userA.address, userShareAccount: getAssociatedTokenAddressSync(shareMint, wallet.publicKey), tokenProgram: TOKEN_PROGRAM_ID })
      .rpc();
    console.log("deposited 100 W11A into the vault");
  }
  await program.methods.setPool(100).accountsPartial({ admin: wallet.publicKey, config, poolState: POOL_ID }).rpc();
  if (!(await connection.getAccountInfo(vaultB))) {
    await program.methods.initStrategyAccount()
      .accountsPartial({ admin: wallet.publicKey, config, mint: mintB, vaultAta: vaultB, tokenProgram: TOKEN_PROGRAM_ID })
      .rpc();
  }

  // ---- quote off-chain, then swap on-chain with min_amount_out ----
  const amountIn = new BN(AMOUNT * ONE);
  const epochInfo = await connection.getEpochInfo();
  const blockTimestamp = (await connection.getBlockTime(await connection.getSlot())) ?? Math.floor(Date.now() / 1000);
  const quote = PoolUtils.computeAmountOutFormat({
    poolInfo: computePoolInfo,
    tickarrayBitmapExtension: computePoolInfo.exBitmapInfo,
    tickArrayCache: tickData[POOL_ID.toBase58()],
    amountIn,
    tokenOut: poolInfo.mintB,
    slippage: SLIPPAGE_BPS / 10_000,
    epochInfo,
    blockTimestamp,
  });
  const minOut = quote.minAmountOut.amount.raw;
  console.log(`quote: ${AMOUNT} W11A -> ${quote.amountOut.amount.toExact()} W11B, min_amount_out ${minOut.toString()} (slippage ${SLIPPAGE_BPS} bps)`);

  const remaining = [new PublicKey(poolKeys.exBitmapAccount), ...quote.remainingAccounts].map((pubkey) => ({ pubkey, isSigner: false, isWritable: true }));
  const sig = await program.methods
    .swapViaRaydium(amountIn, minOut, new BN(0))
    .accountsPartial({
      admin: wallet.publicKey, config, clmmProgram: new PublicKey(poolKeys.programId), ammConfig: new PublicKey(poolKeys.config.id), poolState: POOL_ID,
      inputTokenAccount: vaultA, outputTokenAccount: vaultB, inputVault: new PublicKey(poolKeys.vault.A), outputVault: new PublicKey(poolKeys.vault.B),
      observationState: new PublicKey(poolKeys.observationId), tokenProgram: TOKEN_PROGRAM_ID, tokenProgram2022: TOKEN_2022_PROGRAM_ID, memoProgram: MEMO_PROGRAM_ID,
      inputMint: mintA, outputMint: mintB,
    })
    .remainingAccounts(remaining)
    .preInstructions([ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 })])
    .rpc();

  const received = Number((await getAccount(connection, vaultB)).amount);
  console.log(`vault W11B balance: ${received / ONE}`);
  console.log(`tx: https://explorer.solana.com/tx/${sig}?cluster=devnet`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
