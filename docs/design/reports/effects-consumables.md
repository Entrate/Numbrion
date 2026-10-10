# effects(consumables): berries, consumable items and their abilities

Completes the 16 files / 42 function sites of the `consumables` batch against Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a` (`data/items.ts`, `data/abilities.ts`). Every site is in
`HOOKS`; there are no waivers and no hook is a runtime no-op.

Items: Aguav, Chesto, Figy, Iapapa, Mago, Sitrus, Wiki Berry, Focus Sash, Throat Spray, Weakness
Policy, White Herb. Abilities: Cheek Pouch, Gluttony, Harvest, Ripen, Unnerve.

Base: `76a95a3` (the worktree started on the skeleton and was reset as instructed).

## Provenance of the WIP file

The interrupted agent left exactly one file, `items/consumables/mod.rs` (helper module, no effect
files, no tests). It belongs to this batch; it was read against the TypeScript and the core APIs and
adopted. Review result: no semantic defect found. Changes: added the three shared pinch-berry
bodies (`pinch_on_update`, `pinch_on_try_eat_item`, `pinch_on_eat`), a `None` guard in
`effect_is_berry`, doc comments, and removed the unused `clear_negative_boosts_now` (White Herb's
`fling.effect` is ported inline in `whiteherb.rs`). `tests.rs` (referenced by the WIP) did not
exist; it is written here.

## Layout

`items/aguavberry.rs` owns the private module (`#[path = "consumables/mod.rs"] pub(super) mod
support;`); the other 15 files import `crate::effects::registry::items_aguavberry::support`
(same pattern as `aromaveil.rs` / `battlebond.rs`). Nothing allocates, formats a string or
touches the PRNG except `harvest.rs`.

## Per-effect notes (TS file:line, relay values, draws)

Argument indices are the TS parameter order with `runEvent`'s relay first (`onTryEatItem(item,
pokemon)` => pokemon is argument 1; `onUpdate(pokemon)`/`onEat(pokemon)` => argument 0).

* **Sitrus Berry** (items.ts:5744; hooks 5752/5757/5760). `onUpdate`: `hp <= maxhp / 2` (float
  division) => `eatItem()`; `onTryEatItem`: `runEvent('TryHeal', pokemon, null, this.effect,
  baseMaxhp / 4)` falsy => `false` else `undefined`; `onEat`: `heal(baseMaxhp / 4)` with
  target/source/effect defaulted from the event. `baseMaxhp == maxhp` (no Dynamax). No Gluttony
  clause.
* **Aguav / Figy / Iapapa / Mago / Wiki** (159/2045/2913/3704/7728). `onUpdate`: `hp <= maxhp / 4 ||
  (hp <= maxhp / 2 && hasAbility('gluttony') && abilityState.gluttony)` (short-circuit order kept).
  `onTryEatItem`: TryHeal with `baseMaxhp / 3`. `onEat`: heal 1/3, then
  `addVolatile('confusion')` (source/effect default from the event) when `getNature().minus` equals
  spd/atk/def/spe/spa respectively. Nature stats use the table index (1 atk .. 5 spe, -1 for the five
  neutral natures = JS `undefined`, which never matches). A set without a nature (`get_nature` ==
  `None`) is also never confused.
* **Chesto Berry** (877). `status === 'slp'` => `eatItem()`; `onEat` cures with `cureStatus()` (not
  silent: `-curestatus|mon|slp|[msg]`).
* **Focus Sash** (2269). `onDamage(damage, target, source, effect)` priority -40 (manifest):
  `hp === maxhp && damage >= hp && effect is a Move` then `useItem()`; returns `Number(hp - 1)`
  (read after the item was used) or `undefined`. Uses `event_effect_type == Move` so synthetic
  Move-typed effects (confusion, recoil...) behave like TS.
* **Throat Spray** (6309). `onAfterMoveSecondarySelf(user, target, move)`: `move.flags.sound` =>
  `useItem()` on argument 0 (named `target` in the TS). The `boosts: {spa: 1}` is applied by the
  core's `use_item`.
* **Weakness Policy** (7596). `onDamagingHit(damage, target, source, move)`: runs `useItem()` on the
  target when `!move.damage && !move.damageCallback && getMoveHitData(move).typeMod > 0`.
  `move.damage` is falsy for `DamageSpec::None` and `Fixed(0)`; `damageCallback` is the move's
  direct `DamageCallback` hook; `getMoveHitData` lazily creates the record through
  `Battle::move_hit_data`.
* **White Herb** (7658). Seven sites: `fling.effect` (negative stages => `setBoost` zeros +
  `-clearnegativeboost|mon|[silent]`), `onStart`, the three `onAny*` handlers and `onResidual`
  (direct `.call(this, ...)` of `onStart` with the holder; the `onAny*` ones pass
  `effectState.target`), and `onUse` (`setBoost(effectState.boosts)` then the silent line). The
  boosts table only exists inside `onStart`: payload word 0 = bit mask of the negative keys,
  presence bit 8; published before `useItem()` (which runs `onUse` on the same item state) and
  removed afterwards (`delete this.effectState.boosts`). With the table absent `onUse` still
  prints the line (JS `for (i in undefined)` is a no-op).
  `fling.effect` and `onAnyAfterMega` are not reachable in this format (Fling and Mega Evolution
  are out of scope) but are ported rather than waived.
* **Cheek Pouch** (492). `onEatItem(item, pokemon)`: `heal(baseMaxhp / 3)` (pokemon is argument 1).
* **Gluttony** (1619). `onStart(pokemon)` and `onDamage(item, pokemon)` (the "item" is the damage
  relay; pokemon is argument 1) set `pokemon.abilityState.gluttony = true`: custom presence bit 8
  of the Pokemon's ability-state cell, dropped with the cell when the ability changes.
* **Harvest** (1801; Residual order 28 / sub-order 2 in the manifest). `isWeather(['sunnyday',
  'desolateland']) || randomChance(1, 2)`; **PRNG: one `randomChance(1, 2)` draw per residual unless
  the effective weather is Sun** (short circuit kept; Desolate Land is outside the scoped dex).
  Then `hp && !item && items.get(lastItem).isBerry` => `setItem(lastItem)` (no source/effect),
  `lastItem = ''`, `-item|mon|<getItem()>|[from] ability: Harvest` (the item is read after
  `setItem`).
* **Ripen** (3843). `onTryHeal` (no effect => undefined; `-activate|target|ability: Ripen` for Berry
  Juice/Leftovers; `chainModify(2)` when `effect.isBerry`), `onChangeBoost` (doubles every present
  key of the relayed boost object in place, only for berry effects), `onSourceModifyDamage`
  (priority -1; argument 2 is the defender holding Ripen: consumes `abilityState.berryWeaken` and
  `chainModify(0.5)`), `onTryEatItem` (priority -1; `-activate`), `onEatItem` (assigns
  `berryWeaken` from the 18 resist-berry names). Berry Juice and the resist berries are not in the
  scoped dex: their ids resolve to `EffectId::NONE` at compile time and `is_named`/`is_listed` never
  match NONE, so only the in-scope branches are reachable. `berryWeaken` is custom bit 8 of the
  ability-state cell.
* **Unnerve** (5260). `onStart`: `unnerved` already set => return; else `-ability|mon|Unnerve` and
  set it; `onEnd`: clear it; `onFoeTryEatItem`: returns a real `Bool(!unnerved)` (`false` vetoes the
  foe's `eatItem`, `true` leaves the relay true). `unnerved` is custom bit 8 of the holder's ability
  state. `onSwitchInPriority: 1` has no function site (manifest metadata).

## Payload map

* `whiteherb`: `PAYLOAD_WORDS = 1`; word 0 = negative-key mask, presence bit 8 (item state).
* Gluttony / Ripen / Unnerve keep a single truthy flag as custom presence bit 8 of the ability
  state cell (`PAYLOAD_WORDS = 0`; no payload word is needed because the properties are only ever
  `true`/`false`/absent and every reader tests truthiness).
* Everything else is stateless.

## Core API assumptions

All APIs exist with these signatures at `76a95a3`; no signature change is requested.
`Battle::{eat_item, use_item, heal, add_volatile, cure_status, set_item, set_boost, is_weather,
get_nature, has_ability, move_hit_data, event_hook, chain_modify, scratch_boosts}` were used as
documented. `eat_item`/`use_item` default source and sourceEffect from the running event exactly
like `pokemon.ts:1768-1849`.

## Core defect found (needs an integrator fix; no core file was edited)

**`event/collect.rs`: Ally/Foe/Any listeners of Pokemon with hp 0 are still collected.**
`find_event_handlers` builds the Ally/Foe/Any lists with `living_actives(side)`, which filters on the
`FAINTED` flag. Showdown iterates `target.alliesAndSelf()` / `target.foes()` (`battle.ts:1056-1067`,
`side.ts:390-403`), which filter `!!ally.hp`. In the window between a KO (hp 0, `FAINT_QUEUED`) and
`faintMessages` (which sets `FAINTED`), the engine still runs that Pokemon's `onFoe*/onAlly*/onAny*`
handlers; Showdown does not.

Visible consequence in this batch: **Unnerve** (`onFoeTryEatItem`) keeps vetoing the foe's
berry after the Unnerve holder was knocked out by the move that is running. In Showdown the
`eachEvent('Update')` inside `hitStepMoveHitLoop` (battle-actions.ts:960) runs before `faintMessages`,
so the berry is eaten *before* `|faint|`; the engine eats it after the faint (and after Recoil), with
different HP when recoil intervenes. This is not Unnerve-specific: any hp-0 holder of a
Foe/Ally/Any handler is affected (e.g. the other batches' `onAny*`/`onFoe*`/`onAlly*` abilities when
their holder is KOed mid-action).

Minimal fix (verified: removes all 45/400 + 8/100 failures below; do **not** change
`living_actives` itself, `each_event` shares it and needs the `fainted` filter - doing so dropped
the pass rate from 41 to 28):

```diff
--- a/crates/engine/src/event/collect.rs   (the Ally/Foe loop, ~line 62)
                     for a in mons[..len].iter().copied() {
+                        // side.ts:392-393: allies()/foes() drop hp 0 Pokemon.
+                        if self.state.pokemon[a.0 as usize].hp == 0 {
+                            continue;
+                        }
                         self.find_pokemon_event_handlers(
```

Repros: committed scenario `unnerve_holder_ko_frees_berry` (replayed by the `#[ignore]`d test
`scenarios_blocked_on_core_event_collection`; remove the ignore once core is fixed), and
`difftest replay <batch:consumables corpus> --battle 77` (Aguav Berry + Gluttony Swalot KOs an
Unnerve Mewtwo with Head Smash). Showdown order: `-damage|Mewtwo|0 fnt`, `-enditem|Swalot|Aguav
Berry|[eat]`, heal, `|faint|Mewtwo`, recoil; engine: faint, recoil, then eat.

## Tests and validation

* `cargo build -j 2` and `cargo test -j 2` pass (registry/manifest test covers all 42 sites).
  clippy is clean for these files except the repo-wide `PAYLOAD_WORDS <= 4` assert lint present in
  every effect file.
* `items/consumables/tests.rs`: helper tests (flag bits, nature table, berry flags, White Herb boost
  table ordering) and `scenario_logs_and_prng_match_pinned_showdown`, which replays 15 scenarios
  against `scenarios.tsv` (raw log + final PRNG seed from the pinned Showdown): Sitrus pinch,
  Unnerve veto, Figy/Aguav (Gluttony) and Iapapa/Wiki (no Gluttony) confusion by nature, Mago with
  a neutral neighbour, Gluttony at 50%, Chesto vs sleep, Focus Sash, Weakness Policy, Throat Spray,
  White Herb vs Intimidate, Cheek Pouch + Sitrus, Ripen + Sitrus (doubled heal, `-activate`),
  Harvest in Sun (eat/restore loop) and Harvest without sun (draws). The sixteenth scenario
  (`unnerve_holder_ko_frees_berry`) is the core-defect repro above (`#[ignore]`d test).
  Regenerate with `node tools/probes/consumables/scenarios.mjs [showdown checkout]` (searches the
  first seed whose rolls trigger the required log lines; `--print NAME` shows one log).
* `tools/probes/consumables/inspect-battle.mjs FIXTURES POSITION [--mon NAME]` replays a directed
  fixture on the pinned Showdown and prints HP / item / `abilityState` of matching Pokemon before
  every step (used to find the Unnerve/hp-0 ordering above).

### End-to-end (`--profile batch:consumables`, replay with `difftest ... --sim engine`)

| run | battles | pass | other-batch panics | failures in this batch |
| --- | --- | --- | --- | --- |
| default seed, files exactly as committed | 100 | 41 | 53 (`abilities:naturalcure`, not ported yet) | 6, all the hp-0 defect |
| same, **excluding other-batch panics** | 47 | 41 (87.2%) | - | 6 |
| + Natural Cure borrowed from another worktree (not committed) | 100 | 92 | 0 | 8, all the hp-0 defect |
| + the one-line `collect.rs` fix (not committed) | 100 | **100** | 0 | 0 |
| seed 7771, 400 battles, committed files | 400 | 152 | 215 (Natural Cure) | 33, all the hp-0 defect |
| seed 7771 with Natural Cure + `collect.rs` fix | 400 | **400** | 0 | 0 |
| 11 per-effect corpora (`effect:items:chestoberry|whiteherb|throatspray|weaknesspolicy|focussash|sitrusberry`, `effect:abilities:ripen|harvest|cheekpouch|unnerve|gluttony`, 60 each) with the same two additions | 660 | **660** | 0 | 0 |

Handler coverage (Showdown side, `tools/oracle/coverage.mjs`, all corpora above): 40 of the 42
function sites are invoked; the two never reached are `items:whiteherb fling.effect` (Fling is not
in the scoped dex) and `items:whiteherb onAnyAfterMega` (no Mega Evolution in the format).

## Unresolved / for the integrator

* Apply the `collect.rs` hp-0 filter above, then un-ignore
  `scenarios_blocked_on_core_event_collection` (it should pass unchanged).
* Natural Cure (`abilities:naturalcure`) is the other-batch hook that panics in 53% of the
  `batch:consumables` battles; the files in the other worktrees were used only for the experiment
  above and are not part of this commit.
* Ripen's resist-berry branch (`berryWeaken` consumed by `onSourceModifyDamage`) is unreachable in
  the scoped dex (none of the 18 berries exists); it is implemented and flag-tested but has no
  Showdown-backed scenario.
