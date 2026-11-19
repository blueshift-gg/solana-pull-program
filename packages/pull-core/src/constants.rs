use five8_const::decode_32_const;
use pinocchio::pubkey::Pubkey;

/// 9999-12-31T23:59:59Z: the latest timestamp terms may hold, which keeps
/// time arithmetic from overflowing.
pub const MAX_TIME: i64 = 253_402_300_799;

pub const TOKEN_PROGRAM: Pubkey = decode_32_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM: Pubkey =
    decode_32_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Engine PDA: [ENGINE_SEED]. The SPL delegate of every enabled token account
/// and the signer of every event; the test suite re-derives it.
pub const ENGINE_SEED: &[u8] = b"engine";
pub const ENGINE: Pubkey = decode_32_const("58eSE1WJDvzrwz7BZ23sbsDiqa75pxetzkyRyUcPwb6E");
pub const ENGINE_BUMP: u8 = 255;
/// Profile PDA: [PROFILE_SEED, authority]. One per authority: it numbers the
/// authority's mandates and holds what invalidates them.
pub const PROFILE_SEED: &[u8] = b"profile";
/// Mandate PDA: [MANDATE_SEED, authority, index], `index` little-endian: the
/// authority's mandates are numbered 1, 2, 3, ... by its profile.
pub const MANDATE_SEED: &[u8] = b"mandate";

/// Account tags: the first byte of every account, one per type.
pub const MANDATE_TAG: u8 = 1;
pub const PROFILE_TAG: u8 = 3;

/// Self-CPI event instruction discriminator.
pub const EVENT_DISCRIMINATOR: u8 = 255;

/// Terms: authority, spender, not_before, not_after, from, max, period, to, receive.to, receive.min.
pub const TERMS_LEN: usize = 32 + 32 + 8 + 8 + 32 + 8 + 4 + 32 + 32 + 8; // 196
/// Fixed header; the terms follow it.
pub const MANDATE_LEN: usize = 1 + 8 + 8 + 32 + 8; // 57
pub const PROFILE_LEN: usize = 1 + 32 + 8 + 8; // 49
