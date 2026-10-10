# effects(reactive_stats): damage/KO/boost-triggered stat changes and Ruin abilities

Completes the 27 files / 39 function sites of the `reactive_stats` batch against Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a` (`data/abilities.ts`). Every site is in `HOOKS`;
there are no waivers and no hook is a runtime no-op.

Abilities: Battle Bond, Beads of Ruin, Berserk, Chilling Neigh, Competitive, Dauntless Shield,
Defiant, Download, Gooey, Grim Neigh, Intimidate, Intrepid Sword, Justified, Moxie, Rattled,
Soul-Heart, Speed Boost, Stamina, Sword of Ruin, Tablets of Ruin, Tangling Hair, Thermal Exchange,
Vessel of Ruin, Water Compaction, Weak Armor, Well-Baked Body, Wind Rider.

## Provenance of the files (interrupted WIP, commit 45cbb1a)

Only the 27 key files were brought over from the interrupted agent's commit; the helper tree
(`reactivestats/host.rs`, `adapter.rs`, `support.rs`, the five test modules and ~46k lines of
`vectors/*.tsv`) was deliberately **not** taken. Each WIP file was read against the TypeScript and
then rewritten as a direct port: handlers call the `Battle` core APIs themselves (no `Host`
trait, no recording host), so each body reads next to its pinned source. Defects found in the WIP
while reviewing it, all fixed here:

* **Source defaulting.** WIP `BattleHost::boost` turned every omitted source into
  `EventArg::Null`. `this.boost({ ... })` omits the argument (undefined), and Showdown defaults
  it from `this.event.source`; `Attribution::DEFAULT` (Undefined) is used for those calls, and an
  explicit `Attribution::from_move(mon, EffectRef::None)` only where TS passes a Pokemon.
* **Berserk `onDamage` could panic.** It evaluated `h.mv(3).multihit` (requires a live
  `ActiveMove`) whenever the effect type was `Move`. Confusion self-hits, Struggle recoil,
  Recharge, fainted etc. are *synthetic* `Move`-typed effects with no ActiveMove, so a confused
  Berserk holder would have panicked. `effect_has_multihit` handles ActiveMove, dex move and
  synthetic effects.
* **Runtime string lookups on hot paths.** Move-type checks used `dex::type_id("Dark").unwrap()`
  (string scan per hit), Berserk compared `dex::effect(id).key` against nine names per eat event,
  Battle Bond compared species/move names. These are compile-time constants now
  (`type_named`, `optional_id`).
* `isAlly` was approximated with `side() ==`; the port uses `Battle::is_ally`.

## Layout

`battlebond.rs` owns the private helper module `reactivestats/mod.rs` (`pub(super) mod support`,
the same pattern as `aromaveil.rs`); the other 26 files import it as
`crate::effects::registry::abilities_battlebond::support`. Helpers: `boosts([...])` (ordered
sparse boost object), `by(mon)`, `owner`, `mon/mon_opt/effect_at/effect_id_at` argument readers,
`effect_is_move/effect_has_multihit/effect_has_status`, log line builders, and the two shared
Ruin bodies. No allocation, no strings built, no PRNG access anywhere in the batch.

## Per-effect notes (TS file:line, relay values, draws)

No handler draws directly. "boost events" means the call goes through `Battle::boost`, whose
ChangeBoost/TryBoost/AfterEachBoost/AfterBoost listeners may speed-sort ties (core-owned draws).

* **Intimidate** (2195, hard). `adjacentFoes()` captured once; first foe prints
  `-ability|mon|Intimidate|boost` (only once); Substitute holders get `-immune`, others
  `boost({atk:-1}, target, pokemon, null, isSecondary=true)`.
* **Beads/Sword/Tablets/Vessel of Ruin** (385-5299, hard). `onStart` prints `-ability` unless
  `suppressingAbility(pokemon)`. The `onAny*` bodies keep the exact asymmetry: Beads/Sword test
  `target.hasAbility` and re-validate the stored holder
  (`!move.ruinedX?.hasAbility(...)`), Tablets/Vessel test `source.hasAbility` and only check
  presence (`!move.ruinedX`). Holder identity is `ActiveMove::ruined_stats[atk,def,spa,spd]`;
  all four return undefined after `chain_modify(0.75, 1)`. Two holders of one Ruin ability tie in
  the ModifyX speed sort (core draw).
* **Berserk** (415-439). `checkedBerserk` is tri-state (absent/false/true):
  payload word 0, custom presence bit 8. `onDamage` assigns it from the damage effect;
  `onTryEatItem` returns that value (Undefined when never assigned, which leaves the relay `true`)
  only for the nine healing berries/juice, else `true`; `onAfterMoveSecondary` sets it `true`,
  then applies the half-HP crossing test with `lastAttackedBy.damage` (`last_damage`) or
  `move.totalDamage` for non-smart multihit.
* **Moxie / Chilling Neigh / Grim Neigh** (2751/502/1718). `AfterFaint` args are
  `(length, target, source, effect)`; only a Move-typed effect counts; boost `length` (relay
  number) with defaulted source/effect.
* **Battle Bond** (363-378). `bondTriggered` is `mon_flags::BOND_TRIGGERED`; order is boost,
  `-activate`, then flag. `onModifyMove` needs Water Shuriken *and* Greninja-Ash, neither of
  which is in the scoped dex (06-scope.md: greninjaash rejected), so the compare uses
  `optional_id` and can never match; the hook is still reached for every Greninja-Bond move, so
  it is implemented rather than waived.
* **Competitive / Defiant** (646/902). Source null or ally => return; scans the single-key
  `currentBoost` for a negative; self boost uses `(target, target, null, isSecondary=false,
  isSelf=true)`.
* **Rattled** (3773/3778). Type in {Dark, Bug, Ghost} on hit; `onAfterBoost` fires for the whole
  boost object only when the effect is the ability Intimidate and `boost.atk` is present and
  non-zero (a Clear Amulet style `delete boost.atk` must suppress it).
* **Dauntless Shield / Intrepid Sword** (854/2215). `shieldBoost`/`swordBoost` are
  `mon_flags::SHIELD_BOOST/SWORD_BOOST`, set before the boost.
* **Download** (1018). Sums unmodified `getStat` of every living foe (`unboosted=false,
  unmodified=true`; 2 reads per foe, no events), `totaldef && totaldef >= totalspd` => SpA else
  `totalspd` => Atk.
* **Gooey / Tangling Hair** (1644/4907). `check_move_makes_contact(move, source, target, true)`
  announces Protective Pads; then `-ability` and `boost({spe:-1}, source, target, null, true)`.
* **Justified / Stamina / Water Compaction / Weak Armor / Thermal Exchange hit** use the
  overlay `move_type`/`category` (so type-changing abilities are honored). Weak Armor boost keys
  apply in the order `def`, `spe`. Stamina has no condition and a defaulted source (the attacker).
* **Soul-Heart** (4429). Boosts `effectState.target` (the holder) on any faint.
* **Speed Boost** (4457). `activeTurns` truthiness; Residual order 28 / sub-order 2 is manifest
  metadata.
* **Thermal Exchange** (4993-5010). `onUpdate` prints `-activate` then `cure_status(false)`;
  `onSetStatus` returns `false` for every burn and prints the `[from]` immune line only when the
  effect's own `status` field is set (`effect_has_status`: ActiveMove overlay, dex move, or the
  Synchronize source object).
* **Well-Baked Body / Wind Rider** (5462/5536). `TryHit` returns `null` for Fire / wind moves from
  another Pokemon; the `-immune` line is printed only when `boost` returns a falsy relay
  (`Null`, `Number(0)`, `Bool(false)` all count). Wind Rider `onStart` reads Tailwind via
  `get_side_condition`; `onSideConditionStart` compares the canonical id of arg 2.

## Payload map

Berserk only: `word0 = checkedBerserk (0/1)`, presence bit 8. Everything else is stateless
(`PAYLOAD_WORDS = 0`); the once-per-battle flags use existing `mon_flags` bits.

## Core API assumptions the integrator must check

All APIs exist with these signatures; several still have `todo!()` bodies in e784f91 and are being
implemented by other owners. No signature change is requested.

* `Battle::boost(boosts, target: Option<MonId>, Attribution, is_secondary, is_self)` must apply
  `target ||= event.target`, `source ||= event.source`, `effect ||= this.effect` (battle.ts:2028).
  `Attribution.effect == EffectRef::None` means "null"; Battle Bond passes
  `EffectRef::Dex(ABILITY_BATTLEBOND)` for `this.effect`. Its Relay is truthy only for `true`
  (`success`), so `0`/`false`/`null` returns all trigger the `-immune` lines.
* `Relay::Boosts(slot)` for `AfterEachBoost`/`AfterBoost` arg 0 (read via `scratch.boosts`
  without consuming it); `Relay::Effect(id)` (or `EventArg::Effect`) for `item` of `TryEatItem`
  and `status` of `SetStatus`; `EffectRef::None` for null effects; Move-typed synthetic effects
  (`event_effect_type`) for confusion/recoil/recharge.
* `ActiveMove` overlay: `ruined_stats` initialised to `MonId::NONE`; `MULTIHIT_PRESENT`
  (+`MULTIHIT_RANGE` for arrays) set from dex multihit so `effect.multihit` truthiness works;
  `SMART_TARGET` set only while truthy; `total_damage` is 0 for `false`.
* `Battle::is_ally`, `get_side_condition`, `cure_status`, `check_move_makes_contact` (implemented),
  `adjacent_foes/foes`, `get_volatile`, `get_stat`, `suppressing_ability/secondaries`,
  `state.last_attacked_by` (latest record, numeric `last_damage`).
* `AfterFaint` must deliver `(length, target, source, effect)` with `effect` already resolved from
  the queued `EffectToken`.

## Tests and validation

* Registry/manifest test passes (all 39 sites covered by `HOOKS`); `cargo build -j 2` and
  `cargo test -j 2` pass; `cargo clippy` is clean for these files apart from the repo-wide
  `PAYLOAD_WORDS <= 4` assert lint present in every effect file.
* `reactivestats/tests.rs`: helper unit tests (type constants vs the generated type table, boost
  object ordering/presence), and `scenario_teams_load_without_touching_the_prng` which builds all
  nine scenario teams through the real `Battle` constructor.
* `scenario_logs_and_prng_match_pinned_showdown` (`#[ignore = "needs core"]`) replays nine
  scenarios and compares the raw log and final PRNG state with Showdown: Intimidate with
  Defiant/Competitive, Intimidate with Rattled and Download, Moxie + Soul-Heart on a KO (with
  forced switch), Gooey/Weak Armor/Stamina on contact and spread hits, Wind Rider with Tailwind
  and Hurricane, Well-Baked Body and Thermal Exchange vs Fire / Will-O-Wisp, Berserk crossing half
  HP after recoil, Speed Boost across turns, and the four Ruin abilities in one damage turn.
  Regenerate with `node tools/probes/reactive_stats/scenarios.mjs [showdown checkout]`
  (uses `tools/oracle/lib/session.mjs`, so it also fails if the oracle build drifts).

## Unresolved / for the integrator

* Replay the ignored scenario test once start/choose/boost/damage/log land; the Berserk scenario
  is the best check of the `Damage` -> `AfterMoveSecondary` ordering relative to recoil.
* Battle Bond's Greninja-Ash `onModifyMove` cannot be exercised in this format; only its absence
  of panics is checked.
* Water Veil-style `(effect as Move)?.status` handling is duplicated in
  `passivedefense::set_status` (`status_field`); both agree on ActiveMove / dex move / Synchronize.
