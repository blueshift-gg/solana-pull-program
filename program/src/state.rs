//! Account views. Every `unsafe` needed to overlay a layout on account bytes
//! lives here; handlers only see checked views. The program never holds a
//! checked borrow, so callers must not alias a view.

use crate::helpers::{check_pda, create_pda};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError};
pub use pull_core::state::{Nonces, Policy, Profile};
use pull_core::{constants::*, errors::PullError};

/// The account's bytes, after checking its owner, length and tag.
#[allow(clippy::mut_from_ref)]
fn bytes(account: &AccountInfo, len: usize, tag: u8) -> Result<&mut [u8], ProgramError> {
    if !account.is_owned_by(&crate::ID) {
        return Err(PullError::InvalidAccountOwner.into());
    }
    if account.data_len() < len {
        return Err(PullError::InvalidAccountLength.into());
    }
    // SAFETY: see the module doc; nothing else borrows the data.
    let data = unsafe { account.borrow_mut_data_unchecked() };
    if data[0] != tag {
        return Err(PullError::InvalidTag.into());
    }
    Ok(data)
}

/// A policy's header and the canonical terms after it.
#[allow(clippy::mut_from_ref)]
pub fn policy(account: &AccountInfo) -> Result<(&mut Policy, &[u8]), ProgramError> {
    let (header, rest) = bytes(account, POLICY_LEN, POLICY_TAG)?.split_at_mut(POLICY_LEN);
    // SAFETY: `header` holds exactly the layout; all fields have alignment 1.
    let header = unsafe { Policy::from_bytes_unchecked_mut(header) };
    let terms = rest
        .get(..header.terms_len() as usize)
        .ok_or(PullError::InvalidAccountLength)?;
    Ok((header, terms))
}

/// An existing page of nonces.
#[allow(clippy::mut_from_ref)]
pub fn nonces(account: &AccountInfo) -> Result<&mut Nonces, ProgramError> {
    // SAFETY: length checked by `bytes`; all fields have alignment 1.
    Ok(unsafe { Nonces::from_bytes_unchecked_mut(bytes(account, NONCES_LEN, NONCES_TAG)?) })
}

/// The profile of `authority`. Only `Open` creates one, at its PDA, and it
/// records whose it is.
#[allow(clippy::mut_from_ref)]
pub fn profile<'a>(
    account: &'a AccountInfo,
    authority: &[u8; 32],
) -> Result<&'a mut Profile, ProgramError> {
    let data = bytes(account, PROFILE_LEN, PROFILE_TAG).map_err(|_| PullError::InvalidProfile)?;
    // SAFETY: length checked by `bytes`; all fields have alignment 1.
    let profile = unsafe { Profile::from_bytes_unchecked_mut(data) };
    if profile.authority.ne(authority) {
        return Err(PullError::InvalidProfile.into());
    }
    Ok(profile)
}

/// The page that holds the nonce of an intent of `authority` with this salt,
/// under nonce index `index`, created with rent from `payer` if the intent
/// is its first.
#[allow(clippy::mut_from_ref)]
pub fn nonces_for<'a>(
    payer: &AccountInfo,
    account: &'a AccountInfo,
    authority: &[u8; 32],
    index: u64,
    salt: u64,
) -> Result<&'a mut Nonces, ProgramError> {
    let number = salt / NONCE_BITS as u64;
    if account.is_owned_by(&crate::ID) {
        // The page recorded its place when it was created at its PDA
        let page = nonces(account)?;
        if page.authority.ne(authority) || page.index() != index || page.page() != number {
            return Err(PullError::InvalidSeeds.into());
        }
        return Ok(page);
    }
    let (index_bytes, number_bytes) = (index.to_le_bytes(), number.to_le_bytes());
    let seeds: [&[u8]; 4] = [NONCES_SEED, authority, &index_bytes, &number_bytes];
    let bump = check_pda(account, &seeds)?;
    create_pda(payer, account, NONCES_LEN, &seeds, bump)?;
    // SAFETY: the account was just created with `NONCES_LEN` zeroed bytes.
    let page = unsafe { Nonces::from_bytes_unchecked_mut(account.borrow_mut_data_unchecked()) };
    page.set_tag(NONCES_TAG);
    page.authority = *authority;
    page.set_index(index);
    page.set_page(number);
    page.payer = *payer.key();
    Ok(page)
}
