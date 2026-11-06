//! Minimal Raydium CLMM integration without depending on the Raydium crate:
//! - program ids
//! - zero-copy read of the fields we need from `PoolState` / `AmmConfig`
//! - `swap_v2` CPI built by hand and signed by the vault PDA
//!
//! Byte offsets were verified against a live devnet pool
//! (see README "PoolState layout").

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
};

use crate::errors::VaultError;

/// Raydium CLMM program. Devnet by default, mainnet with `--features mainnet`.
/// DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH
#[cfg(not(feature = "mainnet"))]
pub const RAYDIUM_CLMM_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    184, 152, 151, 52, 252, 179, 140, 145, 104, 216, 83, 199, 83, 182, 184, 164, 54, 16, 205, 211,
    37, 175, 187, 199, 47, 212, 21, 54, 219, 205, 194, 88,
]);
/// CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK
#[cfg(feature = "mainnet")]
pub const RAYDIUM_CLMM_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    165, 213, 202, 158, 4, 207, 93, 181, 144, 183, 20, 186, 47, 227, 44, 177, 89, 19, 63, 193,
    193, 146, 183, 34, 87, 253, 7, 211, 156, 176, 64, 30,
]);

/// TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb
pub const TOKEN_2022_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    6, 221, 246, 225, 238, 117, 143, 222, 24, 66, 93, 188, 228, 108, 205, 218, 182, 26, 252, 77,
    131, 185, 13, 39, 254, 189, 249, 40, 216, 161, 139, 252,
]);
/// MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr
pub const MEMO_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    5, 74, 83, 90, 153, 41, 33, 6, 77, 36, 232, 113, 96, 218, 56, 124, 124, 53, 181, 221, 188,
    146, 187, 129, 228, 31, 168, 64, 65, 5, 68, 141,
]);

/// sha256("global:swap_v2")[..8]
pub const SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

/// Raydium price limits (exclusive) — used when caller passes 0 = "no limit"
pub const MIN_SQRT_PRICE_X64: u128 = 4295048016;
pub const MAX_SQRT_PRICE_X64: u128 = 79226673521066979257578248091;

/// Fields of Raydium `PoolState` we need (Anchor account, 1544 bytes).
pub struct PoolView {
    pub amm_config: Pubkey,
    pub mint_0: Pubkey,
    pub mint_1: Pubkey,
    pub vault_0: Pubkey,
    pub vault_1: Pubkey,
    pub observation: Pubkey,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
}

// offsets: 8 discriminator + 1 bump
const O_AMM_CONFIG: usize = 9;
const O_MINT_0: usize = 73; // after amm_config(32) + owner(32)
const O_MINT_1: usize = 105;
const O_VAULT_0: usize = 137;
const O_VAULT_1: usize = 169;
const O_OBSERVATION: usize = 201;
const O_LIQUIDITY: usize = 237; // after decimals(1+1) + tick_spacing(2)
const O_SQRT_PRICE: usize = 253;
const O_TICK: usize = 269;
const POOL_MIN_LEN: usize = 273;

/// AmmConfig: 8 disc + 1 bump + 2 index + 32 owner + 4 protocol_fee_rate
const O_TRADE_FEE_RATE: usize = 47;
/// Raydium fee denominator: 1_000_000 (2500 = 0.25%)
pub const FEE_RATE_DENOMINATOR: u64 = 1_000_000;

fn pk(d: &[u8], o: usize) -> Pubkey {
    Pubkey::new_from_array(d[o..o + 32].try_into().unwrap())
}
fn u128_le(d: &[u8], o: usize) -> u128 {
    u128::from_le_bytes(d[o..o + 16].try_into().unwrap())
}

impl PoolView {
    pub fn load(pool: &AccountInfo) -> Result<Self> {
        require_keys_eq!(*pool.owner, RAYDIUM_CLMM_PROGRAM_ID, VaultError::InvalidPool);
        let d = pool.try_borrow_data()?;
        require!(d.len() >= POOL_MIN_LEN, VaultError::InvalidPool);
        Ok(Self {
            amm_config: pk(&d, O_AMM_CONFIG),
            mint_0: pk(&d, O_MINT_0),
            mint_1: pk(&d, O_MINT_1),
            vault_0: pk(&d, O_VAULT_0),
            vault_1: pk(&d, O_VAULT_1),
            observation: pk(&d, O_OBSERVATION),
            liquidity: u128_le(&d, O_LIQUIDITY),
            sqrt_price_x64: u128_le(&d, O_SQRT_PRICE),
            tick_current: i32::from_le_bytes(d[O_TICK..O_TICK + 4].try_into().unwrap()),
        })
    }
}

pub fn read_trade_fee_rate(amm_config: &AccountInfo) -> Result<u64> {
    require_keys_eq!(*amm_config.owner, RAYDIUM_CLMM_PROGRAM_ID, VaultError::InvalidPool);
    let d = amm_config.try_borrow_data()?;
    require!(d.len() >= O_TRADE_FEE_RATE + 4, VaultError::InvalidPool);
    Ok(u32::from_le_bytes(d[O_TRADE_FEE_RATE..O_TRADE_FEE_RATE + 4].try_into().unwrap()) as u64)
}

/// (a * s) >> 64 without overflow for a < 2^64, s < 2^128
fn mul_shr64(a: u128, s: u128) -> Option<u128> {
    let hi = s >> 64;
    let lo = s & u64::MAX as u128;
    a.checked_mul(hi)?.checked_add((a * lo) >> 64)
}

/// Expected output at the CURRENT pool price, after the pool fee, before price impact.
/// price (token1 per token0, raw units) = (sqrt_price_x64 / 2^64)^2
pub fn spot_amount_out(amount_in: u64, sqrt_price_x64: u128, zero_for_one: bool, fee_rate: u64) -> Result<u64> {
    require!(sqrt_price_x64 > 0, VaultError::InvalidPool);
    let a = amount_in as u128 * (FEE_RATE_DENOMINATOR - fee_rate) as u128 / FEE_RATE_DENOMINATOR as u128;
    let out = if zero_for_one {
        // a * P = a * s^2 / 2^128
        mul_shr64(mul_shr64(a, sqrt_price_x64).ok_or(VaultError::MathOverflow)?, sqrt_price_x64)
            .ok_or(VaultError::MathOverflow)?
    } else {
        // a / P = (a * 2^64 / s) * 2^64 / s
        let x = (a << 64) / sqrt_price_x64;
        require!(x <= u64::MAX as u128, VaultError::MathOverflow);
        (x << 64) / sqrt_price_x64
    };
    u64::try_from(out).map_err(|_| error!(VaultError::MathOverflow))
}

/// Accounts for `swap_v2`, in Raydium's order.
pub struct SwapV2Accounts<'a, 'info> {
    pub clmm_program: &'a AccountInfo<'info>,
    pub payer: &'a AccountInfo<'info>, // vault config PDA (authority of input token account)
    pub amm_config: &'a AccountInfo<'info>,
    pub pool_state: &'a AccountInfo<'info>,
    pub input_token_account: &'a AccountInfo<'info>,
    pub output_token_account: &'a AccountInfo<'info>,
    pub input_vault: &'a AccountInfo<'info>,
    pub output_vault: &'a AccountInfo<'info>,
    pub observation_state: &'a AccountInfo<'info>,
    pub token_program: &'a AccountInfo<'info>,
    pub token_program_2022: &'a AccountInfo<'info>,
    pub memo_program: &'a AccountInfo<'info>,
    pub input_vault_mint: &'a AccountInfo<'info>,
    pub output_vault_mint: &'a AccountInfo<'info>,
    /// tickarray_bitmap_extension + tick arrays
    pub remaining: &'a [AccountInfo<'info>],
}

/// CPI: Raydium CLMM `swap_v2(amount, other_amount_threshold, sqrt_price_limit_x64, is_base_input)`
/// signed by the vault PDA.
pub fn swap_v2_cpi(
    acc: SwapV2Accounts,
    amount: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let mut data = Vec::with_capacity(8 + 8 + 8 + 16 + 1);
    data.extend_from_slice(&SWAP_V2_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit_x64.to_le_bytes());
    data.push(is_base_input as u8);

    let mut metas = vec![
        AccountMeta::new_readonly(acc.payer.key(), true),
        AccountMeta::new_readonly(acc.amm_config.key(), false),
        AccountMeta::new(acc.pool_state.key(), false),
        AccountMeta::new(acc.input_token_account.key(), false),
        AccountMeta::new(acc.output_token_account.key(), false),
        AccountMeta::new(acc.input_vault.key(), false),
        AccountMeta::new(acc.output_vault.key(), false),
        AccountMeta::new(acc.observation_state.key(), false),
        AccountMeta::new_readonly(acc.token_program.key(), false),
        AccountMeta::new_readonly(acc.token_program_2022.key(), false),
        AccountMeta::new_readonly(acc.memo_program.key(), false),
        AccountMeta::new_readonly(acc.input_vault_mint.key(), false),
        AccountMeta::new_readonly(acc.output_vault_mint.key(), false),
    ];
    metas.extend(acc.remaining.iter().map(|a| AccountMeta::new(a.key(), false)));

    let ix = Instruction { program_id: RAYDIUM_CLMM_PROGRAM_ID, accounts: metas, data };

    let mut infos = vec![
        acc.payer.clone(),
        acc.amm_config.clone(),
        acc.pool_state.clone(),
        acc.input_token_account.clone(),
        acc.output_token_account.clone(),
        acc.input_vault.clone(),
        acc.output_vault.clone(),
        acc.observation_state.clone(),
        acc.token_program.clone(),
        acc.token_program_2022.clone(),
        acc.memo_program.clone(),
        acc.input_vault_mint.clone(),
        acc.output_vault_mint.clone(),
    ];
    infos.extend(acc.remaining.iter().cloned());
    infos.push(acc.clmm_program.clone());

    invoke_signed(&ix, &infos, signer_seeds)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // live devnet pool W11A/W11B: price ≈ 1.99137 (B per A)
    const SQRT_P: u128 = 26031301204159531733;

    #[test]
    fn spot_a_to_b() {
        // 20 A (6 decimals), 0.25% fee -> ≈ 20 * 0.9975 * 1.99137 = 39.727 B
        let out = spot_amount_out(20_000_000, SQRT_P, true, 2500).unwrap();
        assert!((39_720_000..39_735_000).contains(&out), "{out}");
    }

    #[test]
    fn spot_b_to_a() {
        // 40 B -> ≈ 40 * 0.9975 / 1.99137 = 20.037 A
        let out = spot_amount_out(40_000_000, SQRT_P, false, 2500).unwrap();
        assert!((20_030_000..20_045_000).contains(&out), "{out}");
    }

    #[test]
    fn price_one() {
        let s = 1u128 << 64; // price 1.0
        assert_eq!(spot_amount_out(1_000_000, s, true, 0).unwrap(), 1_000_000);
        assert_eq!(spot_amount_out(1_000_000, s, false, 0).unwrap(), 1_000_000);
    }

    #[test]
    fn discriminator_matches_anchor() {
        let h = anchor_lang::solana_program::hash::hash(b"global:swap_v2");
        assert_eq!(&h.to_bytes()[..8], &SWAP_V2_DISCRIMINATOR);
    }
}
