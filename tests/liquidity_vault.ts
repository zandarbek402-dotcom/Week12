import * as anchor from "@coral-xyz/anchor";
import { Program, BN } from "@coral-xyz/anchor";
import {
  createMint,
  getOrCreateAssociatedTokenAccount,
  getAssociatedTokenAddressSync,
  mintTo,
  getAccount,
  getMint,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Keypair, LAMPORTS_PER_SOL, PublicKey } from "@solana/web3.js";
import { assert } from "chai";
import { LiquidityVault } from "../target/types/liquidity_vault";

describe("Week 10 — CPI & withdraw from vault", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.LiquidityVault as Program<LiquidityVault>;
  const conn = provider.connection;
  const admin = provider.wallet as anchor.Wallet;
  const payer = admin.payer;

  const DECIMALS = 6;
  const ONE = 10 ** DECIMALS;

  let underlyingMint: PublicKey;
  let config: PublicKey;
  let shareMint: PublicKey;
  let vaultTokenAccount: PublicKey;

  type User = { kp: Keypair; position: PublicKey; tokens: PublicKey; shares: PublicKey };
  const bobKp = Keypair.generate();
  let alice: User;
  let bob: User;

  const pda = (seeds: Buffer[]) => PublicKey.findProgramAddressSync(seeds, program.programId)[0];
  const balance = async (acc: PublicKey) => Number((await getAccount(conn, acc)).amount);
  const supply = async () => Number((await getMint(conn, shareMint)).supply);
  const signers = (u: User) => (u.kp === payer ? [] : [u.kp]);

  async function makeUser(kp: Keypair, tokens: number): Promise<User> {
    const ata = await getOrCreateAssociatedTokenAccount(conn, payer, underlyingMint, kp.publicKey);
    await mintTo(conn, payer, underlyingMint, ata.address, payer, tokens * ONE);
    return {
      kp,
      tokens: ata.address,
      position: pda([Buffer.from("user"), config.toBuffer(), kp.publicKey.toBuffer()]),
      shares: getAssociatedTokenAddressSync(shareMint, kp.publicKey),
    };
  }

  const deposit = (u: User, amount: number) =>
    program.methods
      .deposit(new BN(amount), new BN(0))
      .accountsPartial({
        owner: u.kp.publicKey, config, underlyingMint, shareMint, vaultTokenAccount,
        userPosition: u.position, userTokenAccount: u.tokens, userShareAccount: u.shares,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers(signers(u))
      .rpc();

  const withdraw = (u: User, shares: number, minOut = 0, override: Record<string, PublicKey> = {}) =>
    program.methods
      .withdraw(new BN(shares), new BN(minOut))
      .accountsPartial({
        owner: u.kp.publicKey, config, underlyingMint, shareMint, vaultTokenAccount,
        userPosition: u.position, userTokenAccount: u.tokens, userShareAccount: u.shares,
        tokenProgram: TOKEN_PROGRAM_ID,
        ...override,
      })
      .signers(signers(u));

  const previewWithdraw = async (shares: number) =>
    ((await program.methods
      .previewWithdraw(new BN(shares))
      .accountsPartial({ config, shareMint, vaultTokenAccount })
      .view()) as BN).toNumber();

  async function expectError(p: Promise<any>, ...codes: string[]) {
    try {
      await p;
      assert.fail(`expected one of ${codes.join(", ")}`);
    } catch (e: any) {
      const s = e.toString() + JSON.stringify(e.logs ?? []);
      assert.ok(codes.some((c) => s.includes(c)), `got: ${s.slice(0, 300)}`);
    }
  }

  before(async () => {
    underlyingMint = await createMint(conn, payer, admin.publicKey, null, DECIMALS);
    config = pda([Buffer.from("config"), underlyingMint.toBuffer()]);
    shareMint = pda([Buffer.from("shares"), config.toBuffer()]);
    vaultTokenAccount = pda([Buffer.from("vault"), config.toBuffer()]);

    const sig = await conn.requestAirdrop(bobKp.publicKey, 2 * LAMPORTS_PER_SOL);
    await conn.confirmTransaction(sig, "confirmed");

    await program.methods
      .initializeVault(50, new BN(0)) // 0.5% withdraw fee, no cap
      .accountsPartial({
        admin: admin.publicKey, underlyingMint, config, shareMint, vaultTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    alice = await makeUser(payer, 1_000);
    bob = await makeUser(bobKp, 1_000);

    await deposit(alice, 100 * ONE);
    await deposit(bob, 100 * ONE);
    // vault earns yield: 300 assets / 200 shares -> share price 1.5
    await mintTo(conn, payer, underlyingMint, vaultTokenAccount, payer, 100 * ONE);
  });

  it("vault token account is owned by the config PDA (not by any wallet)", async () => {
    const vault = await getAccount(conn, vaultTokenAccount);
    assert.ok(vault.owner.equals(config));
    assert.equal(Number(vault.amount), 300 * ONE);
  });

  it("preview_withdraw shows tokens after fee", async () => {
    const out = await previewWithdraw(40 * ONE);
    // 40 shares * 1.5 = 60 tokens, minus 0.5% fee = 59.7
    assert.approximately(out, 59.7 * ONE, 2);
  });

  it("withdraw: shares are burned (CPI burn) and tokens come back (CPI transfer, PDA signs)", async () => {
    const expected = await previewWithdraw(40 * ONE);
    const tokensBefore = await balance(alice.tokens);
    const vaultBefore = await balance(vaultTokenAccount);
    const supplyBefore = await supply();

    const sig = await withdraw(alice, 40 * ONE, expected).rpc({ commitment: "confirmed" });

    // shares burned
    assert.equal(await balance(alice.shares), 60 * ONE);
    assert.equal(await supply(), supplyBefore - 40 * ONE);
    // tokens returned exactly as previewed
    assert.equal((await balance(alice.tokens)) - tokensBefore, expected);
    // fee stayed in the vault
    assert.equal(await balance(vaultTokenAccount), vaultBefore - expected);

    const pos = await program.account.userPosition.fetch(alice.position);
    assert.equal(pos.shares.toNumber(), 60 * ONE);
    assert.equal(pos.deposited.toNumber(), 60 * ONE); // cost basis reduced by 40%

    const tx = await conn.getTransaction(sig, { commitment: "confirmed", maxSupportedTransactionVersion: 0 });
    const logs = tx!.meta!.logMessages!;
    // both CPIs into the Token program are visible in the logs
    const tokenCalls = logs.filter((l) => l.startsWith(`Program ${TOKEN_PROGRAM_ID.toBase58()} invoke [2]`));
    assert.equal(tokenCalls.length, 2, "burn + transfer_checked CPIs");

    const parser = new anchor.EventParser(program.programId, program.coder);
    const ev = [...parser.parseLogs(logs)].find((e) => /withdrawEvent/i.test(e.name));
    assert.ok(ev, "WithdrawEvent emitted");
    assert.equal(ev!.data.amount.toNumber(), expected);
    assert.equal(ev!.data.sharesLeft.toNumber(), 60 * ONE);
  });

  it("slippage: fails when tokens out < min_assets_out", async () => {
    const expected = await previewWithdraw(10 * ONE);
    await expectError(withdraw(alice, 10 * ONE, expected + 1).rpc(), "SlippageExceeded");
  });

  it("cannot burn more shares than owned", async () => {
    await expectError(withdraw(alice, 61 * ONE).rpc(), "InsufficientShares");
  });

  it("rejects zero shares", async () => {
    await expectError(withdraw(alice, 0).rpc(), "ZeroAmount");
  });

  it("Bob cannot withdraw using Alice's position (PDA seeds include owner)", async () => {
    await expectError(
      withdraw(bob, 10 * ONE, 0, { userPosition: alice.position }).rpc(),
      "ConstraintSeeds", "ConstraintHasOne"
    );
  });

  it("cannot substitute a fake vault token account", async () => {
    await expectError(
      withdraw(bob, 10 * ONE, 0, { vaultTokenAccount: bob.tokens }).rpc(),
      "ConstraintHasOne", "ConstraintSeeds", "ConstraintTokenOwner"
    );
  });

  it("withdraw still works while the vault is paused (funds never locked)", async () => {
    await program.methods.updateConfig(null, null, true)
      .accountsPartial({ admin: admin.publicKey, config }).rpc();

    await withdraw(bob, 10 * ONE).rpc();
    assert.equal(await balance(bob.shares), 90 * ONE);

    await program.methods.updateConfig(null, null, false)
      .accountsPartial({ admin: admin.publicKey, config }).rpc();
  });

  it("full withdraw: all shares burned, user gets their whole part", async () => {
    const expected = await previewWithdraw(90 * ONE);
    const before = await balance(bob.tokens);
    await withdraw(bob, 90 * ONE, expected).rpc();

    assert.equal(await balance(bob.shares), 0);
    assert.equal((await balance(bob.tokens)) - before, expected);
    const pos = await program.account.userPosition.fetch(bob.position);
    assert.equal(pos.shares.toNumber(), 0);
  });

  it("cannot close a position that still has shares", async () => {
    await expectError(
      program.methods.closePosition()
        .accountsPartial({ owner: admin.publicKey, config, userPosition: alice.position }).rpc(),
      "PositionNotEmpty"
    );
  });

  it("empty position can be closed and rent is returned", async () => {
    const lamportsBefore = await conn.getBalance(bobKp.publicKey);
    await program.methods.closePosition()
      .accountsPartial({ owner: bobKp.publicKey, config, userPosition: bob.position })
      .signers([bobKp])
      .rpc();
    assert.isNull(await conn.getAccountInfo(bob.position));
    assert.isAbove(await conn.getBalance(bobKp.publicKey), lamportsBefore - 10_000);
  });

  it("vault stays solvent: remaining shares are fully backed", async () => {
    const remaining = await supply(); // only Alice's 60 shares
    assert.equal(remaining, 60 * ONE);
    const value = await previewWithdraw(remaining);
    assert.isAtMost(value, await balance(vaultTokenAccount));
  });
});
