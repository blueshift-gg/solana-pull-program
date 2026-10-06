use crate::events::emit;
use crate::helpers::close;
use crate::state::{nonces_for, policy, profile};
use pinocchio::log::sol_log;
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::errors::PullError;
use pull_core::terms::Terms;

/// # Cancel
///
/// The authority ends one thing, whichever account it passes: a policy, which
/// closes at once with its rent back to whoever paid it, or one signed
/// intent, whose nonce is used up so it can never be filled. `Invalidate`
/// ends all of either at once.
///
/// > A policy: close it into its payer
/// > An intent: mark its nonce used, creating its page if needed
///
/// Accounts:
///
/// 1. authority:       [signer]
/// 2. payer:           [mut]           a policy: its recorded payer, receives the rent;
///    an intent: also a signer, funds the page of nonces if it is new
/// 3. profile:                         the authority's
/// 4. account:         [mut]           a policy, or the page of nonces at
///    PDA [NONCES_SEED, authority, nonce_index, salt / NONCE_BITS]
/// 5. system_program:  [executable]
/// 6. engine:                          event signer
/// 7. program:         [executable]    this program, for the event CPI
///
/// Parameters:
/// 1. salt: u64,       // the intent's salt; nothing for a policy
///
/// Account Checks:
/// - Authority: signer; the policy's authority, or the one the page is derived from
/// - Payer: a policy's recorded payer, checked in process
/// - Profile: the authority's, checked in process for an intent
/// - Account: writable; a policy, or the page's PDA, checked in process
/// - SystemProgram, Engine, Program: no need to check since the CPIs fail otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 11u8)
/// - authority: Pubkey,
/// - account: Pubkey,
/// - salt: u64, (zero for a policy)
pub struct Cancel<'a> {
    pub authority: &'a AccountInfo,
    pub payer: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub account: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
    pub salt: Option<u64>,
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Cancel<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        sol_log("Cancel");

        let [authority, payer, profile, account, _system_program, engine, program] = accounts
        else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };
        let salt = match data.len() {
            0 => None,
            8 => Some(u64::from_le_bytes(data.try_into().unwrap())),
            _ => return Err(ProgramError::InvalidInstructionData),
        };

        // Account Checks
        if !authority.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if !account.is_writable() || !payer.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        Ok(Self {
            authority,
            payer,
            profile,
            account,
            engine,
            program,
            salt,
        })
    }
}

impl<'a> Cancel<'a> {
    pub const DISCRIMINATOR: &'a u8 = &11;

    pub fn process(&mut self) -> ProgramResult {
        let authority = self.authority.key();

        match self.salt {
            // Without a salt the account is a policy: the authority's own, closed for good
            None => {
                let (policy, bytes) = policy(self.account)?;
                if Terms::decode(bytes)?.authority.ne(authority) {
                    return Err(PullError::InvalidAuthority.into());
                }
                if policy.payer.ne(self.payer.key()) {
                    return Err(PullError::InvalidPayer.into());
                }
                close(self.account, self.payer)?;
            }
            // Cancelling what was used or cancelled already changes nothing
            Some(salt) => {
                let index = profile(self.profile, authority)?.nonce_index();
                nonces_for(self.payer, self.account, authority, index, salt)?.take(salt);
            }
        }

        // Log the Cancel Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[
                authority,
                self.account.key(),
                &self.salt.unwrap_or(0).to_le_bytes(),
            ],
        )
    }
}
