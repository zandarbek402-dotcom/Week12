use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::{
    constants::*,
    errors::VaultError,
    events::VaultSwapEvent,
    raydium::{
        read_trade_fee_rate, spot_amount_out, swap_v2_cpi, PoolView, SwapV2Accounts,
        MAX_SQRT_PRICE_X64, MEMO_PROGRAM_ID, MIN_SQRT_PRICE_X64, RAYDIUM_CLMM_PROGRAM_ID,
        TOKEN_2022_PROGRAM_ID,
    },
    state::VaultConfig,
};

/// Week 12 — Swap through Raydium: the vault exchanges ITS OWN tokens
/// in the Raydium CLMM pool. The vault PDA signs the CPI, and the result
/// is protected by a minimum amount out (slippage).
#[derive(Accounts)]
pub struct SwapViaRaydium<'info> {
    /// Vault admin / strategy keeper
    pub admin: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED, config.underlying_mint.as_ref()],
        bump = config.bump,
        has_one = admin @ VaultError::Unauthorized,
    )]
    pub config: Box<Account<'info, VaultConfig>>,

    /// CHECK: must be the Raydium CLMM program
    #[account(address = RAYDIUM_CLMM_PROGRAM_ID @ VaultError::InvalidPool)]
    pub clmm_program: UncheckedAccount<'info>,

    /// CHECK: must equal pool_state.amm_config (checked in handler)
    pub amm_config: UncheckedAccount<'info>,

    /// CHECK: must be the pool configured in the vault (set_pool), owned by Raydium
    #[account(
        mut,
        address = config.pool @ VaultError::InvalidPool,
        owner = RAYDIUM_CLMM_PROGRAM_ID @ VaultError::InvalidPool,
    )]
    pub pool_state: UncheckedAccount<'info>,

    /// Vault-owned token account that pays (authority = config PDA)
    #[account(
        mut,
        token::mint = input_mint,
        token::authority = config,
    )]
    pub input_token_account: Box<Account<'info, TokenAccount>>,

    /// Vault-owned token account that receives (authority = config PDA)
    #[account(
        mut,
        token::mint = output_mint,
        token::authority = config,
    )]
    pub output_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: pool vault for input mint (checked against pool state)
    #[account(mut)]
    pub input_vault: UncheckedAccount<'info>,

    /// CHECK: pool vault for output mint (checked against pool state)
    #[account(mut)]
    pub output_vault: UncheckedAccount<'info>,

    /// CHECK: pool observation account (checked against pool state)
    #[account(mut)]
    pub observation_state: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,

    /// CHECK: Token-2022 program id
    #[account(address = TOKEN_2022_PROGRAM_ID)]
    pub token_program_2022: UncheckedAccount<'info>,

    /// CHECK: SPL Memo program id
    #[account(address = MEMO_PROGRAM_ID)]
    pub memo_program: UncheckedAccount<'info>,

    pub input_mint: Box<Account<'info, Mint>>,
    pub output_mint: Box<Account<'info, Mint>>,
    // remaining_accounts: [tickarray_bitmap_extension, tick_array, tick_array, ...]
}

/// * `amount_in`       — exact input amount (vault tokens)
/// * `min_amount_out`  — slippage protection: minimum output tokens
/// * `sqrt_price_limit_x64` — optional price limit (0 = none)
pub fn handler<'info>(
    mut ctx: Context<'_, '_, 'info, 'info, SwapViaRaydium<'info>>,
    amount_in: u64,
    min_amount_out: u64,
    sqrt_price_limit_x64: u128,
) -> Result<()> {
    let a = &ctx.accounts;
    require!(!a.config.paused, VaultError::VaultPaused);
    require!(amount_in > 0, VaultError::ZeroAmount);
    require!(min_amount_out > 0, VaultError::MinOutTooLow);
    require_keys_neq!(a.input_mint.key(), a.output_mint.key(), VaultError::SameMint);
    require!(a.input_token_account.amount >= amount_in, VaultError::InsufficientLiquidity);

    // ---------- 1. validate Raydium accounts against PoolState ----------
    let pool = PoolView::load(&a.pool_state.to_account_info())?;
    require_keys_eq!(a.amm_config.key(), pool.amm_config, VaultError::PoolAccountMismatch);
    require_keys_eq!(a.observation_state.key(), pool.observation, VaultError::PoolAccountMismatch);

    let zero_for_one = if a.input_mint.key() == pool.mint_0 && a.output_mint.key() == pool.mint_1 {
        require_keys_eq!(a.input_vault.key(), pool.vault_0, VaultError::PoolAccountMismatch);
        require_keys_eq!(a.output_vault.key(), pool.vault_1, VaultError::PoolAccountMismatch);
        true
    } else if a.input_mint.key() == pool.mint_1 && a.output_mint.key() == pool.mint_0 {
        require_keys_eq!(a.input_vault.key(), pool.vault_1, VaultError::PoolAccountMismatch);
        require_keys_eq!(a.output_vault.key(), pool.vault_0, VaultError::PoolAccountMismatch);
        false
    } else {
        return err!(VaultError::PoolMintMismatch);
    };

    // ---------- 2. slippage guard #1: min_amount_out vs pool spot price ----------
    // The caller (keeper / frontend) computes min_amount_out from an off-chain quote.
    // The program additionally refuses a min_amount_out that allows more than
    // `max_slippage_bps` below the current spot price -> protects against
    // min_amount_out = 1 (sandwich attacks) or a stale quote.
    let fee_rate = read_trade_fee_rate(&a.amm_config.to_account_info())?;
    let spot_out = spot_amount_out(amount_in, pool.sqrt_price_x64, zero_for_one, fee_rate)?;
    let floor = (spot_out as u128 * (MAX_BPS - a.config.max_slippage_bps) as u128 / MAX_BPS as u128) as u64;
    require!(min_amount_out >= floor, VaultError::MinOutTooLow);

    // price limit: 0 -> no limit (edge of the allowed range in the swap direction)
    let price_limit = if sqrt_price_limit_x64 == 0 {
        if zero_for_one { MIN_SQRT_PRICE_X64 + 1 } else { MAX_SQRT_PRICE_X64 - 1 }
    } else {
        sqrt_price_limit_x64
    };

    let in_before = a.input_token_account.amount;
    let out_before = a.output_token_account.amount;

    // ---------- 3. CPI: Raydium CLMM swap_v2, signed by the vault PDA ----------
    let mint_key = a.config.underlying_mint;
    let bump = [a.config.bump];
    let seeds: &[&[u8]] = &[CONFIG_SEED, mint_key.as_ref(), &bump];

    let config_info = a.config.to_account_info();
    let clmm_program = a.clmm_program.to_account_info();
    let amm_config = a.amm_config.to_account_info();
    let pool_state = a.pool_state.to_account_info();
    let input_token_account = a.input_token_account.to_account_info();
    let output_token_account = a.output_token_account.to_account_info();
    let input_vault = a.input_vault.to_account_info();
    let output_vault = a.output_vault.to_account_info();
    let observation_state = a.observation_state.to_account_info();
    let token_program = a.token_program.to_account_info();
    let token_program_2022 = a.token_program_2022.to_account_info();
    let memo_program = a.memo_program.to_account_info();
    let input_mint = a.input_mint.to_account_info();
    let output_mint = a.output_mint.to_account_info();

    swap_v2_cpi(
        SwapV2Accounts {
            clmm_program: &clmm_program,
            payer: &config_info,
            amm_config: &amm_config,
            pool_state: &pool_state,
            input_token_account: &input_token_account,
            output_token_account: &output_token_account,
            input_vault: &input_vault,
            output_vault: &output_vault,
            observation_state: &observation_state,
            token_program: &token_program,
            token_program_2022: &token_program_2022,
            memo_program: &memo_program,
            input_vault_mint: &input_mint,
            output_vault_mint: &output_mint,
            remaining: ctx.remaining_accounts,
        },
        amount_in,
        min_amount_out, // Raydium itself also enforces other_amount_threshold
        price_limit,
        true, // is_base_input: amount_in is exact
        &[seeds],
    )?;

    // ---------- 4. slippage guard #2: verify by real balance change ----------
    let a = &mut ctx.accounts;
    a.input_token_account.reload()?;
    a.output_token_account.reload()?;
    let spent = in_before
        .checked_sub(a.input_token_account.amount)
        .ok_or(VaultError::MathOverflow)?;
    let received = a
        .output_token_account
        .amount
        .checked_sub(out_before)
        .ok_or(VaultError::MathOverflow)?;
    require!(spent <= amount_in, VaultError::InputOverspent);
    require!(received >= min_amount_out, VaultError::SlippageExceeded);

    let pool_after = PoolView::load(&a.pool_state.to_account_info())?;
    emit!(VaultSwapEvent {
        config: a.config.key(),
        pool: a.pool_state.key(),
        input_mint: a.input_mint.key(),
        output_mint: a.output_mint.key(),
        amount_in: spent,
        amount_out: received,
        min_amount_out,
        spot_amount_out: spot_out,
        sqrt_price_before: pool.sqrt_price_x64,
        sqrt_price_after: pool_after.sqrt_price_x64,
        timestamp: Clock::get()?.unix_timestamp,
    });
    msg!(
        "Vault swap: {} in -> {} out (min {}, spot {}), tick {} -> {}",
        spent, received, min_amount_out, spot_out, pool.tick_current, pool_after.tick_current
    );
    Ok(())
}
