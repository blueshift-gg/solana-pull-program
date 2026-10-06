use crate::events::emit;
use crate::helpers::{check_pda, create_pda};
use crate::state::{profile, Policy};
use pinocchio::log::sol_log;
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::{constants::*, errors::PullError, terms::Terms};

/// # Create
///
/// Put a policy on chain: a standing permission its spender uses with `Pull`.
/// Only a transaction the authority signs creates one, so closing it is final.
///
/// > Take the next number from the authority's profile
/// > Create the Policy PDA for that number
/// > Copy the canonical terms after its header
///
/// Accounts:
///
/// 1. authority:       [signer]
/// 2. payer:           [signer, mut]   funds the rent; refunded by `Close`
/// 3. profile:         [mut]           the authority's
/// 4. policy:          [mut]           PDA [POLICY_SEED, authority, index]
/// 5. system_program:  [executable]
/// 6. engine:                          event signer
/// 7. program:         [executable]    this program, for the event CPI
///
/// Parameters:
/// 1. terms: [u8],     // canonical terms: the rest of the instruction data
///
/// Account Checks:
/// - Authority: signer, and the terms' authority
/// - Profile: writable; the authority's, checked in process
/// - Policy: writable; the PDA for the next number, checked in process
/// - Payer, SystemProgram, Engine, Program: no need to check since the CPIs fail otherwise
///
/// Instruction Checks:
/// - Terms: canonical and valid
///
/// Event Data:
/// - discriminator: u8, (255u8, 0u8)
/// - policy: Pubkey,
/// - authority: Pubkey,
pub struct Create<'a> {
    pub authority: &'a AccountInfo,
    pub payer: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub policy: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
    pub bytes: &'a [u8],
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Create<'a> {
    type Error = ProgramError;

    fn try_from((bytes, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        sol_log("Create");

        let [authority, payer, profile, policy, _system_program, engine, program] = accounts else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Instruction Checks
        let terms = Terms::decode(bytes)?;

        // Account Checks
        if !authority.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if terms.authority.ne(authority.key()) {
            return Err(PullError::InvalidAuthority.into());
        }
        if !profile.is_writable() || !policy.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        Ok(Self {
            authority,
            payer,
            profile,
            policy,
            engine,
            program,
            bytes,
        })
    }
}

impl<'a> Create<'a> {
    pub const DISCRIMINATOR: &'a u8 = &0;

    pub fn process(&mut self) -> ProgramResult {
        let authority = self.authority.key();

        // The policy takes the next number, and must be the PDA for it
        let profile = profile(self.profile, authority)?;
        let index = profile
            .policies()
            .checked_add(1)
            .ok_or(PullError::Overflow)?;
        profile.set_policies(index);
        let index_bytes = index.to_le_bytes();
        let seeds: [&[u8]; 3] = [POLICY_SEED, authority, &index_bytes];
        let bump = check_pda(self.policy, &seeds)?;

        // Create it and copy the terms after the header
        let len = POLICY_LEN + self.bytes.len();
        create_pda(self.payer, self.policy, len, &seeds, bump)?;
        // SAFETY: the account was just created with `len` bytes, and nothing else borrows it.
        let data = unsafe { self.policy.borrow_mut_data_unchecked() };
        data[POLICY_LEN..len].copy_from_slice(self.bytes);
        // SAFETY: `data` holds at least the header; all fields have alignment 1.
        let policy = unsafe { Policy::from_bytes_unchecked_mut(data) };
        policy.set_tag(POLICY_TAG);
        policy.payer = *self.payer.key();
        policy.set_index(index);
        policy.set_terms_len(self.bytes.len() as u16);

        // Log the Create Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[self.policy.key(), authority],
        )
    }
}
