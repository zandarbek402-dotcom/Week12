use anchor_lang::prelude::*;

#[error_code]
pub enum VaultError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Vault is paused")]
    VaultPaused,
    #[msg("Deposit would exceed the vault deposit cap")]
    DepositCapExceeded,
    #[msg("Fee is too high")]
    FeeTooHigh,
    #[msg("Not enough shares")]
    InsufficientShares,
    #[msg("Calculated shares amount is zero (deposit too small)")]
    ZeroShares,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Only the vault admin can do this")]
    Unauthorized,
    #[msg("Slippage: result is below the minimum requested")]
    SlippageExceeded,
    #[msg("Not enough tokens in user account")]
    InsufficientFunds,
    #[msg("Not enough idle liquidity in the vault")]
    InsufficientLiquidity,
    #[msg("Position still has shares, withdraw them first")]
    PositionNotEmpty,
    #[msg("Account is not the configured Raydium CLMM pool")]
    InvalidPool,
    #[msg("Raydium pool accounts do not match the pool state")]
    PoolAccountMismatch,
    #[msg("Pool does not contain the vault underlying token")]
    PoolMintMismatch,
    #[msg("min_amount_out is too low: below the allowed slippage vs pool spot price")]
    MinOutTooLow,
    #[msg("Slippage setting is too high")]
    SlippageTooHigh,
    #[msg("Input and output mint must differ")]
    SameMint,
    #[msg("Swap spent more input than requested")]
    InputOverspent,
}
