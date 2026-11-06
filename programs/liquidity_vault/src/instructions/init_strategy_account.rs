use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};

use crate::{constants::*, errors::VaultError, state::VaultConfig};

/// Admin: create a token account owned by the vault PDA for the other
/// token of the pool (the vault receives it when it swaps).
/// It is the ATA of (config PDA, mint).
#[derive(Accounts)]
pub struct InitStrategyAccount<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED, config.underlying_mint.as_ref()],
        bump = config.bump,
        has_one = admin @ VaultError::Unauthorized,
    )]
    pub config: Account<'info, VaultConfig>,

    pub mint: Account<'info, Mint>,

    #[account(
        init,
        payer = admin,
        associated_token::mint = mint,
        associated_token::authority = config,
    )]
    pub vault_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<InitStrategyAccount>) -> Result<()> {
    msg!("Vault token account {} for mint {}", ctx.accounts.vault_ata.key(), ctx.accounts.mint.key());
    Ok(())
}
