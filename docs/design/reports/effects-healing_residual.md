# Effect batch `healing_residual`

Base `76a95a3` (whole core implemented), branch `worktree-agent-ad00da993a3fec476`. Oracle: Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a` (`data/{moves,abilities,items}.ts`, `sim/{battle,pokemon,side}.ts`),
read only. The effect files were first written by an interrupted agent (`wip/agent-af2654e454fa938c7`, 23
files, no tests); every file was re-reviewed line by line against the pinned TypeScript here, the
helper module got its missing `tests.rs`, the probe was rewritten, and the batch was validated end to end.
No `todo!()` remains; no core API is missing; no waivers.

## Files

21 effect files covering all **32 function sites** of the batch (`EFFECT-BATCHES.json`), `WAIVERS = &[]`
everywhere. `HOOK_ABILITY_HOSPITALITY_ONSWITCHIN` is manifest ordering metadata (`onSwitchInPriority: -2`);
the callback is `onStart`, reached through the SwitchIn -> onStart fallback.

| Family | Files |
|---|---|
| moves (6) | healpulse junglehealing leechseed (`onTryImmunity`) lunarblessing strengthsap takeheart |
| abilities (8) | baddreams dryskin healer hospitality naturalcure poisonheal regenerator shedskin |
| items (3) | flameorb leftovers toxicorb |
| conditions (4) | leechseed saltcure syrupbomb wish (embedded move conditions / slot condition) |

Private helpers: `crates/engine/src/effects/abilities/healingresidual/{mod.rs,tests.rs,scenarios.txt}`,
declared once in `abilities/baddreams.rs` (`#[path = "healingresidual/mod.rs"] pub(super) mod support;`) and
reached from the other 20 files as `crate::effects::registry::abilities_baddreams::support` (same pattern
as `weatherterrain`, `aromaveil`). The test module is therefore
`effects::registry::abilities_baddreams::support::tests`. Probe: `tools/probes/healing_residual/scenarios.mjs`.

## Per-effect notes (TS file:line is on every handler; PRNG on every handler)

Direct PRNG draws exist in exactly two handlers; everything else is nested event draws (speed-sort tie
shuffles inside `damage`/`heal`/`boost`/`cureStatus`/`trySetStatus` events).

| Effect | Sites | TS | Notes |
|---|---|---|---|
| abilities:healer | onResidual | abilities.ts:1818 | `randomChance(3, 10)` only for statused adjacent allies (`&&` short-circuit); allies list captured once |
| abilities:shedskin | onResidual | abilities.ts:4188 | `randomChance(33, 100)` only when `hp && status` |
| abilities:baddreams | onResidual | abilities.ts:320 | `foes()` captured once; `status === 'slp'` or Comatose; `damage(baseMaxhp/8, target, pokemon)` |
| abilities:dryskin | onTryHit, onSourceBasePower, onWeather | abilities.ts:1098 | `heal(...)` falsy -> `-immune ... [from] ability: Dry Skin`, returns `Null` either way; `chainModify(1.25)` returns Undefined; Primordial Sea / Desolate Land do not exist in the scoped dex so those `effect.id` alternatives cannot match |
| abilities:poisonheal | onDamage | abilities.ts:3332 | args (damage, target, source, effect); heals `baseMaxhp/8`, returns `false` for psn/tox |
| abilities:regenerator | onSwitchOut | abilities.ts:3834 | `Pokemon.heal` (raw, no events, no log) via `raw_heal(baseMaxhp/3)` |
| abilities:hospitality | onStart | abilities.ts:1875 | `heal(ally.baseMaxhp/4, ally, pokemon)` per adjacent ally; log `[from] ability: Hospitality\|[of]` comes from the core `heal` |
| abilities:naturalcure | onCheckShow, onSwitchOut | abilities.ts:2812 | see below |
| items:leftovers / flameorb / toxicorb | onResidual | items.ts:3338 / 2142 / 6342 | `heal(baseMaxhp/16)`; `trySetStatus('brn'/'tox', pokemon)` (the `[from] item:` tag is printed by the status condition's onStart via `log_status`) |
| moves:healpulse | onHit | moves.ts:8399 | Mega Launcher: `modify(baseMaxhp, 0.75)` else `ceil(baseMaxhp*0.5)`; failure prints `-fail\|target\|heal` and returns `NotFail`; `staleness = 'external'` dropped (Endless Battle Clause is not in this format) |
| moves:junglehealing / lunarblessing | onHit | moves.ts:9856 / 10547 | heal runs first, then `cureStatus()`; returns `Bool(cured \|\| success)` |
| moves:takeheart | onHit | moves.ts:18929 | `boost({spa:1, spd:1})` with event defaults, key order preserved (`OrderedBoosts`), then `cureStatus()` |
| moves:strengthsap | onHit | moves.ts:18174 | `atk === -6` -> `false`; `getStat('atk', false, true)`; `boost({atk:-1}, target, source, null, false, true)`; `heal(atk, source, target)` runs before `success` is consulted |
| moves:leechseed | onTryImmunity | moves.ts:10202 | returns `Bool(!hasType('Grass'))` |
| conditions:leechseed | onStart, onResidual | moves.ts:10212 | seeder is the *slot* (`source_slot` of the volatile's cell, resolved with `get_at_slot`), so a replacement receives the HP; `damage(baseMaxhp/8, pokemon, seeder)` then `heal(damage, seeder, pokemon)` where a `true` damage result coerces to 1 (`relay_amount`) |
| conditions:saltcure | onStart, onResidual, onEnd | moves.ts:15641 | `damage(baseMaxhp / (hasType(['Water','Steel']) ? 4 : 8))` with event defaults |
| conditions:syrupbomb | onStart, onUpdate, onResidual, onEnd | moves.ts:18761 | bomber = `source` of the cell; onUpdate removes the volatile when the source is not active; onResidual `boost({spe:-1}, pokemon, source)`; an unset source falls back to the event source |
| conditions:wish | onStart, onResidual, onEnd | moves.ts:20915 | see payload map below |

### Natural Cure (the batch's hard effect)

* State is `Pokemon::show_cure` (`ResultFlag::{Undefined, False, True}`), exactly the TS `showCure?: boolean`
  (never `Null`). The ability's effect state is unused.
* `onCheckShow` runs through `singleEvent('CheckShow', naturalcure, null, pokemon)` (battle.ts:2768, core
  `turn.rs:359`). It walks `side.active` in slot order, keeps the TS skip order (no status -> `showCure`
  already true -> species cannot have Natural Cure -> species has only slot 0 -> not switching) and only
  then splits into cured vs. not-cured. Fainted actives (status `fnt`) are *not* skipped, as in TS.
* `species.abilities` is `[0, 1, H, S]` in the generated dex; "only one ability" is
  `abilities[1]` and `abilities[2]` empty (`abilities['1']`/`['H']`).
* Ambiguous case prints `-message|(N of <player name>'s pokemon was/were cured by Natural Cure.)` with
  `LogArg::PlayerName(side)`; `onSwitchOut` prints the silent `-curestatus ... [from] ability: Natural
  Cure [silent]`, calls `clear_status` and resets `showCure` to undefined unless it is true.

### Wish payload map (`PAYLOAD_WORDS = 3`, slot-condition cell)

* words[0..2] = `effectState.hp` (`source.maxhp / 2`, a JS float: x.5 for odd max HP) as
  `f64::to_bits`, low word first; presence bit `CUSTOM_START` (8).
* words[2] = `effectState.startingTurn` = `getOverflowedTurnCount()` = `(turn - 1) % 256` (0..=255);
  presence bit 9.
* The wisher is the common `source` / `sourceSlot` pair stored by `addSlotCondition`; Wish has no
  duration and no restart.
* `onEnd` heals `Some(target), Some(target)` with the Wish condition as effect (the core `heal` prints
  nothing for `wish`), then prints `-heal|target|health|[from] move: Wish|[wisher] <source.name>` as a
  split entry. `source.name` is read from the team set (`teams().sides[..].sets[idx % 6].name`, the same
  source `log/format.rs` uses for idents) into a 64-byte stack buffer (names are at most 20 UTF-16 units,
  so at most 60 bytes); the copy is skipped for `NoLog` (`L::ENABLED`).
* The turn-255 hint (`In Gen 8+, Wish will never resolve when used on the 256th turn.`) is ported with
  `LogArg::Parts`; it cannot occur in a normal-length battle.

## Core APIs used (all present; nothing requested)

`heal`, `damage`, `boost`, `get_stat`, `try_set_status`, `cure_status`, `clear_status`, `raw_heal`,
`remove_volatile`, `remove_slot_condition`, `get_at_slot`, `get_slot`, `foes`, `adjacent_allies`,
`queue_will_switch`, `effective_weather`, `has_ability`, `has_type`, `modify`, `chain_modify`,
`event_effect_id`, `hint`, `hook_state[_mut]` (`source`, `source_slot`, payload), `teams()`.
Conventions relied on: `Attribution::DEFAULT` / `from_move(source, EffectRef::None)` defaults `source` to the
event source and `effect` to `scratch.current_effect`; `heal(.., None, None, HealEffect::Context)` is the
TS `this.heal(x)`.

## Validation

* `cargo build -j 2`, `cargo test -j 2 -p engine`: registry manifest test passes (all 32 sites covered by
  `HOOKS`, no waivers); 23 helper-module tests pass.
* **18 recorded whole-battle scenarios** (`healingresidual/scenarios.txt`, from the pinned Showdown build:
  log + turn + PRNG seed at every decision boundary), all replayed bit-exactly against the Rust engine:
  `orbs_leftovers_poisonheal leech_seed_drain_and_grass_immune leech_seed_slot_after_switch
  wish_heals_replacement wish_in_place_and_repeat natural_cure_ambiguous_switch
  natural_cure_and_regenerator healer_ally_status shed_skin_statused bad_dreams_sleeping_and_comatose
  dry_skin_water_rain_fire dry_skin_sun_damage salt_cure_water_steel syrup_bomb_speed_drops
  syrup_bomb_source_leaves heal_pulse_and_jungle_healing lunar_blessing_take_heart_strength_sap
  hospitality_on_entry`. The scenarios deliberately avoid Protect, Protean and other abilities/moves of
  batches that are not merged yet, so they run on any base. A marker test pins what each scenario must
  contain (Healer / Shed Skin scenarios search seeds until the 30% / 33% branch fires).
  Regenerate: `node tools/probes/healing_residual/scenarios.mjs` (`DEBUG_PROBE=1` prints failing scripts).
* **End to end** (`docs/design/CORPUS.md`):
  * `gen-directed.mjs --profile batch:healing_residual --count 100 --threads 2 --verify` (seed 1110):
    **100/100** battles identical (3855/3855 steps, 85,399 log lines); a second run with `--seed 424242
    --count 500`: **500/500** (19,970 steps, 436,576 log lines).
  * Coverage of the 100-battle corpus: 22/32 handlers reached; the 10 missing are the three conditions
    whose creators (Wish, Salt Cure, Syrup Bomb) are not in the batch profile's move list. They were
    covered with `--profile effect:moves:wish|saltcure|syrupbomb --count 60`: **60/60 each**, every
    handler reached except `conditions:saltcure onEnd` (see below).
  * `--profile effect:<kind>:<id> --count 40 --seed 9090` for each of the 20 creators/holders in the batch
    (6 moves, 8 abilities, 3 items, wish, saltcure, syrupbomb): 40/40 identical each.
  * `data/fixtures/sample-50.jsonl.gz`: all 50 fail at step 0 on *other* batches' unimplemented hooks
    (protect, sitrusberry, quarkdrive, choicelock, frisk, illusion, ...); none of the panics involves this
    batch.

## Notes for the integrator

* `conditions:saltcure onEnd` is a trivial `-end|mon|Salt Cure` port that no scoped play reaches:
  `clearVolatile` (switch-out, faint) deletes volatiles without End events and nothing in scope calls
  `removeVolatile('saltcure')`. It is covered by review only.
* Dry Skin ignores Primordial Sea / Desolate Land (absent from the generated dex) and Heal Pulse drops
  `target.staleness = 'external'`; both are unobservable in this format.
* Debug lines (`this.debug(...)`) produce no protocol output and are omitted.
* After the other batches merge, re-run `cargo test -j 2 -p engine abilities_baddreams::support` and the
  `batch:healing_residual` corpus above; they exercise Protect-free sets, so they stay valid when
  Protect/Intimidate/etc. change.
