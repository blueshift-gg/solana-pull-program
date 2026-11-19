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

/// An authority's profile. Its mandates are numbered from 1 as they are
/// created; `mandates` is the last number used, and every mandate numbered
/// `stale` or less is invalid. It is never closed: a profile opened again would count from zero
/// and bring old mandates back.
#[repr(C)]
pub struct Profile {
    tag: [u8; 1],
    pub authority: Pubkey,
    mandates: [u8; 8],
    stale: [u8; 8],
}

account!(Profile);

impl Profile {
    field!(tag, set_tag, u8);
    field!(mandates, set_mandates, u64);
    field!(stale, set_stale, u64);
}

/// A mandate: this header, then the canonical terms.
#[repr(C)]
pub struct Mandate {
    tag: [u8; 1],
    /// When `consumed` was last written.
    rolled: [u8; 8],
    consumed: [u8; 8],
    /// Paid the rent; refunded by `Close`.
    pub payer: Pubkey,
    /// Its number among the authority's mandates.
    index: [u8; 8],
    terms_len: [u8; 2],
}

account!(Mandate);

impl Mandate {
    field!(tag, set_tag, u8);
    field!(rolled, set_rolled, i64);
    field!(consumed, set_consumed, u64);
    field!(index, set_index, u64);
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

const _: () = {
    assert!(core::mem::size_of::<Mandate>() == MANDATE_LEN);
    assert!(core::mem::size_of::<Profile>() == PROFILE_LEN);
};
