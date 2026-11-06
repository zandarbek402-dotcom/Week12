use anchor_lang::prelude::*;

/// Emitted on every successful deposit
#[event]
pub struct DepositEvent {
    pub user: Pubkey,
    pub config: Pubkey,
    /// Underlying tokens deposited
    pub amount: u64,
    /// Share tokens minted to the user
    pub shares: u64,
    /// Vault state AFTER the deposit
    pub total_assets: u64,
    pub total_shares: u64,
    pub timestamp: i64,
}

/// Emitted when the vault swaps through Raydium CLMM
#[event]
pub struct VaultSwapEvent {
    pub config: Pubkey,
    pub pool: Pubkey,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub amount_in: u64,
    /// Actually received (measured by balance change)
    pub amount_out: u64,
    pub min_amount_out: u64,
    /// Expected at spot price after fee (before price impact)
    pub spot_amount_out: u64,
    pub sqrt_price_before: u128,
    pub sqrt_price_after: u128,
    pub timestamp: i64,
}

/// Emitted on every successful withdraw
#[event]
pub struct WithdrawEvent {
    pub user: Pubkey,
    pub config: Pubkey,
    /// Share tokens burned
    pub shares: u64,
    /// Underlying tokens sent to the user (after fee)
    pub amount: u64,
    /// Fee kept in the vault
    pub fee: u64,
    /// User's shares remaining
    pub shares_left: u64,
    pub timestamp: i64,
}
