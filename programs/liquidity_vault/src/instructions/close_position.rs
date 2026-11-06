use anchor_lang::prelude::*;

use crate::{
    constants::*,
    errors::VaultError,
    state::{UserPosition, VaultConfig},
};

/// After a full withdraw the user can close their position PDA
/// and get the rent (lamports) back.
#[derive(Accounts)]
pub struct ClosePosition<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED, config.underlying_mint.as_ref()],
        bump = config.bump,
    )]
    pub config: Account<'info, VaultConfig>,

    #[account(
        mut,
        close = owner,
        seeds = [USER_SEED, config.key().as_ref(), owner.key().as_ref()],
        bump = user_position.bump,
        has_one = owner,
        constraint = user_position.shares == 0 @ VaultError::PositionNotEmpty,
    )]
    pub user_position: Account<'info, UserPosition>,
}

pub fn handler(_ctx: Context<ClosePosition>) -> Result<()> {
    msg!("User position closed, rent returned");
    Ok(())
}
