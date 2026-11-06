use anchor_lang::prelude::*;

use crate::{
    constants::*,
    state::{UserPosition, VaultConfig},
};

#[derive(Accounts)]
pub struct InitializeUser<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED, config.underlying_mint.as_ref()],
        bump = config.bump
    )]
    pub config: Account<'info, VaultConfig>,

    #[account(
        init,
        payer = owner,
        space = 8 + UserPosition::INIT_SPACE,
        seeds = [USER_SEED, config.key().as_ref(), owner.key().as_ref()],
        bump
    )]
    pub user_position: Account<'info, UserPosition>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<InitializeUser>) -> Result<()> {
    let user = &mut ctx.accounts.user_position;
    user.owner = ctx.accounts.owner.key();
    user.config = ctx.accounts.config.key();
    user.shares = 0;
    user.deposited = 0;
    user.last_action_ts = Clock::get()?.unix_timestamp;
    user.bump = ctx.bumps.user_position;
    Ok(())
}
