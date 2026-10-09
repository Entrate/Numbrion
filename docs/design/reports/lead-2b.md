# Stage 2B recovery checkpoint

The existing Stage 2B Codex task stopped at its usage limit before its final report.
This checkpoint preserves its partial implementation and fixes the small errors
that prevented it from compiling. It does not pass the full Stage 2B acceptance
gate or establish battle/event parity.

## Recovered implementation

- Event lookup, captured handler collection, dispatcher entry points, direct hook
  invocation, duration handling, frame management and mutable relay storage.
- Priority comparators, selection sort with tie shuffles, stable redirect/slot
  ordering and modifier helpers, with their existing oracle-derived tests.
- Static registry discovery/dispatch, whole-manifest accounting and optional
  process-wide reached-hook counters (`trace-hooks`). Pending callbacks remain
  explicit and panic if reached; they are never silently treated as no-ops.
- Nine effect files: burn, paralysis, sleep, freeze, poison, toxic poison,
  confusion, flinch and Struggle. They declare 22 function sites; the other
  830 function sites remain pending. Effects rely on unfinished core mutators.

## Recovery fixes

- Matched the freeze callback's `forme_change` call to the existing `FormeOptions`
  interface, and matched ability checks to the existing `ignoring_ability` API.
- Removed references to an absent Axe Kick ID and sleep-usable move trait.
  The pinned scope contains neither Axe Kick nor either sleep-usable move
  (Sleep Talk and Snore), so confusion uses its normal duration range and sleep
  blocks the available moves. No generated dex files or IDs were changed.
- Removed unfinished test-probe dispatch wiring that referenced a nonexistent
  scratch field/function. The empty placeholder test module was removed too.
- Formatted the recovered Rust files and removed unused imports.

## Shared changes already present in the recovered work

- `Listener` now carries `selector: HookSelector`. Priority resolution uses the
  requested callback key even when SwitchIn falls back to an `onStart` hook.
- `EndHandler` adds ability and item cleanup variants.
- `Scratch.handlers` uses `CALL_DEPTH` buffers; scratch adds handler nesting,
  current context, occupied relay masks and initial-modifier presence/value.
- `EffectImpl` adds its canonical source filename. Registry metadata now exposes
  manifest disposition, incomplete-port accounting and reached-hook utilities.
- Cargo adds `trace-hooks`, `event-probes` and `event-trace` features. The latter
  enables hook counters only; full schema-compatible event traces are unfinished.

## Validation and remaining work

The all-features engine test run passed 24 tests, including constructor parity,
deterministic codegen, state lifetime, ordering/modifier tests and registry audits.
The standalone differential-test harness passed all 18 tests. These checks do not
execute a complete Rust battle or verify the new effects against a running engine.

Still required:

- Implement all three methods in `event/suppression.rs` and finish their integration
  with the Pokemon ability/item suppression methods owned by core mutators.
- Add the directed dispatcher probes from the original Stage 2B brief: handler
  discovery/order, nesting and state lifetime, scalar/spread relay behavior,
  expiry and suppression, and PRNG states against the pinned Showdown source.
- Port the initial format/rules callbacks and remaining assigned effect families.
- Complete the effect authoring examples/guide and full event trace output.
- Review the recovered collection/dispatch and initial effect bodies against the
  pinned source before counting them as proven parity.
- Complete the lifecycle, choices, moves, damage, mutators and protocol logging
  modules, then replay the oracle corpora through the real engine.

Resume from the current `engine` branch. The pre-fix sources and original task
logs are preserved in the local archive referenced by `docs/RECOVERY.md`.
