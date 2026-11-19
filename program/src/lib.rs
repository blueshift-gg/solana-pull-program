use pinocchio::{
    account_info::AccountInfo, default_panic_handler, no_allocator, program_entrypoint,
    program_error::ProgramError, pubkey::Pubkey, ProgramResult,
};

// The program never allocates; `no_allocator!` turns an accidental heap use
// into a hard failure.
program_entrypoint!(process_instruction);
no_allocator!();
default_panic_handler!();

mod close;
mod create;
mod events;
mod helpers;
mod invalidate;
mod open;
mod pull;
mod state;

use close::Close;
use create::Create;
use invalidate::Invalidate;
use open::Open;
use pull::Pull;
use pull_core::{constants::EVENT_DISCRIMINATOR, ID};

fn process_instruction(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    match instruction_data.split_first() {
        // Mandates: a standing permission on chain. Pull runs on every payment
        Some((&Pull::DISCRIMINATOR, data)) => Pull::try_from((data, accounts))?.process(),
        Some((&Create::DISCRIMINATOR, data)) => Create::try_from((data, accounts))?.process(),
        Some((&Close::DISCRIMINATOR, _)) => Close::try_from(accounts)?.process(),

        // The profile: opened once, and what ends everything at once
        Some((&Open::DISCRIMINATOR, _)) => Open::try_from(accounts)?.process(),
        Some((&Invalidate::DISCRIMINATOR, _)) => Invalidate::try_from(accounts)?.process(),

        // The engine-signed self-CPI that carries an event
        Some((&EVENT_DISCRIMINATOR, _)) => events::emit_event(accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
