# effects(formes_tera)

Ports all 48 records / 85 function sites against Showdown
`7332b60e22b9e8194bb53549549eba241d73cc9a`. Every function site is implemented in
`HOOKS`; `WAIVERS` is empty throughout. All payloads use at most two words. No
handler allocates, builds strings, or draws directly from the PRNG. Nested core
calls preserve the source event ordering and own their draws.

The branch merged `engine` at `76a95a3` (merge `aaa5be5`). The pinned Showdown
checkout was read only. Initial ports were committed in `0375148` and `3301d4b`;
the final core integration correction is `53fd2d1`.

## Files and layout

Canonical files under `crates/engine/src/effects/`:

* `species/`: `arceus`, `arceusbug`, `arceusdark`, `arceusdragon`, `arceuselectric`,
  `arceusfairy`, `arceusfighting`, `arceusfire`, `arceusflying`, `arceusghost`,
  `arceusgrass`, `arceusground`, `arceusice`, `arceuspoison`, `arceuspsychic`,
  `arceusrock`, `arceussteel`, `arceuswater`, `zacian`, `zaciancrowned`,
  `zamazenta`, `zamazentacrowned`.
* `moves/`: `aurawheel`, `ivycudgel`, `judgment`, `relicsong`, `revelationdance`,
  `terablast`, `terastarstorm`.
* `abilities/`: `disguise`, `embodyaspectcornerstone`, `embodyaspecthearthflame`,
  `embodyaspectteal`, `embodyaspectwellspring`, `gulpmissile`, `hungerswitch`,
  `iceface`, `libero`, `protean`, `protosynthesis`, `quarkdrive`, `shieldsdown`,
  `teraformzero`, `terashift`, `zerotohero`.
* `items/boosterenergy.rs`, `conditions/protosynthesis.rs`,
  `conditions/quarkdrive.rs`.

The private `abilities/formestera/` directory contains test support and checked-in
oracle TSVs. `protosynthesis.rs` anchors that test module. Production callbacks
reuse the already merged Battle Bond argument/boost helpers. Arceus and Crowned
aliases delegate to canonical species bodies with their distinct IDs and HookIds.
Tera Shell has no function site in this batch; its damage mechanic is core-owned.

## Semantics and source references

Exact callback line ranges and PRNG notes are also beside each dispatch arm.

| Effect | Source and behavior |
| --- | --- |
| Arceus and 17 typed views | `data/conditions.ts:853-863`, inherited by `sim/dex-species.ts:529-536`. Read raw ability and item, regardless of suppression. Transformed/non-Multitype users return the original Types relay; Multitype returns one plate type or Normal. |
| Zacian / Zamazenta and Crowned views | `data/conditions.ts:882-936`, inherited species view routing. Matching Rusted item permanently changes the species and ability; replace Iron Head in base/current slots with Behemoth Blade/Bash and calculated PP. Source startup looks up the condition by current species ID: already-Crowned packed input retains Iron Head. Both inherited function bodies still route to their canonical implementation when directly dispatched. Four oracle startup fixtures verify this distinction. |
| Aura Wheel | `data/moves.ts:792-807`. Only Morpeko's base species may use it; failure logs and returns Null. Current Hangry species sets Dark, otherwise Electric. |
| Ivy Cudgel | `data/moves.ts:9768-9785`. Animation uses original set Tera type; current Ogerpon mask/forme selects Grass, Water, Fire, or Rock. Unrelated species retain the input move type. |
| Judgment | `data/moves.ts:9827-9833`. Plate lookup honors item suppression, unlike Arceus's species callback. |
| Relic Song | `data/moves.ts:14947-14952`. Successful secondary-self callback alternates Meloetta/ Pirouette only when not transformed. Sleep chance and draws stay in move execution. |
| Revelation Dance | `data/moves.ts:15042-15048`. Select the user's first current type; skip leading ??? where another type exists. |
| Tera Blast | `data/moves.ts:19208-19236`. Live basePower callback returns 100 for Stellar Tera, otherwise the overlay power. Animation uses set Tera type. Terastallized users use that type and Physical only when unmodified Atk exceeds SpA. Stellar replaces the self object with ordered Atk/SpA drops; it does not retain static self hooks. |
| Tera Starstorm | `data/moves.ts:19249-19261`. Stellar Terapagos changes type, stat-dependent category, and spread target based on current species. |
| Disguise | `data/abilities.ts:972-1008`. Damage returns numeric zero and marks busted; critical guard returns false, effectiveness guard zero. Substitute/bypass/infiltration and immunity branches are distinct. Update permanently changes forme, then deals maxHP/8 with Mimikyu-Busted attribution. Synthetic Move effects can break Disguise without an ActiveMove. |
| Ice Face | `data/abilities.ts:1972-2018`. Absorbs Physical hits only; same distinct guard sentinels. Update changes to Noice; Start/WeatherChange restore in snow, reset busted to false, and preserve weather-suppression/zero-HP guards. Synthetic confusion has no Physical category. |
| Four Embody Aspect abilities | `data/abilities.ts:1225-1286`. Exact base Tera forme plus terastallization required; set embodied before the boost. Teal boosts Spe, Wellspring SpD, Hearthflame Atk, Cornerstone Def. |
| Gulp Missile | `data/abilities.ts:1747-1765`. Surf's TryPrimaryHit changes forme even before damage success; <= half HP chooses Gorging. Hit response checks attacker HP/active state and target semi-invulnerability, damages the attacker by maxHP/4, lowers Def or paralyzes, then restores Cramorant, preserving source attribution and order. Dive is absent from the scoped move dex. |
| Hunger Switch | `data/abilities.ts:1899-1903`. Residual toggles current Morpeko species only while not terastallized. |
| Protean / Libero | `data/abilities.ts:3499-3508` / `2530-2539`. Once per ability state, skip bounced moves and unknown type, compare current types, and set used only after setType succeeds. Same-type attempts do not spend activation. Tera rejection belongs to setType. |
| Protosynthesis / Quark Drive abilities | `data/abilities.ts:3516-3530` / `3653-3666`. Start invokes the environment single event with captured state. Active environment adds the volatile; loss removes non-Booster activation. Ability End erases the volatile without its End callback and emits the silent ability-end line, including at zero HP. |
| Paradox conditions | `data/abilities.ts:3533-3574` / `3669-3710`. Cache getBestStat(false,true) once; respect tie order and boosts. Booster origin adds the fromitem activation tag. Recheck ability suppression on every stat query; non-speed modifier is exactly 5325/4096, speed 3/2. Ordinary removal logs End. |
| Booster Energy | `data/items.ts:629-646`. Start sets started then directly invokes Update with the same event/effect/state. Update excludes transformed users; weather/terrain prevents consumption. The two ability branches remain separate if statements after useItem. TakeItem reads raw baseSpecies Paradox tags. |
| Shields Down | `data/abilities.ts:4242-4279`. Start/Residual compare maxHP/2 and restore the original set species below threshold. Transformed users are excluded. Meteor status rejection returns false; Yawn rejection returns Null. Only effects with their own status field get the status immune log. |
| Tera Shift | `data/abilities.ts:4968-4974`. SwitchIn, rather than Start, activates then permanently changes Terapagos to Terastal. |
| Teraform Zero | `data/abilities.ts:4946-4953`. Requires Stellar baseSpecies. If weather or terrain exists, announce once, clear weather, then terrain. |
| Zero to Hero | `data/abilities.ts:5629-5642`. SwitchOut permanently changes Palafin and clears the Pokemon hero-message flag; SwitchIn announces once and sets it. The flag survives ability-state replacement. |

No whole-function waivers or no-op substitutes were needed. Closed-format branch
restrictions are explicitly documented in code/probes: Mimikyu Totem, hail, Bird
type, Dive, Future Sight/Doom Desire, Snatch and all seven callsMove moves are
absent from the generated gen9 scope. Protean/Libero retain the reachable bounce
guard. The probes assert the relevant excluded move IDs remain absent.

## Payload map

| Files | Words and custom presence bits |
| --- | --- |
| Disguise / Ice Face | word 0: busted boolean; bit 8. |
| Four Embody Aspect files | word 0: embodied boolean; bit 8. |
| Protean / Libero | word 0: used boolean; bit 8. |
| Booster Energy | word 0: started boolean; bit 8. |
| Two Paradox conditions | word 0: fromBooster boolean, bit 8; word 1: bestStat (Atk=0, Def=1, SpA=2, SpD=3, Spe=4), bit 9. Absence remains distinct from false. |
| Remaining files | Zero words. Pokemon HP/types/forme/PP and HERO_MESSAGE use existing core state. |

## Core integration

No unresolved API request or `todo!` remains in these effects. Under the user's
explicit permission for small core corrections, commit `53fd2d1` changes only
`actions/mutators/pokemon.rs` outside the effects/test/report tree:

* Added `Battle::calculate_pp(move_id: EffectId, pp_ups: u8) -> u8`, porting
  `sim/battle.ts:2378-2383` (including noPPBoosts), to replace the earlier documented
  missing-API todo in the two Crowned species bodies. PP Ups are bounded 0..3;
  scoped PP is integral and fits u8. Four Crowned fixture battles check the result.
* Fixed permanent forme-change details logging. The text formatter already
  includes the real mon's Tera suffix and illusion view. The mutator's additional
  suffix produced `..., tera:Dark, tera:Dark`. It now sends `Details(pokemon)` once.
  Repro: `/tmp/formes.jsonl.gz --battle 1`, run seed 1114, turn 1, permanent Disguise
  change after Tera. This accounted for all 13 initial non-panic divergences.
  Investigation used the battle repro and `tools/oracle/trace.mjs`; the trace tool
  writes useful event rows but its random-team header regeneration rejects these
  directed teams, so the difftest repro was the authoritative replay check.

## Validation and remaining integration checks

`cargo build -j 3` and `cargo test -j 3` pass. The registry/manifest test and the
batch's independent site-count test require all 85 function sites implemented.
The following pinned Node probes regenerate only worktree fixtures:

```sh
node tools/probes/formes_tera/callbacks.mjs
node tools/probes/formes_tera/scenarios.mjs
cargo test -j 3 --lib abilities_protosynthesis -- --include-ignored
```

All 27 tests in that explicit run pass: eight callback/registry/team checks and
19 full battle fixtures. Callback tables cover 49 move overlays, 40 Paradox
activation/modifier/end combinations, 434 Disguise/Ice Face guard cases, 144
Arceus views, four TakeItem base-species cases, and 16 Shields Down status/Yawn
cases. Full battles compare complete protocol logs and final PRNG state for
sun-to-Booster, electric terrain-to-Booster plus Teraform Zero, Disguise, snow
restoration, Morpeko, Palafin, Relic Song, four Ogerpon Tera forms, Stellar Tera
Blast, Shields Down, four Crowned inputs, and both Gulp Missile forms. Fixture
seeds are `[101 + row_index,202,303,404]`; packed teams/choices and oracle logs are
checked into `formestera/scenarios.tsv`. Battle tests retain the requested
`#[ignore = "needs core"]` annotation and were explicitly run after the merge.

Requested end-to-end gate:

```sh
node tools/oracle/gen-directed.mjs --profile batch:formes_tera --count 100 --threads 2 --out /tmp/formes.jsonl.gz --verify
cd crates/difftest
cargo build --release -j 3
./target/release/difftest replay /tmp/formes.jsonl.gz --sim engine --jobs 3 --top 20
```

Run seed **1114**: all 100 oracle battles verified; engine **67 passed / 33
other-batch panics**, zero remaining log/request/PRNG divergences. Every exclusion
is `abilities:naturalcure onCheckShow` (hook 340). Adjusted pass rate:
**67/67 = 100%**. Machine-readable replay is `/tmp/formes-replay.json`.

Combined oracle coverage for both requested corpora replayed all 200 oracle
battles exactly. This batch invokes **53/85** sites in that random corpus; the
supplemental fixtures/callbacks target the absent species, Embody Aspect,
Terapagos, weather-change, TakeItem, status and condition-End branches. Coverage
does not imply every branch was exercised in a complete battle. Already-Crowned
startup intentionally does not call an inherited condition hook in the pinned
engine. After the other batch lands, rerun the 33 excluded Natural Cure battles.

The common core regression corpus `data/fixtures/slice0-200.jsonl.gz` also passes
**200/200**, with 4,357 matched steps and 88,208 matched log lines.
