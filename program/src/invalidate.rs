use crate::events::emit;
use crate::helpers::entropy;
use crate::state::profile;
use pinocchio::log::sol_log;
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::errors::PullError;

/// # Invalidate
///
/// End everything of one kind at once: every policy the authority created so
/// far, every intent it signed so far, or both. What it ends can then be
/// closed by anyone, and its rent goes back to whoever paid it.
///
/// > Policies: mark every number handed out so far as stale
/// > Intents: move the nonce index by an amount nobody could know before
///
/// Accounts:
///
/// 1. authority:       [signer]
/// 2. profile:         [mut]           the authority's
/// 3. engine:                          event signer
/// 4. program:         [executable]    this program, for the event CPI
///
/// Parameters:
/// 1. what: u8,        // 1 for policies, 2 for intents, 3 for both
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
/// - nonce_index: u64,
pub struct Invalidate<'a> {
    pub authority: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
    pub policies: bool,
    pub intents: bool,
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Invalidate<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        sol_log("Invalidate");

        let [authority, profile, engine, program] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };
        let what = match data {
            [what @ 1..=3] => *what,
            _ => return Err(ProgramError::InvalidInstructionData),
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
            policies: what & 1 != 0,
            intents: what & 2 != 0,
        })
    }
}

impl<'a> Invalidate<'a> {
    pub const DISCRIMINATOR: &'a u8 = &4;

    pub fn process(&mut self) -> ProgramResult {
        let authority = self.authority.key();

        let profile = profile(self.profile, authority)?;
        if self.policies {
            profile.set_stale(profile.policies());
        }
        if self.intents {
            // Never by zero, and never by an amount known ahead: a signature
            // made for a later index cannot be waiting
            let step = entropy()? | 1;
            profile.set_nonce_index(profile.nonce_index().wrapping_add(step));
        }
        let (stale, nonce_index) = (profile.stale(), profile.nonce_index());

        // Log the Invalidate Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[authority, &stale.to_le_bytes(), &nonce_index.to_le_bytes()],
        )
    }
}
