//! Account lifecycle, token views and transfers, and hashing.

use pinocchio::{
    account_info::AccountInfo,
    cpi::invoke_signed,
    instruction::{AccountMeta, Instruction, Seed, Signer},
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    sysvars::{rent::Rent, Sysvar},
    ProgramResult,
};
use pull_core::{constants::*, errors::PullError};

/// `account` must be the PDA for `seeds`; returns its bump.
#[inline(always)]
pub fn check_pda(account: &AccountInfo, seeds: &[&[u8]]) -> Result<u8, ProgramError> {
    let (key, bump) = find_program_address(seeds, &crate::ID);
    if key.ne(account.key()) {
        return Err(PullError::InvalidSeeds.into());
    }
    Ok(bump)
}

/// Create a program-owned PDA, rent paid by `payer`.
pub fn create_pda(
    payer: &AccountInfo,
    account: &AccountInfo,
    space: usize,
    seeds: &[&[u8]],
    bump: u8,
) -> ProgramResult {
    let bump = [bump];
    let padded: [&[u8]; 4] = core::array::from_fn(|i| match i.cmp(&seeds.len()) {
        core::cmp::Ordering::Less => seeds[i],
        core::cmp::Ordering::Equal => &bump,
        core::cmp::Ordering::Greater => &[],
    });
    let signer_seeds = padded.map(Seed::from);
    let signer = [Signer::from(&signer_seeds[..=seeds.len()])];

    let lamports = Rent::get()?.minimum_balance(space);
    if account.lamports() == 0 {
        return pinocchio_system::instructions::CreateAccount {
            from: payer,
            to: account,
            lamports,
            space: space as u64,
            owner: &crate::ID,
        }
        .invoke_signed(&signer);
    }
    // Anyone can send lamports to a PDA before it exists.
    let missing = lamports.saturating_sub(account.lamports());
    if missing > 0 {
        pinocchio_system::instructions::Transfer {
            from: payer,
            to: account,
            lamports: missing,
        }
        .invoke()?;
    }
    pinocchio_system::instructions::Allocate {
        account,
        space: space as u64,
    }
    .invoke_signed(&signer)?;
    pinocchio_system::instructions::Assign {
        account,
        owner: &crate::ID,
    }
    .invoke_signed(&signer)
}

/// Move every lamport to `to` and close a program-owned account.
pub fn close(account: &AccountInfo, to: &AccountInfo) -> ProgramResult {
    // SAFETY: the program never holds a checked borrow on lamports, and `to`
    // is distinct from `account` at every call site.
    unsafe {
        *to.borrow_mut_lamports_unchecked() = to
            .lamports()
            .checked_add(account.lamports())
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }
    account.close()
}

/// Token account or mint data, by the base-layout length or Token-2022's
/// account-type byte after it.
fn token_data(account: &AccountInfo, base_len: usize, kind: u8) -> Result<&[u8], PullError> {
    if !account.is_owned_by(&TOKEN_PROGRAM) && !account.is_owned_by(&TOKEN_2022_PROGRAM) {
        return Err(PullError::InvalidTarget);
    }
    // SAFETY: the program never holds a mutable borrow of a token program's account.
    let data = unsafe { account.borrow_data_unchecked() };
    if data.len() == base_len || (data.len() > 165 && data[165] == kind) {
        return Ok(data);
    }
    Err(PullError::InvalidTarget)
}

pub fn decimals(mint: &AccountInfo) -> Result<u8, PullError> {
    Ok(token_data(mint, 82, 1)?[44])
}

/// The balance of token account `account`, after checking it is what the
/// terms name: this mint, and the authority's own. The engine is the delegate
/// of many wallets, so this is what keeps a policy to its authority's funds.
pub fn balance(account: &AccountInfo, mint: &Pubkey, owner: &Pubkey) -> Result<u64, PullError> {
    let data = token_data(account, 165, 2)?;
    if data[..32].ne(mint) || data[32..64].ne(owner) {
        return Err(PullError::InvalidTarget);
    }
    Ok(u64::from_le_bytes(data[64..72].try_into().unwrap()))
}

/// What is left of the engine's approval on token account `account`.
pub fn allowance(account: &AccountInfo) -> Result<u64, PullError> {
    let data = token_data(account, 165, 2)?;
    // delegate: COption<Pubkey> at 72, delegated_amount at 121
    if data[72..76].ne(&[1, 0, 0, 0]) || data[76..108].ne(&ENGINE) {
        return Err(PullError::NotDelegate);
    }
    Ok(u64::from_le_bytes(data[121..129].try_into().unwrap()))
}

/// `TransferChecked`, signed by the engine PDA when `authority` is the engine
/// (a pull, as delegate) and by the transaction otherwise (the spender paying
/// the price). The instruction layout is shared by SPL Token and Token-2022.
pub fn transfer(
    from: &AccountInfo,
    mint: &AccountInfo,
    to: &AccountInfo,
    authority: &AccountInfo,
    amount: u64,
) -> ProgramResult {
    let mut data = [12; 10];
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    data[9] = decimals(mint)?;
    let bump = [ENGINE_BUMP];
    let seeds = [Seed::from(ENGINE_SEED), Seed::from(&bump)];
    let engine = [Signer::from(&seeds)];
    let signers: &[Signer] = match authority.key().eq(&ENGINE) {
        true => &engine,
        false => &[],
    };
    invoke_signed(
        &Instruction {
            // SAFETY: `owner` is only read here, before any CPI changes it.
            program_id: unsafe { from.owner() },
            accounts: &[
                AccountMeta::writable(from.key()),
                AccountMeta::readonly(mint.key()),
                AccountMeta::writable(to.key()),
                AccountMeta::readonly_signer(authority.key()),
            ],
            data: &data,
        },
        &[from, mint, to, authority],
        signers,
    )
}

#[cfg(target_os = "solana")]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    let parts = [bytes];
    // SAFETY: `sol_sha256` reads an array of slices and writes exactly 32 bytes.
    unsafe {
        pinocchio::syscalls::sol_sha256(
            parts.as_ptr() as *const u8,
            parts.len() as u64,
            out.as_mut_ptr(),
        );
    }
    out
}

/// Host builds (unit tests, clippy) have no syscall to link against.
#[cfg(not(target_os = "solana"))]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}
