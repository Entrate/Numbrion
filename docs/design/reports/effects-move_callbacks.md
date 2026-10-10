# Effect batch `move_callbacks` (variable damage / type / effects)

Base: `c3f85db` (whole core implemented). Branch `worktree-agent-a860acb6fe5ee8dcf`.
Keys (`docs/design/EFFECT-BATCHES.json`): 36 moves + `conditions:glaiverush` + `conditions:roost`.
55 function sites in the manifest; 54 ported by this batch, `moves:struggle` (1 site, `onModifyMove`) already had a
file at the base commit and was left untouched (it is exercised and passes: 33 calls in the main corpus, 60/60 alone).
`conditions:roost` has one more manifest entry, `onResidual` (order 25), which is order-only metadata (`Absent`), not a
function site, so it is not in `HOOKS`.

## Files written

All under `crates/engine/src/effects/`:

| file | function sites |
| --- | --- |
| `moves/acrobatics.rs` | basePowerCallback (+ hosts the batch's test module) |
| `moves/alluringvoice.rs` | secondary.onHit, secondaries.0.onHit |
| `moves/avalanche.rs` | basePowerCallback |
| `moves/bellydrum.rs` | onHit |
| `moves/burningjealousy.rs` | secondary.onHit, secondaries.0.onHit |
| `moves/clangoroussoul.rs` | onTry, onTryHit, onHit |
| `moves/clearsmog.rs` | onHit |
| `moves/collisioncourse.rs`, `moves/electrodrift.rs` | onBasePower |
| `moves/doubleshock.rs` | onTryMove, self.onHit |
| `moves/dragonenergy.rs`, `moves/eruption.rs`, `moves/waterspout.rs` | basePowerCallback |
| `moves/endeavor.rs` | damageCallback, onTryImmunity |
| `moves/facade.rs`, `moves/lashout.rs`, `moves/gravapple.rs` | onBasePower |
| `moves/ficklebeam.rs` | onBasePower (1 PRNG draw) |
| `moves/finalgambit.rs`, `moves/ruination.rs`, `moves/superfang.rs` | damageCallback |
| `moves/freezedry.rs` | onEffectiveness |
| `moves/fusionbolt.rs`, `moves/fusionflare.rs` | onBasePower |
| `moves/grassknot.rs`, `moves/lowkick.rs`, `moves/heatcrash.rs`, `moves/heavyslam.rs` | basePowerCallback, onTryHit |
| `moves/haze.rs` | onHitField |
| `moves/hyperspacefury.rs` | onTry |
| `moves/lastrespects.rs`, `moves/ragefist.rs`, `moves/stompingtantrum.rs` | basePowerCallback |
| `moves/photongeyser.rs` | onModifyMove |
| `moves/shellsidearm.rs` | onPrepareHit, onModifyMove (0-1 PRNG draw), onHit, onAfterSubDamage |
| `conditions/glaiverush.rs` | onStart, onAccuracy, onSourceModifyDamage, onBeforeMove |
| `conditions/roost.rs` | onStart, onType |
| `moves/movecallbacks/mod.rs` | private helpers (no effect file; not discovered by build.rs) |
| `moves/movecallbacks/tests.rs`, `scenarios.txt` | tests, included once from `acrobatics.rs` |

No WAIVERS anywhere (every reached site has a real handler). `PAYLOAD_WORDS = 0` except Roost (1).
Probes: `tools/probes/move_callbacks/{scenarios,per-effect,log-shapes}.mjs`.

Shared helpers follow the `itemsmodifiers` pattern: each file that needs them declares
`#[path = "movecallbacks/mod.rs"] mod support;` (roost.rs: `../moves/movecallbacks/mod.rs`). The helper module holds
only pure functions (type-name const lookup, the Grass Knot/Low Kick and Heavy Slam/Heat Crash weight tables, the
HP-scaled base power, the half-HP damage callbacks).

## Per-effect notes (tricky semantics)

Argument layouts used (from the call sites, all checked against `battle-actions.ts`):
BasePower event = `[basePower, attacker, defender, move]` (the move's own `onBasePower(basePower, source, target, move)`
therefore reads attacker at index 1 and defender at 2); basePowerCallback/damageCallback are direct calls with
`[pokemon, target, move]` / `[pokemon, target]`; `ModifyMove` = `[move, pokemon, target]`;
`AfterSubDamage` = `[damage, target, source, move]` (raised by Substitute, data/moves.ts:18368).

* **Variable base power** (`acrobatics`, `stompingtantrum`, `avalanche`, `ragefist`, `lastrespects`, the HP-scaled
  trio, the four weight moves) returns the unrounded JS number; the core floors/clamps. `move.basePower` is the live
  ActiveMove value. HP-scaled power keeps JS evaluation order `(basePower * hp) / maxhp` in f64.
* **Avalanche** uses `Pokemon.attacks[target].flags & 4` (the compact attack history's "positive damage this turn" bit,
  set by `record_attack(damage > 0)` and cleared by `prune_attacks` at end of turn), equivalent to
  `attackedBy.some(p => p.source === target && p.damage > 0 && p.thisTurn)`.
* **Stomping Tantrum** is a strict `moveLastTurnResult === false` (`ResultFlag::False`, not Null/Undefined).
* **Heat Crash / Heavy Slam** keep call order `target.getWeight()` then `pokemon.getWeight()` (each runs ModifyWeight);
  comparisons keep the JS operand form `pokemonWeight >= targetWeight * k`.
* **Grass Knot / Low Kick / Heat Crash / Heavy Slam `onTryHit`**: the only branch is `target.volatiles['dynamax']`.
  Unreachable in this format: `side.ts:264` sets `dynamaxUsed = gen !== 8` so `canDynamaxNow()` is false, and the dex
  has no Dynamax condition. The sites are reached (every use of these moves), so they are real handlers returning
  `undefined`, not waivers (a reached waiver would panic).
* **Gravity** (Grav Apple): no `gravity` condition exists in the generated dex (the move is outside the scope), so the
  pseudo-weather lookup is a compile-time `optional_id` that is `NONE` and the handler is a no-op; it starts working
  unchanged if Gravity is ever added.
* **Belly Drum / Clangorous Soul**: `directDamage(maxhp/2 | maxhp*33/100)` with target/source/effect defaulted from the
  running event (`Attribution::DEFAULT`, `target = None`); thresholds are compared in f64 (`hp <= maxhp/2`,
  `hp <= maxhp*33/100`). Clangorous Soul `onTryHit` runs `boost(move.boosts)` (null on failure) and then
  `delete move.boosts`, expressed as clearing `ActiveMove.effects.boosts` to the empty `OrderedBoosts`
  (the primary-boost step tests `boosts.len > 0`). Verified under Contrary (`-setboost atk -6`, `-unboost ... atk 0`).
* **Double Shock**: `onTryMove` returns null after `-fail ... move: Double Shock` + `[still]`; `self.onHit` maps
  `getTypes(true)` Electric -> `???`, calls `setType(types, enforce=false)` (refused when Terastallized) and logs
  `-start|typechange|<getTypes() joined with '/'>|[from] move: Double Shock` built from stack `LogArg::Parts`.
* **Fickle Beam**: exactly one `randomChance(3, 10)` per BasePower event (per target and per hit); on success
  `attrLastMove('[anim] Fickle Beam All Out')`, `-activate`, `chainModify(2)`.
* **Shell Side Arm**: `onModifyMove` order of `getStat` calls is atk, spa, def, spd (all `unmodified`); the four-floor
  formula is ported exactly; `randomChance(1, 2)` is drawn only on `physical === special`. Setting Physical also sets
  `flags.contact` on the live move. `onPrepareHit` uses `attrLastMove('[anim] Shell Side Arm <category>')`,
  `onHit`/`onAfterSubDamage` emit `-hint|<category> Shell Side Arm` (not for ally targets).
* **Hyperspace Fury**: `hint`, `[still]` edit and `-fail` order as in the source; the `[forme]` tag only for species
  `Hoopa` exactly (`Hoopa-Unbound` returns undefined).
* **Alluring Voice / Burning Jealousy**: gated on `STATS_RAISED`; confusion/burn are added with
  `Attribution::from_move(source, ActiveMove)`; results ignored, handler returns undefined.
* **Final Gambit**: `faint()` with null source/effect (`Attribution::NONE`) inside the damage callback; returns the
  pre-faint HP.
* **Endeavor**: damageCallback is `targetHP - userHP` (unclamped); the `onTryImmunity` boolean returns a real `Bool`.
* **Haze**: `-clearallboost`, then every `getAllActive()` (non-fainted) mon's boosts zeroed. **Clear Smog**:
  `clearBoosts` then `-clearboost`.
* **Fusion Bolt / Flare** read `BattleState.last_successful_move` (= `lastSuccessfulMoveThisTurn`).
* **Glaive Rush**: `onStart` logs `-singlemove ... [silent]`; `onAccuracy` returns `true` (no accuracy roll is drawn);
  `onSourceModifyDamage` is `chainModify(2)`; `onBeforeMove` (priority 100) removes the volatile.
* **Roost** (`conditions/roost.rs`): `onStart` returns `false` for a Terastallized user (hint only when it still has
  Flying); `onType` snapshots the incoming type list into the payload and returns a *new* filtered `Types` slot
  (the incoming relay aliases `Pokemon.types`, which `getTypes` writes back, so it must not be mutated in place).

## Payload map

`conditions:roost` is the only stateful effect:

| word | meaning |
| --- | --- |
| 0 | `effectState.typeWas`: pre-Roost type list packed as `type0 \| type1 << 8` (TypeId bytes, NONE = 0) |
| present bit 8 (`CUSTOM_START`) | `typeWas` has been assigned |

`actions/mutators/pokemon.rs::transform_into` already reads word 0 of the target's Roost cell (pokemon.ts:1295), so
this layout is a contract with that function; the `roost_transform` scenario checks it end to end.
No other effect keeps state (`glaiverush` `noCopy` is declarative metadata).

## PRNG draws

Only two handlers draw: Fickle Beam (`random(10)` per BasePower event) and Shell Side Arm `onModifyMove`
(`random(2)` on an exact physical/special tie). Everything else draws nothing directly; the nested core mutators and
events they trigger (`boost`, `directDamage`, `addVolatile`, `trySetStatus`, `getWeight`, `runEffectiveness`, Type
events) make their own draws at their own statements.

## Requested core APIs

None. Every handler is written against the existing signatures. No core file was touched.

## Notes for the integrator

* **Ignored tests**: `shellsidearm_substitute` and `clearsmog_substitute` (in `movecallbacks/scenarios.txt`, ignored in
  `movecallbacks/tests.rs`) need `moves:substitute` from another batch. After merging run
  `cargo test -p engine -- --ignored scenario_tests`; Shell Side Arm's `onAfterSubDamage` hint and Clear Smog behind a
  Substitute are only reachable that way (the batch corpus has no Substitute). Everything else passes today.
* The scenario replays (`acrobatics.rs` -> `movecallbacks/tests.rs`) deliberately avoid effects other batches port
  (Protect, Substitute, Knock Off, Prankster, Magician, Lightning Rod, ...) so they run on a tree holding only this
  batch; they stay valid after the merge.
* **Possible Types-relay slot leak (core, low risk)**: `run_inner` does not release a Types relay that a later handler
  replaces; `getTypes` only releases the final and the original slot. Roost returns a new slot per query, so a chain of two
  handlers that both allocate (e.g. an Arceus Multitype `onType` at priority 1 followed by Roost at -1) would leak the
  first slot (the scratch has 8). Not reachable with the scoped moves of Arceus; noted in case the scratch is not reset
  at decision boundaries.
* `const _: () = assert!(PAYLOAD_WORDS <= 4);` trips clippy's `absurd_extreme_comparisons` when `PAYLOAD_WORDS = 0`;
  the line is mandated by EFFECTS.md and appears in every effect file, so it was kept.

## Acceptance results

`cargo build -j 2` clean (no warnings); `cargo test -j 2 -p engine` all green (registry/manifest audit included), plus
26 new tests (23 Showdown scenario replays, 3 table/index tests) and 2 ignored.

End-to-end (`gen-directed.mjs --verify`, then `difftest replay --sim engine`; panics from other batches' unported
hooks would be excluded, but none were reached because these worlds contain only this batch plus declarative content):

| corpus | battles | passed |
| --- | --- | --- |
| `batch:move_callbacks` (seed 1117, count 100) | 100 | 100 |
| `batch:move_callbacks+tera` (seed 1117, count 300) | 300 | 300 |
| `batch:move_callbacks+tera` (seed 777, count 1000) | 1000 | 1000 |
| `effect:moves:<id>+tera` for each of the 38 keys, 60 each (seed 5) | 2280 | 2280 |
| `effect:moves:roost`, `effect:moves:glaiverush` (100 each, no tera) | 200 | 200 |

Coverage (`tools/oracle/coverage.mjs`) on the base corpus reaches 48 of 55 sites; the other 7 (Shell Side Arm
`onAfterSubDamage`, 4 Glaive Rush, 2 Roost) are reached by the single-effect profiles (Glaive Rush/Roost: 4+2 handlers
invoked, 100 battles each) and by the scenarios (`onAfterSubDamage` only in the ignored Substitute scenario).
The corpora's Levitate-heavy world reaches Stomping Tantrum's callback rarely, Roost/Hoopa/Substitute/Terastallize
branches never or rarely; the scenarios (replayed on the real pinned Showdown: exact log lines and PRNG state after
every decision) cover those: Stomping Tantrum doubling after a failed move, Roost (damaged Flying/Steel, pure Flying,
Terastallized Flying, Terastallized non-Flying, Transform copying `typeWas`), Hyperspace Fury for Hoopa-Unbound / Hoopa
/ Mew, Belly Drum (+6, thresholds, Contrary), Clangorous Soul (threshold chain, Contrary), Double Shock (Electric,
Electric/Flying, Fighting/Electric, Terastallized), Glaive Rush auto-hit and double damage, Heavy Metal / Transformed
Ditto weights, Freeze-Dry into Water / Water-Flying / Ground-Water / Tera Water, Endeavor / Final Gambit / Super Fang /
Ruination, Photon Geyser category flips with Swords Dance, Fusion Bolt/Flare in both orders, Last Respects with 1-2
fainted, Haze / Clear Smog, Alluring Voice / Burning Jealousy with and without a boost that turn.
