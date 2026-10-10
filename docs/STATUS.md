# Status

## 2026-10-10 — parity reached

The Rust engine reproduces the pinned Showdown (`7332b60`) bit-for-bit on `gen9randomdoublesbattle`:
identical protocol log, identical requests (byte-level JSON), identical PRNG state at every decision.

| Corpus | Battles | Result |
|---|---:|---|
| fuzz-10k-s777 (fresh random, never used during development) | 10,000 | 10,000 pass — 254,892 decisions, 4.88M log lines |
| fuzz-2000 (random) | 2,000 | 2,000 pass |
| fuzz-switchy (switch/Tera-heavy policy) | 1,000 | 1,000 pass |
| directed-smoke (24 profiles: slices, all 18 effect batches) | 384 | 384 pass |
| slice0/1/2 (core-only to common effects) | 600 | 600 pass |
| sample-50 | 50 | 50 pass |

Reproduce: `cd crates/difftest && cargo build --release && ./target/release/difftest replay <corpus> --sim engine`.

Speed (first unoptimized reading, slice1 replay, NoLog): ~350 battles/s on one thread vs Showdown's
18.3 battles/s/core. Optimization is the next milestone (target >= 900 battles/s/core).
