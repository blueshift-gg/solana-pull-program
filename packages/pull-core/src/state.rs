//! Account layouts. Every struct has alignment 1: fields are little-endian
//! byte arrays decoded on access, so a struct overlays raw account bytes.
//! Every account starts with a tag naming its type, so one account can never
//! be read as another. Callers validate owner, length and tag before taking views.

use crate::constants::*;
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

/// A mandate: who it is from, then how much of the limit is used, then its
/// terms. `consumed` and `window` are the only bytes `Pull` writes.
#[repr(C)]
pub struct Mandate {
    tag: [u8; 1],
    /// Signed `Create`; must own the account the terms take from.
    pub authority: Pubkey,
    /// Paid the rent; refunded by `Close`.
    pub payer: Pubkey,
    /// Its number among the authority's mandates.
    index: [u8; 8],
    /// What the limit has used in `window`.
    consumed: [u8; 8],
    /// The window `consumed` counts: windows of the period from `not_before`,
    /// or always 0 for a limit in total.
    window: [u8; 8],
}

account!(Mandate);

impl Mandate {
    field!(tag, set_tag, u8);
    field!(window, set_window, u64);
    field!(consumed, set_consumed, u64);
    field!(index, set_index, u64);

    /// What the limit has used in `window`: a new window starts with nothing used.
    pub fn spent(&self, window: u64) -> u64 {
        match window == self.window() {
            true => self.consumed(),
            false => 0,
        }
    }
}

const _: () = {
    assert!(core::mem::size_of::<Mandate>() == MANDATE_LEN);
    assert!(core::mem::size_of::<Profile>() == PROFILE_LEN);
};
