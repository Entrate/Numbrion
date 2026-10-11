//! Fair-information search support (docs/design/SEARCH-PREREQUISITES.md). Nothing here reads a live
//! battle's hidden state: every input is what one player observes (its own team, its own choices
//! and its player-view protocol lines), so the same code serves self-play and a ladder client.
//!
//! * `protocol`: parsing helpers for player-view protocol lines.
//! * `reveal`: `RevealTracker`, the public information per Pokemon (moves used, item, ability and
//!   Tera type shown, HP, status) built from protocol lines.
//! * `sets`: `SetPool`, a set sampler over generated random-battle teams, consistent with what was
//!   revealed.
//! * `world`: `WorldBuilder`, determinized worlds built from sampled teams by replaying the observed
//!   history (rejecting samples whose replayed player view differs from the observed one).
pub mod protocol;
pub mod reveal;
pub mod sets;
pub mod world;

/// SplitMix64: the deterministic generator of every sampling decision in this module. Results
/// depend only on the seed and the call sequence, never on threads or the platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`), by multiply-shift (bias below 2^-32 for the sizes used here).
    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        (((self.next_u64() >> 32) * n as u64) >> 32) as usize
    }

    /// True with probability `p`.
    #[inline]
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64) < p
    }

    /// A battle seed (four 16-bit words).
    pub fn seed(&mut self) -> [u16; 4] {
        let v = self.next_u64();
        [(v >> 48) as u16, (v >> 32) as u16, (v >> 16) as u16, v as u16]
    }

    /// An independent generator derived from this one.
    pub fn fork(&mut self) -> Self {
        SplitMix64(self.next_u64())
    }
}
