//! Terms, canonical encoding, account layouts and errors shared by the
//! Pull program and its clients.
#![no_std]

pub mod constants;
pub mod errors;
pub mod state;
pub mod terms;

use pinocchio::pubkey::Pubkey;

pub const ID: Pubkey = five8_const::decode_32_const("PULLrgDYqK1yFKVTSbWieX3ARP7U2XUyrjxWXqKgVzA");

/// Anything bytes can be written to: a hasher on chain, a buffer off chain.
/// Encoding writes through it, so it never needs to allocate.
pub trait Sink {
    fn put(&mut self, bytes: &[u8]);
}

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "std")]
impl Sink for std::vec::Vec<u8> {
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}
