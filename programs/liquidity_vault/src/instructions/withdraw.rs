use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::{
    constants::*,
    token_cpi::{burn_shares, transfer_from_vault},
    errors::VaultError,
    events::WithdrawEvent,
    math::{assets_for_shares, fee_for, proportional},
    state::{UserPosition, VaultConfig},
};

/// Week 10 — Withdraw: share tokens are burned, the vault returns the
/// underlying tokens to the user via the Token program, signed by the PDA.
#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [CONFIG_SEED, config.underlying_mint.as_ref()],
        bump = config.bump,
        has_one = share_mint,
        has_one = vault_token_account,
        has_one = underlying_mint,
    )]
    pub config: Box<Account<'info, VaultConfig>>,

    pub underlying_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [SHARE_MINT_SEED, config.key().as_ref()],
        bump = config.share_mint_bump,
    )]
    pub share_mint: Box<Account<'info, Mint>>,

    /// Vault treasury — authority is the config PDA
    #[account(
        mut,
        seeds = [VAULT_TOKEN_SEED, config.key().as_ref()],
        bump = config.vault_bump,
        token::authority = config,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// Seeds include `owner` -> nobody can withdraw from someone else's position
    #[account(
        mut,
        seeds = [USER_SEED, config.key().as_ref(), owner.key().as_ref()],
        bump = user_position.bump,
        has_one = owner,
        has_one = config,
    )]
    pub user_position: Box<Account<'info, UserPosition>>,

    /// Destination of the underlying tokens
    #[account(
        mut,
        token::mint = underlying_mint,
        token::authority = owner,
    )]
    pub user_token_account: Box<Account<'info, TokenAccount>>,

    /// Share tokens to burn
    #[account(
        mut,
        token::mint = share_mint,
        token::authority = owner,
    )]
    pub user_share_account: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

/// * `shares` — how many share tokens to burn
/// * `min_assets_out` — slippage protection: minimum underlying tokens to receive
pub fn handler(ctx: Context<Withdraw>, shares: u64, min_assets_out: u64) -> Result<()> {
    // ---------- 1. validation ----------
    // NOTE: withdrawals are allowed even when the vault is paused —
    // pause only stops new deposits, user funds are never locked.
    require!(shares > 0, VaultError::ZeroAmount);
    let position_shares = ctx.accounts.user_position.shares;
    require!(position_shares >= shares, VaultError::InsufficientShares);
    require!(
        ctx.accounts.user_share_account.amount >= shares,
        VaultError::InsufficientShares
    );

    // ---------- 2. how many tokens? (price taken BEFORE burning) ----------
    let total_shares = ctx.accounts.share_mint.supply;
    let total_assets = ctx.accounts.vault_token_account.amount;

    let gross = assets_for_shares(shares, total_shares, total_assets)?;
    let fee = fee_for(gross, ctx.accounts.config.fee_bps)?;
    let amount_out = gross.checked_sub(fee).ok_or(VaultError::MathOverflow)?;

    require!(amount_out > 0, VaultError::ZeroAmount);
    require!(amount_out >= min_assets_out, VaultError::SlippageExceeded);
    // TODO (week 11+): pull liquidity from the pool if the vault has not enough idle tokens
    require!(total_assets >= amount_out, VaultError::InsufficientLiquidity);

    // ---------- 3. CPI #1: burn user's shares (signed by the user) ----------
    burn_shares(
        &ctx.accounts.token_program,
        &ctx.accounts.share_mint,
        &ctx.accounts.user_share_account,
        &ctx.accounts.owner,
        shares,
    )?;

    // ---------- 4. CPI #2: vault -> user (signed by the config PDA) ----------
    transfer_from_vault(
        &ctx.accounts.token_program,
        &ctx.accounts.vault_token_account,
        &ctx.accounts.underlying_mint,
        &ctx.accounts.user_token_account,
        &ctx.accounts.config,
        amount_out,
    )?;

    // ---------- 5. accounting ----------
    let now = Clock::get()?.unix_timestamp;
    let user = &mut ctx.accounts.user_position;
    // reduce cost basis proportionally to the burned shares
    let basis_removed = proportional(user.deposited, shares, position_shares)?;
    user.shares = position_shares - shares;
    user.deposited = user.deposited.saturating_sub(basis_removed);
    user.last_action_ts = now;
    let shares_left = user.shares;

    let config = &mut ctx.accounts.config;
    config.total_deposited = config.total_deposited.saturating_sub(basis_removed);

    // ---------- 6. event ----------
    emit!(WithdrawEvent {
        user: ctx.accounts.owner.key(),
        config: config.key(),
        shares,
        amount: amount_out,
        fee,
        shares_left,
        timestamp: now,
    });

    msg!("Burned {} shares, withdrew {} tokens (fee {})", shares, amount_out, fee);
    Ok(())
}
