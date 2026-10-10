# effects(recursive_switch)

All eight assigned records / 15 function sites are ported against the read-only
Showdown checkout at `7332b60e22b9e8194bb53549549eba241d73cc9a`. Every function
site is in `HOOKS`; `WAIVERS` is empty throughout. Work is isolated to branch
`gpt/recursive`, based on `878d619`.

## Files and semantics

Canonical files under `crates/engine/src/effects/`:

| File | Pinned source and semantics |
| --- | --- |
| `moves/batonpass.rs` | `data/moves.ts:1101-1111`. Cannot-switch/Commanded failure marks the move still, logs failure, and returns `NotFail`. The self callback sets `SKIP_BEFORE_SWITCH_OUT`; actual boost/volatile transfer is core-owned. |
| `moves/instruct.rs` | `data/moves.ts:9650-9673`. Rejects missing last move, Dynamax, forbidden/charge/recharge/Z/Max moves, pending preparation volatiles, and an exhausted existing PP slot. Logs Instruct, resolves a fresh move action at the saved target location, and prioritizes it without canceling the target's other queued action. Resolving the action retains the core's priority/target event draws. |
| `moves/partingshot.rs` | `data/moves.ts:13174-13179`. Ordered Atk then SpA decreases; deletes the live move's self-switch overlay only if boost failed and target does not have active Mirror Armor. |
| `moves/revivalblessing.rs` | `data/moves.ts:15120-15124`. Returns false without a fainted teammate. Declarative slot condition, midturn request, fainted party selection, revival HP and continuation belong to core. |
| `moves/shedtail.rs` | `data/moves.ts:16171-16193`. Preserves gate order: cannot switch/Commanded, existing Substitute, HP <= ceil(maxHP/2). Pays ceil(maxHP/2) through direct damage and sets skip-before-switch-out. Core transfers only Substitute. |
| `moves/substitute.rs` | `data/moves.ts:18314-18327`. Existing-Substitute and weak failures return `NotFail`, retaining their distinct logs. HP gate compares against fractional maxHP/4; the direct-damage mutator floors the paid cost. |
| `abilities/magicbounce.rs` | `data/abilities.ts:2440-2462`. Preserves self/ally, already-bounced, reflectable and semi-invulnerability gates. Allocates a fresh move by ID, sets hasBounced and clears pranksterBoosted, then calls inside `use_move` against the original source. The ally-side callback uses the captured ability holder and marks the original move bounced after the nested call. Returns null. Scratch reclamation preserves core's globally active nested move. |
| `conditions/substitute.rs` | `data/moves.ts:18329-18377`. Start distinguishes Shed Tail attribution and deletes partial trapping without its End callback. TryPrimaryHit preserves bypass/infiltration, calls getDamage once, caps damage to substitute HP, logs break/absorption, then applies recoil, drain, single AfterSubDamage, and run AfterSubDamage in source order. Returns numeric zero (`HIT_SUBSTITUTE`), preserving self effects. |

Dancer has no function sites or assigned effect file: its special activation and
recursive execution already belong to core. No other batches' effect files,
generated dex, registry, build script, or core modules were changed.

## Payload and APIs

Only `conditions/substitute.rs` uses payload: word 0 is integer Substitute HP;
custom presence bit 8 records its creation. All other files use zero words.
No heap allocations, formatted strings, extra PRNG draws, or scratch handles in
persistent state. Core owns transfers, requests, boosts, HP, queue and moves.
The partial-trapping deletion mirrors the source's direct property deletion by
removing the insertion-list entry and retiring the original arena cell, preserving
listener pins and omitting End.

No missing/requested core APIs or core bug fixes. Two format exclusions are
explicit in source comments: all four OHKO moves are absent from `data/scope.json`,
and `lastDamage` is the scoped dead store described in IMPLEMENTATION-PLAN.md.
Out-of-scope preparation volatiles are optional-ID checks that never match NONE.

## Validation

```sh
node tools/oracle/gen-directed.mjs --profile batch:recursive_switch --count 100 --threads 2 --out /tmp/recursive.jsonl.gz --verify
cd crates/difftest
cargo build --release -j 3
./target/release/difftest replay /tmp/recursive.jsonl.gz --sim engine --jobs 3 --top 20
./target/release/difftest replay ../../data/fixtures/slice0-200.jsonl.gz --sim engine --jobs 3 --top 20
```

Run seed **1116**: 100/100 fixtures verified by the oracle. Engine: **67 passed,
33 other-batch panics, zero divergences or ordering warnings**. Every exclusion
is `abilities:naturalcure onCheckShow`, hook 340 (healing_residual). Adjusted
pass rate **67/67 = 100%**. JSON: `/tmp/recursive-replay.json`; human replay:
`/tmp/recursive-replay.txt`. Core regression remains **200/200**.

An initial local Instruct port error (battle 0, step 9; also other Instruct uses)
was fixed: generated `isZ`/`isMax` fields are present with false values, so field
presence cannot substitute for JS truthiness. Final replay is clean apart from
the named unrelated panic. This was an effect-port fix, not a core change.

`tools/probes/recursive_switch/scenarios.mjs` generates independent full-protocol
and final-PRNG TSV expectations in `moves/batonpass/recursive_switch.tsv` and
`item_exchange.tsv`. The private `moves/batonpass/tests.rs` runs 15 recursive
battles covering successful, repeated and weak Substitute, status blocking,
break/recoil and drain, Baton Pass transfer/failure, Shed Tail transfer/weak,
Magic Bounce status/side/reflected Parting Shot, Instruct repeat/missing last
move, and Revival Blessing failure/success with party permutation.
Tests retain `#[ignore = "needs core"]` and are explicitly run with
`cargo test -j 2 --lib moves_batonpass -- --include-ignored`.
The shared test suite also includes 15 item battles and eight oracle callback
branch vectors. All pass. Normal `cargo build -j 2` and `cargo test -j 2` pass,
including registry/manifest coverage and NoLog compilation.

Initial implementation commit: `272ba47`. Validation/probes/report are in the
subsequent validation commit on this branch. No push or oracle modification.
