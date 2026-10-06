use crate::events::emit;
use crate::helpers::{allowance, balance, transfer};
use crate::state::policy;
use pinocchio::log::sol_log;
use pinocchio::sysvars::{clock::Clock, Sysvar};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::errors::PullError;
use pull_core::terms::{Limit, Terms};

/// The accounts a pull moves tokens between, shared by `Pull` and `Fill`.
pub struct Legs<'a> {
    pub spender: &'a AccountInfo,
    pub from: &'a AccountInfo,
    pub mint: &'a AccountInfo,
    pub to: &'a AccountInfo,
    pub engine: &'a AccountInfo,
    /// `[pay_from, pay_mint, pay_to]` when the authority receives something.
    pub payment: &'a [AccountInfo],
}

impl Legs<'_> {
    /// Only inside the window, and only by the spender the terms name.
    pub fn check(&self, terms: &Terms, now: i64) -> ProgramResult {
        if now < terms.not_before {
            return Err(PullError::NotYetValid.into());
        }
        if terms.not_after.is_some_and(|t| now >= t) {
            return Err(PullError::Expired.into());
        }
        if terms.spender.is_some_and(|s| s.ne(self.spender.key())) {
            return Err(PullError::InvalidSpender.into());
        }
        Ok(())
    }

    /// The source and its mint are the ones the terms limit.
    pub fn check_source(&self, limit: &Limit) -> ProgramResult {
        if limit.from.ne(self.from.key()) {
            return Err(PullError::InvalidPull.into());
        }
        if limit.mint.ne(self.mint.key()) {
            return Err(PullError::InvalidTarget.into());
        }
        Ok(())
    }

    /// Take `amount` as the delegate, only from the authority's own account.
    /// If the terms say what the authority receives, the spender pays it, and
    /// all of it must arrive. Returns what was paid.
    pub fn settle(&self, terms: &Terms, amount: u64, now: i64) -> Result<u64, ProgramError> {
        let held = balance(self.from, self.mint.key(), terms.authority)?;
        // Name what the token program would only refuse with its own codes
        if allowance(self.from)? < amount {
            return Err(PullError::AllowanceExceeded.into());
        }
        if held < amount {
            return Err(PullError::InsufficientFunds.into());
        }
        transfer(self.from, self.mint, self.to, self.engine, amount)?;

        let Some(receive) = terms.receive else {
            return Ok(0);
        };
        let [pay_from, pay_mint, pay_to, ..] = self.payment else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };
        if receive.to.ne(pay_to.key()) || receive.mint.ne(pay_mint.key()) {
            return Err(PullError::InvalidTarget.into());
        }
        let due = receive.due(now);
        let before = balance(pay_to, receive.mint, terms.authority)?;
        transfer(pay_from, pay_mint, pay_to, self.spender, due)?;
        let after = balance(pay_to, receive.mint, terms.authority)?;
        if after.checked_sub(before).is_none_or(|got| got < due) {
            return Err(PullError::NotReceived.into());
        }
        Ok(due)
    }
}

/// # Pull
///
/// Take tokens under a policy, within its limit. If the
/// policy says what the authority receives, the spender pays it here, in the
/// same instruction: there is nothing in between to trust. Callable by CPI.
///
/// > Check the window and the spender
/// > Check the amount against what is left of the limit, and record it
/// > Transfer from the source, as the engine delegate
/// > Receive: transfer the payment from the spender, and check what arrived
///
/// Accounts:
///
/// 1. spender:         [signer]
/// 2. policy:         [mut]
/// 3. from:            [mut]           the authority's token account
/// 4. mint:                            its mint
/// 5. to:              [mut]           where the spender sends the tokens
/// 6. engine:                          SPL delegate and event signer
/// 7. program:         [executable]    this program, for the event CPI
/// 8. token_program:   [executable]    of `from`
///
/// If the authority receives something, also:
///
/// 9. pay_from:        [mut]           the spender's token account
/// 10. pay_mint:                       the mint the authority receives
/// 11. pay_to:         [mut]           the authority's token account the terms name
/// 12. pay_program:    [executable]    token program of `pay_from`
///
/// Parameters:
/// 1. amount: u64,
/// 2. reference: [u8; 32],    optional: the spender's own id for this pull,
///    an invoice or an order; emitted, never stored
///
/// Account Checks:
/// - Spender: signer; the one the terms name, checked in process
/// - Policy: writable, loaded in process
/// - From, Mint, PayTo, PayMint: the accounts the terms name, checked in process
/// - To, PayFrom, Engine, Program, token programs: no need to check since the CPIs fail otherwise
///
/// Event Data:
/// - discriminator: u8, (255u8, 1u8)
/// - policy: Pubkey,
/// - spender: Pubkey,
/// - amount: u64,
/// - paid: u64,
/// - reference: [u8; 32], (zeros when the pull gave none)
pub struct Pull<'a> {
    pub policy: &'a AccountInfo,
    pub program: &'a AccountInfo,
    pub legs: Legs<'a>,
    pub amount: u64,
    pub reference: [u8; 32],
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Pull<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        sol_log("Pull");

        let [spender, policy, from, mint, to, engine, program, _token_program, payment @ ..] =
            accounts
        else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };

        // Account Checks
        if !spender.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if !policy.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        let (amount, reference) = match data.len() {
            8 => (data, [0; 32]),
            40 => (&data[..8], data[8..].try_into().unwrap()),
            _ => return Err(ProgramError::InvalidInstructionData),
        };

        Ok(Self {
            policy,
            program,
            legs: Legs {
                spender,
                from,
                mint,
                to,
                engine,
                payment,
            },
            amount: u64::from_le_bytes(amount.try_into().unwrap()),
            reference,
        })
    }
}

impl<'a> Pull<'a> {
    pub const DISCRIMINATOR: &'a u8 = &1;

    pub fn process(&mut self) -> ProgramResult {
        let now = Clock::get()?.unix_timestamp;
        let (legs, amount) = (&self.legs, self.amount);

        let (policy, bytes) = policy(self.policy)?;
        let terms = Terms::decode(bytes)?;
        legs.check(&terms, now)?;

        // The amount must fit what is left of the limit
        let limit = terms.limit;
        legs.check_source(&limit)?;
        let spent = policy.spent(limit.per, terms.not_before, now);
        let spent = spent.checked_add(amount).ok_or(PullError::Overflow)?;
        if spent > limit.max {
            return Err(PullError::LimitExceeded.into());
        }
        policy.set_consumed(spent);
        policy.set_rolled(now);

        let paid = legs.settle(&terms, amount, now)?;

        // Log the Pull Event
        emit(
            legs.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[
                self.policy.key(),
                legs.spender.key(),
                &amount.to_le_bytes(),
                &paid.to_le_bytes(),
                &self.reference,
            ],
        )
    }
}
