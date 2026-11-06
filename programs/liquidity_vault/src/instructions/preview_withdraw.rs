use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, TokenAccount};

use crate::{
    constants::*,
    math::{assets_for_shares, fee_for},
    state::VaultConfig,
};

/// Read-only: how many underlying tokens (after fee) burning `shares` gives now.
/// Frontend calls it via `.view()` and uses it to compute `min_assets_out`.
#[derive(Accounts)]
pub struct PreviewWithdraw<'info> {
    #[account(
        seeds = [CONFIG_SEED, config.underlying_mint.as_ref()],
        bump = config.bump,
        has_one = share_mint,
        has_one = vault_token_account,
    )]
    pub config: Account<'info, VaultConfig>,
    pub share_mint: Account<'info, Mint>,
    pub vault_token_account: Account<'info, TokenAccount>,
}

pub fn handler(ctx: Context<PreviewWithdraw>, shares: u64) -> Result<u64> {
    let gross = assets_for_shares(
        shares,
        ctx.accounts.share_mint.supply,
        ctx.accounts.vault_token_account.amount,
    )?;
    let fee = fee_for(gross, ctx.accounts.config.fee_bps)?;
    Ok(gross - fee)
}
