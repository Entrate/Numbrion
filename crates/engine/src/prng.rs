//! Showdown's Gen5 PRNG (sim/prng.ts `Gen5RNG` + `PRNG`), bit-exact.
//!
//! Showdown keeps the 64-bit LCG state as four 16-bit words, most significant first;
//! seed `"a,b,c,d"` packs to `a<<48 | b<<32 | c<<16 | d`. Every helper here must consume
//! exactly the same number of draws, with the same range mapping, as the TypeScript.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Prng {
    state: u64,
}

const MUL: u64 = 0x5D58_8B65_6C07_8965;
const ADD: u64 = 0x0026_9EC3;

impl Prng {
    pub const fn from_seed(seed: [u16; 4]) -> Self {
        let state = (seed[0] as u64) << 48 | (seed[1] as u64) << 32 | (seed[2] as u64) << 16 | seed[3] as u64;
        Prng { state }
    }

    pub const fn seed(&self) -> [u16; 4] {
        let s = self.state;
        [(s >> 48) as u16, (s >> 32) as u16, (s >> 16) as u16, s as u16]
    }

    /// `Gen5RNG.next()`: advance one frame, return the upper 32 bits.
    #[inline(always)]
    pub fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(MUL).wrapping_add(ADD);
        (self.state >> 32) as u32
    }

    /// `random()` with no arguments: a float in [0, 1).
    #[inline]
    pub fn random_f64(&mut self) -> f64 {
        self.next_u32() as f64 / 4294967296.0
    }

    /// `random(n)`: integer in [0, n). Matches `Math.floor(result * n / 2**32)`.
    /// `n == 0` returns 0 but still draws, like Showdown.
    #[inline(always)]
    pub fn random(&mut self, n: u32) -> u32 {
        ((self.next_u32() as u64 * n as u64) >> 32) as u32
    }

    /// `random(m, n)`: integer in [m, n). Note Showdown's `random(0, n)` takes this path too
    /// and `random(m, 0)` degenerates to `random(m)`; callers port the call literally.
    #[inline(always)]
    pub fn random_range(&mut self, from: u32, to: u32) -> u32 {
        if to == 0 {
            return self.random(from);
        }
        (((self.next_u32() as u64) * (to - from) as u64) >> 32) as u32 + from
    }

    /// `randomChance(numerator, denominator)`.
    #[inline(always)]
    pub fn random_chance(&mut self, numerator: u32, denominator: u32) -> bool {
        self.random(denominator) < numerator
    }

    /// `sample(items)`: one draw of `random(len)`. Panics on empty input, like Showdown throws.
    #[inline]
    pub fn sample<T: Copy>(&mut self, items: &[T]) -> T {
        assert!(!items.is_empty(), "Cannot sample an empty array");
        items[self.random(items.len() as u32) as usize]
    }

    /// `sample` returning the index instead of the item.
    #[inline]
    pub fn sample_index(&mut self, len: usize) -> usize {
        assert!(len != 0, "Cannot sample an empty array");
        self.random(len as u32) as usize
    }

    /// `shuffle(items, start, end)`: forward Fisher-Yates over `items[start..end]`.
    #[inline]
    pub fn shuffle<T>(&mut self, items: &mut [T], mut start: usize, end: usize) {
        while start + 1 < end {
            let next = self.random_range(start as u32, end as u32) as usize;
            if next != start {
                items.swap(start, next);
            }
            start += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors produced by tools/oracle/prng-vectors.mjs against the pinned Showdown.
    #[test]
    fn matches_showdown_sequence() {
        let mut p = Prng::from_seed([1, 2, 3, 4]);
        let got: Vec<u32> = (0..6).map(|_| p.next_u32()).collect();
        assert_eq!(got, EXPECTED_NEXT);
        assert_eq!(p.seed(), EXPECTED_SEED_AFTER_6);
        let mut p = Prng::from_seed([1, 2, 3, 4]);
        let r: Vec<u32> = (0..6).map(|i| p.random(16 + i)).collect();
        assert_eq!(r, EXPECTED_RANDOM_N);
        let mut v: Vec<u32> = (0..10).collect();
        let mut p = Prng::from_seed([9, 8, 7, 6]);
        p.shuffle(&mut v, 0, 10);
        assert_eq!(v, EXPECTED_SHUFFLE);
    }

    include!("prng_vectors.rs");
}
