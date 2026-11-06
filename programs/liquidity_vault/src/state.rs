use anchor_lang::prelude::*;

/// Global vault settings. PDA: ["config", underlying_mint]
#[account]
#[derive(InitSpace)]
pub struct VaultConfig {
    /// Who can change settings / pause the vault
    pub admin: Pubkey,
    /// Token that users deposit (e.g. USDC)
    pub underlying_mint: Pubkey,
    /// Share token (LP token of the vault). PDA: ["shares", config]
    pub share_mint: Pubkey,
    /// Token account that holds deposited tokens. PDA: ["vault", config]
    pub vault_token_account: Pubkey,
    /// Raydium CLMM pool the vault trades / provides liquidity in (set_pool)
    pub pool: Pubkey,
    /// Max allowed slippage vs. the pool spot price for vault swaps (bps)
    pub max_slippage_bps: u16,
    /// Withdraw fee in basis points (100 = 1%)
    pub fee_bps: u16,
    /// Max total underlying the vault accepts (0 = no limit)
    pub deposit_cap: u64,
    /// Sum of all deposits minus withdrawals (accounting)
    pub total_deposited: u64,
    /// Emergency stop
    pub paused: bool,
    pub bump: u8,
    pub share_mint_bump: u8,
    pub vault_bump: u8,
}

/// Per-user position. PDA: ["user", config, owner]
#[account]
#[derive(InitSpace)]
pub struct UserPosition {
    pub owner: Pubkey,
    pub config: Pubkey,
    /// Shares minted to this user through the vault
    pub shares: u64,
    /// Net underlying deposited by the user
    pub deposited: u64,
    pub last_action_ts: i64,
    pub bump: u8,
}
