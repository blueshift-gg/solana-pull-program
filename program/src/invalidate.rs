use crate::events::emit;
use crate::state::profile;
use pinocchio::log::sol_log;
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::errors::PullError;

/// # Invalidate
///
/// End every mandate the authority created so far at once. What it ends can
/// then be closed by anyone, and its rent goes back to whoever paid it.
///
/// > Mark every number handed out so far as stale, and one more, so a
/// > `Create` signed earlier and held back cannot land after
///
/// Accounts:
///
/// 1. authority:       [signer]
/// 2. profile:         [mut]           the authority's
/// 3. engine:                          event signer
/// 4. program:         [executable]    this program, for the event CPI
///
/// Account Checks:
/// - Authority: signer
/// - Profile: writable; the authority's, checked in process
/// - Engine, Program: no need to check since the event CPI fails otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 4u8)
/// - authority: Pubkey,
/// - stale: u64,
pub struct Invalidate<'a> {
    pub authority: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
}

impl<'a> TryFrom<&'a [AccountInfo]> for Invalidate<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        sol_log("Invalidate");

        let [authority, profile, engine, program] = accounts else {
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
            profile,
            engine,
            program,
        })
    }
}

impl<'a> Invalidate<'a> {
    pub const DISCRIMINATOR: u8 = 4;

    pub fn process(&mut self) -> ProgramResult {
        let authority = self.authority.key();

        let profile = profile(self.profile, authority)?;
        // One number is used up and never given to a mandate. A `Create`
        // signed before this and held back names the address of that
        // number, so it can no longer land
        let stale = profile
            .mandates()
            .checked_add(1)
            .ok_or(PullError::Overflow)?;
        profile.set_mandates(stale);
        profile.set_stale(stale);
        emit(
            self.engine,
            self.program,
            Self::DISCRIMINATOR,
            &[authority, &stale.to_le_bytes()],
        )
    }
}
