use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::{constants::*, errors::VaultError, state::VaultConfig};

// NOTE: accounts are boxed (heap) — unboxed, `try_accounts` needed a 4376-byte
// stack frame (> 4096 SBF limit) and corrupted memory on-chain.
#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    pub underlying_mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = admin,
        space = 8 + VaultConfig::INIT_SPACE,
        seeds = [CONFIG_SEED, underlying_mint.key().as_ref()],
        bump
    )]
    pub config: Box<Account<'info, VaultConfig>>,

    /// Share token: mint authority = config PDA, same decimals as underlying
    #[account(
        init,
        payer = admin,
        seeds = [SHARE_MINT_SEED, config.key().as_ref()],
        bump,
        mint::decimals = underlying_mint.decimals,
        mint::authority = config,
    )]
    pub share_mint: Box<Account<'info, Mint>>,

    /// Vault treasury: token account owned by config PDA
    #[account(
        init,
        payer = admin,
        seeds = [VAULT_TOKEN_SEED, config.key().as_ref()],
        bump,
        token::mint = underlying_mint,
        token::authority = config,
    )]
    pub vault_token_account: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handler(ctx: Context<InitializeVault>, fee_bps: u16, deposit_cap: u64) -> Result<()> {
    require!(fee_bps <= MAX_FEE_BPS, VaultError::FeeTooHigh);

    let config = &mut ctx.accounts.config;
    config.admin = ctx.accounts.admin.key();
    config.underlying_mint = ctx.accounts.underlying_mint.key();
    config.share_mint = ctx.accounts.share_mint.key();
    config.vault_token_account = ctx.accounts.vault_token_account.key();
    config.pool = Pubkey::default(); // set later by `set_pool`
    config.max_slippage_bps = DEFAULT_MAX_SLIPPAGE_BPS;
    config.fee_bps = fee_bps;
    config.deposit_cap = deposit_cap;
    config.total_deposited = 0;
    config.paused = false;
    config.bump = ctx.bumps.config;
    config.share_mint_bump = ctx.bumps.share_mint;
    config.vault_bump = ctx.bumps.vault_token_account;

    msg!(
        "Vault initialized: mint={} share_mint={} fee_bps={} cap={}",
        config.underlying_mint,
        config.share_mint,
        fee_bps,
        deposit_cap
    );
    Ok(())
}
