use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, TransferChecked},
};

use crate::{
    constants::*,
    token_cpi::mint_shares,
    errors::VaultError,
    events::DepositEvent,
    math::shares_for_deposit,
    state::{UserPosition, VaultConfig},
};

/// Week 9 — Deposit: user puts underlying tokens into the vault
/// and receives share tokens = their part of the vault.
#[derive(Accounts)]
pub struct Deposit<'info> {
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

    #[account(
        mut,
        seeds = [VAULT_TOKEN_SEED, config.key().as_ref()],
        bump = config.vault_bump,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    /// Created automatically on the first deposit
    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + UserPosition::INIT_SPACE,
        seeds = [USER_SEED, config.key().as_ref(), owner.key().as_ref()],
        bump,
    )]
    pub user_position: Box<Account<'info, UserPosition>>,

    /// User's underlying tokens (source)
    #[account(
        mut,
        token::mint = underlying_mint,
        token::authority = owner,
    )]
    pub user_token_account: Box<Account<'info, TokenAccount>>,

    /// User's share tokens (destination), ATA created if missing
    #[account(
        init_if_needed,
        payer = owner,
        associated_token::mint = share_mint,
        associated_token::authority = owner,
    )]
    pub user_share_account: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// * `amount` — underlying tokens to deposit (base units)
/// * `min_shares_out` — slippage protection: tx fails if user would get fewer shares
pub fn handler(ctx: Context<Deposit>, amount: u64, min_shares_out: u64) -> Result<()> {
    // ---------- 1. validation ----------
    require!(amount > 0, VaultError::ZeroAmount);
    require!(!ctx.accounts.config.paused, VaultError::VaultPaused);
    require!(
        ctx.accounts.user_token_account.amount >= amount,
        VaultError::InsufficientFunds
    );

    let total_assets = ctx.accounts.vault_token_account.amount;
    let total_shares = ctx.accounts.share_mint.supply;

    let cap = ctx.accounts.config.deposit_cap;
    if cap > 0 {
        let after = total_assets.checked_add(amount).ok_or(VaultError::MathOverflow)?;
        require!(after <= cap, VaultError::DepositCapExceeded);
    }

    // ---------- 2. how many shares? (price is taken BEFORE the transfer) ----------
    let shares = shares_for_deposit(amount, total_shares, total_assets)?;
    require!(shares > 0, VaultError::ZeroShares);
    require!(shares >= min_shares_out, VaultError::SlippageExceeded);

    // ---------- 3. user -> vault (transfer_checked validates mint + decimals) ----------
    token::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TransferChecked {
                from: ctx.accounts.user_token_account.to_account_info(),
                mint: ctx.accounts.underlying_mint.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.underlying_mint.decimals,
    )?;

    // ---------- 4. mint shares to user (config PDA signs) ----------
    mint_shares(
        &ctx.accounts.token_program,
        &ctx.accounts.share_mint,
        &ctx.accounts.user_share_account,
        &ctx.accounts.config,
        shares,
    )?;

    // TODO (week 11+): CPI into the liquidity pool (add_liquidity) with vault funds

    // ---------- 5. accounting ----------
    let now = Clock::get()?.unix_timestamp;
    let config_key = ctx.accounts.config.key();

    let config = &mut ctx.accounts.config;
    config.total_deposited = config
        .total_deposited
        .checked_add(amount)
        .ok_or(VaultError::MathOverflow)?;

    let user = &mut ctx.accounts.user_position;
    if user.owner == Pubkey::default() {
        // first deposit -> position was just created
        user.owner = ctx.accounts.owner.key();
        user.config = config_key;
        user.bump = ctx.bumps.user_position;
    }
    user.shares = user.shares.checked_add(shares).ok_or(VaultError::MathOverflow)?;
    user.deposited = user.deposited.checked_add(amount).ok_or(VaultError::MathOverflow)?;
    user.last_action_ts = now;

    // ---------- 6. event ----------
    emit!(DepositEvent {
        user: ctx.accounts.owner.key(),
        config: config_key,
        amount,
        shares,
        total_assets: total_assets + amount,
        total_shares: total_shares + shares,
        timestamp: now,
    });

    msg!("Deposited {} tokens, minted {} shares", amount, shares);
    Ok(())
}
