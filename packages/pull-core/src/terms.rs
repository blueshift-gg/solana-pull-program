//! Terms: what an authority permits under a mandate. Types, binary codec and
//! validity rules. The authority is not part of them: it is whoever signs
//! `Create`, and the mandate's header records it.
//!
//! Every field sits at a fixed offset. An absent value is all zeroes, which
//! is never a valid present one: the zero key is the system program, which is
//! no token account and never signs, and an expiry at 0, a period of zero
//! seconds or a cap per pull of zero is invalid. `in` is a suffix, present only when the terms have one.
//!
//! `Terms::decode` is the only way to read terms from bytes. It accepts exactly
//! the encodings of valid terms, so every other module trusts them.

use crate::{constants::*, errors::PullError, Sink};
use pinocchio::pubkey::Pubkey;

type Result<T> = core::result::Result<T, PullError>;

/// What may go out: "at most `max` may leave `from`", a token account of the
/// authority, in its mint, "and only to `to`" if the terms name where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Out<'a> {
    pub from: &'a Pubkey,
    /// The only token account the tokens may go to. `None` leaves it to the spender.
    pub to: Option<&'a Pubkey>,
    pub max: u64,
    /// Fixed windows of this many seconds, counted from `not_before`, with
    /// nothing carried over. `None` counts over the life of the mandate.
    pub period: Option<u32>,
    /// The most one pull may take, at most `max`. `None` lets one pull take
    /// all that is left.
    pub max_per_pull: Option<u64>,
}

/// What must come back: "each pull, at least `min` arrives in `to`", a token
/// account of the authority, in its mint. The spender pays it inside the
/// pull, whatever it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct In<'a> {
    pub to: &'a Pubkey,
    pub min: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terms<'a> {
    /// Who may pull. `None` is anyone, which is only valid if the terms name
    /// where the tokens go or what comes back.
    pub spender: Option<&'a Pubkey>,
    pub not_before: i64,
    pub not_after: Option<i64>,
    pub out: Out<'a>,
    pub r#in: Option<In<'a>>,
}

const NONE: Pubkey = [0; 32];

impl<'a> Terms<'a> {
    /// Read the fields in the order `write` puts them.
    pub fn decode(bytes: &'a [u8]) -> Result<Self> {
        let has_in = match bytes.len() {
            TERMS_LEN => false,
            len if len == TERMS_LEN + IN_LEN => true,
            _ => return Err(PullError::MalformedTerms),
        };
        let mut rest = bytes;
        let mut take = |n: usize| -> &'a [u8] {
            let (head, tail) = rest.split_at(n);
            rest = tail;
            head
        };
        let key = |b: &'a [u8]| -> &'a Pubkey { b.try_into().unwrap() };
        let int = |b: &[u8]| u64::from_le_bytes(b.try_into().unwrap());
        let some = |k: &'a Pubkey| Some(k).filter(|&k| k.ne(&NONE));
        let spender = some(key(take(32)));
        let (not_before, not_after) = (int(take(8)) as i64, int(take(8)) as i64);
        let (from, to, max) = (key(take(32)), some(key(take(32))), int(take(8)));
        let period = u32::from_le_bytes(take(4).try_into().unwrap());
        let max_per_pull = int(take(8));
        let r#in = match has_in {
            true => Some(In {
                to: key(take(32)),
                min: int(take(8)),
            }),
            false => None,
        };
        let terms = Terms {
            spender,
            not_before,
            not_after: Some(not_after).filter(|&t| t != 0),
            out: Out {
                from,
                to,
                max,
                period: Some(period).filter(|&s| s != 0),
                max_per_pull: Some(max_per_pull).filter(|&m| m != 0),
            },
            r#in,
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
        let out = self.out.max > 0
            && self.out.period != Some(0)
            && self
                .out
                .max_per_pull
                .is_none_or(|m| m > 0 && m <= self.out.max);
        let r#in = self.r#in.is_none_or(|x| x.to.ne(&NONE) && x.min > 0);
        // Something must be bound: who takes, where it goes, or what comes
        // back. Otherwise the terms pay whoever finds them
        let bound = self.spender.is_some() || self.out.to.is_some() || self.r#in.is_some();
        if !(window && out && r#in && bound) {
            return Err(PullError::InvalidTerms);
        }
        Ok(())
    }

    /// The window of the limit that `now` falls in: windows of the period
    /// counted from `not_before`, or always 0 for a limit in total. `now` is
    /// not before `not_before`.
    pub fn window(&self, now: i64) -> u64 {
        self.out
            .period
            .map_or(0, |s| (now - self.not_before).div_euclid(s as i64) as u64)
    }

    /// The encoding: every absent value as zeroes, and `in` only if present.
    pub fn write(&self, w: &mut impl Sink) {
        w.put(self.spender.unwrap_or(&NONE));
        w.put(&self.not_before.to_le_bytes());
        w.put(&self.not_after.unwrap_or(0).to_le_bytes());
        w.put(self.out.from);
        w.put(self.out.to.unwrap_or(&NONE));
        w.put(&self.out.max.to_le_bytes());
        w.put(&self.out.period.unwrap_or(0).to_le_bytes());
        w.put(&self.out.max_per_pull.unwrap_or(0).to_le_bytes());
        if let Some(x) = self.r#in {
            w.put(x.to);
            w.put(&x.min.to_le_bytes());
        }
    }
}
