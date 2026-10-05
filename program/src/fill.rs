use crate::events::emit;
use crate::helpers::decimals;
use crate::pull::Legs;
use crate::state::nonces_for;
use brine_ed25519::hasher::{FastSha512, Hasher};
use pinocchio::log::sol_log;
use pinocchio::sysvars::{clock::Clock, Sysvar};
use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pull_core::render::{envelope, render};
use pull_core::{errors::PullError, terms::Terms, Sink};

/// # Fill
///
/// Run a signed intent: terms the authority signed as text, good for one use.
/// Nothing is stored but one bit, the intent's nonce, which stays set
/// forever: intents run in any order and none can run twice. The authority
/// sends no transaction.
///
/// > Check the window and the spender, and the amount against the limit
/// > Verify the authority's signature over the rendered text
/// > Use the nonce
/// > Transfer from the source, as the engine delegate
/// > Receive: transfer the payment from the spender, and check what arrived
///
/// Accounts:
///
/// 1. spender:         [signer]
/// 2. payer:           [signer, mut]   funds the page of nonces if it is new
/// 3. nonces:          [mut]           PDA [NONCES_SEED, authority, salt / NONCE_BITS]
/// 4. system_program:  [executable]
/// 5. from:            [mut]           the authority's token account
/// 6. mint:                            its mint
/// 7. to:              [mut]           where the spender sends the tokens
/// 8. engine:                          SPL delegate and event signer
/// 9. program:         [executable]    this program, for the event CPI
/// 10. token_program:  [executable]    of `from`
///
/// If the authority receives something, also:
///
/// 11. pay_from:       [mut]           the spender's token account
/// 12. pay_mint:                       the mint the authority receives
/// 13. pay_to:         [mut]           the authority's token account the terms name
/// 14. pay_program:    [executable]    token program of `pay_from`
///
/// Parameters:
/// 1. amount: u64,
/// 2. signature: [u8; 64],     // over the OCMS v1 envelope and rendered text
/// 3. terms: [u8],             // canonical terms: the rest of the instruction data
///
/// Account Checks:
/// - Spender: signer; the one the terms name, checked in process
/// - Nonces: writable; the PDA, or created there, checked in process
/// - From, Mint, PayTo, PayMint: the accounts the terms name, checked in process
/// - Payer, SystemProgram, To, PayFrom, Engine, Program, token programs: no need to
///   check since the CPIs fail otherwise
///
/// Instruction Checks:
/// - Terms: canonical and valid
///
/// Event Data:
/// - discriminator: u8, (255u8, 10u8)
/// - authority: Pubkey,
/// - spender: Pubkey,
/// - salt: u64,
/// - amount: u64,
/// - paid: u64,
pub struct Fill<'a> {
    pub payer: &'a AccountInfo,
    pub nonces: &'a AccountInfo,
    pub program: &'a AccountInfo,
    pub legs: Legs<'a>,
    pub amount: u64,
    pub signature: &'a [u8; 64],
    pub terms: Terms<'a>,
}

impl<'a> TryFrom<(&'a [u8], &'a [AccountInfo])> for Fill<'a> {
    type Error = ProgramError;

    fn try_from((data, accounts): (&'a [u8], &'a [AccountInfo])) -> Result<Self, Self::Error> {
        sol_log("Fill");

        let [spender, payer, nonces, _system_program, from, mint, to, engine, program, _token_program, payment @ ..] =
            accounts
        else {
            return Err(ProgramError::NotEnoughAccountKeys);
        };
        let malformed = ProgramError::InvalidInstructionData;
        let (amount, rest) = data.split_first_chunk::<8>().ok_or(malformed.clone())?;
        let (signature, bytes) = rest.split_first_chunk::<64>().ok_or(malformed)?;

        // Instruction Checks
        let terms = Terms::decode(bytes)?;

        // Account Checks
        if !spender.is_signer() {
            return Err(PullError::NotSigner.into());
        }
        if !nonces.is_writable() {
            return Err(PullError::NotMutable.into());
        }

        Ok(Self {
            payer,
            nonces,
            program,
            legs: Legs {
                spender,
                from,
                mint,
                to,
                engine,
                payment,
            },
            amount: u64::from_le_bytes(*amount),
            signature,
            terms,
        })
    }
}

impl<'a> Fill<'a> {
    pub const DISCRIMINATOR: &'a u8 = &10;

    pub fn process(&mut self) -> ProgramResult {
        let now = Clock::get()?.unix_timestamp;
        let (legs, terms, amount) = (&self.legs, &self.terms, self.amount);

        legs.check(terms, now)?;
        // One use: the limit starts unspent, whatever it counts over
        legs.check_source(&terms.limit)?;
        if amount > terms.limit.max {
            return Err(PullError::LimitExceeded.into());
        }
        self.verify()?;

        // The nonce is what makes it one use
        let page = nonces_for(self.payer, self.nonces, terms.authority, terms.salt)?;
        if !page.take(terms.salt) {
            return Err(PullError::NonceUsed.into());
        }

        let paid = legs.settle(terms, amount, now)?;

        // Log the Fill Event
        emit(
            legs.engine,
            self.program,
            *Self::DISCRIMINATOR,
            &[
                terms.authority,
                legs.spender.key(),
                &terms.salt.to_le_bytes(),
                &amount.to_le_bytes(),
                &paid.to_le_bytes(),
            ],
        )
    }

    /// Verify the authority's signature over the OCMS v1 envelope and the text
    /// rendered from the terms, streaming the render into the challenge hash.
    fn verify(&self) -> ProgramResult {
        struct Challenge(FastSha512);
        impl Sink for Challenge {
            fn put(&mut self, bytes: &[u8]) {
                self.0.update(bytes);
            }
        }

        let terms = &self.terms;
        let mut challenge = Challenge(FastSha512::new());
        challenge.put(&self.signature[..32]);
        challenge.put(terms.authority);
        envelope(terms.authority, &mut challenge);
        // The text names the source's mint and the one received: both are accounts of this instruction
        let mints = core::iter::once(self.legs.mint).chain(self.legs.payment.get(1));
        let decimals = |mint: &[u8; 32]| {
            let account = mints.clone().find(|m| m.key().eq(mint));
            decimals(account.ok_or(PullError::InvalidTarget)?)
        };
        render(terms, decimals, &mut challenge)?;

        brine_ed25519::verify_prehashed_strict(
            &brine_ed25519::Address::new_from_array(*terms.authority),
            self.signature,
            &challenge.0.finalize(),
        )
        .map_err(|_| PullError::InvalidSignature.into())
    }
}
