//! Pre-generated packed-team pools (one packed team per line, `.txt` or `.txt.gz`), plus the small
//! PRNG the environments use to sample them.

use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::Path,
    sync::Arc,
};

/// Teams of one or more pool files, stored back to back.
#[derive(Default)]
pub struct TeamPool {
    teams: Vec<Arc<str>>,
}

impl TeamPool {
    pub fn from_lines<I: IntoIterator<Item = S>, S: AsRef<str>>(lines: I) -> Self {
        let mut pool = TeamPool::default();
        for l in lines {
            pool.push_line(l.as_ref());
        }
        pool
    }

    /// Append one packed team (blank lines are ignored).
    pub fn push_line(&mut self, line: &str) {
        let t = line.trim();
        if !t.is_empty() {
            self.teams.push(Arc::from(t));
        }
    }

    /// Append every non-empty line of `path` (gzip if the name ends in `.gz`), at most `limit` lines.
    pub fn load_file(&mut self, path: &Path, limit: Option<usize>) -> io::Result<usize> {
        let file = File::open(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let reader: Box<dyn Read> = if path.extension().is_some_and(|e| e == "gz") {
            Box::new(flate2::read::MultiGzDecoder::new(file))
        } else {
            Box::new(file)
        };
        let before = self.teams.len();
        for line in BufReader::with_capacity(1 << 20, reader).lines() {
            self.push_line(&line?);
            if limit.is_some_and(|l| self.teams.len() - before >= l) {
                break;
            }
        }
        Ok(self.teams.len() - before)
    }

    pub fn load<P: AsRef<Path>>(paths: &[P], limit_per_file: Option<usize>) -> io::Result<Self> {
        let mut pool = TeamPool::default();
        for p in paths {
            pool.load_file(p.as_ref(), limit_per_file)?;
        }
        if pool.teams.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "team pool is empty"));
        }
        Ok(pool)
    }

    pub fn len(&self) -> usize {
        self.teams.len()
    }

    pub fn is_empty(&self) -> bool {
        self.teams.is_empty()
    }

    pub fn get(&self, i: usize) -> &Arc<str> {
        &self.teams[i]
    }
}

/// SplitMix64: tiny, fast, and good enough for sampling teams and seeds. Every environment owns its own
/// streams, derived from the BatchEnv seed and the environment index, so results do not depend on the
/// number of threads.
#[derive(Clone, Debug)]
pub struct SplitMix(u64);

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl SplitMix {
    pub fn new(seed: u64) -> Self {
        SplitMix(mix(seed ^ 0x9E37_79B9_7F4A_7C15))
    }

    /// Independent stream `k` of environment `index` under `seed`.
    pub fn stream(seed: u64, index: u64, k: u64) -> Self {
        SplitMix::new(mix(mix(seed).wrapping_add(index.wrapping_mul(0xD1B5_4A32_D192_ED03))).wrapping_add(k))
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }

    /// Uniform in `0..n` (`n > 0`).
    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        ((self.next_u64() >> 32) * n as u64 >> 32) as usize
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Four random 16-bit words, a battle seed.
    pub fn battle_seed(&mut self) -> [u16; 4] {
        let x = self.next_u64();
        [(x >> 48) as u16, (x >> 32) as u16, (x >> 16) as u16, x as u16]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_deterministic_and_distinct() {
        let a: Vec<u64> = (0..4).map(|_| SplitMix::stream(7, 3, 0).next_u64()).collect();
        assert!(a.windows(2).all(|w| w[0] == w[1]));
        assert_ne!(SplitMix::stream(7, 3, 0).next_u64(), SplitMix::stream(7, 4, 0).next_u64());
        assert_ne!(SplitMix::stream(7, 3, 0).next_u64(), SplitMix::stream(7, 3, 1).next_u64());
        let mut r = SplitMix::new(1);
        for n in [1usize, 2, 7, 1000] {
            for _ in 0..100 {
                assert!(r.below(n) < n);
            }
        }
    }
}
