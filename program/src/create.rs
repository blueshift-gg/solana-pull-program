use crate::events::emit;
use crate::helpers::{check_pda, create_pda};
use crate::state::{profile, Mandate};
use pinocchio::log::sol_log;
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::{constants::*, errors::PullError, terms::Terms};

/// # Create
///
/// Put a mandate on chain: a standing permission its spender uses with `Pull`.
/// Only a transaction the authority signs creates one, so closing it is final.
///
/// > Take the next number from the authority's profile
/// > Create the Mandate PDA for that number
/// > Copy the terms after its header
///
/// Accounts:
///
/// 1. authority:       [signer]
/// 2. payer:           [signer, mut]   funds the rent; refunded by `Close`
/// 3. profile:         [mut]           the authority's
/// 4. mandate:         [mut]           PDA [MANDATE_SEED, authority, index]
/// 5. system_program:  [executable]
/// 6. engine:                          event signer
/// 7. program:         [executable]    this program, for the event CPI
///
/// Parameters:
/// 1. terms: [u8; TERMS_LEN],
///
/// Account Checks:
/// - Authority: signer, and the terms' authority
/// - Profile: writable; the authority's, checked in process
/// - Mandate: writable; the PDA for the next number, checked in process
/// - Payer, SystemProgram, Engine, Program: no need to check since the CPIs fail otherwise
///
/// Instruction Checks:
/// - Terms: well-formed and valid
///
/// Event Data:
/// - discriminator: u8, (255u8, 0u8)
/// - mandate: Pubkey,
/// - authority: Pubkey,
pub struct Create<'a> {
    pub authority: &'a AccountInfo,
    pub payer: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub mandate: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
    pub bytes: &'a [u8],
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Create<'a> {
    type Error = ProgramError;

    fn try_from((bytes, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        sol_log("Create");

        let [authority, payer, profile, mandate, _system_program, engine, program] = accounts
        else {
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
        if !profile.is_writable() || !mandate.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        Ok(Self {
            authority,
            payer,
            profile,
            mandate,
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

        // The mandate takes the next number, and must be the PDA for it
        let profile = profile(self.profile, authority)?;
        let index = profile
            .mandates()
            .checked_add(1)
            .ok_or(PullError::Overflow)?;
        profile.set_mandates(index);
        let index_bytes = index.to_le_bytes();
        let seeds: [&[u8]; 3] = [MANDATE_SEED, authority, &index_bytes];
        let bump = check_pda(self.mandate, &seeds)?;

        // Create it and copy the terms after the header
        let len = MANDATE_LEN + TERMS_LEN;
        create_pda(self.payer, self.mandate, len, &seeds, bump)?;
        // SAFETY: the account was just created with `len` bytes, and nothing else borrows it.
        let data = unsafe { self.mandate.borrow_mut_data_unchecked() };
        data[MANDATE_LEN..len].copy_from_slice(self.bytes);
        // SAFETY: `data` holds at least the header; all fields have alignment 1.
        let mandate = unsafe { Mandate::from_bytes_unchecked_mut(data) };
        mandate.set_tag(MANDATE_TAG);
        mandate.payer = *self.payer.key();
        mandate.set_index(index);

        // Log the Create Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[self.mandate.key(), authority],
        )
    }
}
