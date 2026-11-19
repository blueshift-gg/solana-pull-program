//! Account lifecycle, token views and transfers.

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

/// Create the program-owned PDA for `seeds` (at most 3) at `account`, rent
/// paid by `payer`. The bump is the canonical one, found here: an address at
/// any other bump would be a second profile, or a mandate no wallet finds.
pub fn create_pda(
    payer: &AccountInfo,
    account: &AccountInfo,
    space: usize,
    seeds: &[&[u8]],
) -> ProgramResult {
    // Whoever is recorded as the payer signed for it, even when the address
    // already holds its rent: the rent of a closed account goes back there
    if !payer.is_signer() {
        return Err(PullError::NotSigner.into());
    }
    let (key, bump) = find_program_address(seeds, &crate::ID);
    if key.ne(account.key()) {
        return Err(PullError::InvalidSeeds.into());
    }
    let bump = [bump];
    let signer_seeds: [Seed; 4] =
        core::array::from_fn(|i| Seed::from(seeds.get(i).copied().unwrap_or(&bump)));
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
    if account.key().eq(to.key()) {
        return Err(PullError::InvalidPayer.into());
    }
    // SAFETY: the program never holds a checked borrow on lamports, and `to`
    // is not `account`.
    unsafe {
        *to.borrow_mut_lamports_unchecked() = to
            .lamports()
            .checked_add(account.lamports())
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }
    account.close()
}

/// Token account or mint data, by the base-layout length or, for Token-2022
/// with extensions, the account-type byte after it. A multisig is 355 bytes
/// in both programs, and never an account or a mint.
fn token_data(account: &AccountInfo, base_len: usize, kind: u8) -> Result<&[u8], PullError> {
    let extended = account.is_owned_by(&TOKEN_2022_PROGRAM);
    if !account.is_owned_by(&TOKEN_PROGRAM) && !extended {
        return Err(PullError::InvalidTarget);
    }
    // SAFETY: the program never holds a mutable borrow of a token program's account.
    let data = unsafe { account.borrow_data_unchecked() };
    let len = data.len();
    if len == base_len || (extended && len > 165 && len != 355 && data[165] == kind) {
        return Ok(data);
    }
    Err(PullError::InvalidTarget)
}

pub fn decimals(mint: &AccountInfo) -> Result<u8, PullError> {
    Ok(token_data(mint, 82, 1)?[44])
}

/// The balance of token account `account`, after checking it is what the
/// terms name: this mint, and the authority's own. The engine is the delegate
/// of many wallets, so this is what keeps a mandate to its authority's funds.
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

/// `TransferChecked` through token program `program`, which the caller has
/// checked. `as_engine` signs as the engine PDA: only the pull itself, as the
/// delegate. The transaction signs otherwise: the spender paying what the
/// authority receives. The instruction layout is shared by SPL Token and
/// Token-2022.
pub fn transfer(
    program: &Pubkey,
    from: &AccountInfo,
    mint: &AccountInfo,
    to: &AccountInfo,
    authority: &AccountInfo,
    amount: u64,
    as_engine: bool,
) -> ProgramResult {
    let mut data = [12; 10];
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    data[9] = decimals(mint)?;
    let bump = [ENGINE_BUMP];
    let seeds = [Seed::from(ENGINE_SEED), Seed::from(&bump)];
    let engine = [Signer::from(&seeds)];
    let signers: &[Signer] = if as_engine { &engine } else { &[] };
    invoke_signed(
        &Instruction {
            program_id: program,
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
