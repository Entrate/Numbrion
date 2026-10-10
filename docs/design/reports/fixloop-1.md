# Fix loop 1 — implemented-effect correctness

2026-10-10. Worktree `gpt-fix1`, branch `gpt/fix1`, base `f4d334c`.
Oracle: unchanged `~/src/pokemon-showdown`, commit
`7332b60e22b9e8194bb53549549eba241d73cc9a`.

## Results

Exclude a battle only when its first failure is an **unimplemented effect hook** owned by
`protect_redirection`, `disable_lock_trap`, `charge_priority_multihit`, or `move_callbacks`.
No other failures are excluded. These are full-battle pass rates, not prefix pass rates.

| Corpus | Before: passed / eligible | Before: excluded | After: passed / eligible | After: excluded |
| --- | ---: | ---: | ---: | ---: |
| Random 2,000 | 0 / 4 (0%) | 1,996 | 0 / 0 (N/A) | 2,000 |
| Directed smoke 384 | 284 / 285 (99.65%) | 99 | 285 / 285 (100%) | 99 |
| Core slice0 | 200 / 200 (100%) | 0 | 200 / 200 (100%) | 0 |
| Core slice1 | 200 / 200 (100%) | 0 | 200 / 200 (100%) | 0 |

Core before numbers are the supplied base baseline; both slices were replayed after the first
fix and again with all fixes applied.

All five observed log/request failures are resolved. Random battles now reach pending hooks:
this does **not** establish full correctness for those battles. Random matched decision steps
increase from 748 to 786; smoke from 7,758 to 7,787. There are no remaining implemented-effect
divergences, core panics, unexpected rejections, or warnings in these runs.

## Fixes and root causes

1. **Hypothetical trapping callbacks were silently omitted** (`be4bc0f`).
   The lifecycle calls `singleEvent('FoeMaybeTrapPokemon', ...)` for every possible ability of
   a visible foe species. Generated Rust hooks represent `onFoeMaybeTrapPokemon` as
   `MaybeTrapPokemon` + `HookRel::Foe`, so the default `On` lookup never found the callback.
   Resolve that hook explicitly and pass it as the custom callback, retaining the original
   event name and arguments. Add Arena Trap, Magnet Pull, and Shadow Tag's two callbacks each:
   request callbacks were necessary to preserve existing core tests, and the registry requires
   whole-effect declarations. Their source fallback, adjacency, groundedness/known-type,
   Steel typing, and Shadow Tag immunity checks follow the pinned source.
   Enable the formerly ignored full-startup test (50 pinned log/seed/request vectors).

   Repros: random positions **523, 586, 846**, all missing `active[1].maybeTrapped` at startup;
   smoke **176**, missing it at turn 2. Smoke 176 now completes identically. The random repros
   now stop at expected Choice Lock, Sucker Punch, and First Impression hooks respectively.
   The pending trap batch must reconcile these three ability files when it merges.

2. **Teraform Zero activated on an empty field** (`30a034f`).
   Weather and terrain arena cells are allocated even with no effect. Comparing their
   `CellId`s against `NONE` always passed, emitting an extra `|-ability|...|Teraform Zero`.
   Compare the cells' effect IDs instead. Add weatherless Terapagos terastallization coverage;
   existing electric-terrain scenarios cover activation when the field has an effect.
   Random **1434** previously diverged at the first turn. It now matches through step 2 and
   stops at expected Protect on step 3 / turn 3.

3. **Nested hit callbacks used the live move as the current effect** (`388e5e5`).
   Both `hit_single_event` and `run_move_effects` dispatched nested objects as the primary
   move. Showdown uses the anonymous secondary/self object as `battle.effect`, while the
   callback's `sourceEffect` argument remains the live move. `HitView.callback_effect` now
   preserves this distinction. Remove Dire Claw/Tri Attack's local empty-effect workarounds.
   A regression exercises ordinary default attribution for Dire Claw sleep, checking the
   exact untagged status log and both PRNG draws. Existing status-lottery scenario replays
   remain exact. Close the open issue in `integration-notes.md`.

The other integration-note items already have central implementations: event target defaulting
in `effective_terrain`, event source defaulting in `heal`, and source/effect defaulting in
`mutation_attribution`. Their oracle vectors and relevant effect integration tests pass.

## Tests

- Initial `cargo test -j 4 -- --ignored`: **65 pass, 6 fail**. Startup's request mismatch was
  fixed; the text-log test required its documented external input; four fail at pending hooks.
- Enable **66** formerly ignored tests (startup with the trapping fix; another 65 in `d5ccd28`).
- Final `cargo test -j 4`: **270 engine tests pass, 5 ignored**; auxiliary suites **6 + 2 pass**.
- Final ignored run with `TEXTLOG_REPLAY=/tmp/fixloop-textlog.tsv`: **1 pass, 4 fail**, all four
  expected pending-hook failures. Generate that input with:

  ```sh
  node tools/probes/textlog/extract-replay.mjs --out /tmp/fixloop-textlog.tsv --select 60 \
    /home/aminaliu/.t3/projects/numbrion/fuzz-out/fuzz-2000.jsonl \
    data/fixtures/directed-smoke.jsonl.gz
  ```

  This covers all **428** observed log shape/feature keys: 60 battles, 27,605 log entries.
  The external-input test remains opt-in because an ordinary test run has no `TEXTLOG_REPLAY`.

Remaining ignored tests have precise blockers in their ignore reasons:

| Test | Pending hook |
| --- | --- |
| Weather `rain` | `conditions:roost onStart` — move_callbacks |
| Weather `snow` | `moves:protect onPrepareHit` — protect_redirection |
| Reactive stats scenarios | `moves:protect onPrepareHit` — protect_redirection |
| Hazards/screens scenarios | `conditions:partiallytrapped durationCallback` — disable_lock_trap |

## Replay and trace verification

Built `crates/difftest` with `cargo build --release -j 4`. Ran both requested corpora with
`--sim engine --jobs 4 --top 40` in text and JSON forms; final JSON summaries/signatures are
preserved, together with baseline summaries, in [fixloop-1-signatures.json](fixloop-1-signatures.json).
Both core slices remain 200/200. All fix commits carry the requested co-author trailer.

`tools/oracle/trace.mjs` verified random 523 and 1434 at level 3 against their fixtures.
The full turn-1 trace for 523 confirms the hypothetical Magnet Pull single events; use a trace
without `--skip-empty` here because request-only side effects can be discarded as trivial pairs.
For 1434, a temporary Rust harness drained the engine's real TextLog to level-1 JSONL records.
`trace-diff --kinds log --merge-logedits` compared its two-step prefix with the oracle prefix:
**68 identical log records**. This was a log projection; the engine's `trace-hooks` feature
records reached hook IDs, and does not emit the full event/RNG JSONL format.

## Remaining divergence signatures

No non-pending signatures remain. Every remaining panic maps to one of the four allowed batches:

| Pending batch | Random | Smoke |
| --- | ---: | ---: |
| protect_redirection | 1,069 | 18 |
| disable_lock_trap | 450 | 42 |
| charge_priority_multihit | 401 | 23 |
| move_callbacks | 80 | 16 |

Most frequent random signatures: Protect `onPrepareHit` **737**, Choice Lock `onStart` **392**,
Prankster `onModifyPriority` **155**, Helping Hand `onTryHit` **95**, Fake Out `onTry` **70**,
Lightning Rod `onAnyRedirectTarget` **45**, Storm Drain `onAnyRedirectTarget` **40**,
Loaded Dice `onModifyMove` **39**, Sucker Punch `onTry` **32**, Truant `onStart` **25**.
The JSON companion contains the complete before/after signature lists, counts, batch ownership,
and up to five `--battle` repro positions for each signature.
