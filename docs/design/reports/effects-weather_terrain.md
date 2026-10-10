# Effect batch `weather_terrain`

Base `e784f91`, branch `worktree-agent-aedeb90cfb8820d7f`. Oracle: Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a` (`data/{moves,abilities,conditions}.ts`, `sim/field.ts`,
`sim/pokemon.ts`), read only. Core lifecycle/choices/moves/mutators/log are still `todo!()` in this
worktree; every handler is written against the frozen signatures and no `todo!()` remains in this
batch's files.

## Files

44 effect files covering all **88 function sites** of the batch (`EFFECT-BATCHES.json`), no waivers,
`PAYLOAD_WORDS = 0` everywhere, no hook site left to the central dispatcher.

| Family | Files |
|---|---|
| moves (15) | bleakwindstorm blizzard expandingforce floralhealing grassyglide hurricane moonlight morningsun psyblade sandsearstorm shoreup synthesis thunder weatherball wildboltstorm |
| abilities (21) | airlock chlorophyll cloudnine drizzle drought electricsurge grassysurge hadronengine hydration icebody leafguard orichalcumpulse psychicsurge sandforce sandrush sandstream seedsower slushrush snowwarning solarpower surgesurfer swiftswim |
| conditions (7) | electricterrain grassyterrain psychicterrain raindance sandstorm snowscape sunnyday |

Shared private helpers live in `crates/engine/src/effects/abilities/weatherterrain/mod.rs`
(discovery ignores nested dirs). It is declared once, in `abilities/airlock.rs`
(`#[path = "weatherterrain/mod.rs"] pub(super) mod support;`) and referenced from the other 43 files as
`crate::effects::registry::abilities_airlock::support` (the same pattern as `abilities/aromaveil.rs`).
The test module is therefore `effects::registry::abilities_airlock::support::tests`.
Other new files: `weatherterrain/{tests.rs,callbacks.tsv,scenarios.txt}`,
`tools/probes/weather_terrain/{callbacks.mjs,scenarios.mjs}`.

## Per-effect source map

Every dispatch arm carries the TS line range and its PRNG note. No handler draws directly; the only
reachable draws are nested: `each_event` speedSort tie shuffles (Air Lock/Cloud Nine WeatherChange, the
weather residual Weather event), Type events behind `isGrounded`/`hasType`, and the handlers of
`set_weather`/`set_terrain`/`heal`/`damage`/`cure_status`.

| Effect | Sites | TS |
|---|---|---|
| conditions:raindance | durationCallback, onWeatherModifyDamage, onFieldStart, onFieldResidual, onFieldEnd | conditions.ts:474-511 |
| conditions:sunnyday | + onImmunity | conditions.ts:544-589 |
| conditions:sandstorm | durationCallback, onModifySpD, onFieldStart, onFieldResidual, onWeather, onFieldEnd | conditions.ts:626-663 |
| conditions:snowscape | durationCallback, onModifyDef, onFieldStart, onFieldResidual, onFieldEnd | conditions.ts:694-726 |
| conditions:electricterrain | durationCallback, onSetStatus, onTryAddVolatile, onBasePower, onFieldStart, onFieldEnd | moves.ts:4497-4551 |
| conditions:grassyterrain | durationCallback, onBasePower, onFieldStart, onResidual, onFieldEnd | moves.ts:7673-7727 |
| conditions:psychicterrain | durationCallback, onTryHit, onBasePower, onFieldStart, onFieldEnd | moves.ts:14095-14151 |
| abilities (setters) | drizzle, drought, sandstream, snowwarning (setWeather); electricsurge, grassysurge, psychicsurge, seedsower (setTerrain) | abilities.ts:1078,1088,3998,4388; 1179,1708,3581,4120 |
| abilities:hadronengine / orichalcumpulse | onStart + onModifySpA / onModifyAtk | abilities.ts:1783 / 3089 |
| abilities:airlock / cloudnine | onSwitchIn, onStart, onEnd | abilities.ts:90 / 543 |
| abilities (stat/damage) | chlorophyll swiftswim sandrush slushrush surgesurfer (ModifySpe); solarpower (ModifySpA + onWeather); sandforce (BasePower + Immunity); icebody (onWeather + Immunity); hydration (Residual); leafguard (SetStatus + TryAddVolatile) | abilities.ts:512,4810,3975,4348,4757,4397,3957,1956,1926,2292 |
| moves | accuracy: bleakwindstorm blizzard hurricane sandsearstorm thunder wildboltstorm; weatherball (type + bp); expandingforce (bp + target); psyblade bp; grassyglide priority; healing: moonlight morningsun synthesis shoreup floralhealing | moves.ts (see arm comments) |

## Tricky semantics (how they were ported)

* **Return values.** `chainModify` handlers return `Undefined`; Sandstorm/Snowscape stat handlers return
  `Number(modify(stat, 1.5))` (not a chain). Immunity handlers return `Bool(false)` or `Undefined`.
  Leaf Guard `onSetStatus` returns `Bool(false)`, `onTryAddVolatile` `Null`. Electric Terrain
  `onSetStatus` `Bool(false)`, `onTryAddVolatile` `Null`; Psychic Terrain `onTryHit` `Null`/`Undefined`.
  Healing moves return `Bool(true)` or `NotFail` after `-fail|p|heal`. durationCallback returns
  `Number(5|8)`. Grassy Glide returns `Number(priority + 1)`.
* **Short-circuit order is preserved** wherever the callee runs events: Electric Terrain
  `slp && isGrounded() && !isSemiInvulnerable()`; Psychic Terrain priority/target early return, then
  semi-invulnerable/ally, then `isGrounded` (a Type event), then the airborne hint; Sandstorm/Snowscape
  `hasType` before `effectiveWeather`; Expanding Force/Grassy Glide `isTerrain` before `isGrounded`.
* **Leaf Guard `(effect as Move)?.status`**: true for an ActiveMove/dex move with a primary `status`
  and for Synchronize's synthetic `{id, status}` effect; false for secondary-status moves (silent
  `false`, no `-immune`). Electric Terrain's `effect.id === 'yawn' || (Move && !secondaries)` is tested
  against both the Yawn condition and move ids, and Synchronize (no effectType) stays silent.
* **`isTerrain(...)` implicit target.** TS defaults the target to `battle.event.target`
  (field.ts:169-184). Every handler passes the explicit event target (`Some(mon)`), which equals
  `event.target` at each call site (checked per hook: Start/ModifySpe/ModifySpA target = the mon,
  BasePower target = attacker, ModifyMove/ModifyPriority target = user, Hit target = target).
* **Air Lock / Cloud Nine** write the common `present::ENDING` bit of the holder's `ability_state`
  (`abilityState.ending`, the bit `Battle::suppressing_weather` already reads), then
  `each_event(WeatherChange, this.effect)` with `scratch.current_effect`. `onSwitchIn` announces
  (`-ability|p|Air Lock`) and calls the same start body directly (no new event frame).
* **Out-of-scope branches** (generated dex omits them; no id was fabricated): Hail, Primordial Sea,
  Desolate Land, Delta Stream, Misty Terrain, Hydro Steam (Sun's attacker branch), Bulldoze/Magnitude
  (Grassy Terrain), Utility Umbrella/Mega Sol (already handled by `effective_weather`), `gen <= 5`
  duration resets. The Damp/Heat/Smooth/Icy Rock and Terrain Extender checks in durationCallback and the
  Blue/Red Orb checks in Drizzle/Drought are still performed through static keys
  (`query_has_item`, `dex::effect(item).key`), which are never true for the scoped item list.
* **Dropped bookkeeping:** Floral Healing's `target.staleness = 'external'` (Endless Battle Clause,
  `docs/showdown/04-state-model.md:313`); `this.debug(...)` calls produce no protocol output.

## Core APIs used and conventions the integrator must keep

No API is missing; no signature change is requested. Conventions the handlers rely on:

1. `set_weather(id, Attribution::DEFAULT) -> Relay` / `set_terrain(id, Attribution::DEFAULT) -> bool`
   must default `source` to the running event's target and `sourceEffect` to the executing effect
   (field.ts:44-46,131-135). Orichalcum Pulse tests the Relay with `.truthy()`; Hadron Engine uses the bool.
2. `FieldStart` receives `(Field, source, sourceEffect)`; the source effect must be
   `EventArg::Effect(EffectRef::Dex(ability))` for ability setters (so `event_effect_type == Ability`),
   `Undefined`/`Null`/`EffectRef::None` when absent. `durationCallback` is a direct call whose argument 0
   is the source Pokemon (field.ts:75,152; `.call(battle, source, source, sourceEffect)`).
3. `is_terrain(&[id], Some(mon))` must run `TryTerrain` for that mon (the pre-existing doc string of
   `is_terrain` says "field terrain for no target", which would differ from TS when an event is running;
   handlers never pass `None`). Today `is_terrain` is `todo!()`.
4. `heal(amount, Some(target), source, HealEffect::Context)`: `source: None` means TS-omitted, i.e.
   `source ||= event.source` (moves: event source = user; Ice Body: null). Grassy Terrain residual passes
   `Some(pokemon)` explicitly, like the source.
5. `damage(amount, Some(target), Attribution)`: Sandstorm `Attribution::DEFAULT`; Solar Power
   `Attribution { source: target, effect: None }` (effect defaults to the executing ability).
6. SetStatus/TryAddVolatile argument 0 (a Condition) is read through `EventArg::Effect`,
   `Relay::Effect` or `Relay::Move`; the Immunity argument through `EventArg::StatusImmunity` or
   `Relay::StatusImmunity`; weather/terrain events' effect arguments through `EventArg::Effect`.
7. `cure_status(mon, false)` (Hydration), `hint(LogArg::Text, false, None)` (Psychic Terrain) and
   `each_event(Weather | WeatherChange, effect, Undefined)` are used as documented.
8. `scratch.current_effect` must be the executing ability/condition while a handler runs
   (`invoke` already does this); `each_event(.., EffectRef::None, ..)` relies on it for Weather.

## Tests

* `cargo build -j 2` and `cargo test -j 2`: 71 lib tests pass (registry manifest test included), 9 ignored.
* **854 pinned callback rows** (`callbacks.tsv`, from `tools/probes/weather_terrain/callbacks.mjs`):
  the real Showdown handlers run on stub contexts (weather x terrain x types x semi-invulnerable x move
  type/id x input); Rust replays the same situation through `call_hook` and compares relay, final
  modifier, accuracy, base power, move type, move target and the unchanged PRNG seed.
  798 rows run now; the 56 rows that consult `Field::isTerrain` are in
  `terrain_dependent_callbacks_match_pinned_showdown` (`#[ignore = "needs core: Battle::is_terrain"]`).
  They were also run once, green, against a throw-away local `is_terrain` (reverted, not committed).
* Branch tests with a recording sink: field start/end/upkeep log shapes, duration callbacks, Leaf Guard,
  Electric Terrain sleep/Yawn branches, Psychic Terrain blocking, Air Lock/Cloud Nine ending flag vs
  `field_effective_weather`.
* **8 recorded whole-battle scenarios** (`scenarios.txt`, from `scenarios.mjs`; log, turn and PRNG seed at
  every decision boundary): `weather_leads sun rain sand snow electric_terrain grassy_terrain
  psychic_terrain`, each `#[ignore = "needs core"]`. A non-ignored test checks the file parses, every
  team is in scope and the battles construct.
* Regenerate: `node tools/probes/weather_terrain/callbacks.mjs`, `node tools/probes/weather_terrain/scenarios.mjs`
  (add `--out FILE` to inspect without touching the checked-in data).
* Directed corpus: `node tools/oracle/gen-directed.mjs --profile batch:weather_terrain --count 40
  --threads 2 --out /tmp/wt.jsonl.gz` (seed 1101) followed by
  `node tools/oracle/coverage.mjs /tmp/wt.jsonl.gz` invokes **88/88** of this batch's handlers.

## Integrator checklist

* After the core lands run `cargo test -j 2 -- --ignored abilities_airlock::support::tests` (the eight
  scenarios plus the terrain callback rows) and the common sample-50/fuzz gate; the batch corpus above is
  the cheapest broad replay for these effects.
* Watch for: weather replacement order at switch-in (tie shuffles in `weather_leads`, `electric_terrain`
  with two equal-speed Hadron Engine holders), the `[from] ability:` / `[of]` tags on `-weather` and
  `-fieldstart` lines, Hydration curing before burn damage at residual, Seed Sower replacing a rival
  terrain (`-fieldend` then `-fieldstart`), and the `|-hint|` line for airborne targets under Psychic
  Terrain (needs T's `hint`).
* Sandstorm's own `onWeather` damage and Grassy Terrain's residual heal depend on D's `damage`/`heal`
  defaulting rules in items 4-5 above; Solar Power and Ice Body likewise.
