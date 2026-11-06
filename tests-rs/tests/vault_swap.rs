//! Week 12 — Swap through Raydium: end-to-end test.
//!
//! Runs in-process (`solana-program-test`) with:
//!   * our program            fixtures/liquidity_vault.so   (cargo build-sbf)
//!   * Raydium CLMM program   fixtures/raydium_clmm.so      (`solana program dump`, devnet)
//!   * the REAL W11A/W11B CLMM pool from week 11, dumped from devnet:
//!     PoolState, AmmConfig, vaults, observation, tick-array bitmap, tick arrays, mints
//!
//! Run:  cargo test -- --nocapture      (inside tests-rs/)

use std::{path::PathBuf, str::FromStr};

use anchor_lang::{InstructionData, ToAccountMetas};
use base64::Engine;
use liquidity_vault::raydium::{spot_amount_out, MEMO_PROGRAM_ID, RAYDIUM_CLMM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID};
use solana_program_test::{BanksClient, ProgramTest};
use solana_sdk::{
    account::Account,
    compute_budget::ComputeBudgetInstruction,
    instruction::{AccountMeta, Instruction, InstructionError},
    program_pack::Pack,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_program,
    transaction::{Transaction, TransactionError},
};

const POOL: &str = "8HUDR4pgizvYtgRdAE82RyyK8jTEqVNLi9c9xNFbvMmH";
const AMM_CONFIG: &str = "CD4aJtX11cqTCAc83nxSPkkh5JW2yjD6uwHeovjqQ1qu";
const MINT_A: &str = "G8hGd72yEJcCisWAEwewga7uFzXwvxwmHtRbDqDiw2D1"; // W11A = vault underlying
const MINT_B: &str = "GYYZDRWdqCexjk9o5aWA7Fdtm1uQio47Mu5KehdrUgLG"; // W11B
const POOL_VAULT_A: &str = "2yA2g5NDooPDkVtAjvJirndub3T6VzsEHkNGqLMi6dPs";
const POOL_VAULT_B: &str = "58oRgZrqF8kzg1WDE6yJi7LRuktJf5QDy91MzUWRG1Wg";
const OBSERVATION: &str = "CLAch37inT7YFau2sS52HhnEMyftq54u4pnmu2jeKiuH";
const EX_BITMAP: &str = "cu8CKR2eSnqjfvNcBpEUV6fLBSUhAfKy4zVQ3RkYJKF";
const TICK_ARRAY_3600: &str = "5FSzBUT4dj7EPu6CpgxZiDuP6CSMdDd2M8Y39ZNBaMzk"; // ticks 3600..7199 (current 6888)
const TICK_ARRAY_7200: &str = "A9zj5b5YybVTbJr9iumyAL6Ris4WcPu7QyMaiz3uSU94"; // ticks 7200..10799

const ONE: u64 = 1_000_000; // 6 decimals

// VaultError codes (6000 + enum index)
const E_VAULT_PAUSED: u32 = 6001;
const E_UNAUTHORIZED: u32 = 6007;
const E_POOL_ACCOUNT_MISMATCH: u32 = 6013;
const E_MIN_OUT_TOO_LOW: u32 = 6015;

fn pk(s: &str) -> Pubkey {
    Pubkey::from_str(s).unwrap()
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Load an account dumped with `solana account <addr> --output json-compact`
fn load_account(addr: &str) -> Account {
    let path = fixtures().join("accounts").join(format!("{addr}.json"));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let a = &v["account"];
    Account {
        lamports: a["lamports"].as_u64().unwrap(),
        data: base64::engine::general_purpose::STANDARD.decode(a["data"][0].as_str().unwrap()).unwrap(),
        owner: pk(a["owner"].as_str().unwrap()),
        executable: a["executable"].as_bool().unwrap(),
        rent_epoch: 0,
    }
}

/// Mint dumped from devnet, but with the mint authority replaced by `authority`
/// (so the test can mint W11A without the real wallet key).
fn load_mint_with_authority(addr: &str, authority: &Pubkey) -> Account {
    let mut acc = load_account(addr);
    let mut mint = spl_token::state::Mint::unpack(&acc.data).unwrap();
    mint.mint_authority = solana_sdk::program_option::COption::Some(*authority);
    spl_token::state::Mint::pack(mint, &mut acc.data).unwrap();
    acc
}

struct Env {
    banks: BanksClient,
    admin: Keypair,
    stranger: Keypair,
    config: Pubkey,
    share_mint: Pubkey,
    vault_a: Pubkey, // vault PDA token account (W11A, underlying)
    vault_b: Pubkey, // ATA(config, W11B) — strategy account
}

struct TxResult {
    err: Option<TransactionError>,
    logs: Vec<String>,
}

impl TxResult {
    fn custom_code(&self) -> Option<u32> {
        match &self.err {
            Some(TransactionError::InstructionError(_, InstructionError::Custom(c))) => Some(*c),
            _ => None,
        }
    }
    fn logs_contain(&self, s: &str) -> bool {
        self.logs.iter().any(|l| l.contains(s))
    }
}

impl Env {
    async fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
        let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(600_000)];
        all.extend_from_slice(ixs);
        let bh = self.banks.get_latest_blockhash().await.unwrap();
        let tx = Transaction::new_signed_with_payer(&all, Some(&signers[0].pubkey()), signers, bh);
        let res = self.banks.process_transaction_with_metadata(tx).await.unwrap();
        TxResult {
            err: res.result.err(),
            logs: res.metadata.map(|m| m.log_messages).unwrap_or_default(),
        }
    }

    async fn ok(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
        let r = self.send(ixs, signers).await;
        if let Some(e) = &r.err {
            for l in &r.logs {
                println!("  {l}");
            }
            panic!("transaction failed: {e:?}");
        }
        r
    }

    async fn balance(&mut self, token_account: Pubkey) -> u64 {
        let acc = self.banks.get_account(token_account).await.unwrap().unwrap();
        spl_token::state::Account::unpack(&acc.data).unwrap().amount
    }

    async fn sqrt_price(&mut self) -> u128 {
        let acc = self.banks.get_account(pk(POOL)).await.unwrap().unwrap();
        u128::from_le_bytes(acc.data[253..269].try_into().unwrap())
    }

    async fn tick(&mut self) -> i32 {
        let acc = self.banks.get_account(pk(POOL)).await.unwrap().unwrap();
        i32::from_le_bytes(acc.data[269..273].try_into().unwrap())
    }

    async fn trade_fee_rate(&mut self) -> u64 {
        let acc = self.banks.get_account(pk(AMM_CONFIG)).await.unwrap().unwrap();
        u32::from_le_bytes(acc.data[47..51].try_into().unwrap()) as u64
    }

    /// Build `swap_via_raydium`. `a_to_b` = sell W11A (underlying) for W11B.
    fn swap_ix(&self, signer: Pubkey, a_to_b: bool, amount_in: u64, min_out: u64) -> Instruction {
        let (input_ta, output_ta, input_vault, output_vault, input_mint, output_mint) = if a_to_b {
            (self.vault_a, self.vault_b, pk(POOL_VAULT_A), pk(POOL_VAULT_B), pk(MINT_A), pk(MINT_B))
        } else {
            (self.vault_b, self.vault_a, pk(POOL_VAULT_B), pk(POOL_VAULT_A), pk(MINT_B), pk(MINT_A))
        };
        self.swap_ix_raw(signer, input_ta, output_ta, input_vault, output_vault, input_mint, output_mint, a_to_b, amount_in, min_out)
    }

    #[allow(clippy::too_many_arguments)]
    fn swap_ix_raw(
        &self, signer: Pubkey, input_ta: Pubkey, output_ta: Pubkey, input_vault: Pubkey, output_vault: Pubkey,
        input_mint: Pubkey, output_mint: Pubkey, a_to_b: bool, amount_in: u64, min_out: u64,
    ) -> Instruction {
        let mut accounts = liquidity_vault::accounts::SwapViaRaydium {
            admin: signer,
            config: self.config,
            clmm_program: RAYDIUM_CLMM_PROGRAM_ID,
            amm_config: pk(AMM_CONFIG),
            pool_state: pk(POOL),
            input_token_account: input_ta,
            output_token_account: output_ta,
            input_vault,
            output_vault,
            observation_state: pk(OBSERVATION),
            token_program: spl_token::ID,
            token_program_2022: TOKEN_2022_PROGRAM_ID,
            memo_program: MEMO_PROGRAM_ID,
            input_mint,
            output_mint,
        }
        .to_account_metas(None);
        // remaining accounts: bitmap extension, then tick arrays starting with the current one
        accounts.push(AccountMeta::new(pk(EX_BITMAP), false));
        accounts.push(AccountMeta::new(pk(TICK_ARRAY_3600), false));
        if !a_to_b {
            accounts.push(AccountMeta::new(pk(TICK_ARRAY_7200), false)); // price goes up
        }
        Instruction {
            program_id: liquidity_vault::ID,
            accounts,
            data: liquidity_vault::instruction::SwapViaRaydium {
                amount_in,
                min_amount_out: min_out,
                sqrt_price_limit_x64: 0,
            }
            .data(),
        }
    }

    async fn spot(&mut self, a_to_b: bool, amount_in: u64) -> u64 {
        let s = self.sqrt_price().await;
        let fee = self.trade_fee_rate().await;
        spot_amount_out(amount_in, s, a_to_b, fee).unwrap()
    }
}

/// Vault with 500 W11A deposited, Raydium pool attached, W11B strategy account created.
async fn setup() -> Env {
    std::env::set_var("SBF_OUT_DIR", fixtures());
    let mut pt = ProgramTest::default();
    pt.prefer_bpf(true);
    pt.add_program("liquidity_vault", liquidity_vault::ID, None);
    pt.add_program("raydium_clmm", RAYDIUM_CLMM_PROGRAM_ID, None);

    let admin = Keypair::new();
    let stranger = Keypair::new();
    for k in [&admin, &stranger] {
        pt.add_account(k.pubkey(), Account { lamports: 100_000_000_000, owner: system_program::ID, ..Account::default() });
    }
    for a in [POOL, AMM_CONFIG, POOL_VAULT_A, POOL_VAULT_B, OBSERVATION, EX_BITMAP, TICK_ARRAY_3600, TICK_ARRAY_7200] {
        pt.add_account(pk(a), load_account(a));
    }
    pt.add_account(pk(MINT_A), load_mint_with_authority(MINT_A, &admin.pubkey()));
    pt.add_account(pk(MINT_B), load_mint_with_authority(MINT_B, &admin.pubkey()));

    let (banks, _payer, _bh) = pt.start().await;

    let mint_a = pk(MINT_A);
    let mint_b = pk(MINT_B);
    let pid = liquidity_vault::ID;
    let config = Pubkey::find_program_address(&[b"config", mint_a.as_ref()], &pid).0;
    let share_mint = Pubkey::find_program_address(&[b"shares", config.as_ref()], &pid).0;
    let vault_a = Pubkey::find_program_address(&[b"vault", config.as_ref()], &pid).0;
    let vault_b = spl_associated_token_account::get_associated_token_address(&config, &mint_b);

    let mut env = Env { banks, admin, stranger, config, share_mint, vault_a, vault_b };
    let admin_pk = env.admin.pubkey();
    let admin_kp = env.admin.insecure_clone();

    // 1. initialize vault (underlying = W11A)
    let ix = Instruction {
        program_id: pid,
        accounts: liquidity_vault::accounts::InitializeVault {
            admin: admin_pk,
            underlying_mint: mint_a,
            config,
            share_mint,
            vault_token_account: vault_a,
            token_program: spl_token::ID,
            system_program: system_program::ID,
            rent: solana_sdk::sysvar::rent::ID,
        }
        .to_account_metas(None),
        data: liquidity_vault::instruction::InitializeVault { fee_bps: 50, deposit_cap: 0 }.data(),
    };
    env.ok(&[ix], &[&admin_kp]).await;

    // 2. admin gets 1000 W11A and deposits 500 into the vault
    let admin_ata_a = spl_associated_token_account::get_associated_token_address(&admin_pk, &mint_a);
    let admin_shares = spl_associated_token_account::get_associated_token_address(&admin_pk, &share_mint);
    let user_position = Pubkey::find_program_address(&[b"user", config.as_ref(), admin_pk.as_ref()], &pid).0;
    let ixs = vec![
        spl_associated_token_account::instruction::create_associated_token_account(&admin_pk, &admin_pk, &mint_a, &spl_token::ID),
        spl_token::instruction::mint_to(&spl_token::ID, &mint_a, &admin_ata_a, &admin_pk, &[], 1_000 * ONE).unwrap(),
        Instruction {
            program_id: pid,
            accounts: liquidity_vault::accounts::Deposit {
                owner: admin_pk,
                config,
                underlying_mint: mint_a,
                share_mint,
                vault_token_account: vault_a,
                user_position,
                user_token_account: admin_ata_a,
                user_share_account: admin_shares,
                token_program: spl_token::ID,
                associated_token_program: spl_associated_token_account::ID,
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: liquidity_vault::instruction::Deposit { amount: 500 * ONE, min_shares_out: 500 * ONE }.data(),
        },
    ];
    env.ok(&ixs, &[&admin_kp]).await;

    // 3. attach the Raydium CLMM pool, max slippage 1% vs spot
    let ix = Instruction {
        program_id: pid,
        accounts: liquidity_vault::accounts::SetPool { admin: admin_pk, config, pool_state: pk(POOL) }.to_account_metas(None),
        data: liquidity_vault::instruction::SetPool { max_slippage_bps: 100 }.data(),
    };
    env.ok(&[ix], &[&admin_kp]).await;

    // 4. vault-owned W11B account (ATA of the config PDA)
    let ix = Instruction {
        program_id: pid,
        accounts: liquidity_vault::accounts::InitStrategyAccount {
            admin: admin_pk,
            config,
            mint: mint_b,
            vault_ata: vault_b,
            token_program: spl_token::ID,
            associated_token_program: spl_associated_token_account::ID,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: liquidity_vault::instruction::InitStrategyAccount {}.data(),
    };
    env.ok(&[ix], &[&admin_kp]).await;

    assert_eq!(env.balance(vault_a).await, 500 * ONE);
    assert_eq!(env.balance(vault_b).await, 0);
    env
}

#[tokio::test]
async fn vault_swaps_through_raydium_with_slippage_protection() {
    let mut env = setup().await;
    let admin = env.admin.insecure_clone();
    let amount_in = 20 * ONE;

    let tick_before = env.tick().await;
    let sqrt_before = env.sqrt_price().await;
    let spot = env.spot(true, amount_in).await;
    let min_out = spot * 9_900 / 10_000; // exactly the 1% floor
    println!("pool tick {tick_before}, spot out for 20 W11A = {spot} (min_out {min_out})");

    // ---------- A -> B: vault sells 20 W11A for W11B ----------
    let ix = env.swap_ix(admin.pubkey(), true, amount_in, min_out);
    let r = env.ok(&[ix], &[&admin]).await;
    assert!(r.logs_contain(&format!("Program {} invoke [2]", RAYDIUM_CLMM_PROGRAM_ID)), "CPI into Raydium");
    assert!(r.logs_contain("Vault swap:"));
    for l in r.logs.iter().filter(|l| l.contains("Vault swap") || l.contains("invoke")) {
        println!("  {l}");
    }

    let a_after = env.balance(env.vault_a).await;
    let received_b = env.balance(env.vault_b).await;
    println!("vault: W11A 500 -> {}, W11B 0 -> {}", a_after as f64 / 1e6, received_b as f64 / 1e6);
    assert_eq!(a_after, 480 * ONE, "exactly amount_in left the vault");
    assert!(received_b >= min_out, "received >= min_amount_out");
    assert!(received_b <= spot, "price impact: never more than spot");
    let sqrt_after = env.sqrt_price().await;
    assert!(sqrt_after < sqrt_before, "selling A pushes price (B per A) down");
    println!("pool tick {tick_before} -> {}", env.tick().await);

    // ---------- B -> A: swap half of the W11B back ----------
    let back_in = received_b / 2;
    let spot_back = env.spot(false, back_in).await;
    let ix = env.swap_ix(admin.pubkey(), false, back_in, spot_back * 9_900 / 10_000);
    env.ok(&[ix], &[&admin]).await;
    assert_eq!(env.balance(env.vault_b).await, received_b - back_in);
    let a_final = env.balance(env.vault_a).await;
    assert!(a_final > 480 * ONE && a_final < 490 * ONE, "got ~10 W11A back, {a_final}");
    assert!(env.sqrt_price().await > sqrt_after, "buying A pushes price up");
    println!("vault after swap back: W11A {}, W11B {}", a_final as f64 / 1e6, (received_b - back_in) as f64 / 1e6);
}

#[tokio::test]
async fn rejects_min_amount_out_below_slippage_floor() {
    let mut env = setup().await;
    let admin = env.admin.insecure_clone();
    let spot = env.spot(true, 20 * ONE).await;

    // min_out = 1 would let a sandwich bot take everything -> refused by the vault
    let ix = env.swap_ix(admin.pubkey(), true, 20 * ONE, 1);
    let r = env.send(&[ix], &[&admin]).await;
    assert_eq!(r.custom_code(), Some(E_MIN_OUT_TOO_LOW));

    // just below the 1% floor -> also refused
    let ix = env.swap_ix(admin.pubkey(), true, 20 * ONE, spot * 9_900 / 10_000 - 1);
    let r = env.send(&[ix], &[&admin]).await;
    assert_eq!(r.custom_code(), Some(E_MIN_OUT_TOO_LOW));
    assert_eq!(env.balance(env.vault_a).await, 500 * ONE, "nothing moved");
}

#[tokio::test]
async fn raydium_enforces_min_amount_out_on_price_impact() {
    let mut env = setup().await;
    let admin = env.admin.insecure_clone();
    // big swap: 200 W11A -> price impact is several %; ask for the spot amount
    let spot = env.spot(true, 200 * ONE).await;
    let ix = env.swap_ix(admin.pubkey(), true, 200 * ONE, spot);
    let r = env.send(&[ix], &[&admin]).await;
    assert!(r.err.is_some(), "must fail");
    assert!(r.logs_contain("TooLittleOutputReceived"), "Raydium slippage check fired");
    assert_eq!(env.balance(env.vault_a).await, 500 * ONE, "whole tx reverted");
    assert_eq!(env.balance(env.vault_b).await, 0);
}

#[tokio::test]
async fn only_admin_can_swap() {
    let mut env = setup().await;
    let stranger = env.stranger.insecure_clone();
    let spot = env.spot(true, 20 * ONE).await;
    let ix = env.swap_ix(stranger.pubkey(), true, 20 * ONE, spot * 99 / 100);
    let r = env.send(&[ix], &[&stranger]).await;
    assert_eq!(r.custom_code(), Some(E_UNAUTHORIZED));
}

#[tokio::test]
async fn rejects_wrong_pool_vaults() {
    let mut env = setup().await;
    let admin = env.admin.insecure_clone();
    let spot = env.spot(true, 20 * ONE).await;
    // input/output pool vaults swapped -> could drain the wrong side
    let ix = env.swap_ix_raw(
        admin.pubkey(), env.vault_a, env.vault_b, pk(POOL_VAULT_B), pk(POOL_VAULT_A),
        pk(MINT_A), pk(MINT_B), true, 20 * ONE, spot * 99 / 100,
    );
    let r = env.send(&[ix], &[&admin]).await;
    assert_eq!(r.custom_code(), Some(E_POOL_ACCOUNT_MISMATCH));
}

#[tokio::test]
async fn paused_vault_cannot_swap() {
    let mut env = setup().await;
    let admin = env.admin.insecure_clone();
    let ix = Instruction {
        program_id: liquidity_vault::ID,
        accounts: liquidity_vault::accounts::UpdateConfig { admin: admin.pubkey(), config: env.config }.to_account_metas(None),
        data: liquidity_vault::instruction::UpdateConfig { fee_bps: None, deposit_cap: None, paused: Some(true) }.data(),
    };
    env.ok(&[ix], &[&admin]).await;
    let spot = env.spot(true, 20 * ONE).await;
    let ix = env.swap_ix(admin.pubkey(), true, 20 * ONE, spot * 99 / 100);
    let r = env.send(&[ix], &[&admin]).await;
    assert_eq!(r.custom_code(), Some(E_VAULT_PAUSED));
    let _ = env.share_mint;
}
