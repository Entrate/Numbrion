# Effect batch `disable_lock_trap`

Base `76a95a3` (the worktree started at the older `5d3f0da` and was reset to it as instructed), branch
`worktree-agent-a12f92ee457928654`. Oracle: Showdown `7332b60e22b9e8194bb53549549eba241d73cc9a`
(`data/{moves,abilities,conditions}.ts`, `sim/pokemon.ts`, `sim/battle.ts`), read only. No core file was edited.

## Files

All **58 function sites** of the 19 effects in `EFFECT-BATCHES.json` are ported (`HOOKS` only, **no waivers**;
`complete_batch_manifest_coverage` asserts every site is `HookCoverage::Implemented`).

| Family | Files |
|---|---|
| moves (5) | curse disable noretreat spiritshackle throatchop |
| abilities (3) | arenatrap magnetpull shadowtag |
| conditions (11) | choicelock curse disable encore healblock imprison noretreat partiallytrapped taunt throatchop trapped |

Shared private helpers (`opt_mon_arg`, `stored_move`/`store_move`, `state_source`, `state_source_effect`,
`state_target`, `move_slot_ids`, `disable_matching`, `holds_choice_item`, type ids, flag aliases) live in
`crates/engine/src/effects/conditions/locktrap/mod.rs`. It is declared once, in `conditions/choicelock.rs`
(`#[path = "locktrap/mod.rs"] pub(super) mod support;`), and referenced from the other 18 files as
`crate::effects::registry::conditions_choicelock::support` (same pattern as `reflect.rs`/`airlock.rs`). The batch's
tests are therefore `effects::registry::conditions_choicelock::support::tests::*`.
Other new files: `locktrap/{tests.rs,scenarios.txt}`, `tools/probes/disable_lock_trap/scenarios.mjs`.

## Per-effect source map and tricky semantics

Every dispatch arm carries its TS line range, argument layout and PRNG note. The only direct draw in the batch is
`conditions:partiallytrapped` durationCallback (`random(5, 7)`, skipped by Grip Claw, which is outside the item
scope and kept as a static-key query).

| Effect | Sites | TS |
|---|---|---|
| conditions:choicelock | onStart, onBeforeMove, onDisableMove | conditions.ts:324-363 |
| conditions:trapped / partiallytrapped | trapped: onTrapPokemon, onStart; partiallytrapped: durationCallback, onStart, onResidual, onEnd, onTrapPokemon | conditions.ts:208-221 / 222-258 |
| moves:curse + conditions:curse | onModifyMove, onTryHit, onHit; onStart, onResidual | moves.ts:3266-3306 |
| moves:disable + conditions:disable | onTryHit; onStart, onEnd, onBeforeMove, onDisableMove | moves.ts:3648-3716 |
| conditions:encore | onStart, onOverrideAction, onResidual, onEnd, onDisableMove | moves.ts:4735-4781 |
| conditions:healblock | durationCallback, onStart, onDisableMove, onBeforeMove, onModifyMove, onEnd, onTryHeal, onRestart | moves.ts:8290-8352 |
| conditions:imprison | onStart, onFoeDisableMove, onFoeBeforeMove | moves.ts:9500-9524 |
| moves:noretreat + conditions:noretreat | onTry; onStart, onTrapPokemon | moves.ts:12787-12823 |
| moves:spiritshackle / throatchop (+ conditions:throatchop) | secondary(.N) onHit (both sites share one body); throatchop condition: onStart, onDisableMove, onBeforeMove, onModifyMove, onEnd | moves.ts:17620, 19389 |
| conditions:taunt | onStart, onEnd, onDisableMove, onBeforeMove | moves.ts:18985-19016 |
| abilities arenatrap / magnetpull / shadowtag | onFoeTrapPokemon, onFoeMaybeTrapPokemon | abilities.ts:196, 2517, 4157 |

* **Payload maps.** `PAYLOAD_WORDS = 1` for choicelock, disable and encore: `words[0]` = `effectState.move` as a raw
  `EffectId`, custom present bit 8 (`MOVE_PRESENT`). `partiallytrapped`: `words[0]` = `boundDivisor` (6 with Binding
  Band, else 8), custom present bit 8. Everything else is 0 words. The trapper (`effectState.source`) and the trapping
  move/item (`effectState.sourceEffect`) are read from the shared `EffectCell.source` / `source_effect` token
  (`EffectToken(..).resolve()`), never duplicated in the payload.
* **Choice lock.** onStart panics like TS when no move is active, fails (`return false`, so the volatile is dropped)
  for an empty id or `hasBounced`. The bare `|move|p|Name` line, `[still]`, `|-fail|p|` branch of onBeforeMove is
  reached in practice through Dancer copying a dance move (scenario 1) and is checked byte-for-byte. The disables use
  the stored item as source effect (`disableMove(id, false, effectState.sourceEffect)`).
* **Curse.** `delete move.volatileStatus` / `delete move.onHit` / `move.self = {boosts: {spe:-1, atk:1, def:1}}` are
  overlay edits: `effects.volatile_status = NONE`, bit `i` of `effects.suppressed_hooks` where `i` is the index of
  `HOOK_MOVE_CURSE_ONHIT` in `effects.base.hooks`, and `self_effect = Some(..)` built on
  `actions::moves::EMPTY_EFFECTS` with insertion order spe, atk, def. The Ghost cost is `direct_damage(max_hp/2)`
  with the user as target and source; the target of an ally-aimed Ghost curse becomes `randomNormal` (one draw in the
  core retarget).
* **Disable / Encore / Taunt.** The `duration--` / `duration++` adjustments use `queue_will_move` and, for Disable,
  `scratch.active_pokemon`/`active_move`/`EXTERNAL` (Cursed Body). Encore's onOverrideAction returns `Relay::Move(id)`;
  Struggle never reaches it (the executor skips OverrideAction for Struggle), which is what lets a Choice lock +
  Encore + Disable stack end in Struggle (scenario 1).
* **Imprison.** `pokemon.disableMove(id, true)` (hidden) for every non-Struggle slot of `effectState.source`, then
  `maybeDisabled = true` (`mon_flags::MAYBE_DISABLED`). The visible effect is the request-level hidden-information
  rejection plus `|cant|..|move: Imprison|..` when the foe's move was queued before the Imprison.
* **Heal Block.** durationCallback is a direct call (Psychic Noise = 2, Persistent source = 7 with the
  `-activate` line, else 5). onStart sets `source.moveThisTurnResult = true`; onRestart is a no-op for Psychic Noise,
  otherwise `|-fail|` and `moveThisTurnResult` becomes false unless it was true. onTryHeal: third-party Pollen Puff on
  a damaged target logs `[still]` + `|cant|source|move: Heal Block|Pollen Puff` and returns **null**; everything else
  returns **false**. The Z-Power early return is outside the format.
* **Partially trapped.** onResidual deletes the volatile *without* an End event when the trapper left/fainted/has
  not been active a turn (`Battle::erase_volatile`, `pub(crate)`, the same primitive `remove_volatile` uses after End),
  logging `-end|p|<sourceEffect>|[partiallytrapped]|[silent]`; otherwise `damage(max_hp / boundDivisor)`.
  `'move: ' + sourceEffect` is `LogArg::Parts([Text("move: "), Effect(..)])`. G-Max Centiferno/Sandblast
  (`gmaxEffect`) do not exist in this format.
* **Trapping abilities.** `this.effectState.target` is the ability cell's `target` holder (`present::TARGET`);
  the FoeMaybeTrapPokemon `singleEvent` path (see the core issue below) has no state and passes `source` explicitly,
  which is why `source = arg 1 or effectState.target` is ported literally. `isGrounded(!pokemon.knownType)` uses
  `mon_flags::KNOWN_TYPE`; `maybeTrapped = true` is `mon_flags::MAYBE_TRAPPED`; `tryTrap(true)` is hidden.
* **Out-of-format branches** (verified absent from the generated dex, kept as comments or static-key queries, no ids
  fabricated): `sourceEffect === 'snatch'`, Me First, Z/Max flags, Dynamax, G-Max trapping, Z-Power heal bypass,
  Grip Claw, Binding Band, Persistent (the last three through `query_has_item/ability`).

## Core issue the integrator must fix (not edited here)

**`FoeMaybeTrapPokemon` single events never reach the trapping abilities.**
`sim/lifecycle/turn.rs:662-673` (`switch-in/turn start` loop over the foes' species ability slots) calls
`single_event(EventId::FoeMaybeTrapPokemon, EffectRef::Dex(slot.id), None, .., None)`. The generated manifest models
`onFoeMaybeTrapPokemon` as event `MaybeTrapPokemon` with `HookRel::Foe` (see `HOOK_ABILITY_ARENATRAP_ONFOEMAYBETRAPPOKEMON`
etc.), so `event_hook(effect, FoeMaybeTrapPokemon, HookRel::On)` finds nothing and the call is a silent no-op; the
comment "Abilities outside the executable closure have no FoeMaybeTrapPokemon callback (verified by codegen)" is wrong
for Arena Trap, Magnet Pull and Shadow Tag. Effect on the protocol: a Pokemon facing a foe whose *species* has one of
these abilities in a slot it does not currently hold (Golem-Alola with Galvanize, Dugtrio with Sand Veil, ...) must
report `maybeTrapped` in its request; the engine omits it.

Repro (before the fix): `difftest replay data/fixtures/directed-smoke.jsonl.gz --sim engine --battle 176`
-> `request.active[].maybeTrapped (missing)` at step 1 (p1b Excadrill, foe Golem-Alola holding Galvanize).
Fix verified in a scratch copy (battle 176 and the whole batch corpora pass): look the hook up with the Foe relation
and pass it as `custom_hook`:

```rust
let hook = self.event_hook(EffectRef::Dex(slot.id), EventId::MaybeTrapPokemon, HookRel::Foe);
self.single_event(EventId::FoeMaybeTrapPokemon, EffectRef::Dex(slot.id), None,
    mon_arg(pokemon), mon_arg(source), EffectRef::None, Relay::Undefined, hook);
```

(`event_hook` requires `site.is_empty()`, which holds for the three abilities.) The same pattern (a `single_event`
with an `EventId::Foe*`/`Ally*`/`Any*` name) is worth grepping for elsewhere.

## Requested / missing core APIs

None. Used as-is: `try_trap`, `add_volatile` (with `linked_status` for Spirit Shackle), `remove_volatile`,
`get_volatile`, `disable_move`, `has_move`, `get_move_data`, `direct_damage`, `damage`, `is_grounded`, `has_type`,
`is_adjacent`, `is_ally`, `queue_will_move`, `active_move[_mut]`, `attr_last_move`, `add_move`, plus the `pub(crate)`
`erase_volatile`, `query_has_item`, `query_has_ability`. Conventions relied on: `condition_source` fills
`source`/`source_effect` (frozen token) on the volatile cell; the `Start` event receives `[pokemon, source, effect]`;
`DurationCallback` is a direct call with `[target, source, effect]`; `OverrideAction` accepts `Relay::Move(id)`.

## Tests and acceptance

* `cargo build -j 2` clean (no warnings); `cargo test -j 2`: 159 passed, 16 ignored (one of them mine, below).
* `locktrap/scenarios.txt` (9 scenarios from `tools/probes/disable_lock_trap/scenarios.mjs`, real Showdown, every
  scenario self-checks its intended log lines at generation): exact log + PRNG seed after every decision boundary,
  with explicit rejected choices (`bad || good`, trapped switches, Imprison/trap hidden information).
  8 run in the default `scenarios_match_pinned_showdown` test: Choice lock + Dancer + Encore/Disable/Taunt to Struggle,
  Heal Block vs Grassy Terrain, Taunt/Encore/Disable/Imprison durations and Cursed Body, Infestation expiry and
  trapper leaving, Spirit Shackle/No Retreat, Arena Trap/Magnet Pull/Shadow Tag switching, Curse (Ghost, normal,
  ally retarget, user faint). `healblock_pollenpuff` needs `moves:pollenpuff` (move_callbacks batch) and is replayed by
  `deferred_scenarios_match_pinned_showdown` (`#[ignore = "needs moves:pollenpuff (move_callbacks batch)"]`);
  it passed against a throw-away local Pollen Puff port (never committed), validating the onTryHeal Pollen Puff branch.
  **Integrator: un-ignore it once Pollen Puff lands.**
* End-to-end (`gen-directed --profile batch:disable_lock_trap --count 100 --verify`, then `difftest replay`): raw
  86/100; **every one of the 14 failures is the same other-batch panic** (`abilities:naturalcure onCheckShow`,
  healing_residual). Excluding other-batch panics: **100%**. With that single file temporarily overlaid from a sibling
  worktree (not committed): 100/100. Further directed corpora, all battles passing with the overlay:
  `batch:disable_lock_trap` seed 77 x500 and `+tera` seed 78 x300; single-effect profiles for items
  choicescarf/choiceband/choicespecs and moves taunt/encore/imprison/psychicnoise/infestation/spiritshackle/
  throatchop/curse/disable/noretreat at seeds default (100 each) and 31 (300 each); abilities arenatrap/magnetpull/
  shadowtag seed 41 x300 each. Raw pass rates of those single-effect profiles were 63-92% with only the same
  naturalcure panic as failures.
* Oracle callback coverage over 1,400 of those battles: 57 of 58 function handlers invoked; the one never reached is
  `conditions:healblock onTryHeal`, now covered by the scenarios above.
* Conditions that the batch profile calls "unreachable" (choicelock, encore, healblock, imprison, partiallytrapped,
  taunt) only have creators outside the batch (items/moves without function hooks); they are exercised through the
  `effect:items:choice*` and `effect:moves:{taunt,encore,imprison,psychicnoise,infestation}` profiles.
* Broader check in a scratch copy overlaying every other agent's finished effect files plus the turn.rs fix:
  directed 150-battle corpora of weather_terrain, hazards_screens, passive_offense, passive_defense, reactive_stats,
  healing_residual, items_modifiers, identity_abilities, move_callbacks all pass 150/150; no divergence in any
  non-panic category involves this batch (the only non-panic divergences seen are `|detailschange|` tera suffixes,
  formes_tera).

## Integrator checklist

1. Apply the `turn.rs` fix above and re-run `difftest replay data/fixtures/directed-smoke.jsonl.gz --battle 176`.
2. Un-ignore `deferred_scenarios_match_pinned_showdown` after `moves:pollenpuff` is ported.
3. Re-run the common sample-50 / fuzz replay once Protect, consumables and the remaining batches land; Choice lock,
   Taunt, Encore, Disable, trapping and Heal Block were the most frequent panics in the random corpus before this
   batch and are now all implemented.
