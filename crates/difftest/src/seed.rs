//! Gen5 LCG arithmetic on Showdown seeds, used to explain PRNG divergences ("actual drew 3 extra times").
//!
//! `[a,b,c,d]` is the 64-bit state, most significant word first. One `Gen5RNG.next()` is
//! `state = state * MUL + ADD (mod 2^64)`. This module is deliberately independent of the engine crate.

pub type Seed = [u16; 4];

const MUL: u64 = 0x5D58_8B65_6C07_8965;
const ADD: u64 = 0x0026_9EC3;

pub fn to_state(s: Seed) -> u64 {
    (s[0] as u64) << 48 | (s[1] as u64) << 32 | (s[2] as u64) << 16 | s[3] as u64
}

pub fn from_state(s: u64) -> Seed {
    [(s >> 48) as u16, (s >> 32) as u16, (s >> 16) as u16, s as u16]
}

/// One PRNG step.
pub fn advance(s: Seed) -> Seed {
    from_state(to_state(s).wrapping_mul(MUL).wrapping_add(ADD))
}

/// Inverse of [`advance`]: the seed one PRNG step earlier.
pub fn retreat(s: Seed) -> Seed {
    // Multiplicative inverse of MUL modulo 2^64 by Newton iteration.
    let mut inv: u64 = MUL;
    for _ in 0..6 {
        inv = inv.wrapping_mul(2u64.wrapping_sub(MUL.wrapping_mul(inv)));
    }
    from_state(to_state(s).wrapping_sub(ADD).wrapping_mul(inv))
}

/// `a,b,c,d` (Showdown's `getSeed()` form).
pub fn fmt(s: Seed) -> String {
    format!("{},{},{},{}", s[0], s[1], s[2], s[3])
}

/// Number of PRNG steps (0..=max) that takes `from` to `to`, if any.
pub fn draws_between(from: Seed, to: Seed, max: u32) -> Option<u32> {
    let target = to_state(to);
    let mut state = to_state(from);
    for n in 0..=max {
        if state == target {
            return Some(n);
        }
        state = state.wrapping_mul(MUL).wrapping_add(ADD);
    }
    None
}

/// How the actual seed relates to the expected one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedRelation {
    Equal,
    /// The actual PRNG consumed `n` more draws than expected (extra random calls).
    ActualAhead(u32),
    /// The actual PRNG consumed `n` fewer draws than expected (missing random calls).
    ActualBehind(u32),
    /// Not reachable from each other within the search limit (different stream entirely).
    Unrelated,
}

const SEARCH_LIMIT: u32 = 1 << 16;

pub fn relate(expected: Seed, actual: Seed) -> SeedRelation {
    if expected == actual {
        return SeedRelation::Equal;
    }
    if let Some(n) = draws_between(expected, actual, SEARCH_LIMIT) {
        return SeedRelation::ActualAhead(n);
    }
    if let Some(n) = draws_between(actual, expected, SEARCH_LIMIT) {
        return SeedRelation::ActualBehind(n);
    }
    SeedRelation::Unrelated
}

impl SeedRelation {
    pub fn describe(self) -> String {
        match self {
            SeedRelation::Equal => "seeds equal".into(),
            SeedRelation::ActualAhead(n) => {
                format!("actual PRNG is {n} draw(s) AHEAD of expected (extra random calls)")
            }
            SeedRelation::ActualBehind(n) => {
                format!("actual PRNG is {n} draw(s) BEHIND expected (missing random calls)")
            }
            SeedRelation::Unrelated => "seeds are unrelated (not reachable from each other by LCG steps)".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_matches_manual_state_math() {
        let s = [1, 2, 3, 4];
        let st = to_state(s);
        assert_eq!(to_state(advance(s)), st.wrapping_mul(MUL).wrapping_add(ADD));
        assert_eq!(from_state(to_state(s)), s);
        assert_eq!(retreat(advance(s)), s);
        assert_eq!(advance(retreat(s)), s);
    }

    #[test]
    fn relation_detects_extra_and_missing_draws() {
        let base = [34875, 39376, 17730, 7959];
        let mut ahead = base;
        for _ in 0..7 {
            ahead = advance(ahead);
        }
        assert_eq!(relate(base, base), SeedRelation::Equal);
        assert_eq!(relate(base, ahead), SeedRelation::ActualAhead(7));
        assert_eq!(relate(ahead, base), SeedRelation::ActualBehind(7));
        assert_eq!(relate(base, [1, 1, 1, 1]), SeedRelation::Unrelated);
    }
}
