use crate::events::emit;
use crate::helpers::close;
use crate::state::policy;
use pinocchio::log::sol_log;
use pinocchio::sysvars::{clock::Clock, Sysvar};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::errors::PullError;
use pull_core::terms::Terms;

/// # Close
///
/// End a policy and return its rent to whoever paid it: its authority or its
/// spender at any time, and anyone once it has expired. Only a transaction can
/// create a policy, so it is gone for good.
///
/// > Close the Policy into its payer
///
/// Accounts:
///
/// 1. closer:          [signer]
/// 2. policy:          [mut]
/// 3. payer:           [mut]           the recorded payer, receives the rent
/// 4. engine:                          event signer
/// 5. program:         [executable]    this program, for the event CPI
///
/// Account Checks:
/// - Closer: signer; allowed to close, checked in process
/// - Policy: writable, loaded in process
/// - Payer: writable, the recorded payer, checked in process
/// - Engine, Program: no need to check since the event CPI fails otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 2u8)
/// - policy: Pubkey,
/// - closer: Pubkey,
pub struct Close<'a> {
    pub closer: &'a AccountInfo,
    pub account: &'a AccountInfo,
    pub payer: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
}

impl<'a> TryFrom<&'a [AccountInfo]> for Close<'a> {
    type Error = ProgramError;

    fn try_from(accounts: &'a [AccountInfo]) -> Result<Self, Self::Error> {
        sol_log("Close");

        let [closer, account, payer, engine, program] = accounts else {
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
            engine,
            program,
        })
    }
}

impl<'a> Close<'a> {
    pub const DISCRIMINATOR: &'a u8 = &2;

    pub fn process(&mut self) -> ProgramResult {
        let now = Clock::get()?.unix_timestamp;
        let closer = self.closer.key();

        // The authority and the spender decide; after the expiry anyone may
        let (policy, bytes) = policy(self.account)?;
        let terms = Terms::decode(bytes)?;
        let party = terms.authority.eq(closer) || terms.spender.is_some_and(|s| s.eq(closer));
        let allowed = party || terms.not_after.is_some_and(|t| now >= t);
        let payer = policy.payer;
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
