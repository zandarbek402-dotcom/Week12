use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, TokenAccount};

use crate::{constants::*, math::shares_for_deposit, state::VaultConfig};

/// Read-only: how many shares would `amount` give right now?
/// Frontend calls it via `.view()` (simulation) to show the user
/// the expected shares and to compute `min_shares_out`.
#[derive(Accounts)]
pub struct PreviewDeposit<'info> {
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

pub fn handler(ctx: Context<PreviewDeposit>, amount: u64) -> Result<u64> {
    shares_for_deposit(
        amount,
        ctx.accounts.share_mint.supply,
        ctx.accounts.vault_token_account.amount,
    )
}
