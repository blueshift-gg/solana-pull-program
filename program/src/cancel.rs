use crate::events::emit;
use crate::state::nonces_for;
use pinocchio::log::sol_log;
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::errors::PullError;

/// # Cancel
///
/// Use up the nonce of a signed intent, so it can never be filled; or every
/// nonce of its page, which cancels up to 1,024 intents at once.
///
/// > Mark the nonce used, or the whole page, creating the page if needed
///
/// Accounts:
///
/// 1. authority:       [signer]
/// 2. payer:           [signer, mut]   funds the page of nonces if it is new
/// 3. nonces:          [mut]           PDA [NONCES_SEED, authority, salt / NONCE_BITS]
/// 4. system_program:  [executable]
/// 5. engine:                          event signer
/// 6. program:         [executable]    this program, for the event CPI
///
/// Parameters:
/// 1. salt: u64,       // the intent's salt
/// 2. page: u8,        // 1 to cancel every nonce of the salt's page, 0 for the salt alone
///
/// Account Checks:
/// - Authority: signer; the page is derived from it, so it can only cancel its own
/// - Nonces: writable; the PDA, or created there, checked in process
/// - Payer, SystemProgram, Engine, Program: no need to check since the CPIs fail otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 11u8)
/// - authority: Pubkey,
/// - salt: u64,
/// - page: u8,
pub struct Cancel<'a> {
    pub authority: &'a AccountInfo,
    pub payer: &'a AccountInfo,
    pub nonces: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
    pub salt: u64,
    pub page: bool,
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Cancel<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        sol_log("Cancel");

        let [authority, payer, nonces, _system_program, engine, program] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };
        let (salt, page) = match data.split_first_chunk::<8>() {
            Some((salt, [0])) => (salt, false),
            Some((salt, [1])) => (salt, true),
            _ => return Err(ProgramError::InvalidInstructionData),
        };

        // Account Checks
        if !authority.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if !nonces.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        Ok(Self {
            authority,
            payer,
            nonces,
            engine,
            program,
            salt: u64::from_le_bytes(*salt),
            page,
        })
    }
}

impl<'a> Cancel<'a> {
    pub const DISCRIMINATOR: &'a u8 = &11;

    pub fn process(&mut self) -> ProgramResult {
        let authority = self.authority.key();

        // Cancelling what was used or cancelled already changes nothing
        let nonces = nonces_for(self.payer, self.nonces, authority, self.salt)?;
        match self.page {
            true => nonces.take_all(),
            false => {
                nonces.take(self.salt);
            }
        }

        // Log the Cancel Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[authority, &self.salt.to_le_bytes(), &[self.page as u8]],
        )
    }
}
