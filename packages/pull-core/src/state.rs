//! Account layouts. Every struct has alignment 1: fields are little-endian
//! byte arrays decoded on access, so a struct overlays raw account bytes.
//! Every account starts with a tag naming its type, so one account can never
//! be read as another. Callers validate owner, length and tag before taking views.

use crate::{constants::*, terms::Per};
use pinocchio::pubkey::Pubkey;

macro_rules! field {
    ($get:ident, $set:ident, $t:ty) => {
        #[inline(always)]
        pub fn $get(&self) -> $t {
            <$t>::from_le_bytes(self.$get)
        }
        #[inline(always)]
        pub fn $set(&mut self, v: $t) {
            self.$get = v.to_le_bytes();
        }
    };
}

macro_rules! account {
    ($name:ident) => {
        impl $name {
            /// # Safety
            /// `bytes` holds at least `size_of::<Self>()` bytes.
            #[inline(always)]
            pub unsafe fn from_bytes_unchecked(bytes: &[u8]) -> &Self {
                &*(bytes.as_ptr() as *const Self)
            }

            /// # Safety
            /// `bytes` holds at least `size_of::<Self>()` bytes.
            #[inline(always)]
            pub unsafe fn from_bytes_unchecked_mut(bytes: &mut [u8]) -> &mut Self {
                &mut *(bytes.as_mut_ptr() as *mut Self)
            }
        }
    };
}

/// A policy: this header, then the canonical terms.
#[repr(C)]
pub struct Policy {
    tag: [u8; 1],
    /// When `consumed` was last written.
    rolled: [u8; 8],
    consumed: [u8; 8],
    /// Paid the rent; refunded by `Close`.
    pub payer: Pubkey,
    terms_len: [u8; 2],
}

account!(Policy);

impl Policy {
    field!(tag, set_tag, u8);
    field!(rolled, set_rolled, i64);
    field!(consumed, set_consumed, u64);
    field!(terms_len, set_terms_len, u16);

    /// What the limit has consumed at `now`. A periodic limit starts each
    /// window, counted from `start`, with nothing consumed.
    pub fn spent(&self, per: Per, start: i64, now: i64) -> u64 {
        match per {
            Per::Total => self.consumed(),
            Per::Every(seconds) => {
                let window = |t: i64| (t - start).div_euclid(seconds as i64);
                match window(now) == window(self.rolled()) {
                    true => self.consumed(),
                    false => 0,
                }
            }
        }
    }
}

/// One page of the nonces an authority's signed intents have used or
/// cancelled: one bit each, so intents run in any order. It is never closed.
#[repr(C)]
pub struct Nonces {
    tag: [u8; 1],
    /// The page's place, recorded when it was created at its PDA, so a fill
    /// compares these instead of deriving the address again.
    pub authority: Pubkey,
    page: [u8; 8],
    bits: [u8; NONCE_BITS / 8],
}

account!(Nonces);

impl Nonces {
    field!(tag, set_tag, u8);
    field!(page, set_page, u64);

    /// Mark the nonce of `salt` used; false if it already was.
    pub fn take(&mut self, salt: u64) -> bool {
        let bit = (salt % NONCE_BITS as u64) as usize;
        let (byte, mask) = (&mut self.bits[bit / 8], 1 << (bit % 8));
        let fresh = *byte & mask == 0;
        *byte |= mask;
        fresh
    }

    /// Mark every nonce of the page used.
    pub fn take_all(&mut self) {
        self.bits = [u8::MAX; NONCE_BITS / 8];
    }
}

const _: () = {
    assert!(core::mem::size_of::<Policy>() == POLICY_LEN);
    assert!(core::mem::size_of::<Nonces>() == NONCES_LEN);
};
