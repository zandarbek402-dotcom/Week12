use anchor_lang::prelude::*;

use crate::{constants::*, errors::VaultError, state::VaultConfig};

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [CONFIG_SEED, config.underlying_mint.as_ref()],
        bump = config.bump,
        has_one = admin @ VaultError::Unauthorized,
    )]
    pub config: Account<'info, VaultConfig>,
}

pub fn handler(
    ctx: Context<UpdateConfig>,
    fee_bps: Option<u16>,
    deposit_cap: Option<u64>,
    paused: Option<bool>,
) -> Result<()> {
    let config = &mut ctx.accounts.config;
    if let Some(fee) = fee_bps {
        require!(fee <= MAX_FEE_BPS, VaultError::FeeTooHigh);
        config.fee_bps = fee;
    }
    if let Some(cap) = deposit_cap {
        config.deposit_cap = cap;
    }
    if let Some(p) = paused {
        config.paused = p;
    }
    msg!("Config updated: fee_bps={} cap={} paused={}", config.fee_bps, config.deposit_cap, config.paused);
    Ok(())
}
