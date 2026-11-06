/// PDA seeds
pub const CONFIG_SEED: &[u8] = b"config";
pub const SHARE_MINT_SEED: &[u8] = b"shares";
pub const VAULT_TOKEN_SEED: &[u8] = b"vault";
pub const USER_SEED: &[u8] = b"user";

/// 100% = 10_000 basis points
pub const MAX_BPS: u16 = 10_000;
/// Max allowed withdraw fee: 10%
pub const MAX_FEE_BPS: u16 = 1_000;

/// Vault swaps: default / max allowed slippage vs pool spot price
pub const DEFAULT_MAX_SLIPPAGE_BPS: u16 = 100; // 1%
pub const MAX_SLIPPAGE_LIMIT_BPS: u16 = 1_000; // 10%

/// Virtual offset for share price math (inflation-attack protection)
pub const VIRTUAL_SHARES: u64 = 1;
pub const VIRTUAL_ASSETS: u64 = 1;
