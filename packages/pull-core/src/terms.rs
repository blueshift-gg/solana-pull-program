//! Terms: what an authority permits. Types, canonical binary codec and
//! validity rules.
//!
//! `Terms::decode` is the only way to read terms from bytes. It accepts exactly
//! the canonical encodings of valid terms, so every other module trusts them.

use crate::{constants::*, errors::PullError, Sink};
use pinocchio::pubkey::Pubkey;

type Result<T> = core::result::Result<T, PullError>;

/// What the limit counts over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Per {
    /// The life of a policy; the one use of an intent.
    Total,
    /// Fixed windows of `seconds`, counted from `not_before`. Nothing carries over.
    Every(u32),
}

/// What may go out: "at most `max` of `mint` may leave `from`", a token
/// account of the authority, "and only to `to`" if the terms name where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit<'a> {
    pub from: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub max: u64,
    pub per: Per,
    /// The only token account the tokens may go to. `None` leaves it to the spender.
    pub to: Option<&'a Pubkey>,
}

/// The minimum moves in a straight line to `min` between `t0` and `t1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decay {
    pub t0: i64,
    pub t1: i64,
    pub min: u64,
}

/// What must come in: "each use, at least `min` of `mint` arrives in `to`", a
/// token account of the authority. The spender pays it inside the pull,
/// whatever it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receive<'a> {
    pub to: &'a Pubkey,
    pub mint: &'a Pubkey,
    pub min: u64,
    pub decay: Option<Decay>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terms<'a> {
    pub cluster: u8,
    pub authority: &'a Pubkey,
    /// Who may pull. `None` is anyone, which is only valid if the terms name
    /// where the tokens go or what the authority receives.
    pub spender: Option<&'a Pubkey>,
    pub not_before: i64,
    pub not_after: Option<i64>,
    /// Tells apart otherwise identical terms. For an intent it is the nonce.
    pub salt: u64,
    pub limit: Limit<'a>,
    pub receive: Option<Receive<'a>>,
}

impl Receive<'_> {
    /// What must arrive for a use at `now`.
    pub fn due(&self, now: i64) -> u64 {
        match self.decay {
            None => self.min,
            Some(Decay { t0, t1, min }) => {
                // Truncates toward the starting minimum
                let moved = (min as i128 - self.min as i128) * (now.clamp(t0, t1) - t0) as i128;
                (self.min as i128 + moved / (t1 - t0) as i128) as u64
            }
        }
    }
}

impl<'a> Terms<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self> {
        let mut r = Reader(bytes);
        if r.u8()? != VERSION {
            return Err(PullError::MalformedTerms);
        }
        let terms = Terms {
            cluster: r.u8()?,
            authority: r.key()?,
            spender: r.option(|r| r.key())?,
            not_before: r.i64()?,
            not_after: r.option(|r| r.i64())?,
            salt: r.u64()?,
            limit: Limit {
                from: r.key()?,
                mint: r.key()?,
                max: r.u64()?,
                per: match r.u8()? {
                    0 => Per::Total,
                    1 => Per::Every(r.u32()?),
                    _ => return Err(PullError::MalformedTerms),
                },
                to: r.option(|r| r.key())?,
            },
            receive: r.option(|r| {
                Ok(Receive {
                    to: r.key()?,
                    mint: r.key()?,
                    min: r.u64()?,
                    decay: r.option(|r| {
                        Ok(Decay {
                            t0: r.i64()?,
                            t1: r.i64()?,
                            min: r.u64()?,
                        })
                    })?,
                })
            })?,
        };
        if !r.0.is_empty() {
            return Err(PullError::MalformedTerms);
        }
        terms.validate()?;
        Ok(terms)
    }

    /// Everything a decoder must reject beyond malformed bytes.
    pub fn validate(&self) -> Result<()> {
        if self.cluster != CLUSTER {
            return Err(PullError::WrongCluster);
        }
        // Every timestamp is renderable, which also keeps time arithmetic from overflowing
        let renderable = |t: i64| (0..=MAX_TIME).contains(&t);
        let window = renderable(self.not_before)
            && self
                .not_after
                .is_none_or(|t| t > self.not_before && renderable(t));
        let limit = self.limit.max > 0 && self.limit.per != Per::Every(0);
        let receive = self.receive.is_none_or(|x| {
            let decay = x
                .decay
                .is_none_or(|d| renderable(d.t0) && renderable(d.t1) && d.t0 < d.t1 && d.min > 0);
            x.min > 0 && decay
        });
        // Something must be bound: who takes, where it goes, or what the
        // authority receives. Otherwise the terms pay whoever finds them
        let bound = self.spender.is_some() || self.limit.to.is_some() || self.receive.is_some();
        if !(window && limit && receive && bound) {
            return Err(PullError::InvalidTerms);
        }
        Ok(())
    }

    pub fn write(&self, w: &mut impl Sink) {
        w.put(&[VERSION, self.cluster]);
        w.put(self.authority);
        option(w, self.spender, |w, key| w.put(key));
        w.put(&self.not_before.to_le_bytes());
        option(w, self.not_after, |w, t| w.put(&t.to_le_bytes()));
        w.put(&self.salt.to_le_bytes());
        w.put(self.limit.from);
        w.put(self.limit.mint);
        w.put(&self.limit.max.to_le_bytes());
        match self.limit.per {
            Per::Total => w.put(&[0]),
            Per::Every(seconds) => {
                w.put(&[1]);
                w.put(&seconds.to_le_bytes());
            }
        }
        option(w, self.limit.to, |w, key| w.put(key));
        option(w, self.receive, |w, x| {
            w.put(x.to);
            w.put(x.mint);
            w.put(&x.min.to_le_bytes());
            option(w, x.decay, |w, d| {
                w.put(&d.t0.to_le_bytes());
                w.put(&d.t1.to_le_bytes());
                w.put(&d.min.to_le_bytes());
            });
        });
    }
}

fn option<W: Sink, T>(w: &mut W, value: Option<T>, write: impl FnOnce(&mut W, T)) {
    match value {
        None => w.put(&[0]),
        Some(value) => {
            w.put(&[1]);
            write(w, value);
        }
    }
}

/// A cursor over canonical bytes. Every read fails on truncation.
struct Reader<'a>(&'a [u8]);

macro_rules! read_le {
    ($($name:ident: $t:ty),*) => {$(
        fn $name(&mut self) -> Result<$t> {
            Ok(<$t>::from_le_bytes(*self.take()?))
        }
    )*};
}

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Result<&'a [u8; N]> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(PullError::MalformedTerms)?;
        self.0 = rest;
        Ok(head)
    }

    read_le!(u8: u8, u32: u32, u64: u64, i64: i64);

    fn key(&mut self) -> Result<&'a Pubkey> {
        self.take()
    }

    fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        match self.u8()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(PullError::MalformedTerms),
        }
    }
}
