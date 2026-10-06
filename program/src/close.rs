use crate::events::emit;
use crate::helpers::close;
use crate::state::{nonces, policy, profile};
use pinocchio::log::sol_log;
use pinocchio::sysvars::{clock::Clock, Sysvar};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::terms::Terms;
use pull_core::{constants::*, errors::PullError};

/// # Close
///
/// Close what can never be used again, and return its rent to whoever paid
/// it. Anyone may: a policy once it has expired or the authority invalidated
/// it, and a page of nonces once the authority's nonce index has moved on
/// from the page's. The authority ends a policy that is in force with `Cancel`.
///
/// > Close the account into its payer
///
/// Accounts:
///
/// 1. closer:          [signer]
/// 2. account:         [mut]           a policy or a page of nonces
/// 3. payer:           [mut]           the recorded payer, receives the rent
/// 4. profile:                         the authority's
/// 5. engine:                          event signer
/// 6. program:         [executable]    this program, for the event CPI
///
/// Account Checks:
/// - Closer: signer; anyone
/// - Account: writable, loaded in process
/// - Payer: writable, the recorded payer, checked in process
/// - Profile: the authority's, checked in process
/// - Engine, Program: no need to check since the event CPI fails otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 2u8)
/// - account: Pubkey,
/// - closer: Pubkey,
pub struct Close<'a> {
    pub closer: &'a AccountInfo,
    pub account: &'a AccountInfo,
    pub payer: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
}

impl<'a> TryFrom<&'a [AccountInfo]> for Close<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        sol_log("Close");

        let [closer, account, payer, profile, engine, program] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Account Checks
        if !closer.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if !account.is_writable() || !payer.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        Ok(Self {
            closer,
            account,
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
        if self.account.key().eq(self.profile.key()) {
            return Err(PullError::InvalidProfile.into());
        }
        // SAFETY: nothing else borrows the data; the loaders below check the owner.
        let tag = unsafe { self.account.borrow_data_unchecked() }.first();
        let (allowed, payer) = match tag == Some(&NONCES_TAG) {
            // Its intents no longer verify, so its bits protect nothing
            true => {
                let page = nonces(self.account)?;
                let current = profile(self.profile, &page.authority)?.nonce_index();
                (page.index() != current, page.payer)
            }
            // A policy that can never pull again
            false => {
                let (policy, bytes) = policy(self.account)?;
                let terms = Terms::decode(bytes)?;
                let now = Clock::get()?.unix_timestamp;
                let stale = policy.index() <= profile(self.profile, terms.authority)?.stale();
                let expired = terms.not_after.is_some_and(|t| now >= t);
                (stale || expired, policy.payer)
            }
        };
        if !allowed {
            return Err(PullError::NotClosable.into());
        }
        if payer.ne(self.payer.key()) {
            return Err(PullError::InvalidPayer.into());
        }
        close(self.account, self.payer)?;

        // Log the Close Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[self.account.key(), closer],
        )
    }
}
