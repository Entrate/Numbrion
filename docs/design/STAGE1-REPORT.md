# Stage 1 report

Stage 1 is complete for the pinned `gen9randomdoublesbattle` scope. `cargo build -j 6`
and `cargo test -j 6` pass. The engine has zero dependencies. The oracle checkout is
unchanged and clean; no commits were made. The verified PRNG/math modules and the
protected oracle, benchmark, scope and Showdown-spec directories were left untouched.

## Built

- `docs/design/ARCHITECTURE.md`: architecture v1, with every draft specification
  placeholder replaced by decisions, source references and the effect-file contract.
- `tools/codegen/gen-dex.mjs` and `oracle.mjs`: dependency-free Node 24 generation,
  read-only commit checks, deterministic ordering and content hashes.
- `crates/engine/src/dex/{ids,events,hooks,data}_generated.rs` and handwritten
  `dex/mod.rs`: 572 species, 359 moves, 207 abilities, 61 items, 75 conditions,
  12 rules, 127 event names, and 879 hook entries. An independent source walk matched
  all 852 function sites and four constant handlers; 23 entries retain ordering
  metadata for absent callbacks. Typed hot tables, ordered declarative trees,
  type/status immunities, natures, display names and callback-body fingerprints are included.
- `crates/engine/src/ids.rs`, `state/{mod,effects,choices,scratch}.rs`, `teams.rs`,
  `battle.rs`, `log.rs`, and module exports: fixed Copy state, effect arena/lists,
  relay/context/move shapes, packed input, immutable team definitions, pre-start
  `Battle::new`, and the generic logging contract with NoLog.
- `tools/codegen/dump-test-teams.mjs`, `tests/data/{constructor,scope-counts}.tsv`
  and `tests/stage1.rs`: pinned constructor fixtures and structural verification.

## Validation

Nine tests pass, including the three existing PRNG/math tests. Constructor parity
covers 323 battles / 646 teams: 300 generated teams per side, 20 omitted-gender/nature
variants per side, and three directed packed-format cases. Rust matches the oracle's
post-construction PRNG seed, resolved genders, names/details, HP/stats, moves/PP,
types, abilities/items, Tera type and levels. Cases exercise neutral/nonneutral
natures, shiny details, no-PP-boost Struggle, ability shorthand, defaults, whitespace
and hexadecimal numeric fields. Generated files match both a second codegen run
and the checked-in bytes. Tests also compare ids to the scope catalog, check
removed-effect identity and insertion order, and compare compact attack histories
against full lists over 5,000 operations.

`BattleState` is **37,664 bytes**, asserted at compile time; Pokemon is 540 bytes,
EffectCell 44, Action 40, SideRequest 96 and Relay 16. Battle simulation throughput
has not been measured because turn execution is a later stage.

## Decisions and draft deviations

- Use a uniform 12-Pokemon layout and generic 640-cell arena first. This exceeds
  the draft's 8–16 KiB budget; full closed-scope capacities and easy effect authoring
  take precedence over speculative packing. Counts observed in the census are not
  hard bounds. Queue capacity is 64; transient retired-cell/queue peaks remain
  guarded limits to instrument during Stage 2.
- Pin captured effect objects until dispatch finishes. Removal retires them:
  runEvent can still invoke their captured callbacks; fieldEvent checks current
  identity/location. Keep mutable EffectState.target separately from physical owner.
- Keep ordinary move slots aliased to base slots, with a separate Transform overlay.
  Preserve sparse boost order and distinguish every relay failure sentinel.
- Store one attack summary per stable source, including separate latest numeric
  attacks and positive-damage-this-turn bits. Avoid unbounded history and six-source
  assumptions. Keep ordered hit targets in ActiveMove scratch.
- Use generated HookIds and generic static effect dispatch. Per-effect files own
  payload accessors and bindings; build-time discovery removes shared registry edits.
  Conditions embedded in moves have distinct ids. Sleep Clause's condition/rule
  catalogs have a canonical alias; species condition views preserve actual lookups.
- `Battle::new` deliberately stops before start. The oracle's second setPlayer would
  auto-start; fixtures suspend only that instance's start method at this boundary.
  Missing gender uses the oracle's uniform M/F sample, not the species ratio.

## Spec corrections found

01 §12 recommends implementing every Gen 9 handler for arbitrary legal teams,
which conflicts with this format's closed generated-team scope. Its approximate RD
inventory also excludes lockedmove, while scope.json conservatively retains that
engine source effect. Catalog presence does not prove that volatile is installable.
01 §10 item 14's summary of effectOrder omits RedirectTarget and Dancer ordering;
the pinned source and earlier sections include both. No additional factual error
was established in the construction/stat formulas. The spec files were not edited.

## Open questions and Stage 2 plan

The remaining questions are measured capacity high-water marks and snapshot cost,
not unresolved Stage 1 architecture choices. JSON-array input, out-of-scope ids,
automatic generation and execution after construction are outside this stage's API.
TextLog, choice validation, requests and move execution have contracts/types but
await their assigned stages. The 50x throughput target remains an eventual measured
objective, not a Stage 1 result.

Stage 2 will:

1. Add worker-owned event/call-argument/listener scratch and build.rs effect discovery,
   the generic dispatcher, manifest coverage checks and explicit unreachable-site waivers.
2. Implement priority resolution, literal selection-sort tie shuffling and stable
   event sorts, then collection in the exact Pokemon/relationship/side/field order.
3. Implement singleEvent/runEvent/priorityEvent/eachEvent/fieldEvent, including
   suppression, independent array relays, duration expiry, modifier presence, parent
   context restoration, direct-callback contexts and retired-cell pin release.
4. Add directed Node event probes outside the protected tools/oracle directory;
   compare handler order, relay/state mutations and PRNG seeds. Exercise removed
   effects, constant false/zero/null/undefined, target retagging, tied suppressed
   handlers, duration-only metadata, SwitchIn/redirect ordering and depth guards.
5. Instrument listener/arena/queue peaks and establish copy/dispatch benchmarks
   before changing storage. Publish the tested per-effect contract for later fan-out.
