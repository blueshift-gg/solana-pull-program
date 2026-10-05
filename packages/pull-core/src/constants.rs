use five8_const::decode_32_const;
use pinocchio::pubkey::Pubkey;

/// Terms format version.
pub const VERSION: u8 = 1;
/// The cluster this build accepts terms for. There is no genesis-hash
/// syscall, so the cluster is a compile-time constant.
pub const CLUSTER: u8 = if cfg!(feature = "localnet") {
    3
} else if cfg!(feature = "testnet") {
    2
} else if cfg!(feature = "devnet") {
    1
} else {
    0
};

/// 9999-12-31T23:59:59Z: the last instant the canonical text can render.
pub const MAX_TIME: i64 = 253_402_300_799;

pub const TOKEN_PROGRAM: Pubkey = decode_32_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022_PROGRAM: Pubkey =
    decode_32_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Engine PDA: [ENGINE_SEED]. The SPL delegate of every enabled token account
/// and the signer of every event; the test suite re-derives it.
pub const ENGINE_SEED: &[u8] = b"engine";
pub const ENGINE: Pubkey = decode_32_const("58eSE1WJDvzrwz7BZ23sbsDiqa75pxetzkyRyUcPwb6E");
pub const ENGINE_BUMP: u8 = 255;
/// Policy PDA: [POLICY_SEED, authority, sha256(terms)]. The authority is a
/// seed so nobody can pre-create another authority's policy.
pub const POLICY_SEED: &[u8] = b"policy";

/// The used nonces of an authority's signed intents: [NONCES_SEED, authority,
/// page], `page` little-endian. An intent's nonce is its salt: the page is
/// `salt / NONCE_BITS` and the bit is the remainder. Pages stay forever, so a
/// used or cancelled nonce stays used, whenever the intent expires or not.
pub const NONCES_SEED: &[u8] = b"nonces";
/// Nonces in one page.
pub const NONCE_BITS: usize = 1024;

/// Account tags: the first byte of every account, one per type.
pub const POLICY_TAG: u8 = 1;
pub const NONCES_TAG: u8 = 2;

/// Self-CPI event instruction discriminator.
pub const EVENT_DISCRIMINATOR: u8 = 255;

/// Fixed header; the canonical terms follow it.
pub const POLICY_LEN: usize = 1 + 8 + 8 + 32 + 2; // 51
pub const NONCES_LEN: usize = 1 + 32 + 8 + NONCE_BITS / 8; // 169
