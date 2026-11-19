//! Terms: what an authority permits. Types, canonical binary codec and
//! validity rules.
//!
//! The encoding has a fixed size and every field a fixed offset. An absent
//! value is all zeroes, which is never a valid present one: the zero key is
//! the system program, which is no token account and never signs; a period
//! of zero seconds, an expiry at 0 and a minimum of zero are invalid terms.
//!
//! `Terms::decode` is the only way to read terms from bytes. It accepts exactly
//! the encodings of valid terms, so every other module trusts them.

use crate::{constants::*, errors::PullError, Sink};
use pinocchio::pubkey::Pubkey;

type Result<T> = core::result::Result<T, PullError>;

/// What may go out: "at most `max` may leave `from`", a token account of the
/// authority, "and only to `to`" if the terms name where. The mint is the
/// one of `from`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit<'a> {
    pub from: &'a Pubkey,
    pub max: u64,
    /// Fixed windows of this many seconds, counted from `not_before`, with
    /// nothing carried over. `None` counts over the life of the mandate.
    pub period: Option<u32>,
    /// The only token account the tokens may go to. `None` leaves it to the spender.
    pub to: Option<&'a Pubkey>,
}

/// What must come in: "each use, at least `min` arrives in `to`", a token
/// account of the authority, in its mint. The spender pays it inside the
/// pull, whatever it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receive<'a> {
    pub to: &'a Pubkey,
    pub min: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terms<'a> {
    pub authority: &'a Pubkey,
    /// Who may pull. `None` is anyone, which is only valid if the terms name
    /// where the tokens go or what the authority receives.
    pub spender: Option<&'a Pubkey>,
    pub not_before: i64,
    pub not_after: Option<i64>,
    pub limit: Limit<'a>,
    pub receive: Option<Receive<'a>>,
}

const NONE: Pubkey = [0; 32];

impl<'a> Terms<'a> {
    /// Read the fields in the order `write` puts them.
    pub fn decode(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() != TERMS_LEN {
            return Err(PullError::MalformedTerms);
        }
        let mut rest = bytes;
        let mut take = |n: usize| -> &'a [u8] {
            let (head, tail) = rest.split_at(n);
            rest = tail;
            head
        };
        let key = |b: &'a [u8]| -> &'a Pubkey { b.try_into().unwrap() };
        let int = |b: &[u8]| u64::from_le_bytes(b.try_into().unwrap());
        let (authority, spender) = (key(take(32)), key(take(32)));
        let (not_before, not_after) = (int(take(8)) as i64, int(take(8)) as i64);
        let (from, max) = (key(take(32)), int(take(8)));
        let period = u32::from_le_bytes(take(4).try_into().unwrap());
        let (to, receive_to, min) = (key(take(32)), key(take(32)), int(take(8)));
        let some = |k: &'a Pubkey| Some(k).filter(|&k| k.ne(&NONE));
        let terms = Terms {
            authority,
            spender: some(spender),
            not_before,
            not_after: Some(not_after).filter(|&t| t != 0),
            limit: Limit {
                from,
                max,
                period: Some(period).filter(|&s| s != 0),
                to: some(to),
            },
            receive: match (some(receive_to), min) {
                (None, 0) => None,
                (Some(to), min) if min > 0 => Some(Receive { to, min }),
                _ => return Err(PullError::MalformedTerms),
            },
        };
        terms.validate()?;
        Ok(terms)
    }

    /// Everything a decoder must reject beyond malformed bytes.
    pub fn validate(&self) -> Result<()> {
        // Bounded timestamps keep time arithmetic from overflowing
        let time = |t: i64| (0..=MAX_TIME).contains(&t);
        let window = time(self.not_before)
            && self
                .not_after
                .is_none_or(|t| t > self.not_before && time(t));
        let limit = self.limit.max > 0 && self.limit.period != Some(0);
        let receive = self.receive.is_none_or(|x| x.min > 0);
        // Something must be bound: who takes, where it goes, or what the
        // authority receives. Otherwise the terms pay whoever finds them
        let bound = self.spender.is_some() || self.limit.to.is_some() || self.receive.is_some();
        if !(window && limit && receive && bound) {
            return Err(PullError::InvalidTerms);
        }
        Ok(())
    }

    /// The fixed-size encoding: every absent value as zeroes.
    pub fn write(&self, w: &mut impl Sink) {
        let receive = self.receive.map_or((&NONE, 0), |x| (x.to, x.min));
        w.put(self.authority);
        w.put(self.spender.unwrap_or(&NONE));
        w.put(&self.not_before.to_le_bytes());
        w.put(&self.not_after.unwrap_or(0).to_le_bytes());
        w.put(self.limit.from);
        w.put(&self.limit.max.to_le_bytes());
        w.put(&self.limit.period.unwrap_or(0).to_le_bytes());
        w.put(self.limit.to.unwrap_or(&NONE));
        w.put(receive.0);
        w.put(&receive.1.to_le_bytes());
    }
}
