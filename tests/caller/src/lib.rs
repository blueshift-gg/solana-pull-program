//! Test fixture: forwards its instruction to the Pull program, so the
//! tests can run a `Pull` at stack height 2, as a merchant's own program would.
#![cfg_attr(not(target_os = "solana"), allow(dead_code, unused_imports))]

use pinocchio::{
    account_info::AccountInfo,
    cpi::invoke,
    default_panic_handler,
    instruction::{AccountMeta, Instruction},
    no_allocator, program_entrypoint,
    program_error::ProgramError,
    pubkey::Pubkey,
    ProgramResult,
};

program_entrypoint!(process_instruction);
no_allocator!();
default_panic_handler!();

/// A `Pull` without a price.
const ACCOUNTS: usize = 9;

fn process_instruction(_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if accounts.len() != ACCOUNTS {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let infos: [&AccountInfo; ACCOUNTS] = core::array::from_fn(|i| &accounts[i]);
    let metas = infos.map(|a| AccountMeta::new(a.key(), a.is_writable(), a.is_signer()));
    invoke(
        &Instruction {
            program_id: &pull_core::ID,
            accounts: &metas,
            data,
        },
        &infos,
    )
}
