//! CPI helpers into the SPL Token program.
//!
//! Two kinds of CPI are used by the vault:
//!
//! 1. **Signed by the user** (`CpiContext::new`) — the user's signature from the
//!    outer transaction is forwarded to the Token program (`invoke`).
//!    Example: burning the user's share tokens.
//!
//! 2. **Signed by a PDA** (`CpiContext::new_with_signer`) — the vault config PDA
//!    has no private key; the program "signs" for it by passing the PDA seeds.
//!    The runtime re-derives the address from `seeds + bump + program_id`
//!    and, if it matches, marks the PDA as a signer (`invoke_signed`).
//!    Examples: moving tokens out of the vault, minting share tokens.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, MintTo, Token, TokenAccount, TransferChecked};

use crate::{constants::CONFIG_SEED, state::VaultConfig};

/// User burns their own share tokens (user is the authority -> plain CPI).
pub fn burn_shares<'info>(
    token_program: &Program<'info, Token>,
    share_mint: &Account<'info, Mint>,
    from: &Account<'info, TokenAccount>,
    owner: &Signer<'info>,
    amount: u64,
) -> Result<()> {
    token::burn(
        CpiContext::new(
            token_program.to_account_info(),
            Burn {
                mint: share_mint.to_account_info(),
                from: from.to_account_info(),
                authority: owner.to_account_info(),
            },
        ),
        amount,
    )
}

/// Vault -> user transfer. Authority of the vault token account is the
/// config PDA, so the CPI is signed with seeds ["config", underlying_mint, bump].
pub fn transfer_from_vault<'info>(
    token_program: &Program<'info, Token>,
    vault_token_account: &Account<'info, TokenAccount>,
    underlying_mint: &Account<'info, Mint>,
    to: &Account<'info, TokenAccount>,
    config: &Account<'info, VaultConfig>,
    amount: u64,
) -> Result<()> {
    let mint_key = config.underlying_mint;
    let bump = [config.bump];
    let seeds: &[&[u8]] = &[CONFIG_SEED, mint_key.as_ref(), &bump];
    let signer_seeds = &[seeds];

    token::transfer_checked(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            TransferChecked {
                from: vault_token_account.to_account_info(),
                mint: underlying_mint.to_account_info(),
                to: to.to_account_info(),
                authority: config.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
        underlying_mint.decimals,
    )
}

/// Mint share tokens to a user. Mint authority is the config PDA -> signed CPI.
pub fn mint_shares<'info>(
    token_program: &Program<'info, Token>,
    share_mint: &Account<'info, Mint>,
    to: &Account<'info, TokenAccount>,
    config: &Account<'info, VaultConfig>,
    amount: u64,
) -> Result<()> {
    let mint_key = config.underlying_mint;
    let bump = [config.bump];
    let seeds: &[&[u8]] = &[CONFIG_SEED, mint_key.as_ref(), &bump];
    let signer_seeds = &[seeds];

    token::mint_to(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            MintTo {
                mint: share_mint.to_account_info(),
                to: to.to_account_info(),
                authority: config.to_account_info(),
            },
            signer_seeds,
        ),
        amount,
    )
}
