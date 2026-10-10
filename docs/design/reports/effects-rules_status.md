# Effect batch `rules_status`

Base `76a95a3`, then merged with `c3f85db` (the lead moved the template base to it while this batch was
being validated; the merge is conflict free and brings in `healing_residual`, whose Natural Cure
`onCheckShow` every voluntary switch of a statused Pokemon needs). Oracle: Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a`, read only.

## Scope of this pass

22 keys in `EFFECT-BATCHES.json`. 17 of them (`conditions:{brn,psn,tox,par,slp,frz,confusion,flinch,sleepclausemod}`
and the eight `rules:*` files) already existed at `76a95a3` together with `moves:struggle`; they were left
alone and the end-to-end replay showed no divergence in them (below). The missing keys were
`moves:{direclaw,rest,triattack,yawn}` and `conditions:yawn`; they come from the interrupted draft on
`wip/main-checkout-0919` (`git checkout wip/main-checkout-0919 -- <path>`), were re-reviewed against the
pinned TypeScript and rewritten in the contract style (`match hook`, named `HookId` constants, TS
file:line and PRNG on every handler). `yawn.rs` did not compile (no `ImmunityId::Sleep`), see below. Every key
of the batch now has a file; `WAIVERS = &[]` everywhere; no `todo!()`; no core file was edited.

| File | Sites | TS | Notes |
|---|---|---|---|
| `moves/direclaw.rs` | `secondary.onHit`, `secondaries.0.onHit` | moves.ts:3629-3647 | one `sample` = `random(3)` before the attempt (psn / par / slp), consumed even when the status cannot land; `trySetStatus(status, source)` |
| `moves/triattack.rs` | same two sites | moves.ts:19845-19864 | same with brn / par / frz |
| `moves/rest.rs` | `onTry`, `onHit` | moves.ts:14957-14994 | see below |
| `moves/yawn.rs` | `onTryHit` | moves.ts:21131-21162 | `target.status \|\| !runStatusImmunity('slp')` -> `Bool(false)`, else `Undefined` |
| `conditions/yawn.rs` | `onStart`, `onEnd` (`onResidual` is the ordering constant `onResidualOrder: 23`) | moves.ts:21146-21157 | `-start ... [of] source`; `-end ... [silent]` then `trySetStatus('slp', effectState.source)` |

All five have `PAYLOAD_WORDS = 0`. Rest writes into the *Sleep* cell (`status_state`) using the layout of
`conditions/slp.rs`: word0 = `startTime` (present bit 8), word1 = `time` (present bit 9, signed). The Yawn
volatile has no private state; the Yawn user is the cell's common `source` field (set by `addVolatile`).

### Yawn: sleep immunity (`ImmunityId::Sleep` does not exist)

`runStatusImmunity('slp')` (sim/pokemon.ts:2269-2289) is `fainted` check, `Dex.getImmunity('slp', pokemon)`
(a `getTypes()` / Type event; 'slp' has no type-chart row so it can never fail) and then
`runEvent('Immunity', pokemon, null, null, 'slp')`. The generated `ImmunityId` only names immunities that
some pinned callback compares against (`brn frz hail par powder prankster psn sandstorm tox trapped`);
no `onImmunity` handler tests `'slp'`, so there is no member and `run_status_immunity` cannot express it.
The core already has the exact sequence for generic condition ids, because `setStatus('slp')` needs it too:
`Battle::condition_immunity(mon, EffectId)` (`actions/mutators/status.rs`, `pub(crate)`, used by
`install_status`). `moves/yawn.rs` calls `b.condition_immunity(target, dex::CONDITION_SLP)`; no core change.
(It passes `Attribution::NONE` to the event; no Immunity handler reads source or effect.)

### Rest

* `onTry(source)` (arg0 is the *user*, `singleEvent('Try', move, null, pokemon, target, move)`): asleep or
  Comatose -> `false` (no line); full HP -> `-fail|user|heal`, `Null`; Insomnia then Vital Spirit -> `-fail|user|[from] ability: X|[of] user`, `Null`.
  The heal check precedes the ability checks (a full-HP Hypno gets the `heal` message).
* `onHit(target, source, move)`: `setStatus('slp', source, move)`; a falsy result is returned unchanged
  (`Null` from a handler stays `Null`); then `statusState.time = startTime = 3` and `heal(maxhp)` with the event
  defaults (`HealEffect::Context`). The Sleep `onStart` has already drawn `random(2,5)` by then, so the PRNG
  is still advanced once (verified in the scenarios: seeds match after every Rest). Resulting log:
  `-status|..|slp|[from] move: Rest`, `-heal|..|[silent]` (the core prints it), two `cant|slp` turns, wake on the third
  (Early Bird: first decrement 3 -> 1 then wake, covered).

### Dire Claw / Tri Attack: the source effect is the secondary object (core divergence, worked around)

Inside `singleEvent('Hit', secondary, {}, target, source, move)` the running `battle.effect` is the plain
**secondary object** (sim/battle.ts:635 `this.effect = effect`), not the move, and `trySetStatus(status, source)`
defaults its `sourceEffect` to `battle.effect`. A plain object has no `id`, `effectType` or `status`, so
the Sleep `onStart` (`sourceEffect.effectType === 'Move'`) prints a bare `-status|target|slp`. The core's
`hit_single_event` (`actions/moves/hit.rs:109-130`) uses `EffectRef::ActiveMove(handle)` for both the
effect and the source effect of secondary `onHit`, so `current_effect` is the live move and the draft's
`Attribution::from_move(source, EffectRef::None)` printed `-status|..|slp|[from] move: Dire Claw`
(found by the directed replay: 7 of the first 100 battles). Fix in this batch's files only: both
handlers pass `EffectRef::Dex(EffectId::NONE)` explicitly, the core's existing truthy "empty effect"
view (`mutation_attribution`'s no-frame default), which behaves like an object without id / effectType /
status for every SetStatus consumer (Safeguard, Leaf Guard, Purifying Salt, Insomnia, terrain `onSetStatus`, ...).

Core request (not applied): make `current_effect` / the `effect` and `source_effect` of
`hit_single_event` for `HitEffect::Secondary` (and `self`) the empty/secondary view instead of the live
move (the `sourceEffect` argument stays the move: `singleEvent('Hit', moveData, {}, target, source, move)`).
Other secondary `onHit` handlers read `battle.effect` implicitly through defaults in the same way:
`allure voice` / `throat chop` (`addVolatile` default sourceEffect), `burning jealousy`, `spirit shackle`
(passes `move` explicitly). Only the Sleep text is visible today, so no other repro exists; once the
core is fixed the explicit `Dex(NONE)` in `direclaw.rs` / `triattack.rs` is redundant but harmless.

## Validation

* `cargo build -j 2`, `cargo test -j 2` (registry manifest test and everything else) pass.
* Directed replay, `batch:rules_status`:
  * 100 battles seed 1100: **100/100** (2716/2716 steps, 51329 log lines)
  * 600 battles seed 424242: **600/600** (16255/16255 steps, 302210 log lines)
  * `effect:moves:{yawn,rest,triattack,direclaw}`, 150 battles each: all pass.
  (`effect:conditions:yawn` is not placeable as a focus effect; it is reached through Yawn.)
  The first 100-battle run on `76a95a3` showed 7 Dire Claw log divergences (fixed above) and 70-74 battles
  masked by the `abilities:naturalcure onCheckShow` panic (healing_residual batch); with that file applied on top
  of `76a95a3`, and later with `c3f85db` merged, no divergence remains. Pass rate excluding other-batch
  panics: 100%. The 100-battle corpus has 155 Dire Claw, 167 Rest, 173 Tri Attack, 226 Yawn uses,
  111 Yawn expiries, 138 Rest sleeps, 84 Sleep Clause activations, 475 flinch `cant`, 552 sleep `cant`.
* `data/fixtures/slice1-200.jsonl.gz` 200/200.
* Scenario replays (`tools/probes/rules_status/scenarios.mjs` plays directed battles on the real
  simulator through the oracle `Session` and writes
  `crates/engine/src/effects/conditions/rulesstatus/scenarios.txt`; the test module
  `rulesstatus/tests.rs` is declared from `conditions/yawn.rs`, so the tests are
  `effects::registry::conditions_yawn::scenario_tests::*`). Four scenarios, compared per decision
  boundary on exact log lines and PRNG state:
  * `yawn_sleep_clause`: Yawn on both foes, expiry (sleep / Sleep Clause refusal), Yawn into asleep,
    statused and already-yawned targets.
  * `yawn_blockers`: Electric Terrain (`onTryAddVolatile`), Insomnia and Vital Spirit (`onTryAddVolatile`),
    Comatose (expiry sleep refused silently), switching out with Yawn pending.
  * `rest_branches`: full-HP fail, Insomnia / Vital Spirit / Comatose failures, Early Bird, a burn replaced
    by Rest, repeated Rest, fixed two sleeping turns.
  * `status_lotteries`: Dire Claw and Tri Attack with Serene Grace against Poison / Fire / Ice / Electric /
    Normal / Ghost / Shield Dust targets (par, psn, slp, frz, brn, immune samples) and a Sleep Clause refusal.
  Scenarios that avoid Substitute and Protect (unported at the time of writing, other batches).

## For the integrator

* If `abilities:naturalcure` is ever missing again the `rest_and_lottery` scenario test panics on the first
  voluntary switch of a statused Pokemon (sim/battle.ts:2767). The Yawn scenarios do not depend on it.
* `Substitute` / `Protect` are unported on this base; Yawn vs Substitute / Protect was deliberately kept out of
  the scenarios but the Yawn `onTryHit` ordering is unchanged by them.
* The only core finding is the secondary-object effect above.
