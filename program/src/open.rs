use crate::events::emit;
use crate::helpers::{check_pda, create_pda, entropy};
use crate::state::Profile;
use pinocchio::log::sol_log;
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::{constants::*, errors::PullError};

/// # Open
///
/// Create the authority's profile, once: what numbers its policies and what
/// its intents are signed under. It goes in the transaction that enables the
/// first token account.
///
/// > Create the Profile PDA
/// > Start the nonce index where nobody could have signed for it
///
/// Accounts:
///
/// 1. authority:       [signer]
/// 2. payer:           [signer, mut]   funds the rent
/// 3. profile:         [mut]           PDA [PROFILE_SEED, authority]
/// 4. system_program:  [executable]
/// 5. engine:                          event signer
/// 6. program:         [executable]    this program, for the event CPI
///
/// Account Checks:
/// - Authority: signer
/// - Profile: writable; the empty PDA, checked in process
/// - Payer, SystemProgram, Engine, Program: no need to check since the CPIs fail otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 3u8)
/// - authority: Pubkey,
/// - nonce_index: u64,
pub struct Open<'a> {
    pub authority: &'a AccountInfo,
    pub payer: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
}

impl<'a> TryFrom<&'a [AccountInfo]> for Open<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        sol_log("Open");

        let [authority, payer, profile, _system_program, engine, program] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Account Checks
        if !authority.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if !profile.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        Ok(Self {
            authority,
            payer,
            profile,
            engine,
            program,
        })
    }
}

impl<'a> Open<'a> {
    pub const DISCRIMINATOR: &'a u8 = &3;

    pub fn process(&mut self) -> ProgramResult {
        let authority = self.authority.key();

        let seeds: [&[u8]; 2] = [PROFILE_SEED, authority];
        let bump = check_pda(self.profile, &seeds)?;
        if self.profile.is_owned_by(&crate::ID) {
            return Err(PullError::AlreadyInitialized.into());
        }
        create_pda(self.payer, self.profile, PROFILE_LEN, &seeds, bump)?;
        // SAFETY: the account was just created with `PROFILE_LEN` zeroed bytes.
        let profile =
            unsafe { Profile::from_bytes_unchecked_mut(self.profile.borrow_mut_data_unchecked()) };
        let nonce_index = entropy()?;
        profile.set_tag(PROFILE_TAG);
        profile.authority = *authority;
        profile.set_nonce_index(nonce_index);

        // Log the Open Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[authority, &nonce_index.to_le_bytes()],
        )
    }
}
