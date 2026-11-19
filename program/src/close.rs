use crate::events::emit;
use crate::helpers::close;
use crate::state::{mandate, profile};
use pinocchio::log::sol_log;
use pinocchio::sysvars::{clock::Clock, Sysvar};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::errors::PullError;
use pull_core::terms::Terms;

/// # Close
///
/// End a mandate and return its rent to whoever paid it. Its authority may
/// at any time; anyone may once it has expired or the authority invalidated
/// it, since it can never be used again.
///
/// > Close the mandate into its payer
///
/// Accounts:
///
/// 1. closer:          [signer]        the authority, or anyone once the mandate is dead
/// 2. mandate:         [mut]
/// 3. payer:           [mut]           the recorded payer, receives the rent
/// 4. profile:                         the authority's; read only if the closer is not
/// 5. engine:                          event signer
/// 6. program:         [executable]    this program, for the event CPI
///
/// Account Checks:
/// - Closer: signer; the authority, or anyone if the mandate is dead, checked in process
/// - Mandate: writable, loaded in process
/// - Payer: writable, the recorded payer, checked in process
/// - Profile: the authority's, checked in process
/// - Engine, Program: no need to check since the event CPI fails otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 2u8)
/// - mandate: Pubkey,
/// - closer: Pubkey,
pub struct Close<'a> {
    pub closer: &'a AccountInfo,
    pub mandate: &'a AccountInfo,
    pub payer: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
}

impl<'a> TryFrom<&'a [AccountInfo]> for Close<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        sol_log("Close");

        let [closer, mandate, payer, profile, engine, program] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Account Checks
        if !closer.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if !mandate.is_writable() || !payer.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        Ok(Self {
            closer,
            mandate,
            payer,
            profile,
            engine,
            program,
        })
    }
}

impl<'a> Close<'a> {
    pub const DISCRIMINATOR: &'a u8 = &2;

    pub fn process(&mut self) -> ProgramResult {
        let closer = self.closer.key();

        // Two views never share an account
        if self.mandate.key().eq(self.profile.key()) {
            return Err(PullError::InvalidProfile.into());
        }
        let (mandate, bytes) = mandate(self.mandate)?;
        let terms = Terms::decode(bytes)?;
        // Its authority ends it at any time; anyone ends one that can never pull again
        if terms.authority.ne(closer) {
            let now = Clock::get()?.unix_timestamp;
            let expired = terms.not_after.is_some_and(|t| now >= t);
            if !expired && mandate.index() > profile(self.profile, terms.authority)?.stale() {
                return Err(PullError::NotClosable.into());
            }
        }
        if mandate.payer.ne(self.payer.key()) {
            return Err(PullError::InvalidPayer.into());
        }
        close(self.mandate, self.payer)?;

        // Log the Close Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[self.mandate.key(), closer],
        )
    }
}
