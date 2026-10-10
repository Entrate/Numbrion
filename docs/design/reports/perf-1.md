# Performance 1 — 2026-10-10

Worktree `gpt-perf1`, branch `gpt/perf1`, base `213512f`.

The final engine exceeds the **900 battles/s on one thread** target: **942 battles/s**
unprofiled on `fuzz-2000`, versus **335 battles/s** at the base commit (2.81×).
The requested pprof run measured **934 battles/s**. All five optimization commits
passed their complete post-commit correctness gates. No effect files were changed.

## Measurements

Ryzen 5 3600, 6 cores / 12 hardware threads, 16 GB; Rust 1.99.0.
Both binaries used `target-cpu=native`, frame pointers, opt-level 3, thin LTO and
four codegen units, as in the original difftest release profile. No build-setting
changes contributed to these gains. Builds used `-j 4`.

Each measurement used the same recorded random `fuzz-2000.jsonl` battles, `NoLog`,
10 requested seconds and the bench's full-corpus warm-up / ending-turn assertions.
The timer checks between complete corpus passes, so actual measurement durations
can exceed 10 seconds. Loading and warm-up are outside the timed region. Each
worker replays the whole corpus; threads measure independent battle throughput.

Final unprofiled runs were paired sequentially by thread count. No builds or
correctness replays overlapped them. This shared PC had varying background load
(roughly 6–9 during the runs); these are short measurements, not isolated scaling
limits. The user's earlier 355 battles/s baseline and the rates below therefore
need not coincide.

| Threads | Base battles/s | Final battles/s | Speedup | Final battles/s/thread |
|---:|---:|---:|---:|---:|
| 1 | 335 | 942 | 2.81× | 941.6 |
| 6 | 1,275 | 3,831 | 3.00× | 638.5 |
| 12 | 1,545 | 6,127 | 3.97× | 510.6 |

## Changes and individual measurements

The rates here are the individual **pprof-enabled**, one-thread runs used for
commit messages, rather than the unprofiled final comparison above.

| Commit | Change | Battles/s | Passing tests after commit |
|---|---|---:|---:|
| `213512f` | Rerun baseline | 293 | — |
| `ddfc792` | Dense exact hook index | 336 | 334 |
| `5320d68` | Prune globally absent callback relations | 643 | 335 |
| `da787d0` | Finish zero-listener events without event frames | 678 | 337 |
| `3cb6fbe` | Prune callbacks absent from current effects | 892 | 337 |
| `d579830` | Initialize only used handler storage | 934 | 337 |

1. **Exact hook index.** `dex/hook_index.rs` derives a dense
   `(EffectId, EventId, HookRel) -> HookId` table at compile time from the generated
   dex metadata. It replaces the manifest hook scan. It keeps the first exact
   match, excludes nested move sites and retains `Absent` hooks for ordering.
   Canonical aliases and species-condition views retain their original lookup
   semantics. An exhaustive metadata test compares the index with manifest scans.
   The index occupies about 1.87 MiB of immutable data.
2. **Global relation masks.** An immutable union proves that some events or
   relations have no callback anywhere in the pinned closure. Collection skips
   only those empty paths. Masks include possible ability/item `SwitchIn -> Start`
   fallback callbacks. Defined constants, including false/zero, remain listeners.
3. **Empty dispatch.** After collection and relay normalization, a zero-listener
   event returns without pushing/popping a frame. Integer numeric relays still
   go through the original modifier arithmetic, including JS wrapping; omitted,
   false, fractional and spread relays retain their distinct semantics. The
   event-limit checks and handler-stack reservation still run.
4. **Current-effect presence.** Per-effect relation masks (about 160 KiB) prove
   absence before the expensive generic callback lookup. Pokémon presence is
   recomputed cheaply from their current ability, item, base species, status,
   volatiles and slot conditions. It avoids most full Pokémon copies and empty
   collections. There is no persistent cache to invalidate on direct state edits,
   nested mutation, removal or snapshot restoration. A positive mask still uses
   normal collection, priority resolution and pinning.
5. **Handler storage.** Each nested buffer reserves its original 2,304-entry
   capacity using `Vec::with_capacity`, pushes initialized listeners and drains
   their pins on release. Construction no longer fills 16 × 2,304 × 80 bytes
   (2.8125 MiB) with `None`. Allocation remains at construction, collection remains
   allocation-free, and the existing capacity guard prevents growth.

Listener order, duplicates and priority sorting are preserved. Duration collection
is excluded from callback-only pruning, since duration expiry can collect a
listener without a hook. No path with two or more collected listeners bypasses
sorting, including suppressed listeners. `each_event` retains its separate active
Pokémon sort: tied active speeds consume PRNG draws even when callbacks are absent.
Dedicated tests cover that distinction and empty-event relay corner cases.

## Before / after profile

pprof, 999 Hz, one thread, same native/frame-pointer release build.
The saved [before flamegraph](perf-1-before.svg) has 13,578 samples; the
[after flamegraph](perf-1-after.svg) has 10,688. Sample counts differ because timed
loops finish whole corpus passes. Values below are sampled **self time**, including
inlined functions as reported by the existing bench.

| Function | Before | After |
|---|---:|---:|
| `find_event_handlers` | 29.5% | 22.2% |
| `find_pokemon_event_handlers` | 13.7% | 1.2% |
| `get_callback` | 12.0% | 0.5% |
| `run_inner` | 8.2% | 10.9% |
| `run_event` | 5.3% | 10.0% |
| `with_log` (construction) | 6.1% | 0.8% |
| `sort_handlers` | 1.5% | 4.7% |
| `each_event` | 1.5% | 2.6% |
| `find_field_event_handlers` | 1.2% | 2.3% |
| `find_battle_event_handlers` | 1.2% | 0.6% |

The remaining profile also includes `use_move_inner_body` (5.2%),
`lc_not_immune_to_trapped` (5.1%), `queue_add_choice` (4.2%) and the new
`pokemon_has_callback` (1.4%). Higher relative percentages do not imply slower
functions: the total time per battle has fallen substantially.

## Correctness gate

After **each** optimization commit, `cargo test -j 4` passed, followed by a release
`difftest replay --sim engine` of **all** seven required corpora. All runs passed
100%, including logs, requests and PRNG checkpoints. Final tests: **337 passed,
0 failed, 10 previously ignored**. No correctness tolerance was relaxed.

| Corpus | Battles | Result after every commit |
|---|---:|---|
| slice0-200 | 200 | 100% |
| slice1-200 | 200 | 100% |
| slice2-200 | 200 | 100% |
| sample-50 | 50 | 100% |
| directed-smoke | 384 | 100% |
| fuzz-2000 | 2,000 | 100% |
| fuzz-10k-s777 | 10,000 | 100% |

Total per gate: **13,034 battles**, **331,149 decision steps**, **6,365,139 log
lines**. The fresh 10k corpus alone matched all 254,892 steps and 4,876,765 lines.
Raw build, benchmark, test and replay summaries for this session remain under
`/tmp/numbrion-perf-gpt/`.

## Reproduce

```sh
cd crates/difftest
RUSTFLAGS="-C target-cpu=native -C force-frame-pointers=yes" \
  cargo build --release -j 4 --bins --target-dir /tmp/numbrion-prof-gpt
/tmp/numbrion-prof-gpt/release/bench \
  /home/aminaliu/.t3/projects/numbrion/fuzz-out/fuzz-2000.jsonl \
  --threads 1 --seconds 10 --profile /tmp/flame-gpt.svg
# Repeat without --profile, with --threads 1, 6 and 12 for throughput.
```

The PC's `/tmp` quota prevented linking during this session. The target directory
was moved into this worktree's `crates/difftest/target/prof-gpt`, with
`/tmp/numbrion-prof-gpt` pointing there. Compiler temporary files also used the
worktree disk. This does not change binary settings or timed benchmark work.

```sh
# From the worktree root:
cargo test -j 4
cd crates/difftest
for fixture in ../../data/fixtures/{slice0-200,slice1-200,slice2-200,sample-50,directed-smoke}.jsonl.gz \
  /home/aminaliu/.t3/projects/numbrion/fuzz-out/fuzz-2000.jsonl \
  /home/aminaliu/.t3/projects/numbrion/fuzz-out/fuzz-10k-s777.jsonl.gz; do
  /tmp/numbrion-prof-gpt/release/difftest replay "$fixture" --sim engine --jobs 4 --quiet
done
```

## Ideas not yet implemented

- A battle-wide presence summary maintained on install/remove, with explicit
  coverage for all mutations and snapshot restoration. The current fresh checks
  deliberately avoid that invalidation burden.
- Reuse collection results across spread targets, preserving duplicate listeners,
  target indices, pins and exact sorting/PRNG behavior.
- Narrow the remaining positive-path Pokémon snapshots instead of copying all
  540 bytes; the absence path already avoids those copies.
- Reuse construction scratch across battles and reduce tie-index buffer clearing
  for small sorts, while retaining fixed capacities and nested-event safety.
- Investigate `lc_not_immune_to_trapped` / `get_types`, including replacing the
  immunity-name search with the generated typed immunity index where equivalent.
- Separately measure one codegen unit, fat LTO and bench-only `panic=abort`.
  None was used here. PGO can follow, trained on separate representative battles
  and subject to the same full correctness gate.
