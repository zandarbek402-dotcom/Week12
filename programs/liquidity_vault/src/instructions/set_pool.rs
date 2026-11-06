use anchor_lang::prelude::*;

use crate::{
    constants::*,
    errors::VaultError,
    raydium::{PoolView, RAYDIUM_CLMM_PROGRAM_ID},
    state::VaultConfig,
};

/// Admin: attach a Raydium CLMM pool to the vault and set the max slippage.
#[derive(Accounts)]
pub struct SetPool<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [CONFIG_SEED, config.underlying_mint.as_ref()],
        bump = config.bump,
        has_one = admin @ VaultError::Unauthorized,
    )]
    pub config: Account<'info, VaultConfig>,

    /// CHECK: owner must be Raydium CLMM; layout parsed in `PoolView::load`
    #[account(owner = RAYDIUM_CLMM_PROGRAM_ID @ VaultError::InvalidPool)]
    pub pool_state: UncheckedAccount<'info>,
}

pub fn handler(ctx: Context<SetPool>, max_slippage_bps: u16) -> Result<()> {
    require!(max_slippage_bps <= MAX_SLIPPAGE_LIMIT_BPS, VaultError::SlippageTooHigh);

    let pool = PoolView::load(&ctx.accounts.pool_state.to_account_info())?;
    let underlying = ctx.accounts.config.underlying_mint;
    require!(
        pool.mint_0 == underlying || pool.mint_1 == underlying,
        VaultError::PoolMintMismatch
    );

    let config = &mut ctx.accounts.config;
    config.pool = ctx.accounts.pool_state.key();
    config.max_slippage_bps = max_slippage_bps;
    msg!("Pool set: {} (max slippage {} bps)", config.pool, max_slippage_bps);
    Ok(())
}
