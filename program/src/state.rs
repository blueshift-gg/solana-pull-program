//! Account views. Every `unsafe` needed to overlay a layout on account bytes
//! lives here; handlers only see checked views. The program never holds a
//! checked borrow, so callers must not alias a view.

use pinocchio::{account_info::AccountInfo, program_error::ProgramError};
pub use pull_core::state::{Mandate, Profile};
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

/// A mandate's header and the terms after it, which `Terms::decode` checks.
#[allow(clippy::mut_from_ref)]
pub fn mandate(account: &AccountInfo) -> Result<(&mut Mandate, &[u8]), ProgramError> {
    let (header, terms) = bytes(account, MANDATE_LEN, MANDATE_TAG)?.split_at_mut(MANDATE_LEN);
    // SAFETY: `header` holds exactly the layout; all fields have alignment 1.
    Ok((unsafe { Mandate::from_bytes_unchecked_mut(header) }, terms))
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
