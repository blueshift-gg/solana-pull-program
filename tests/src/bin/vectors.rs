//! Prints random terms, one JSON object per line, as the Rust core encodes
//! and judges them. The TypeScript client must agree on every line: that is
//! what keeps its codec and its validity rules the same as the program's.
//!
//! `bytes` is hex.

use pull_core::constants::MAX_TIME;
use pull_core::terms::{Limit, Per, Receive, Terms};

struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn key(&mut self) -> [u8; 32] {
        core::array::from_fn(|_| self.next() as u8)
    }

    /// Mostly round or small, and sometimes zero or huge, so every validity
    /// rule is reached.
    fn amount(&mut self) -> u64 {
        match self.below(6) {
            0 => 0,
            1 => self.next(),
            2 => self.below(10),
            _ => self.below(1_000_000) * 10u64.pow(self.below(9) as u32),
        }
    }

    fn time(&mut self) -> i64 {
        match self.below(8) {
            0 => -1,
            1 => MAX_TIME + 1,
            2 => MAX_TIME,
            _ => self.below(4_000_000_000) as i64,
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let mut r = Random(0x2545_F491_4F6C_DD1D);
    for _ in 0..2_000 {
        let keys: [[u8; 32]; 7] = core::array::from_fn(|_| r.key());
        let not_before = r.time();
        let per = match r.below(5) {
            0 => Per::Total,
            1 => Per::Every(r.below(3) as u32),
            2 => Per::Every(r.next() as u32),
            _ => Per::Every([60, 3_600, 86_400, 2_592_000, 90_061][r.below(5) as usize]),
        };
        let receive = (r.below(2) == 0).then(|| Receive {
            to: &keys[5],
            min: r.amount(),
        });
        let terms = Terms {
            authority: &keys[0],
            spender: (r.below(2) == 0).then_some(&keys[1]),
            not_before,
            not_after: (r.below(2) == 0)
                .then(|| not_before.saturating_add(r.below(1_000_000) as i64 - 10)),
            limit: Limit {
                from: &keys[2],
                max: r.amount(),
                per,
                to: (r.below(2) == 0).then_some(&keys[4]),
            },
            receive,
        };
        let mut bytes = Vec::new();
        terms.write(&mut bytes);
        let valid = terms.validate().is_ok();
        assert_eq!(Terms::decode(&bytes).is_ok(), valid);
        println!(r#"{{"bytes":"{}","valid":{valid}}}"#, hex(&bytes));
    }
}
