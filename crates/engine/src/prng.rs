//! Showdown's Gen5 PRNG (sim/prng.ts `Gen5RNG` + `PRNG`), bit-exact.
//!
//! Showdown keeps the 64-bit LCG state as four 16-bit words, most significant first;
//! seed `"a,b,c,d"` packs to `a<<48 | b<<32 | c<<16 | d`. Every helper here must consume
//! exactly the same number of draws, with the same range mapping, as the TypeScript.
//!
//! With the opt-in `search` feature (no Showdown counterpart): a draw counter and a short
//! schedule of *resample points*. A point `(draw, salt)` switches the generator, right before
//! draw number `draw`, to the stream of the schedule key and `salt`, so every draw from there
//! on is fresh while all earlier draws are unchanged. A point with salt `RESUME` switches back
//! to the stream the generator had when the schedule was installed, so a pair of points
//! resamples a window of draws and keeps every later one. Streams are indexed by absolute draw
//! number: splitting a window with another point keeps the draws on both sides. The world builder
//! (`search::world`) uses it to resample the random outcomes behind one protocol line at a time.
//! Equality, hashing and `Debug` ignore the counter while no point is pending: two generators
//! are equal when they will produce the same stream. Without the feature the generator is the
//! plain 64-bit state and every draw costs exactly what it did before.

/// Most resample points one schedule can hold.
#[cfg(feature = "search")]
pub const RESAMPLE_CAPACITY: usize = 16;

/// Salt of a resample point that resumes the stream the schedule was installed on: from its draw
/// on, draws are what they would have been without any point.
#[cfg(feature = "search")]
pub const RESUME: u32 = u32::MAX;

#[cfg(not(feature = "search"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Prng {
    state: u64,
}

#[cfg(feature = "search")]
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
    /// State and draw count when the schedule was installed (what `RESUME` points return to).
    base: u64,
    base_draws: u32,
}

/// LCG state, and while points are pending: key, draw position, points, and the stream `RESUME`
/// points return to.
#[cfg(feature = "search")]
type Stream<'a> = (u64, Option<(u64, u32, &'a [(u32, u32)], Option<(u64, u32)>)>);

#[cfg(feature = "search")]
impl Prng {
    /// What determines the future stream: the LCG state and, while resample points are pending,
    /// the key, the draw position and the points.
    fn stream(&self) -> Stream<'_> {
        let pending = self.pending();
        let resume = pending.iter().any(|p| p.1 == RESUME).then_some((self.base, self.base_draws));
        (self.state, (!pending.is_empty()).then_some((self.key, self.draws, pending, resume)))
    }
}

#[cfg(feature = "search")]
impl PartialEq for Prng {
    fn eq(&self, other: &Self) -> bool {
        self.stream() == other.stream()
    }
}
#[cfg(feature = "search")]
impl Eq for Prng {}

#[cfg(feature = "search")]
impl core::hash::Hash for Prng {
    fn hash<H: core::hash::Hasher>(&self, h: &mut H) {
        self.stream().hash(h);
    }
}

#[cfg(feature = "search")]
impl core::fmt::Debug for Prng {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut d = f.debug_struct("Prng");
        d.field("state", &self.state);
        if self.hook != u32::MAX {
            d.field("key", &self.key)
                .field("draws", &self.draws)
                .field("pending", &self.pending());
            if self.pending().iter().any(|p| p.1 == RESUME) {
                d.field("base", &(self.base, self.base_draws));
            }
        }
        d.finish()
    }
}

/// `state` advanced by `n` LCG steps (composing `x -> MUL * x + ADD` by squaring).
#[cfg(feature = "search")]
const fn skip(mut state: u64, mut n: u32) -> u64 {
    let (mut mul, mut add) = (MUL, ADD);
    while n > 0 {
        if n & 1 == 1 {
            state = state.wrapping_mul(mul).wrapping_add(add);
        }
        add = add.wrapping_mul(mul.wrapping_add(1));
        mul = mul.wrapping_mul(mul);
        n >>= 1;
    }
    state
}

/// SplitMix64 finalizer: the seed of a resample stream.
#[cfg(feature = "search")]
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
        #[cfg(not(feature = "search"))]
        return Prng { state };
        #[cfg(feature = "search")]
        Prng {
            state,
            draws: 0,
            hook: u32::MAX,
            next: 0,
            len: 0,
            key: 0,
            points: [(0, 0); RESAMPLE_CAPACITY],
            base: 0,
            base_draws: 0,
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
        #[cfg(feature = "search")]
        {
            if self.draws == self.hook {
                self.resample();
            }
            self.draws = self.draws.wrapping_add(1);
        }
        self.state = self.state.wrapping_mul(MUL).wrapping_add(ADD);
        (self.state >> 32) as u32
    }

    /// Draws taken since construction (or the last `from_seed`). Search bookkeeping only.
    #[cfg(feature = "search")]
    #[inline]
    pub const fn draws(&self) -> u32 {
        self.draws
    }

    /// Install a resample schedule: before draw `points[i].0` (absolute, see `draws`) the state
    /// becomes a function of `key` and `points[i].1`, or with salt `RESUME` the state the stream had
    /// without any point. Indices must be strictly increasing and not below `draws()`; at most
    /// `RESAMPLE_CAPACITY` points. Replaces any pending schedule.
    #[cfg(feature = "search")]
    pub fn set_resample(&mut self, key: u64, points: &[(u32, u32)]) {
        assert!(points.len() <= RESAMPLE_CAPACITY, "too many resample points");
        assert!(
            points.windows(2).all(|w| w[0].0 < w[1].0)
                && points.first().is_none_or(|p| p.0 >= self.draws && p.0 != u32::MAX),
            "resample points must be increasing and in the future"
        );
        self.key = key;
        (self.base, self.base_draws) = (self.state, self.draws);
        self.points[..points.len()].copy_from_slice(points);
        self.len = points.len() as u8;
        self.next = 0;
        self.hook = points.first().map_or(u32::MAX, |p| p.0);
    }

    /// Drop any pending resample points.
    #[cfg(feature = "search")]
    pub fn clear_resample(&mut self) {
        self.len = 0;
        self.next = 0;
        self.hook = u32::MAX;
    }

    /// The resample points that have not fired yet.
    #[cfg(feature = "search")]
    pub fn pending(&self) -> &[(u32, u32)] {
        if self.hook == u32::MAX {
            &[]
        } else {
            &self.points[self.next as usize..self.len as usize]
        }
    }

    #[cfg(feature = "search")]
    #[cold]
    fn resample(&mut self) {
        let (draw, salt) = self.points[self.next as usize];
        self.state = if salt == RESUME {
            skip(self.base, draw - self.base_draws)
        } else {
            skip(mix(self.key ^ mix(salt as u64)), draw)
        };
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

    #[cfg(feature = "search")]
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
        let mut other_key = base;
        other_key.set_resample(98, &[(4, 1), (8, 2)]);
        let mut same = base;
        same.set_resample(99, &[(4, 1), (8, 2)]);
        assert_ne!(other_key, same, "the key is part of equality while points are pending");
        let mut r = base;
        r.set_resample(99, &[(4, 1), (8, 3)]);
        let other: Vec<u32> = (0..12).map(|_| r.next_u32()).collect();
        assert_eq!(other[..8], got[..8]);
        assert_ne!(other[8..], got[8..]);
        // Splitting a window keeps its draws: (4, 1) then (6, 1) is the same stream as (4, 1).
        let mut split = base;
        split.set_resample(99, &[(4, 1), (6, 1), (8, 2)]);
        assert_eq!((0..12).map(|_| split.next_u32()).collect::<Vec<_>>(), got);
        // A window: draws 4..8 resampled, the plain stream from draw 8 on.
        let mut w = base;
        w.next_u32();
        w.set_resample(99, &[(4, 1), (8, RESUME)]);
        let windowed: Vec<u32> = core::iter::once(plain[0]).chain((1..12).map(|_| w.next_u32())).collect();
        assert_eq!(windowed[..4], plain[..4]);
        assert_eq!(windowed[4..8], got[4..8]);
        assert_eq!(windowed[8..], plain[8..]);
        // The counter is invisible to equality and Debug.
        let (mut a, b) = (Prng::from_seed([1, 1, 1, 1]), Prng::from_seed([1, 1, 1, 1]));
        a.draws = 7;
        assert_eq!(a, b);
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }

    include!("prng_vectors.rs");
}
