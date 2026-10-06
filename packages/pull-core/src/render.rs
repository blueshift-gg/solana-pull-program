//! Canonical text: what the authority sees and signs.
//!
//! Deterministic, and printable ASCII and newlines only: it is valid UTF-8
//! that no wallet can take for a transaction, and it names the program, the
//! cluster and the authority itself, so it needs no envelope to be safe.
//! The program renders
//! straight into the signature hasher; clients render into a buffer. Both run
//! this code, so what is shown is what is verified.

use crate::{
    constants::{CLUSTER, MAX_TIME},
    errors::PullError,
    terms::{Per, Terms},
    Sink, ID,
};
use pinocchio::pubkey::Pubkey;

type Result<T> = core::result::Result<T, PullError>;

/// Offchain Message v1 signing domain.
pub const DOMAIN: &[u8; 16] = b"\xffsolana offchain";
const CLUSTERS: [&str; 4] = ["mainnet", "devnet", "testnet", "localnet"];

/// What comes before the text in the bytes the authority signs. Wallets do
/// not agree on one way to sign a message, so the program verifies either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Envelope {
    /// Nothing: the text alone, as `solana:signMessage` signs it.
    Text,
    /// The Offchain Message v1 preamble (domain, version 1, one signer), as
    /// `solana:signOffchainMessage` signs it.
    OffchainMessage,
}

impl Envelope {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Text),
            1 => Some(Self::OffchainMessage),
            _ => None,
        }
    }

    /// Write what comes before the text.
    pub fn put(self, authority: &Pubkey, out: &mut impl Sink) {
        if self == Self::OffchainMessage {
            out.put(DOMAIN);
            out.put(&[1, 1]);
            out.put(authority);
        }
    }
}

/// Render `terms` as an intent signed under the profile's nonce `index`.
/// `decimals` resolves a mint's on-chain decimals.
pub fn render(
    terms: &Terms,
    index: u64,
    decimals: impl Fn(&Pubkey) -> Result<u8>,
    out: &mut impl Sink,
) -> Result<()> {
    let mut t = Text { out, decimals };

    t.s("Solana Pull v1\ncluster: ");
    t.s(CLUSTERS[CLUSTER as usize]);
    t.s("\nengine: ");
    t.key(&ID);
    t.s("\nauthority: ");
    t.key(terms.authority);
    t.s("\nSPENDER: ");
    match terms.spender {
        None => t.s("anyone"),
        Some(key) => t.key(key),
    }

    let limit = terms.limit;
    t.s("\nMAY TAKE: at most ");
    t.amount(limit.max, (t.decimals)(limit.mint)?);
    t.s(" of mint ");
    t.key(limit.mint);
    t.s(" from ");
    t.key(limit.from);
    if let Some(to) = limit.to {
        t.s(" to ");
        t.key(to);
    }
    match limit.per {
        Per::Total => t.s(" in total"),
        Per::Every(seconds) => {
            t.s(" every ");
            t.duration(seconds as u64);
        }
    }
    if let Some(receive) = terms.receive {
        let decimals = (t.decimals)(receive.mint)?;
        t.s("\nMUST RECEIVE: at least ");
        t.amount(receive.min, decimals);
        t.s(" of mint ");
        t.key(receive.mint);
        t.s(" in ");
        t.key(receive.to);
        t.s(" for each use");
        if let Some(decay) = receive.decay {
            t.s(", moving to ");
            t.amount(decay.min, decimals);
            t.s(" from ");
            t.time(decay.t0)?;
            t.s(" to ");
            t.time(decay.t1)?;
        }
    }

    t.s("\nVALID: from ");
    t.time(terms.not_before)?;
    t.s(" until ");
    match terms.not_after {
        None => t.s("revoked"),
        Some(end) => t.time(end)?,
    }
    t.s("\nINDEX: ");
    t.uint(index);
    t.s("\nSALT: ");
    t.uint(terms.salt);
    Ok(())
}

struct Text<'o, O, D> {
    out: &'o mut O,
    decimals: D,
}

impl<O: Sink, D: Fn(&Pubkey) -> Result<u8>> Text<'_, O, D> {
    fn s(&mut self, s: &str) {
        self.out.put(s.as_bytes());
    }

    fn key(&mut self, key: &Pubkey) {
        let mut out = [0; five8::BASE58_ENCODED_32_MAX_LEN];
        let len = five8::encode_32(key, &mut out) as usize;
        self.out.put(&out[..len]);
    }

    fn uint(&mut self, v: u64) {
        let (digits, len) = digits(v);
        self.out.put(&digits[digits.len() - len..]);
    }

    /// `raw` with exactly `decimals` fractional digits.
    fn amount(&mut self, raw: u64, decimals: u8) {
        let (digits, len) = digits(raw);
        let digits = &digits[digits.len() - len..];
        let d = decimals as usize;
        if d == 0 {
            self.out.put(digits);
        } else if len <= d {
            self.s("0.");
            (len..d).for_each(|_| self.s("0"));
            self.out.put(digits);
        } else {
            self.out.put(&digits[..len - d]);
            self.s(".");
            self.out.put(&digits[len - d..]);
        }
    }

    /// `YYYY-MM-DDTHH:MM:SSZ`, years 1970–9999 (civil-from-days, H. Hinnant).
    fn time(&mut self, t: i64) -> Result<()> {
        if !(0..=MAX_TIME).contains(&t) {
            return Err(PullError::Unrenderable);
        }
        let (days, secs) = (t / 86_400, t % 86_400);
        let z = days + 719_468;
        let (era, doe) = (z / 146_097, z % 146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + (month <= 2) as i64;
        for (v, width, sep) in [
            (year, 4, "-"),
            (month, 2, "-"),
            (day, 2, "T"),
            (secs / 3_600, 2, ":"),
            (secs / 60 % 60, 2, ":"),
            (secs % 60, 2, "Z"),
        ] {
            let (digits, len) = digits(v as u64);
            (len..width).for_each(|_| self.s("0"));
            self.out.put(&digits[digits.len() - len..]);
            self.s(sep);
        }
        Ok(())
    }

    /// `1d2h3m4s`, zero components omitted, `0s` for zero.
    fn duration(&mut self, secs: u64) {
        if secs == 0 {
            return self.s("0s");
        }
        let parts = [
            (secs / 86_400, "d"),
            (secs / 3_600 % 24, "h"),
            (secs / 60 % 60, "m"),
            (secs % 60, "s"),
        ];
        for (n, unit) in parts.into_iter().filter(|(n, _)| *n > 0) {
            self.uint(n);
            self.s(unit);
        }
    }
}

/// Decimal digits of `v`, right-aligned, and how many there are.
fn digits(mut v: u64) -> ([u8; 20], usize) {
    let mut out = [b'0'; 20];
    let mut len = 0;
    loop {
        out[19 - len] = b'0' + (v % 10) as u8;
        len += 1;
        v /= 10;
        if v == 0 {
            return (out, len);
        }
    }
}
