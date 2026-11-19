use crate::events::emit;
use crate::helpers::{allowance, balance, transfer};
use crate::state::{mandate, profile};
use pinocchio::log::sol_log;
use pinocchio::sysvars::{clock::Clock, Sysvar};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::errors::PullError;
use pull_core::terms::Terms;

/// # Pull
///
/// Take tokens under a mandate, within its limit. If the
/// mandate says what the authority receives, the spender pays it here, in the
/// same instruction: there is nothing in between to trust. Callable by CPI.
///
/// > Check the authority has not invalidated the mandate
/// > Check the window and the spender
/// > Check the amount against what is left of the limit, and record it
/// > Transfer from the source, as the engine delegate
/// > Receive: transfer the payment from the spender, and check what arrived
///
/// Accounts:
///
/// 1. spender:         [signer]
/// 2. mandate:         [mut]
/// 3. profile:                         the authority's
/// 4. from:            [mut]           the authority's token account
/// 5. mint:                            its mint
/// 6. to:              [mut]           where the tokens go: the spender's choice, unless the terms name it
/// 7. engine:                          SPL delegate and event signer
/// 8. program:         [executable]    this program, for the event CPI
/// 9. token_program:   [executable]    of `from`
///
/// If the authority receives something, also:
///
/// 10. pay_from:       [mut]           the spender's token account
/// 11. pay_mint:                       the mint of `pay_to`
/// 12. pay_to:         [mut]           the authority's token account the terms name
/// 13. pay_program:    [executable]    token program of `pay_from`
///
/// Parameters:
/// 1. amount: u64,
/// 2. reference: [u8; 32],    optional: the spender's own id for this pull,
///    an invoice or an order; emitted, never stored
///
/// Account Checks:
/// - Spender: signer; the one the terms name, checked in process
/// - Mandate: writable, loaded in process
/// - Profile: the authority's, checked in process
/// - From, PayTo: the accounts the terms name, the authority's, checked in process
/// - Mint, PayMint: the mints of `from` and `pay_to`, checked in process
/// - To, PayFrom, Engine, Program, token programs: no need to check since the CPIs fail otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 1u8)
/// - mandate: Pubkey,
/// - spender: Pubkey,
/// - amount: u64,
/// - paid: u64,
/// - reference: [u8; 32], (zeros when the pull gave none)
pub struct Pull<'a> {
    pub spender: &'a AccountInfo,
    pub mandate: &'a AccountInfo,
    pub profile: &'a AccountInfo,
    pub from: &'a AccountInfo,
    pub mint: &'a AccountInfo,
    pub to: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    pub program: &'a AccountInfo,
    /// `[pay_from, pay_mint, pay_to]` when the authority receives something.
    pub payment: &'a [AccountInfo],
    pub amount: u64,
    pub reference: [u8; 32],
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Pull<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        sol_log("Pull");

        let [spender, mandate, profile, from, mint, to, engine, program, _token_program, payment @ ..] =
            accounts
        else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Account Checks
        if !spender.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if !mandate.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        let (amount, reference) = match data.len() {
            8 => (data, [0; 32]),
            40 => (&data[..8], data[8..].try_into().unwrap()),
            _ => return Err(ProgramError::InvalidInstructionData),
        };

        Ok(Self {
            spender,
            mandate,
            profile,
            from,
            mint,
            to,
            engine,
            program,
            payment,
            amount: u64::from_le_bytes(amount.try_into().unwrap()),
            reference,
        })
    }
}

impl<'a> Pull<'a> {
    pub const DISCRIMINATOR: &'a u8 = &1;

    pub fn process(&mut self) -> ProgramResult {
        let now = Clock::get()?.unix_timestamp;
        let amount = self.amount;
        if amount == 0 {
            return Err(PullError::ZeroAmount.into());
        }

        // Two views never share an account
        if self.mandate.key().eq(self.profile.key()) {
            return Err(PullError::InvalidProfile.into());
        }
        let (mandate, bytes) = mandate(self.mandate)?;
        let terms = Terms::decode(bytes)?;
        if mandate.index() <= profile(self.profile, terms.authority)?.stale() {
            return Err(PullError::Stale.into());
        }

        // Only inside the window, and only by the spender the terms name
        if now < terms.not_before {
            return Err(PullError::NotYetValid.into());
        }
        if terms.not_after.is_some_and(|t| now >= t) {
            return Err(PullError::Expired.into());
        }
        if terms.spender.is_some_and(|s| s.ne(self.spender.key())) {
            return Err(PullError::InvalidSpender.into());
        }

        // From the account the terms limit, to the one they name if they name one
        let limit = terms.limit;
        if limit.from.ne(self.from.key()) {
            return Err(PullError::InvalidPull.into());
        }
        if limit.to.is_some_and(|to| to.ne(self.to.key())) {
            return Err(PullError::InvalidDestination.into());
        }

        // The amount must fit what is left of the limit
        let spent = mandate.spent(limit.period, terms.not_before, now);
        let spent = spent.checked_add(amount).ok_or(PullError::Overflow)?;
        if spent > limit.max {
            return Err(PullError::LimitExceeded.into());
        }
        mandate.set_consumed(spent);
        mandate.set_rolled(now);

        let paid = self.settle(&terms)?;

        // Log the Pull Event
        emit(
            self.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[
                self.mandate.key(),
                self.spender.key(),
                &amount.to_le_bytes(),
                &paid.to_le_bytes(),
                &self.reference,
            ],
        )
    }

    /// Take the amount as the delegate, only from the authority's own account.
    /// If the terms say what the authority receives, the spender pays it, and
    /// all of it must arrive. Returns what was paid.
    fn settle(&self, terms: &Terms) -> Result<u64, ProgramError> {
        let amount = self.amount;
        // `mint` must be the mint of `from`: the terms name only the account
        let held = balance(self.from, self.mint.key(), terms.authority)?;
        // Name what the token program would only refuse with its own codes
        if allowance(self.from)? < amount {
            return Err(PullError::AllowanceExceeded.into());
        }
        if held < amount {
            return Err(PullError::InsufficientFunds.into());
        }
        // SAFETY: `owner` is only read, and `balance` checked it is a token program.
        let program = unsafe { self.from.owner() };
        transfer(
            program,
            self.from,
            self.mint,
            self.to,
            self.engine,
            amount,
            true,
        )?;

        let Some(receive) = terms.receive else {
            return Ok(0);
        };
        let [pay_from, pay_mint, pay_to, ..] = self.payment else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };
        if receive.to.ne(pay_to.key()) {
            return Err(PullError::InvalidTarget.into());
        }
        // `pay_mint` must be the mint of `pay_to`, which the terms name
        let before = balance(pay_to, pay_mint.key(), terms.authority)?;
        // The payment goes through the token program of the account that
        // receives it, which `balance` just checked, whatever `pay_from` is
        // SAFETY: `owner` is only read.
        let program = unsafe { pay_to.owner() };
        transfer(
            program,
            pay_from,
            pay_mint,
            pay_to,
            self.spender,
            receive.min,
            false,
        )?;
        let after = balance(pay_to, pay_mint.key(), terms.authority)?;
        if after
            .checked_sub(before)
            .is_none_or(|got| got < receive.min)
        {
            return Err(PullError::NotReceived.into());
        }
        Ok(receive.min)
    }
}
