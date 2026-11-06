use crate::{constants::*, errors::VaultError};
use anchor_lang::prelude::*;

/// shares = amount * (total_shares + VIRTUAL_SHARES) / (total_assets + VIRTUAL_ASSETS)
///
/// Virtual offset (like ERC-4626 / OpenZeppelin) protects against the
/// "first depositor / inflation attack": an attacker cannot make the
/// share price so high that the next depositor gets 0 shares for free.
/// First deposit is still 1:1 (amount * 1 / 1).
/// Result is rounded DOWN -> rounding always favours the vault.
pub fn shares_for_deposit(amount: u64, total_shares: u64, total_assets: u64) -> Result<u64> {
    let num = (amount as u128)
        .checked_mul(total_shares as u128 + VIRTUAL_SHARES as u128)
        .ok_or(VaultError::MathOverflow)?;
    let den = total_assets as u128 + VIRTUAL_ASSETS as u128;
    u64::try_from(num / den).map_err(|_| error!(VaultError::MathOverflow))
}

/// assets = shares * (total_assets + VIRTUAL_ASSETS) / (total_shares + VIRTUAL_SHARES)
/// Rounded DOWN -> rounding favours the vault.
pub fn assets_for_shares(shares: u64, total_shares: u64, total_assets: u64) -> Result<u64> {
    require!(total_shares > 0, VaultError::InsufficientShares);
    let num = (shares as u128)
        .checked_mul(total_assets as u128 + VIRTUAL_ASSETS as u128)
        .ok_or(VaultError::MathOverflow)?;
    let den = total_shares as u128 + VIRTUAL_SHARES as u128;
    u64::try_from(num / den).map_err(|_| error!(VaultError::MathOverflow))
}

/// Withdraw fee: gross * fee_bps / 10_000 (rounded down)
pub fn fee_for(gross: u64, fee_bps: u16) -> Result<u64> {
    let v = (gross as u128)
        .checked_mul(fee_bps as u128)
        .ok_or(VaultError::MathOverflow)?
        / MAX_BPS as u128;
    Ok(v as u64)
}

/// value * part / whole (rounded down), used to reduce the cost basis
pub fn proportional(value: u64, part: u64, whole: u64) -> Result<u64> {
    if whole == 0 {
        return Ok(0);
    }
    let v = (value as u128)
        .checked_mul(part as u128)
        .ok_or(VaultError::MathOverflow)?
        / whole as u128;
    u64::try_from(v).map_err(|_| error!(VaultError::MathOverflow))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn withdraw_at_same_price_returns_deposit() {
        assert_eq!(assets_for_shares(400, 1_000, 1_000).unwrap(), 400);
    }

    #[test]
    fn withdraw_after_yield_returns_more() {
        // 100 shares, vault grew to 150 -> 40 shares = 60 tokens (minus rounding)
        let a = assets_for_shares(40_000_000, 100_000_000, 150_000_000).unwrap();
        assert!(a >= 59_999_999 && a <= 60_000_000);
    }

    #[test]
    fn full_withdraw_takes_everything_but_dust() {
        let a = assets_for_shares(1_000_000, 1_000_000, 1_500_000).unwrap();
        assert!(a <= 1_500_000 && a >= 1_499_999);
    }

    #[test]
    fn fee_math() {
        assert_eq!(fee_for(1_000_000, 50).unwrap(), 5_000); // 0.5%
        assert_eq!(fee_for(1_000_000, 0).unwrap(), 0);
        assert_eq!(fee_for(199, 50).unwrap(), 0); // rounds down
    }

    #[test]
    fn proportional_basis() {
        assert_eq!(proportional(150, 40, 100).unwrap(), 60);
        assert_eq!(proportional(150, 0, 100).unwrap(), 0);
        assert_eq!(proportional(150, 5, 0).unwrap(), 0);
    }

    #[test]
    fn redeem_with_no_shares_fails() {
        assert!(assets_for_shares(1, 0, 100).is_err());
    }

    #[test]
    fn first_deposit_is_one_to_one() {
        assert_eq!(shares_for_deposit(1_000, 0, 0).unwrap(), 1_000);
    }

    #[test]
    fn second_deposit_same_price_is_one_to_one() {
        assert_eq!(shares_for_deposit(500, 1_000, 1_000).unwrap(), 500);
    }

    #[test]
    fn after_yield_new_depositor_gets_fewer_shares() {
        // 100 shares, vault grew to 150 tokens -> price 1.5
        assert_eq!(shares_for_deposit(150, 100, 150).unwrap(), 100);
        assert_eq!(shares_for_deposit(150_000_000, 100_000_000, 150_000_000).unwrap(), 100_000_000);
    }

    #[test]
    fn rounding_favours_vault() {
        // 1 token at price 1.5 -> 0.66 share -> rounds down to 0
        assert_eq!(shares_for_deposit(1, 100, 150).unwrap(), 0);
    }

    #[test]
    fn inflation_attack_is_unprofitable() {
        // attacker deposits 1, donates 1_000_000 directly to the vault
        // victim deposits 2_000_000 -> still receives a share (not 0)
        let s = shares_for_deposit(2_000_000, 1, 1_000_001).unwrap();
        assert!(s >= 1);
    }

    #[test]
    fn deposit_then_redeem_never_gives_more() {
        let shares = shares_for_deposit(333, 1_000, 1_234).unwrap();
        let back = assets_for_shares(shares, 1_000 + shares, 1_234 + 333).unwrap();
        assert!(back <= 333);
    }

    #[test]
    fn overflow_is_caught_not_wrapped() {
        // u64::MAX * u64::MAX fits in u128, but result does not fit in u64
        assert!(shares_for_deposit(u64::MAX, u64::MAX, 0).is_err());
    }
}
