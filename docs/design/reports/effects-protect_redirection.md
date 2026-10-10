# effects(protect_redirection): Protect family, Stall, Wide Guard, redirection and doubles support

Completes the 27 files / 50 function sites of the `protect_redirection` batch against Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a` (`data/moves.ts`, `data/abilities.ts`,
`data/conditions.ts`). Every site is in `HOOKS`; there are no waivers and no hook is a runtime
no-op that the TypeScript would not also skip. Built on `76a95a3`; also merged and re-run against the
newer `engine` head (`f4d334c`, see Verification). No core file was edited and no core API change is
requested.

## Files

| Family | Files (function sites) |
|---|---|
| moves | `protect` (onPrepareHit, onHit), `spikyshield` (2), `banefulbunker` (2), `burningbulwark` (2), `wideguard` (onTry, onHitSide), `followme` (onTry), `ragepowder` (onTry), `helpinghand` (onTryHit), `pollenpuff` (onTryHit, onTryMove, onHit) |
| conditions | `protect` (onStart, onTryHit), `spikyshield` (onStart, onTryHit, onHit), `banefulbunker` (3), `burningbulwark` (3), `wideguard` (onSideStart, onTryHit), `followme` (onStart, onFoeRedirectTarget), `ragepowder` (2), `helpinghand` (onStart, onRestart, onBasePower), `stall` (onStart, onStallMove, onRestart) |
| abilities | `armortail` (onFoeTryMove), `queenlymajesty` (onFoeTryMove), `friendguard` (onAnyModifyDamage), `lightningrod` (onTryHit, onAnyRedirectTarget), `stormdrain` (same), `powerspot` (onAllyBasePower), `steelyspirit` (onAllyBasePower), `propellertail` (onModifyMove), `stalwart` (onModifyMove) |

Shared code: `conditions/protectredirection/mod.rs`, hosted by `conditions/protect.rs`
(`#[path = "protectredirection/mod.rs"] pub(super) mod support`); sibling files import it as
`crate::effects::registry::conditions_protect::support`. It holds only what several files share: the
four-way shield body (`shield_try_hit`/`shield_start`/`stall_prepare_hit`/`stall_hit`), the
Lightning Rod / Storm Drain pair, the Armor Tail / Queenly Majesty pair, the Propeller Tail / Stalwart
pair, and the small helpers (`smart_target`, `clear_smart_target`, `is_sky_dropped`,
`reset_lockedmove`, `state_target`). Handlers that exist once (Follow Me, Rage Powder, Helping Hand,
Pollen Puff, Stall, Wide Guard, Friend Guard, Power Spot, Steely Spirit) live in their own files next to
the TS file:line. No allocation, no strings, no direct PRNG draws except Stall's.

## Per-effect notes (TS file:line, relay values, draws)

Argument layouts (checked against `event/dispatch.rs set_args` and the core call sites):
`[target, source, effect]` without relay; `[relay, target, source, effect]` with relay. Careful with
direction: `ModifyDamage`/`BasePower` fire as `runEvent(ev, attacker, defender, move, value)`, so Friend
Guard / Power Spot read the attacker at arg 1 and the defender at arg 2; `RedirectTarget` fires as
`priorityEvent('RedirectTarget', user, user, move, target)`, so the redirect handlers read the current
target at arg 0 (relay), the user at arg 1 and the move at arg 3; `TryMove` fires as
`runEvent('TryMove', mover, moveTarget, move)`, so Armor Tail / Queenly Majesty's `target` is the mover
(arg 0) and `source` the move's target (arg 1).

* **Protect / Spiky Shield / Baneful Bunker / Burning Bulwark moves** (moves.ts 996-1001, 2031-2036,
  13972-13977, 17543-17548). `onPrepareHit`: `!!this.queue.willAct() && this.runEvent('StallMove', pokemon)`.
  A false `willAct()` returns `Bool(false)` (the move fails with `-fail`); otherwise the StallMove relay
  (`Bool(true)` when no stall volatile exists, else Stall's result) is returned as is. `onHit`:
  `pokemon.addVolatile('stall')` (source `null`, defaulted from the event) and returns undefined.
* **Shield conditions** (moves.ts 1002-1032, 2037-2067, 13978-13999, 17549-17579), `onTryHitPriority` 3.
  Order: `checkMoveBypassesProtect` (Burning Bulwark alone passes `blockStatus=false`, so status moves are
  not blocked by it), then `smartTarget` is cleared *or* `-activate|target|move: Protect` is logged, then
  the Outrage-counter reset, then the contact effect, then `NOT_FAIL`. Protect's `onStart` logs
  `'Protect'`, the other three `'move: Protect'`. Spiky Shield:
  `damage(source.baseMaxhp / 8, source, target)` (the attacker is damaged, the holder is the source, the
  effect defaults to the shield condition). Baneful Bunker / Burning Bulwark:
  `source.trySetStatus('psn'|'brn', target, this.dex.getActiveMove('<move>'))`; the fresh ActiveMove is
  acquired with `get_active_move(MoveInput::Dex(..))` and released right after the status attempt (the
  same pattern as the hazards batch). The three conditions' `onHit` are guarded by
  `move.isZOrMaxPowered`, which cannot be true in this format (no Z/Max moves, the core's ActiveMove has
  no such property); the sites are implemented as the short-circuit result (undefined, no calls).
* **Outrage-counter reset** (`reset_lockedmove`): `source.getVolatile('lockedmove')` and
  `source.volatiles['lockedmove'].duration === 2` then `delete source.volatiles['lockedmove']`. `delete`
  is a raw map removal (no End event, no linked cleanup), so it uses the core's `erase_volatile`, not
  `remove_volatile`. No scoped move creates `lockedmove` (06-scope.md:17), so it is unreachable but
  ported.
* **Stall** (conditions.ts 437-460; payload below). `onStallMove`: `counter = effectState.counter || 1`,
  **one** `randomChance(1, counter)` per StallMove event (a counter of 1 still draws), the volatile is
  deleted with the same raw `delete` when the roll fails, returns the boolean. `onRestart`:
  `counter *= 3` only while `counter < 729` (`counterMax`), `duration = 2`. `onStart`: `counter = 3`.
  Verified draw for draw by `vectors_stall_move_draws_one_random_chance_per_event`.
* **Wide Guard** (moves.ts 20818-20846). Move `onTry`: `Bool(queue.willAct() != null)` (a real boolean,
  no StallMove event); `onHitSide(side, source)`: `source.addVolatile('stall')` (arg 1 is the user).
  Condition `onSideStart(target, source)` logs `-singleturn|source|Wide Guard` (the user; no `move: `
  prefix); `onTryHit` ignores everything but `allAdjacent`/`allAdjacentFoes`, then bypass check, then
  `-activate|target|move: Wide Guard`, the lockedmove reset and `NOT_FAIL` (priority 4).
* **Follow Me / Rage Powder** (moves.ts 6049-6068, 14608-14626). Moves: `onTry` returns
  `Bool(activePerHalf > 1)`; `activePerHalf` is the doubles constant 2. Conditions: `onFoeRedirectTarget`
  returns `Relay::Pokemon(holder)` or undefined; Rage Powder checks `isSkyDropped()` first, then
  `source.runStatusImmunity('powder')` (silent; Grass types and the Immunity event, e.g. Overcoat),
  then `validTarget(holder, user, move.target)`, in that order; both clear `smartTarget` only when it was
  set. Follow Me's `[zeffect]` branch needs `effect.id === 'zpower'`, which has no dex row here, so it
  can never match (`optional_id`, NONE never matches).
* **Helping Hand** (moves.ts 8583-8600; payload below). Move `onTryHit`: `false` when the target is
  neither `newlySwitched` nor has a queued move. Condition: `onStart` multiplier 1.5, `onRestart`
  `*= 1.5`, both log `-singleturn|target|Helping Hand|[of] source`; `onBasePower` (priority 10) is
  `chainModify(multiplier)` returning undefined (1.5 -> 6144, 2.25 -> 9216, 3.375 -> 13824).
* **Pollen Puff** (moves.ts 13571-13590). `onTryHit` against an ally: `basePower = 0`, `infiltrates`.
  `onTryMove(source, target, move)`: ally target with Heal Block -> `attrLastMove('[still]')`,
  `cant|source|move: Heal Block|move`, `false`. `onHit`: ally target -> `heal(floor(baseMaxhp * 0.5))`
  with target/source/effect all defaulted from the Hit event; a falsy heal (full HP) returns
  `NOT_FAIL`, so the move neither heals nor reports failure (no `-fail`, no `-heal`). Note: while the
  user has Heal Block, Showdown's request already changes Pollen Puff's target kind to `adjacentFoe`,
  so the `cant` line is only reached when Heal Block lands after the choice (the scenario uses a
  faster Psychic Noise).
* **Lightning Rod / Storm Drain** (abilities.ts 2344-2368, 4637-4661). `onTryHit`: another Pokemon's
  Electric/Water move -> `this.boost({ spa: 1 })` with target/source/effect all defaulted (`None` +
  `Attribution::DEFAULT`), `-immune|target|[from] ability: Name` when the boost relay is falsy
  (including the capped `+6` case), returns `Relay::Null`. `onAnyRedirectTarget` args
  `[current target, user, user, move]`: type test, `pledgecombo` (no scoped move has the flag, so only
  the type test remains), `randomNormal`/`adjacentFoe` read as `normal` (`adjacentFoe` is not a scoped
  target kind), `validTarget(holder, user, kind)`, clears `smartTarget`, logs
  `-activate|holder|ability: Name` unless the holder already was the target, returns the holder.
* **Armor Tail / Queenly Majesty** (abilities.ts 216-228, 3718-3730). `foeSide` and `all` moves (except
  `perishsong`/`flowershield`/`rototiller`, none of which exists in the scoped dex, so `optional_id`
  yields NONE) return undefined; otherwise `(source.isAlly(holder) || move.target === 'all') &&
  move.priority > 0.1` -> `[still]`, `cant|holder|ability: Name|move|[of] mover`, `false`. The move
  priority is the live ActiveMove value (Prankster/Gale Wings edits included, battle.ts:2653). Only the
  first listener to return false logs, so a team with both abilities prints the faster holder's line.
* **Friend Guard, Power Spot, Steely Spirit** (abilities.ts 1534, 3413, 4590): `chainModify(0.75)`
  (3072), `chainModify([5325, 4096])`, `chainModify(1.5)`; Power Spot excludes the holder's own moves
  (`attacker !== effectState.target`), Steely Spirit does not (the holder's own Steel moves are boosted),
  and it reads the live move type.
* **Propeller Tail / Stalwart** (abilities.ts 3487, 4504): `move.tracksTarget = move.target !== 'scripted'`.
  `scripted` is not a scoped target kind, so the assignment is always true (sets
  `move_runtime::TRACKS_TARGET`). `get_target` independently special-cases both abilities (the TS
  comment: ModifyMove runs after getTarget), which the core already does.

## Payload maps

* `conditions:stall` (`PAYLOAD_WORDS = 1`): `word0` = `effectState.counter` (u32: 3, 9, 27, 81, 243,
  729); custom presence bit 8 (`COUNTER_PRESENT`) marks the property as assigned. `duration` uses the
  common field (`onRestart` rewrites it to 2).
* `conditions:helpinghand` (`PAYLOAD_WORDS = 2`): `word0`/`word1` = `effectState.multiplier` as f64 bits
  (low/high half); custom presence bit 8 (`MULTIPLIER_PRESENT`). Unassigned reads as NaN like
  `undefined * 1.5`.
* Every other file is stateless (`PAYLOAD_WORDS = 0`).

## Waivers

None. No function site is waived.

## Core APIs used / requests

No signature change is requested and no `todo!("needs ...")` was left. Notes for the integrator:

* `Battle::erase_volatile` (`pub(crate)`) is used for the two raw `delete pokemon.volatiles[..]`
  statements (Stall's `onStallMove` and the lockedmove reset). Please keep it crate-visible.
* `is_sky_dropped` and `smart_target`/`clear_smart_target` are small local helpers in
  `protectredirection/mod.rs` because the core versions are private (`disable_smart_target` is
  `pub(super)`, and the Sky Drop test only exists inside `is_semi_invulnerable`). Sky Drop is out of
  scope, so the query is by key and can never succeed; if the core later exposes `is_sky_dropped`, the
  helper can be replaced.
* `check_move_bypasses_protect` (core) omits the `isZOrMaxPowered` -> `bypassProtect` branch for the
  same reason the three shield `onHit` sites are no-ops.
* `conditions:healblock` is not part of this batch. The Heal Block scenario
  (`pollen_puff_heal_block`) is `#[ignore = "needs conditions:healblock"]` until that condition lands;
  every other test runs.

## Verification

Commands (from the worktree root):

```
node tools/oracle/gen-directed.mjs --profile batch:protect_redirection --count 100 --threads 2 --out /tmp/pr.jsonl.gz --verify
cd crates/difftest && cargo build --release -j 2 && ./target/release/difftest replay /tmp/pr.jsonl.gz --sim engine --jobs 2 --top 20
```

* `--verify`: 100 fixtures, 27/27 focus effects placed, 0 failures.
* This branch alone (base `76a95a3`): 100 battles, 61 pass, 39 fail, **all 39** are the other batch's
  `unimplemented effect hook abilities:naturalcure onCheckShow` (healing_residual). Excluding other-batch
  panics the pass rate is **100% (61/61)**. Per-effect profiles
  (`effect:KIND:ID`, 18 effects x 300 battles = 5,400 battles, seed 31337): every failure is the same
  naturalcure panic, zero non-panic divergences.
* With the other available batches merged into a scratch branch (healing_residual, rules_status,
  consumables, formes_tera) no battle panics: **100/100, 1,500/1,500 (seed 777) and 6,000/6,000
  (seed 4242)** battles pass, 3.55 M log lines and all intermediate PRNG states matched.
* Re-run against the newer `engine` head (`f4d334c`) merged with this branch: 100/100 and 6,000/6,000
  pass again, `cargo test` green (222 passed, 72 ignored).
* Hook coverage of the 1,500-battle run (difftest built with `--features trace-hooks`, instrumentation
  not committed): 49 of the 50 sites were reached. The only unreached site is
  `conditions:helpinghand onRestart` (it needs two Helping Hands on one target in a turn; there is no
  in-scope way to do that), which is covered by the direct vector test below.

## Tests (`conditions/protectredirection/tests.rs`)

* 13 recorded battles (`scenarios.txt`, generated by `tools/probes/protect_redirection/scenarios.mjs`
  from the pinned Showdown): `protect_stall_chain` (four Protect users; slowest fails with an empty
  queue; consecutive Protects draw `randomChance`), `shields_contact_and_status`, `wide_guard`,
  `follow_me_rage_powder`, `redirection_ignored_by_tracking` (Stalwart/Propeller Tail),
  `rage_powder_overcoat`, `lightning_rod_storm_drain` (absorb, redirect and the capped `-immune`),
  `helping_hand_pollen_puff` (stacked boost, ally heal, full-HP NOT_FAIL), `pollen_puff_heal_block`
  (ignored: needs healblock), `ally_damage_modifiers` (Friend Guard, Power Spot, Steely Spirit),
  `armor_tail`, `queenly_majesty`, `dragon_darts_into_protect` (silent smartTarget clear). Each replays
  line by line and PRNG state by PRNG state through the real core; a `markers` test pins the branch each
  scenario exists for. Regenerate with `node tools/probes/protect_redirection/scenarios.mjs`.
* Vectors from `tools/probes/protect_redirection/restart-vectors.mjs` (real handlers): Helping Hand
  restart stacking (1.5, 2.25, 3.375 and the repeated log line), Stall counter 3 -> 729 with the cap and
  `duration = 2`, and a 12-event StallMove sequence (counter, result, PRNG seed after each draw,
  volatile deleted on failure).
* Helper tests (`type_named`, the `spa` boost object, the absent all-target exceptions).
* The batch's own mandated `#[ignore = "needs core"]` marker is not used: the whole core exists at
  `76a95a3`, so these tests run in `cargo test`.

## Checks for the integrator

* The `engine` head still has no `conditions:healblock`; once it lands, un-ignore
  `pollen_puff_heal_block` (it was recorded from the pinned Showdown and only needs that condition).
* Clippy's deny-by-default `absurd_extreme_comparisons` flags the contract's
  `const _: () = assert!(PAYLOAD_WORDS <= 4)` for zero-payload files; the same happens in the other
  batches' files, so it was kept as EFFECTS.md prescribes.
* Instrumented hook-coverage patch (not in the tree): `features = ["trace-hooks"]` on difftest's engine
  dependency plus printing `engine::effects::reached_hooks()` after the replay.
