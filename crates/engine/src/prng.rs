//! Showdown's Gen5 PRNG (sim/prng.ts `Gen5RNG` + `PRNG`), bit-exact.
//!
//! Showdown keeps the 64-bit LCG state as four 16-bit words, most significant first;
//! seed `"a,b,c,d"` packs to `a<<48 | b<<32 | c<<16 | d`. Every helper here must consume
//! exactly the same number of draws, with the same range mapping, as the TypeScript.
//!
//! Search additions (no Showdown counterpart, inert unless used): a draw counter and a short
//! schedule of *resample points*. A point `(draw, salt)` replaces the LCG state right before
//! draw number `draw` with a state derived from the schedule key and `salt`, so every draw
//! from there on is fresh while all earlier draws are unchanged. The world builder
//! (`search::world`) uses it to resample the random outcomes behind one protocol line at a time.
//! Equality, hashing and `Debug` ignore the counter: two generators are equal when they will
//! produce the same stream.

/// Most resample points one schedule can hold.
pub const RESAMPLE_CAPACITY: usize = 16;

#[derive(Clone, Copy)]
pub struct Prng {
    state: u64,
    /// Draws taken since construction or the last `from_seed`.
    draws: u32,
    /// Draw index of the next pending resample point, `u32::MAX` when none is pending.
    hook: u32,
    next: u8,
    len: u8,
    key: u64,
    points: [(u32, u32); RESAMPLE_CAPACITY],
}

impl PartialEq for Prng {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state && self.pending() == other.pending()
    }
}
impl Eq for Prng {}

impl core::hash::Hash for Prng {
    fn hash<H: core::hash::Hasher>(&self, h: &mut H) {
        self.state.hash(h);
        self.pending().hash(h);
    }
}

impl core::fmt::Debug for Prng {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut d = f.debug_struct("Prng");
        d.field("state", &self.state);
        if self.hook != u32::MAX {
            d.field("key", &self.key).field("pending", &self.pending());
        }
        d.finish()
    }
}

/// SplitMix64 finalizer: the state a resample point installs.
#[inline]
const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

const MUL: u64 = 0x5D58_8B65_6C07_8965;
const ADD: u64 = 0x0026_9EC3;

impl Prng {
    pub const fn from_seed(seed: [u16; 4]) -> Self {
        let state = (seed[0] as u64) << 48
            | (seed[1] as u64) << 32
            | (seed[2] as u64) << 16
            | seed[3] as u64;
        Prng {
            state,
            draws: 0,
            hook: u32::MAX,
            next: 0,
            len: 0,
            key: 0,
            points: [(0, 0); RESAMPLE_CAPACITY],
        }
    }

    pub const fn seed(&self) -> [u16; 4] {
        let s = self.state;
        [
            (s >> 48) as u16,
            (s >> 32) as u16,
            (s >> 16) as u16,
            s as u16,
        ]
    }

    /// `Gen5RNG.next()`: advance one frame, return the upper 32 bits.
    #[inline(always)]
    pub fn next_u32(&mut self) -> u32 {
        if self.draws == self.hook {
            self.resample();
        }
        self.draws = self.draws.wrapping_add(1);
        self.state = self.state.wrapping_mul(MUL).wrapping_add(ADD);
        (self.state >> 32) as u32
    }

    /// Draws taken since construction (or the last `from_seed`). Search bookkeeping only.
    #[inline]
    pub const fn draws(&self) -> u32 {
        self.draws
    }

    /// Install a resample schedule: before draw `points[i].0` (absolute, see `draws`) the state
    /// becomes a function of `key` and `points[i].1`. Indices must be strictly increasing and not
    /// below `draws()`; at most `RESAMPLE_CAPACITY` points. Replaces any pending schedule.
    pub fn set_resample(&mut self, key: u64, points: &[(u32, u32)]) {
        assert!(points.len() <= RESAMPLE_CAPACITY, "too many resample points");
        assert!(
            points.windows(2).all(|w| w[0].0 < w[1].0)
                && points.first().is_none_or(|p| p.0 >= self.draws && p.0 != u32::MAX),
            "resample points must be increasing and in the future"
        );
        self.key = key;
        self.points[..points.len()].copy_from_slice(points);
        self.len = points.len() as u8;
        self.next = 0;
        self.hook = points.first().map_or(u32::MAX, |p| p.0);
    }

    /// Drop any pending resample points.
    pub fn clear_resample(&mut self) {
        self.len = 0;
        self.next = 0;
        self.hook = u32::MAX;
    }

    /// The resample points that have not fired yet.
    pub fn pending(&self) -> &[(u32, u32)] {
        if self.hook == u32::MAX {
            &[]
        } else {
            &self.points[self.next as usize..self.len as usize]
        }
    }

    #[cold]
    fn resample(&mut self) {
        let (draw, salt) = self.points[self.next as usize];
        self.state = mix(self.key ^ mix((draw as u64) << 32 | salt as u64));
        self.next += 1;
        self.hook = if self.next < self.len {
            self.points[self.next as usize].0
        } else {
            u32::MAX
        };
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

    #[test]
    fn resample_points_replace_the_stream_from_their_draw_on() {
        let base = Prng::from_seed([5, 6, 7, 8]);
        let plain: Vec<u32> = {
            let mut p = base;
            (0..12).map(|_| p.next_u32()).collect()
        };
        let mut p = base;
        p.set_resample(99, &[(4, 1), (8, 2)]);
        assert_ne!(p, base, "a pending schedule is part of equality");
        let got: Vec<u32> = (0..12).map(|_| p.next_u32()).collect();
        assert_eq!(got[..4], plain[..4]);
        assert_ne!(got[4..8], plain[4..8]);
        assert_eq!(p.draws(), 12);
        assert!(p.pending().is_empty());
        // Same key and points: same stream; another salt: another stream from that point.
        let mut q = base;
        q.set_resample(99, &[(4, 1), (8, 2)]);
        assert_eq!((0..12).map(|_| q.next_u32()).collect::<Vec<_>>(), got);
        let mut r = base;
        r.set_resample(99, &[(4, 1), (8, 3)]);
        let other: Vec<u32> = (0..12).map(|_| r.next_u32()).collect();
        assert_eq!(other[..8], got[..8]);
        assert_ne!(other[8..], got[8..]);
        // The counter is invisible to equality and Debug.
        let (mut a, b) = (Prng::from_seed([1, 1, 1, 1]), Prng::from_seed([1, 1, 1, 1]));
        a.draws = 7;
        assert_eq!(a, b);
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }

    include!("prng_vectors.rs");
}
