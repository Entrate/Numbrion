# effects(charge_priority_multihit): charge/recharge, action priority and multihit support

Base `76a95a3` (whole core implemented), branch `worktree-agent-a384c9c746b1889f6`. Oracle: Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a` (`data/{moves,abilities,items,conditions}.ts`, `sim/*.ts`), read only.
Every function site of the batch (41) is in `HOOKS`; there are no waivers and no site is left to the central
dispatcher. No core module, `mod.rs`, `registry.rs`, `build.rs`, generated dex or other batch file was edited.

## Files (24 effect files + one private helper dir)

| Family | Files |
|---|---|
| moves (12) | electroshot fakeout firstimpression highjumpkick meteorbeam phantomforce quash shadowforce solarbeam suckerpunch thunderclap tripleaxel |
| abilities (6) | galewings myceliummight prankster skilllink triage truant |
| items (2) | loadeddice powerherb |
| conditions (4) | charge lockedmove mustrecharge twoturnmove |

Keys without a file (no function sites, nothing to implement): `conditions:phantomforce` and `conditions:shadowforce`
(`condition: { duration: 2, onInvulnerability: false }`, i.e. data plus a manifest constant the central dispatcher
returns). `mustrecharge.onLockMove: 'recharge'` is likewise a constant; the core turns it into the synthetic Recharge move.

Helper dir `crates/engine/src/effects/moves/chargepriority/` (`mod.rs`, `tests.rs`, `scenarios.txt`, `scenarios_full.txt`)
is declared once from `moves/electroshot.rs` (`#[path = "chargepriority/mod.rs"] pub(super) mod cp;`) and imported by the
other files as `crate::effects::registry::moves_electroshot::cp`. It holds the shared charge TryMove body, the shared Sucker
Punch / Thunderclap body, compile-time `TYPE_ELECTRIC`/`TYPE_FLYING`, the `{ spa: 1 }` boost object and `live_move_arg`
(`move?.` access). Probe: `tools/probes/charge_priority_multihit/scenarios.mjs`.

## Per-effect notes (TS file:line is also in every handler comment; draws are listed there)

No handler draws on its own except `lockedmove` Start (`random(2, 4)`, exactly once) and the unreachable
fainted-defender branch of `twoturnmove` Start (`sample`, one draw even for a singleton). All other draws are nested
core ones (boost events, `addVolatile` Start, `useItem`, speed-sort ties).

* **Charge moves** (Electro Shot moves.ts:4639, Meteor Beam 11748, Phantom Force 13317, Shadow Force 16083, Solar Beam
  17233). One shared body, `cp::charge_try_move(boost_spa, skip_weather)`, kept in source order: `removeVolatile(move.id)`
  (second turn -> `Undefined`, the move fires) -> `-prepare` -> optional `boost({spa: 1}, attacker, attacker, move)` ->
  optional weather short-cut (`[still]` edit of the move line, `-anim`, `Undefined`) -> `prepare_charge_move`
  (= `runEvent('ChargeMove', attacker, defender, move)`, falsy means Power Herb ate the charge turn -> `Undefined`) ->
  `addVolatile('twoturnmove', defender)` -> `Null`. `removeVolatile(move.id)` goes through `condition_id`, which maps the
  move id to its bare condition record (`CONDITION_ELECTROSHOT`, `..._SOLARBEAM`, ...). Weather lists are cut to the scoped
  ids: rain for Electro Shot (Primordial Sea is outside), sun for Solar Beam (Desolate Land outside; the `message: true`
  argument only matters for Mega Sol, outside). Solar Beam `onBasePower` halves in rain/sand/snow (Hail, Primordial Sea
  outside) with `chainModify(0.5)` and returns `Undefined`.
* **Sucker Punch / Thunderclap** (hard). `queue.willMove(target)` only ever returns a `choice === 'move'` action, so
  `move` is null iff no move is queued; the queued ActiveMove's category is still the dex one (ModifyMove has not run).
  The synthetic Recharge action is not a real move (category not `Status`); the target then necessarily has `mustrecharge`,
  which fails the move anyway. `mefirst` is outside the scope (`optional_id` -> `NONE`, never equal to a real id).
  Returns `false`/`Undefined`.
* **Fake Out / First Impression**: `source.activeMoveActions > 1` -> `hint(...)` then `false`.
* **High Jump Kick**: `onMoveFail(target, source, move)` receives `(target, source, move)`; damage is
  `baseMaxhp / 2` (kept fractional, `baseMaxhp == maxhp` here) with source = target = the user and the effect
  `conditions.get('High Jump Kick')`, which is the bare nonexistent condition `highjumpkick` (log `[from] highjumpkick`):
  `EffectRef::Dex(CONDITION_HIGHJUMPKICK)`.
* **Quash**: `queue.willMove(target)` missing -> `false`; else `action.order = 201` in place, then `-activate|target|move: Quash`.
  (`activePerHalf === 1` is the singles check and is false in doubles.)
* **Triple Axel**: direct `basePowerCallback(pokemon, target, move)` -> `20 * move.hit`.
* **Priority abilities**: all read `ModifyPriority` args `(priority, pokemon, target, move)` -> relay number is arg 0, move arg 3.
  Prankster sets `PRANKSTER_BOOSTED` on the ActiveMove (the lifecycle copies it into the queued action) and returns
  `priority + 1`; Gale Wings needs the move's *current* type Flying and `hp == maxhp`; Triage tests the `heal` flag
  (`FLAG_HEAL` on `ActiveMove.flags`) and adds 3. Mycelium Might: `FractionalPriority` (manifest priority -1) returns `-0.1`
  for Status moves (event args `(0, pokemon, null, move)`), `ModifyMove` sets `IGNORE_ABILITY`.
* **Skill Link / Loaded Dice**: multihit model is `MULTIHIT_PRESENT | MULTIHIT_RANGE` with `multihit = [min, max]` for a
  range and `[n, n]` without `RANGE` for a fixed count. Skill Link turns a range into the fixed maximum
  (`Array.isArray` guard) and both delete `multiaccuracy` (clear `MULTIACCURACY`). The extra-hit logic of Loaded Dice is core code.
* **Truant**: `activeTurns` truthiness, `moveThisTurnResult !== undefined` is `!= ResultFlag::Undefined` (null/false/true are
  all defined), `queue.willMove`. BeforeMove (priority 9): `removeVolatile('truant')` -> `cant|ability: Truant`, `false`; else
  `addVolatile('truant')` with no source/sourceEffect (event defaults).
* **mustrecharge**: BeforeMove (priority 11) `cant|recharge`, removes `mustrecharge` then `truant`, returns `null`; Start `-mustrecharge`.
* **Power Herb**: `useItem()` without arguments (`Attribution::DEFAULT`), then `[still]` + `-anim`, returns `false`.
* **charge** (Electromorphosis only creates it in scope): Start/Restart share one body; the Electromorphosis/Wind Power
  variant prints `-start|mon|Charge|<activeMove.name>|[from] ability: ...` (the `[from]` tag is rebuilt as the canonical
  ability so it prints `ability: Electromorphosis`, never the condition view), else `-start|mon|Charge`. `onBasePower`
  (priority 9) is `chainModify(2)` for Electric moves; `onMoveAborted`/`onAfterMove` remove it for an Electric move whose
  id is not `charge`; `onEnd` prints `-end|mon|Charge|[silent]`. The Charge move and Wind Power are outside the scope
  (`optional_id` -> `NONE`, guarded so `NONE` never matches an absent effect).
* **twoturnmove** (hard): Start stores `effectState.move`, `addVolatile(effect.id)`, then
  `attacker.volatiles[effect.id].targetLoc = moveTargetLoc`, `[still]`, `runEvent('PrepareHit', attacker, defender, effect)`.
  The `effect.sourceEffect && target !== 'self'` branch (called by another move: re-sample a fainted defender, use
  `getLocOf(defender)`) is ported but unreachable in this scope: no scoped move calls a charge move (no Metronome, Sleep
  Talk, Copycat...; Instruct refuses `charge`-flag moves; Dancer only repeats dance moves).
  End -> `removeVolatile(move)`, LockMove -> the move id (`Relay::Move`), MoveAborted -> `removeVolatile('twoturnmove')`.
* **lockedmove** (hard): Start `trueDuration = random(2, 4)`, `move = effect.id`; Residual (`target.status === 'slp'` ->
  raw `delete` without End; always `trueDuration--`, also on the detached state); Restart (`trueDuration >= 2` ->
  `duration = 2`); AfterMove (`duration === 1` -> `removeVolatile`); End (`trueDuration <= 1` ->
  `addVolatile('confusion')`, whose Start prints `[fatigue]` because the sourceEffect defaults to this condition);
  LockMove -> the move id (Dynamax is outside). **No scoped move creates it** (no Outrage, Thrash, Petal Dance), so it is
  never reached in real random-doubles battles; it is verified synthetically (below).

## Payload maps

| Effect | `PAYLOAD_WORDS` | Layout |
|---|---|---|
| `conditions:twoturnmove` | 1 | `words[0]` = `effectState.move` (EffectId), custom presence bit 8 |
| (cell of the move's own bare volatile, written by twoturnmove Start) | - | `words[0]` = `targetLoc` as an i32 two's-complement, bit 8: the exact encoding `Battle::ch_locked_target_loc` (sim/choices/validation.rs) reads for `volatiles[lockedMoveID]?.targetLoc` |
| `conditions:lockedmove` | 2 | `words[0]` = `trueDuration` (i32, bit 8); `words[1]` = `effectState.move` (bit 9); `effectState.duration` is the common `duration` field |
| everything else | 0 | stateless |

## Core API assumptions (no signature change is requested)

* Used as-is: `add_volatile/remove_volatile/get_volatile`, `boost`, `damage`, `use_item`, `effective_weather`, `foes`,
  `get_loc_of`, `queue_will_move/queue_action/queue_action_mut`, `prepare_charge_move`, `hint`, `chain_modify`, `run_event`,
  `attr_last_move`/`add_move`, `scratch.moves[..]` overlays, `condition_id` (`pub(crate)`).
* **Request (convenience, not blocking):** `delete_volatile_raw` is `pub(super)` inside `actions::moves::hit`.
  `lockedmove` needs the same `delete target.volatiles[id]` (no End) from an effect file, so it carries a 6-line private copy
  (`EffectList::remove` + `EffectArena::release`, which retires a still-pinned cell exactly like the core helper). A
  `pub fn delete_volatile_raw(&mut self, pokemon, id)` would let it be shared.
* Event argument order relied upon (verified by the scenarios): `TryMove`/`Try` single events `(user, target, move)`;
  `MoveFail` `(target, user, move)`; `ModifyPriority`/`FractionalPriority` `(relay, pokemon, target|null, move)`;
  `BeforeMove`/`AfterMove`/`MoveAborted`/`ChargeMove` `(pokemon, target, move)`; `Start` `(target, source, sourceEffect)`;
  direct `BasePowerCallback` `(pokemon, target, move)`.

## Validation

* `cargo build -j 2`, `cargo test -j 2` (registry/manifest test: all 41 sites accounted for) pass.
* **Oracle scenarios** (`chargepriority/scenarios.txt`, from `tools/probes/charge_priority_multihit/scenarios.mjs`): 18
  directed battles with the exact log lines and PRNG state at every decision boundary; `tests.rs::scenarios_match_pinned_showdown`
  replays them and passes. They cover Truant + Giga Impact recharge, Electromorphosis Charge (start, restart, boost,
  consume, abort by flinch), Solar Beam under rain/sun/sand/snow, Power Herb on all five charge moves, Electro Shot in/out
  of rain, Phantom/Shadow Force invulnerability, Solar Beam interrupted by Fake Out, all priority abilities, Quash,
  Sucker Punch/Thunderclap (success, fail vs status, vs switch, vs recharge), Fake Out/First Impression turn gating,
  Skill Link/Loaded Dice/Population Bomb/Triple Axel, High Jump Kick crash, and `lockedmove` (4 seeds covering both
  `random(2,4)` results incl. `[fatigue]`, plus restart and sleep).
  `lockedmove` is exercised by applying the real `pokemon.addVolatile('lockedmove', null, move)` to both engines between turns
  (`pre` records; the volatile locks the user into a harmless status move such as Swords Dance).
* `cargo test -p engine --lib --features trace-hooks -- --ignored scenarios_reach_every_hook --test-threads=1` asserts that
  the scenarios reach **all 41 function sites** (process-wide tracing, so run it alone).
* **Directed corpus** (the acceptance command): `gen-directed.mjs --profile batch:charge_priority_multihit --count 100`
  `--verify` then `difftest replay --sim engine`: 100 battles, **67 pass, 33 fail, every failure is the other-batch panic
  `abilities:naturalcure onCheckShow`** -> 67/67 (100%) of the battles that do not panic in another batch. Two more runs
  (600 battles seed 7777: 378/378; 400 battles seed 4242: 260/260) and the single-effect corpora
  `effect:abilities:electromorphosis` (80/80, exercises all six `charge` handlers: 785 AfterMove calls), `effect:abilities:truant`
  (55/55) and `effect:moves:gigaimpact` (103/103, exercises `mustrecharge`) show the same single failure signature.
  The batch profile alone reaches 27/41 handlers (`charge`, `lockedmove`, `mustrecharge` have no creator in its world);
  the three corpora above and the scenarios cover the rest.
* No divergence involving this batch's effects was found, so no core bug is recorded.

## Integrator checklist

* Re-run `cargo test -p engine --lib -- --ignored scenarios_with_other_batch_moves` after the Protect / Taunt / Knock Off /
  Rest / Roost / Spikes / Tailwind / Helping Hand / Follow Me / Leech Seed / Super Fang callbacks land. `scenarios_full.txt`
  holds the same battles with those real moves in place of the declarative stand-ins `scenarios.txt` uses (the stand-ins keep the
  slot index, so every choice string is valid for both). Regenerate: `SC_KEEP_OTHER_BATCH_MOVES=1 node
  tools/probes/charge_priority_multihit/scenarios.mjs --out crates/engine/src/effects/moves/chargepriority/scenarios_full.txt`.
  It adds Phantom/Shadow Force breaking Protect and Fake Out into Protect.
* `lockedmove` and the `called` branch of `twoturnmove` Start are unreachable in real battles of this format; their
  behavior is pinned by the synthetic scenario / source reading only.
* The scenarios replace Cursed Body (needs `conditions:disable`) by Pressure on the Gengar sets for the same reason.
