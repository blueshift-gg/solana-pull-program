//! One event per instruction, emitted through a CPI to this program signed by
//! the engine PDA. `EmitEvent` accepts only that signer, so events live in
//! inner instructions and no other program can forge them.
//!
//! Wire layout: `[EVENT_DISCRIMINATOR, instruction discriminator, fields in order]`.

use pinocchio::{
    account_info::AccountInfo,
    cpi::invoke_signed,
    instruction::{AccountMeta, Instruction, Seed, Signer},
    program_error::ProgramError,
    ProgramResult,
};
use pull_core::{constants::*, errors::PullError, Sink};

/// The instruction every event CPI targets. It does nothing; the inner
/// instruction's data is the event.
pub fn emit_event(accounts: &[AccountInfo]) -> ProgramResult {
    let [engine, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !engine.is_signer() || engine.key().ne(&ENGINE) {
        return Err(PullError::InvalidEventAuthority.into());
    }
    Ok(())
}

/// Emit `[EVENT_DISCRIMINATOR, instruction, fields...]`.
pub fn emit(
    engine: &AccountInfo,
    program: &AccountInfo,
    instruction: u8,
    fields: &[&[u8]],
) -> ProgramResult {
    let mut data = Event([0; EVENT_LEN], 0);
    data.put(&[EVENT_DISCRIMINATOR, instruction]);
    fields.iter().for_each(|f| data.put(f));

    let bump = [ENGINE_BUMP];
    let seeds = [Seed::from(ENGINE_SEED), Seed::from(&bump)];
    invoke_signed(
        &Instruction {
            program_id: &crate::ID,
            accounts: &[
                AccountMeta::readonly_signer(&ENGINE),
                AccountMeta::readonly(&crate::ID),
            ],
            data: &data.0[..data.1],
        },
        &[engine, program],
        &[Signer::from(&seeds)],
    )
}

/// The largest event, `Pull`'s: two discriminators, mandate, spender,
/// amount, paid and reference.
const EVENT_LEN: usize = 2 + 32 + 32 + 8 + 8 + 32;

struct Event([u8; EVENT_LEN], usize);

impl Sink for Event {
    fn put(&mut self, bytes: &[u8]) {
        self.0[self.1..self.1 + bytes.len()].copy_from_slice(bytes);
        self.1 += bytes.len();
    }
}
