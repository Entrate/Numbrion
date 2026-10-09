# 04 — Battle state model (Gen 9 Random Doubles)

Source of truth: `/home/aminaliu/src/pokemon-showdown` at commit `7332b60e` (2026-10-08).
Paths below are relative to that checkout. `P:` = `sim/pokemon.ts`, `B:` = `sim/battle.ts`,
`S:` = `sim/side.ts`, `F:` = `sim/field.ts`, `BA:` = `sim/battle-actions.ts`, `Q:` = `sim/battle-queue.ts`,
`C:` = `data/conditions.ts`, `M:` = `data/moves.ts`, `A:` = `data/abilities.ts`, `I:` = `data/items.ts`.

This document lists every piece of mutable state a Rust port must hold to reproduce Showdown
bit-for-bit (same protocol log, same PRNG call sequence) for format `gen9randomdoublesbattle`
(`config/formats.ts:165-172`), plus the rules that mutate it. Event dispatch, move execution,
damage and turn flow are specified in sibling docs; here they appear only where they read or
write state.

---------------------------------------------------------------------------------------------------

## 0. Scope, method, legend

### 0.1 Format facts that shrink the state

| Fact | Where |
|---|---|
| `mod: 'gen9'`, `gameType: 'doubles'`, `team: 'random'`, `playerCount 2`, no `battle`/`pokemon`/`side`/`field`/`actions`/`queue` overrides. `data/scripts.ts` is just `{gen: 9}`. | `config/formats.ts:165`, `data/scripts.ts` |
| Rule table: `potd, obtainable(+obtainablemoves/abilities/formes/misc, evlimit, -unreleased, -tag:unobtainable, -nonexistent), speciesclause, hppercentagemod, cancelmod, illusionlevelmod, sleepclausemod`. **No `endlessbattleclause`, no `teampreview`**, `pickedTeamSize` null, `maxTeamSize` 6. | dumped via `Dex.formats.getRuleTable` |
| Only `sleepclausemod` has a battle-time handler (`onSetStatus`, `data/rulesets.ts:1387`). The Battle constructor therefore inserts it as a **permanent pseudo-weather** (`B:296-308`); it is always the first key of `field.pseudoWeather`. | `B:296-308` |
| `hppercentagemod.onBegin` sets `battle.reportPercentages = true`; `cancelmod.onBegin` sets `battle.supportCancel = true`; `illusionlevelmod` is read in `P:549` (`getFullDetails`) and `A:2084`. | `data/rulesets.ts:1353-1378, 2923` |
| `gen === 9` everywhere ⇒ every `gen <= 8` branch is dead. `side.dynamaxUsed = (gen !== 8) = true` ⇒ Dynamax impossible. `canMegaEvo/canUltraBurst` are always `null` (no mega stone/Z-crystal/Ultranecrozium in the generator item pool; Mega Rayquaza needs gen≤7 or `+tag:past`). | `S:264`, `BA:1856-1883` |

### 0.2 What content is reachable (used to mark fields DROP)

Reachability was initially investigated three ways (scratch scripts in `/tmp/numbrion-scratch`, preserved in the recovery research archive):

1. **Generator census**: 36 000 sets from `Teams.getGenerator('gen9randomdoublesbattle')` →
   357 distinct moves, 199 abilities, 62 items (+ no-item), 556 species/formes, levels 62–100.
2. **Static scan** of all reachable moves/abilities/items/conditions for `addVolatile`,
   `addSideCondition`, `addSlotCondition`, `addPseudoWeather`, `setWeather`, `setTerrain`,
   `setStatus` and `effectState.*` usage.
3. **Dynamic census**: 3 200 complete random-choice battles (≈64 000 turns, 0 crashes) with a hook
   after every `runAction` recording every volatile/side/slot/field condition id and every key in
   every `EffectState`, plus field maxima (§7.1).

Not reachable with generator teams (so the corresponding state is DROP): Commander
(Tatsugiri only gets Storm Drain), Neutralizing Gas, Gravity, Wonder Room, Magic Room, Misty
Terrain, Hail, primal weathers, Delta Stream, Future Sight/Doom Desire, Healing Wish/Lunar Dance,
Sky Drop, Ally Switch, Skill Swap/Role Play/Entrainment, Pursuit, Assurance, Payback, Bolt Beak,
Fishious Rend, Core Enforcer, Electrify, Mat Block, Metronome/Copycat/Sleep Talk/
Assist/Me First, Outrage-family (`lockedmove`), Belch/Stuff Cheeks, Retaliate, Conversion 2,
Hidden Power, Return/Frustration, Emergency Exit/Wimp Out, Eject Button/Pack, Red Card,
Utility Umbrella, Air Balloon, Mirror Herb, Grip Claw, Binding Band, Iron Ball, Float Stone,
Stance Change, Zen Mode, Schooling, Power Construct, Forecast/Flower Gift, Supersweet Syrup,
Supreme Overlord, Cud Chew, Opportunist, Pickup, Gorilla Tactics, Forest's Curse/Trick-or-Treat.
If the differential fuzzer ever feeds **non-generator** teams, revisit §11.

The final 50,000-team scope run in `data/scope.json` supersedes the smaller generator census:
357 generated moves, 201 generated abilities, 61 non-empty generated items plus no-item,
556 distinct set.species strings (558 canonical species records after engine normalization).
Its conservative closure has 572 species, 359 moves, 207 abilities, 61 items and 75 condition
records. The two late abilities are Solar Power and Sand Force; both use existing stat/weather
state and do not require additional per-effect fields. Sampling maxima are observations, not
proofs of a container bound. Use the closed scope rather than a census alone to justify DROP.


### 0.3 Legend used in the tables

| Tag | Meaning |
|---|---|
| **CORE** | Affects mechanics or PRNG consumption. Must be in Rust state. |
| **LOG** | Only influences protocol text (`battle.log`). Needed for log parity; keep. |
| **REQ** | Only influences request JSON / choice validation errors (`sideupdate`). Keep if requests are compared or choices are validated by the engine. |
| **CONST** | Fixed for the whole battle (put in an immutable per-battle team table, not in the hot state). |
| **DERIVE** | Exactly recomputable from other stored state; do not store (or store as a cache that is never authoritative). |
| **XACT** | Transient: only alive inside one action/event; always empty/null at a request boundary. |
| **DROP** | Write-only, never read, or only read by unreachable content in this format. |

"Active block" = state that `clearVolatile()` resets, which can live in a per-active-slot struct
(see §10).

---------------------------------------------------------------------------------------------------

## 1. Cross-cutting conventions (read this first)

### 1.1 Identity and references

* A **Pokémon reference** in Showdown is an object pointer. Store it as a stable
  `PokemonId = side*6 + original_team_index` (u8, 0..11). **Never** use `position` as identity:
  `side.pokemon` is permuted by every switch (`BA:118-132`).
* `pokemon.position` is the index in `side.pokemon`. Invariant: `side.pokemon[p.position] === p`
  (for all p), and after battle start `side.active[k] === side.pokemon[k]` for k∈{0,1}
  (`BA:118-136`). The fainted Pokémon stays in `side.active[k]` until replaced.
* A **slot** (`PokemonSlot`, `P:520-524`) is the string `${side.id}${'ab'[position]}`
  (`p1a,p1b,p2a,p2b`); store as u8 0..3. `getAtSlot(slot)` = `side.active[position]` (`B:1615-1621`).
* `getFieldPositionValue()` = `side.n + 2*position` (`P:526-529`): p1a=0, p2a=1, p1b=2, p2b=3.
* `getAllActive()` order is `p1a, p1b, p2a, p2b`, skipping fainted unless `includeFainted`
  (`B:1369-1379`). Many lists (e.g. handler collection, `eachEvent`) start in this order, and the
  starting order matters because ties are resolved by `prng.shuffle` over the tied sub-range.
* `side.foes()` = `foe.active.filter(p => p && p.hp)` in position order (`S:390-403`);
  `side.allies()` = same on own side. `randomFoe()` = `sample(foes())` (`S:369-373`).

### 1.2 Ordered maps — JS object insertion order is load-bearing

`pokemon.volatiles`, `side.sideConditions`, `side.slotConditions[i]`, `field.pseudoWeather` are
plain objects iterated with `for…in`. All keys are non-numeric strings, so iteration order is
**insertion order**; `delete` + re-insert moves a key to the end. Handler collection
(`findPokemonEventHandlers` `B:1101-1160`, `findSideEventHandlers` `B:1220-1235`,
`findFieldEventHandlers` `B:1185-1218`) walks these maps in that order, and `speedSort`
(`B:433-464`) shuffles each run of tied handlers **in place, in list order** — so the insertion
order changes which element the PRNG puts where. Rust: use an insertion-ordered small vector
(`ArrayVec<(Id, State)>`, remove = shift-left, add = push) or per-id slots plus a monotonically
increasing insertion stamp; never a hash map or a bitset alone.

Order is also mutated wholesale by: `copyVolatileFrom` (Baton Pass / Shed Tail, `P:1246-1268`:
copies in source order) and Court Change (`M:3041-3094`: removes the swappable ids from both
sides, then appends the other side's ones in their original order; non-swappable ids such as
`wideguard` keep their place).

Sparse boost tables (`SparseBoostsTable`) are also ordered objects: `battle.boost()` iterates the
caller's literal order (`B:2042`), e.g. Shell Smash `{def:-1, spd:-1, atk:2, spa:2, spe:2}`.
Message order and `AfterEachBoost` (Defiant/Competitive) order follow it. `getCappedBoost`
(`P:1210-1219`) preserves order and drops entries whose requested delta is 0.

### 1.3 Numeric helpers (exact semantics)

```text
trunc(x, bits=0)      = bits ? ToUint32(x) % 2**bits : ToUint32(x)          // sim/dex.ts:365-368
                        ToUint32: truncate toward 0, then mod 2^32 (negatives wrap!)
clampIntRange(x,a,b)  = (typeof x !== 'number' ? 0 : floor(x)), then clamp   // lib/utils.ts:320-326
modify(v, num, den=1) = m = trunc(num*4096/den); trunc((trunc(v*m) + 2047) / 4096)   // B:2336-2347
chainModify(n, d=1)   : event.modifier = ((trunc(event.modifier*4096) * trunc(n*4096/d) + 2048) >> 12) / 4096   // B:2325-2334
runEvent epilogue     : if relayVar is a non-negative integer: relayVar = modify(relayVar, event.modifier)  // B:933-937
```

### 1.4 `EffectState` — the universal per-effect record (`P:34-39`)

```ts
interface EffectState { id: string; effectOrder: number; duration?: number; [k: string]: any }
```

Created **only** through `initEffectState` (`B:3320-3330`):

```text
initEffectState(obj, effectOrder?):
  if !obj.id: obj.id = ''
  if effectOrder !== undefined:            obj.effectOrder = effectOrder
  elif obj.id && obj.target && (!(obj.target is Pokemon) || obj.target.isActive):
                                           obj.effectOrder = battle.effectOrder++   // POST-increment: first value is 0
  else:                                    obj.effectOrder = 0
```

Consequences you must reproduce (verified at battle start: the 4 leads get ability/item
effectOrders 0..7, counter ends at 8):

| Creation site | Increments `battle.effectOrder`? |
|---|---|
| `switchIn`: `abilityState`, then `itemState` (`BA:141-142`, after `isActive=true`) | ability: yes; item: yes iff item ≠ '' |
| `setAbility` (`P:1925`), `setItem` (`P:1883`), `skillSwap` (`B:1341-1342`) | yes if holder active and id ≠ '' |
| `setStatus` non-empty (`P:1732`) | yes (holder is always active) |
| `addVolatile` (`P:1999`) | yes if holder active |
| `copyVolatileFrom` (`P:1253`): `{...old, target: this}` while the receiver is **not yet active** | **no — effectOrder is overwritten with 0** |
| `addSideCondition` (`S:426`), `addSlotCondition` (`S:478`) (target = Side) | yes |
| `setWeather` (`F:70`), `setTerrain` (`F:141`), `addPseudoWeather` (`F:200`) (no `target`) | no, effectOrder 0 |
| Pokemon constructor states, `formatData`, `{}` temporaries in `singleEvent/runEvent` | no, 0 |

`effectOrder` is only consulted (a) by `resolvePriority` for callbacks whose name ends in
`SwitchIn` or `RedirectTarget` (`B:998-1004`), (b) by `compareRedirectOrder` via
`effectHolder.abilityState.effectOrder` (`B:417-423`, used by `priorityEvent`), (c) by the Dancer
sort (`BA:331-332`). But because it is a global counter, every increment above must be replayed.

`clearEffectState(state)` (`B:3332-3343`): `id = ''`, `effectOrder = 0`, keep `target`, delete all
other keys. Used by `takeItem/useItem/eatItem` on `itemState`, `clearWeather`, `clearTerrain`.

Standard keys that may appear on any state: `id`, `effectOrder`, `target` (mutable holder reference; see the warning below), `source` (PokemonId), `sourceSlot` (slot u8), `sourceEffect` (effect ref), `duration` (u8),
`name` (volatiles only; equals the condition name → DERIVE), `isSlotCondition` (slot conds; DERIVE),
`linkedPokemon`/`linkedStatus` (linked volatiles). Effect-specific keys are listed per effect below.

**Do not assume every state.target is an immutable owner.** `runEvent` writes
`this.effectState.target = effectHolder` (`B:906`) before calling a function handler.
A targetless Condition subsequently held by Field changes default subOrder from 2 to 5; a
slot condition initially targeting Side changes from 3 to 2 if dispatched with a Pokémon
holder (`B:976-986`). `clearEffectState` retains that target. The reference layout must retain
an optional tagged target, or prove the target history irrelevant for each specialized
condition. A Pokemon status/ability/item target can normally be inferred from its holder;
that is not a blanket rule for field/slot conditions. Trick Room itself has only Field callbacks
and a duration handler, so no generic `runEvent` callback mutates its target in this scope
(`M:19940-19978`).

`duration` semantics: decremented only in `fieldEvent('Residual')` (`B:519-527`) for handlers
that have an `end` callback and a truthy `state.duration`; reaching 0 calls the end function
(`removeVolatile`, `clearStatus`, `removeSideCondition`, `removeSlotCondition`, `clearWeather`,
`clearTerrain`, `removePseudoWeather`) instead of the residual callback. A state with
`duration` but no `onResidual` still gets a handler entry when `getKey==='duration'`
(`B:1106, 1115, 1123, 1130, 1147, 1193, 1202, 1210, 1227`).

---------------------------------------------------------------------------------------------------

## 2. Immutable per-Pokémon set data (what the generator produces)

`randomSet` (`data/random-battles/gen9/teams.ts:1498-1640`) returns
`{name, species, speciesId, gender, shiny, level, moves, ability, evs, ivs, item, teraType, role}`.

| Set field | Value in this format | Notes |
|---|---|---|
| `name` | `species.baseSpecies` (e.g. `Greninja` for Greninja-Bond) | becomes `pokemon.name` (≤20 chars), `fullname = "p1: Name"` (`P:334-335`) |
| `species` | forme string (cosmetic formes sampled by `getForme`, `teams.ts:1470-1496`) | |
| `level` | 62..100, from `doubles-sets.json` `"level"` (all 501 entries have one) | `getLevel`, `teams.ts:1442-1468`; constructor clamps to [1,9999] (`P:337`) |
| `evs` | 85 in every stat, except: `atk:0` when no physical move (`teams.ts:1600-1611`); `spe:0` with Gyro Ball/Trick Room (`:1613-1616`); HP lowered in steps of 4 only for Substitute+Sitrus, Minior, Belly Drum/Fillet Away/Shed Tail+Sitrus/Gluttony, Substitute+Endeavor (`:1568-1593`; the "Stealth Rock" branch `break`s immediately in doubles). Observed HP EVs: 85, 81, 69. | `trunc(ev/4)`: 85→21, 81→20, 69→17, 0→0 |
| `ivs` | 31 everywhere; `atk:0` together with `evs.atk:0`; `spe:0` together with `evs.spe:0` | |
| `nature` | **absent** → `dex.natures.get(undefined)` = `EMPTY_NATURE` (`sim/dex-data.ts:163, 189`), no `plus`/`minus` → **neutral** | |
| `gender` | `'M'`/`'F'` (random) or species gender; `'N'` → stored as `''` (`P:340-341`) | always set ⇒ the constructor's `battle.sample(['M','F'])` is **never** called with generator teams |
| `shiny` | `randomChance(1,1024)` | LOG only (details `, shiny`) |
| `happiness` | absent → 255 (`P:342`) | DROP |
| `teraType` | sampled from the set's `teraTypes` (any of 19 incl. `Stellar`) | `pokemon.teraType = set.teraType ‖ types[0]` (`P:452`) |
| `moves` | 4 (fewer for a few sets), shuffled | PP: `calculatePP(move, 3)` = `move.pp * 8/5` unless `noPPBoosts` (`P:361-362`, `B:2378-2383`) |
| `item`, `ability` | as generated | `''` item possible (88/36 000) |

These, plus `teraType`, `level`, `gender`, `shiny`, `name`, the initial species/ability/item and
the move list, form the **CONST team table**. `pokemon.set` is still read at runtime by:
`statModify` (`set.level/evs/ivs/nature`), faint forme regression (`set.species`, `set.ability`,
`B:2562-2566`), Shields Down (`pokemon.set.species`, `A:4250, 4263`), `getUpdatedDetails`
(`set.shiny`, `P:541`).

---------------------------------------------------------------------------------------------------

## 3. Pokémon — field-by-field

Declaration `P:47-307`, constructor `P:309-506` (ends with `clearVolatile()` → `setSpecies`, then
`hp = maxhp`). Ranges are for this format.

### 3.1 Master table

| Field (TS type) | Range here | Mutated by | Read by | Rust |
|---|---|---|---|---|
| `side`, `battle` | — | ctor | everywhere | implicit (`PokemonId`) |
| `set` | — | ctor | see §2 | CONST |
| `name`, `fullname` | ≤20 chars | ctor | protocol (`toString` `P:531-534`), `getPokemon` | CONST / LOG |
| `level` | 62..100 | ctor | `statModify`, damage, Illusion Level Mod details | CONST u8 |
| `gender` | M/F/'' | ctor | details only (no gender mechanics reachable) | CONST, LOG |
| `happiness`, `pokeball`, `dynamaxLevel`, `gigantamax`, `canGigantamax` | — | ctor | requests / unreachable | DROP (pokeball REQ) |
| `baseHpType`, `baseHpPower`, `hpType`, `hpPower` | — | ctor, `clearVolatile`, transform | Hidden Power only (absent in gen 9) | DROP |
| `baseMoveSlots: MoveSlot[]` | ≤4 | ctor; Zacian/Zamazenta `onBattleStart` replaces Iron Head slot (`C:880-939`) | `clearVolatile` copy; `baseMoves` (request for ally) | CORE (see §3.12) |
| `moveSlots: MoveSlot[]` | ≤4 | `clearVolatile` (`= baseMoveSlots.slice()`), `transformInto` (virtual slots), Zacian | PP, disable, requests, `hasMove`, many moves | CORE |
| `ppUps: number[]` | all 3 | ctor | `calculatePP` in Zacian hook and gen<5 transform | DERIVE (=3) |
| `position` | 0..5 | `switchIn` swap (`BA:118-132`), `swapPosition` (unreachable), `addPokemon` | slots, `isLastActive`, Illusion, slot conditions, `getLocOf/getAtLoc`, switch choice indices | CORE u8 |
| `details` | string | ctor, `formeChange(isPermanent)` `P:1443`, faint regression `B:2575`, Zacian `C:887`, Morpeko tera `BA:1950` | `switch`/`detailschange`/`replace` lines, requests | DERIVE from `baseSpecies` (§3.11) |
| `baseSpecies: Species` | | ctor, `formeChange(isPermanent)` `P:1442`, faint regression `B:2564`, Zacian/Zamazenta `C:886/916`, Morpeko tera `BA:1949` | `clearVolatile→setSpecies(baseSpecies)`; species handlers (`findPokemonEventHandlers` uses `pokemon.baseSpecies`, `B:1135`: Arceus `onType`); `.baseSpecies/.name/.tags` checks in Zero to Hero, Tera Shift, Embody Aspect (`baseSpecies.name === 'Ogerpon-Teal-Tera'`), Teraform Zero, Booster Energy `onTakeItem` (Paradox tag), Battle Bond | CORE u16 |
| `species: Species` | | `setSpecies` only (via `formeChange`, `transformInto`, `clearVolatile`, Zacian) | stats, types, weight, `species.id/name` checks (Disguise, Ice Face, Shields Down, Tera Shell, Terapagos, Ogerpon tera, Terastarstorm target), `transformInto` source | CORE u16 (active block; benched ⇒ `baseSpecies`) |
| `speciesState` | `{id, effectOrder:0}` | ctor | passed to species handlers (none use state) | DROP |
| `status: ID` | `'' brn par slp frz psn tox fnt` | `setStatus` `P:1731`, `checkFainted` sets `'fnt'` `B:2532`, `switchIn` clears `'fnt'` `BA:126`, Revival Blessing `B:2797` | huge | CORE u8 enum |
| `statusState` | §3.5 | `setStatus` | Sleep Clause (`statusState.source`), slp/tox callbacks | CORE |
| `volatiles: {[id]: EffectState}` | ≤5 observed | §3.6 | everywhere | CORE ordered map (active block) |
| `showCure?: boolean` | undef/true/false | Natural Cure (`A:2812-2880`) | Natural Cure messages | LOG (2 bits) |
| `baseStoredStats` | | `setSpecies` when `!isTransform` `P:1407` | `getSwitchRequestData` only `P:1159-1165` | REQ, DERIVE |
| `storedStats` (atk,def,spa,spd,spe) | ≤ ~400 | `setSpecies` `P:1409-1412`, `transformInto` copy `P:1301-1304` (Power Trick/Splits/Speed Swap unreachable) | `getStat`, `calculateStat`, Dancer sort, `getBestStat` | CORE [u16;5] (active block) |
| `boosts` (7 stats) | −6..6 | `boostBy`, `setBoost`, `clearBoosts`, `clearVolatile`, `transformInto` copy, `copyVolatileFrom` (Baton Pass) | stat calc, Stored Power-likes, Haze, etc. | CORE [i8;7] (active block) |
| `baseAbility: ID` | | ctor, `formeChange(isPermanent)` `P:1489`, faint regression `B:2565`, Zacian `C:890` | `clearVolatile` (`ability = baseAbility`), request | CORE |
| `ability: ID` | | `setAbility`, `clearVolatile`, `formeChange` Illusion-break `P:1459,1483` (`''`) | `getAbility`, `hasAbility`, handlers | CORE |
| `abilityState` | §3.7 | `switchIn` `BA:141`, `setAbility` `P:1925`, `skillSwap` | ability callbacks; `effectOrder`; `ending`; `gluttony` | CORE |
| `item: ID` | 61 non-empty items + '' | `setItem`, `takeItem`, `useItem`, `eatItem` | `getItem`, `hasItem`, handlers | CORE u8 |
| `itemState` | §3.8 | `switchIn` `BA:142`, `setItem`, `clearEffectState` | item callbacks, `effectOrder` | CORE |
| `lastItem: ID` | | `useItem`/`eatItem` `P:1800,1841`, Harvest/Recycle clear it | Harvest `A:1806`, Recycle `M:14831` | CORE u8 |
| `usedItemThisTurn` | bool | `useItem/eatItem`, `switchIn` reset `BA:122`, `endTurn` reset `B:1681` | only Pickup (absent) | DROP (1 bit if paranoid) |
| `ateBerry` | bool | `eatItem` `P:1804`, Cud Chew/Bug Bite paths | only Belch/Cud Chew/`rulesets` EBC (absent) | DROP |
| `itemKnockedOff` | false | — | gen≤4 | DROP |
| `trapped: boolean\|'hidden'` | | `endTurn` reset+`TrapPokemon` `B:1730-1731`, `tryTrap` `P:1607-1612`, `getMoveRequestData` `P:1087`, Commanded/Commanding (unreachable) | `chooseSwitch` refusal `S:985`, request | CORE (2 bits) |
| `maybeTrapped`, `maybeDisabled`, `maybeLocked` | bool | `endTurn`, `getMoveRequestData`, Imprison `M:9509`, Arena Trap/Magnet Pull/Shadow Tag `A:207/2527/4167`, `updateDisabledRequest` | request fields, `choice.cantUndo` | REQ (3 bits) |
| `illusion: Pokemon\|null` | | Illusion `onBeforeSwitchIn` `A:2057-2071`, Illusion `onEnd` `A:2078`, `onFaint`, `faintMessages` `B:2569` | `toString`, `getFullDetails`, `transformInto` guard, `terastallize`, `formeChange` messages, `endTurn` type display | CORE Option<PokemonId> |
| `transformed` | bool | `transformInto` `P:1291`, `clearVolatile` | many guards (`notransform` abilities, Booster Energy, Arceus `onType`, Battle Bond, regression) | CORE bit |
| `maxhp` | ≤ ~700 | `setSpecies` (first time only, `P:1401-1405`), `updateMaxHp` | everywhere | CORE u16 |
| `baseMaxhp` | | same | residual fractions (`baseMaxhp/16` etc.) | DERIVE (= `maxhp`; only Dynamax separates them) |
| `hp` | 0..maxhp | `damage`, `heal`, `sethp`, `faint` (`=0`), `updateMaxHp`, Revival Blessing `B:2798-2799` | everywhere | CORE u16 |
| `fainted` | bool | `faintMessages` `B:2568`, Revival Blessing `B:2794`, forfeit at start | everywhere | CORE bit |
| `faintQueued` | bool | `faint()` `P:1586`, Revival Blessing `B:2795` | `faint()` early-out | CORE bit |
| `subFainted` | null/false | Revival Blessing writes false | gen 1 only | DROP |
| `formeRegression` | bool | `formeChange(isPermanent)` with `source===null` or Item source `P:1451,1468`; Morpeko tera `BA:1949`; reset in `faintMessages` `B:2578` | `faintMessages` `B:2562,2573` | CORE bit |
| `types: string[]` | 1–2 types, may contain `???` | `setSpecies` (`setType(species.types, true)`), `setType` | `getTypes` | CORE [u8;2] (active block) |
| `addedType` | always '' | `setSpecies`, `setType`, `addType` (Forest's Curse/Trick-or-Treat absent), `terastallize`, `transformInto` | `getTypes` | DROP (keep 1 byte if you want generality) |
| `knownType` | bool | `setSpecies/setType` (true), `transformInto` `P:1297` (false when copying a foe), `terastallize`, `endTurn` `B:1705`, Reflect Type `M:14907` (absent) | gates whether `endTurn` runs `runEvent('MaybeTrapPokemon')` for Ghost types `B:1732` (that event can shuffle tied Arena Trap/Magnet Pull/Shadow Tag handlers) and the `FoeMaybeTrapPokemon` loop `B:1758`; Arena Trap `A:206`, Magnet Pull `A:2526` | CORE bit (edge case: a Pokémon transformed into a foe Ghost) |
| `apparentType` | `"T1"`/`"T1/T2"` | `setSpecies` (`rawSpecies.types.join('/')`), `setType`, `transformInto`, `terastallize`, `formeChange` (tera), `endTurn` `B:1722` (may write a **benched** Illusion target) | `endTurn` silent `-start typechange` emission | LOG [u8;2] per Pokémon (persistent, *not* active block) |
| `baseTypes` | | ctor | nothing | DROP |
| `teraType` | 19 values | ctor | `canTerastallize`, `terastallize`, Tera Blast etc. | CONST u8 |
| `terastallized?: string` | type or undefined | `terastallize` `BA:1931`; **deleted on faint** `B:2572` | `getTypes`, STAB, `setType/addType` guards, Stellar, details suffix, transform guards | CORE Option<u8> |
| `canTerastallize: string\|false\|null` | | ctor (`actions.canTerastallize` → `teraType`), `terastallize` (null for whole side `BA:1932-1934`), `transformInto` (`false` if becoming Ogerpon/Terapagos `P:1377`), `clearVolatile` (`false→teraType` `P:1525`) | request, `chooseMove` validation `S:807` | DERIVE from `side.teraUsed` + 1 `teraBlocked` bit (§3.10) |
| `switchFlag: ID\|boolean` | false / true / move id (`uturn voltswitch flipturn partingshot batonpass shedtail revivalblessing`) | self-switch `BA:1300-1301`, `checkFainted` `B:2533`, `faint()` (false), `resolveAction` (false) `Q:254`, `clearVolatile`, `chooseSwitch` revival (false) | switch requests, `resolveAction` → `sourceEffect` | CORE u8 enum |
| `forceSwitchFlag` | bool | Whirlwind/Dragon Tail/Circle Throw (`BA:1342-1357` `forceSwitch`), `runAction` phazing loop `B:2828-2831`, `clearVolatile` | phazing loop (`B:2826-2833`) | CORE bit |
| `skipBeforeSwitchOutEventFlag` | bool | `runAction` `B:2897`, `switchIn` `BA:86`, Parting Shot/Baton Pass paths | `switchIn` `BA:80`, `runAction` `B:2894` | CORE bit |
| `draggedIn` | null | gen 2 only; reset in `runSwitch` | — | DROP |
| `newlySwitched` | bool | `clearVolatile` (true), `endTurn` (false) | Helping Hand `onTryHit` `M:8584` (Bolt Beak, Fishious Rend, Core Enforcer, Payback readers are absent) | CORE bit |
| `beingCalledBack` | bool | `switchIn` (true) `BA:75`, `clearVolatile` (false) | Illusion `onEnd` `A:2078`, Slow Start `A:4340` (Pursuit `M:14375`, Eject Button `I:1689` absent) | CORE bit |
| `lastMove: ActiveMove\|null` | | `moveUsed` `P:910`, `clearVolatile` | `.id/.name/.flags` (Encore, Disable/Cursed Body, Instruct, cantusetwice, Gigaton Hammer/Blood Moon disable `B:1699`), `.isZ/.isMax` (false) | CORE Option<MoveId> (active block) |
| `lastMoveEncore` | | gen 2 | — | DROP |
| `lastMoveUsed` | | `useMoveInner` `BA:391` | only Conversion 2 (`M:2824-2828`) | DROP |
| `lastMoveTargetLoc?: number` | −2..2 | `moveUsed` `P:913` | locked-move choice `S:677`, `twoturnmove.onStart` `C:296` | CORE i8 |
| `moveThisTurn` | '' / id | `moveUsed`, `endTurn`, `clearVolatile` | no reader in gen 9 code (only a commented sanity check `BA:247`) | DROP |
| `statsRaisedThisTurn` | bool | `boost()` `B:2089`, `switchIn` reset, `endTurn` reset (turn≠1) | Burning Jealousy `M:2083`, Alluring Voice `M:294` | CORE bit |
| `statsLoweredThisTurn` | bool | `boost()` `B:2090`, resets as above | Lash Out `M:10058` | CORE bit |
| `moveLastTurnResult` | undef/null/false/true | `endTurn` `B:1678`, `clearVolatile` | Stomping Tantrum/Temper Flare (`=== false`) `M:18054,19190`, Metronome item (absent) | CORE 2 bits (active block) |
| `moveThisTurnResult` | undef/null/false/true | `runMove` `BA:262,274,285`, `useMove` `BA:371-374`, `M:8298,8339`, `formeChange` mega (unreachable), `endTurn`, `clearVolatile` | `endTurn` copy; Truant `A:5186` (`!== undefined`) | CORE 2 bits (active block) |
| `hurtThisTurn: number\|null` | null / 0..maxhp | `spreadDamage` `B:2142` (`= hp after hit` when damage≠0), `endTurn` (turn≠1), `clearVolatile` | Assurance `M:648` (absent) and the relay value of `runEvent('EmergencyExit')` `BA:1006` (only Emergency Exit/Wimp Out handle it — absent; the event still runs with zero handlers) | DROP here (if kept: Option<u16>, truthiness treats 0 as false) |
| `lastDamage` | | `runMove`, `hitStepMoveHitLoop`, `spreadDamage` `B:2143`, `M:18355` | nothing in gen 9 | DROP |
| `attackedBy: Attacker[]` | unbounded list | `gotAttacked` `P:917-928` (from `BA:985`), `endTurn` prune `B:1707-1714`, `clearVolatile` | §3.16 | CORE (compact form §3.16, active block) |
| `timesAttacked` | 0..∞ (43 seen) | `BA:987` (`+= hits` when damage numeric), `transformInto` copy `P:1308`; **never reset** | Rage Fist `min(350, 50+50*n)` `M:14588` | CORE u8 saturating (≥6 is equivalent) |
| `isActive` | bool | `switchIn` `BA:120,135`, `faintMessages` `B:2570` | `ignoringAbility/Item` (inactive ⇒ ignored), `initEffectState`, `setStatus`, `setItem`, `boost`, `heal`, `findEventHandlers`, `attackedBy` pruning | CORE bit |
| `activeTurns` | 0..∞ | `switchIn` (=0) `BA:137`, `endTurn` (+1 for non-fainted) `B:1769` | only truthiness: partially-trapped source check `C:238`, Slow Start `A:4320`, Speed Boost `A:4458`, Stakeout `A:4480`, Truant `A:5186`, Taunt duration `M:18987` | CORE u8 saturating |
| `activeMoveActions` | 0..∞ | `switchIn` (=0), `runMove` (+1) `BA:217`, Sky Drop (−1, unreachable) | Fake Out `M:5097`, First Impression `M:5483` (`> 1`) | CORE u8 saturating |
| `previouslySwitchedIn` | | `switchIn` `BA:151` | nothing | DROP |
| `truantTurn`, `duringMove`, `isStarted` | | ctor / `runSwitch` / faint | nothing (Truant uses volatile `truant`) | DROP |
| `bondTriggered` | bool, battle-long | Battle Bond `A:362-372` | Battle Bond | CORE bit |
| `heroMessageDisplayed` | bool, battle-long | Zero to Hero `A:5633,5639` | Zero to Hero | CORE/LOG bit |
| `swordBoost`, `shieldBoost` | bool, battle-long | Intrepid Sword / Dauntless Shield | same | CORE bits |
| `syrupTriggered` | | Supersweet Syrup (absent) | | DROP |
| `stellarBoostedTypes: string[]` | ⊆ 19 types, battle-long, never reset/copied | `BA:1769` (not for Terapagos-Stellar) | `BA:1648`, `BA:1765` | CORE u32 bitmask |
| `weighthg` | 1..9999 | `setSpecies` `P:1396`, `transformInto` `P:1292` | `getWeight` (+`ModifyWeight`: Heavy Metal) → Low Kick/Grass Knot/Heavy Slam/Heat Crash | CORE u16 (active block) |
| `speed` | 0..8191 | `updateSpeed` `P:556-558`, `setSpecies` (**raw `storedStats.spe`**, `P:1418`) | every handler's `speed` (`B:1005-1017`), `eachEvent` sort, `runSwitch` sort | CORE u16 (active block) — see §3.3.5 |
| `canMegaEvo*`, `canUltraBurst` | null | ctor | | DROP |
| `staleness`, `pendingStaleness`, `volatileStaleness` | | `eatItem`, `setItem`, `takeItem`, `skillSwap`, `clearVolatile` | Endless Battle Clause only (not in rule table) | DROP |
| `modifiedStats`, `modifyStat`, `recalculateStats`, `m` | | gen1/stadium/mods | | DROP |
| getters `moves`, `baseMoves` | | — | requests, Zacian | DERIVE |

### 3.2 Lifecycle invariants that let you split "persistent" vs "active block"

`clearVolatile(includeSwitchFlags=true)` (`P:1508-1560`) runs (a) in the constructor, (b) on the
outgoing Pokémon in `switchIn` when it still has HP (`BA:118`), (c) in `copyVolatileFrom` on both
Pokémon, (d) in `faintMessages` with `includeSwitchFlags=false` (`B:2567`). Exact order:

```text
clearVolatile(includeSwitchFlags = true):
  boosts = {atk:0,def:0,spa:0,spd:0,spe:0,accuracy:0,evasion:0}
  moveSlots = baseMoveSlots.slice()            // shallow: same MoveSlot objects as baseMoveSlots
  transformed = false
  ability = baseAbility
  hpType/hpPower = base…                       // DROP
  if canTerastallize === false: canTerastallize = teraType
  for id in volatiles (insertion order):
      if volatiles[id].linkedStatus: removeLinkedVolatiles(linkedStatus, linkedPokemon)   // P:2048-2058
  volatiles = {}                               // (Eternamax-Dynamax exception unreachable)
  if includeSwitchFlags: switchFlag = false; forceSwitchFlag = false
  lastMove = null; lastMoveEncore = null; lastMoveUsed = null; moveThisTurn = ''
  moveLastTurnResult = undefined; moveThisTurnResult = undefined
  lastDamage = 0; attackedBy = []; hurtThisTurn = null
  newlySwitched = true; beingCalledBack = false
  volatileStaleness = undefined
  delete abilityState.started; delete itemState.started     // NOTE: itemState.started is Booster Energy's flag
  setSpecies(baseSpecies)                      // species, types, apparentType, knownType, weighthg, storedStats,
                                               // baseStoredStats, speed = storedStats.spe; maxhp/hp untouched
```

Everything *not* listed persists across switches: `hp`, `status`/`statusState`, `item`/`itemState`,
`lastItem`, `abilityState` (overwritten at the next `switchIn` anyway), `timesAttacked`,
`terastallized`, `stellarBoostedTypes`, the battle-long one-shot flags, `apparentType` (can be
changed while benched, see §3.9), PP in `baseMoveSlots`, `lastMoveTargetLoc`, `activeTurns`/
`activeMoveActions` (reset at next switch-in), `illusion` (reset at next `BeforeSwitchIn`).

### 3.3 Stats

#### 3.3.1 Stored stats (`B:2350-2376`, `P:1387-1420`)

```text
statModify(baseStats, set, s):                          // tr = trunc
  if s == 'hp': return tr(tr(2*B + iv + tr(ev/4) + 100) * L/100 + 10)
  st = tr(tr(2*B + iv + tr(ev/4)) * L/100 + 5)          // NOTE: X*L/100 is a float; +5; then truncate
  nature = natures.get(set.nature)                      // EMPTY_NATURE here → no plus/minus branch runs
  (plus: st = tr(tr(st*110, 16)/100); minus: st = tr(tr(st*90, 16)/100))   // unreachable
  return st
spreadModify = statModify for hp,atk,def,spa,spd,spe (key order of species.baseStats)
```

With this format: `ev'=tr(ev/4) ∈ {21,20,17,0}`, `iv ∈ {31,0}`, `L ∈ [62,100]`. No `species.maxHP`
(Shedinja) is reachable.

```text
setSpecies(rawSpecies, source = battle.effect, isTransform = false):     // P:1387-1420
  species = runEvent('ModifySpecies', this, null, source, rawSpecies)    // no handlers in format → rawSpecies
  if !species: return null
  this.species = species
  setType(species.types, enforce=true)          // types, addedType='', knownType=true, apparentType=join('/')
  apparentType = rawSpecies.types.join('/')
  addedType = species.addedType || ''
  knownType = true
  weighthg = species.weighthg
  stats = spreadModify(species.baseStats, set)
  if !maxhp: baseMaxhp = maxhp = hp = stats.hp   // only the very first call (constructor) sets HP
  if !isTransform: baseStoredStats = stats
  for s in [atk,def,spa,spd,spe]: storedStats[s] = stats[s]
  speed = storedStats.spe                       // RAW stat, ignores boosts/items/Trick Room/paralysis
  return species
```

`transformInto` overwrites `storedStats` with the target's afterwards (`P:1301-1304`) but does
**not** touch `speed`, so after Transform `speed` = own-set stat computed from the target's
base Speed until the next `updateSpeed`.

#### 3.3.2 Boost table

`boostTable = [1, 1.5, 2, 2.5, 3, 3.5, 4]`; clamp b to [−6,6]; `b ≥ 0 ? floor(st*T[b]) : floor(st/T[-b])`.
Integer-exact form (all IEEE results are exact or far from integers):
`b ≥ 0 ? (st*(2+b)) div 2 : (st*2) div (2-b)`.

#### 3.3.3 `getStat(stat, unboosted=false, unmodified=false)` (`P:596-639`)

```text
st = storedStats[stat]
if unmodified && 'wonderroom' in field.pseudoWeather: swap def<->spd *name*      // unreachable (no Wonder Room)
if !unboosted:
   boosts = this.boosts
   if !unmodified: boosts = runEvent('ModifyBoost', this, null, null, {...boosts})   // copy; Unaware is onAnyModifyBoost
   b = clamp(boosts[stat], -6, 6); apply table
if !unmodified:
   st = runEvent('Modify' + {atk:'Atk',def:'Def',spa:'SpA',spd:'SpD',spe:'Spe'}[stat], this, null, null, st)
if stat == 'spe' && st > 10000: st = 10000          // format has no battle.trunc override → cap applies
return st
```

`getBestStat(unboosted, unmodified)` (`P:656-668`) loops atk,def,spa,spd,spe, calling
`getStat` twice for each new best (strict `>`; ties keep the earlier stat). Protosynthesis/Quark
Drive call `getBestStat(false, true)` → no events.

**PRNG hazard**: both `runEvent`s above go through `speedSort` (`B:798`), which shuffles ties
(equal order/priority/speed/subOrder/effectOrder). Example: two Unaware Pokémon on opposite
sides (mirror matches are common: both sides may roll the same species and set, hence the same
Speed) both register `onAnyModifyBoost` (priority 0, subOrder 7) with equal cached `speed` →
every non-`unboosted` `getStat` call shuffles them; likewise two holders of the same Ruin
ability for the matching `Modify*` event. `getStat('spe')` runs at every `updateSpeed`, so this
is not academic. Reachable stat-modifier handlers (all others default to priority 0):

| Event | Handlers (priority) |
|---|---|
| ModifyAtk | abilities Blaze, Dragon's Maw, Guts, Huge Power, Hustle, Orichalcum Pulse, Overgrow, Pure Power, Rocky Payload, Slow Start, Stakeout, Swarm, Torrent, Transistor (5); Water Bubble (0); Tablets of Ruin `onAny` (0); Flash Fire/Protosynthesis/Quark Drive volatiles (5); Choice Band, Light Ball (1); `onSource`: Purifying Salt, Thick Fat (6), Water Bubble (5) |
| ModifyDef | Fur Coat (6), Protosynthesis/Quark Drive (6), Snowscape weather (10), Eviolite (2), Sword of Ruin `onAny` (0) |
| ModifySpA | Blaze, Dragon's Maw, Hadron Engine, Overgrow, Rocky Payload, Stakeout, Swarm, Torrent, Transistor, Flash Fire/Proto/Quark (5); Water Bubble (0); Vessel of Ruin `onAny`; Choice Specs, Light Ball (1); `onSource` Purifying Salt, Thick Fat, Water Bubble (5) |
| ModifySpD | Sandstorm weather (10), Proto/Quark (6), Eviolite (2), Assault Vest (1), Beads of Ruin `onAny` |
| ModifySpe | Chlorophyll, Sand Rush, Slow Start, Slush Rush, Surge Surfer, Swift Swim, Proto/Quark, Unburden volatile, Choice Scarf, Tailwind (side condition) — all 0; Paralysis status −101 (uses `finalModify`, `C:29-36`) |
| ModifyBoost | Unaware `onAnyModifyBoost` |
| ModifyWeight | Heavy Metal (1) |
| Type | Arceus species condition (1), Roost volatile (−1) |

(`onSource*` variants never fire from `getStat` because it passes no source; they fire from the
damage formula's `runEvent('ModifyAtk', attacker, defender, move, …)`.)

#### 3.3.4 `calculateStat(stat, boost, modifier?, statUser?)` (`P:560-594`) — damage-formula path

```text
st = storedStats[stat]; (Wonder Room swap — unreachable)
boosts = runEvent('ModifyBoost', statUser || this, null, null, {[stat]: boost})
b = clamp(boosts[stat], -6, 6); apply table
return modify(st, modifier || 1)
```

#### 3.3.5 Action speed, Trick Room, and the `speed` cache

```text
getActionSpeed():                                           // P:641-649
  sp = getStat('spe', false, false)
  if field.getPseudoWeather('trickroom'): sp = 10000 - sp  // ('twisteddimensionmod' absent)
  return trunc(sp, 13)                                      // (sp >>> 0) % 8192
updateSpeed(): this.speed = getActionSpeed()                // P:556-558
battle.updateSpeed(): for p in getAllActive(): p.updateSpeed()   // B:391-395
```

`trunc(…,13)` is the Gen 8+ overflow: without Trick Room, Speeds 8192..10000 wrap to 0..1808;
under Trick Room a raw Speed ≥ 1809 gives `10000−sp ≤ 8191` and moves *first*, while 1808 → 8192
→ 0 moves last. Observed max cached speed in the census: 1790.

`pokemon.speed` is a **cache with Showdown-specific staleness**; reproduce exactly when it is
refreshed:

| Writer | When |
|---|---|
| `battle.updateSpeed()` | `commitChoices` start `B:2997`; `residual` action `B:2818`; after every action when the next queued action is `move`/`runDynamax` (`B:2917-2924`, then every queued action's `getActionSpeed` is recomputed and the queue is re-`sort`ed → may shuffle ties) |
| `pokemon.updateSpeed()` | `queue.insertChoice` for the inserted action's Pokémon `Q:378-380` (e.g. `runSwitch` insertion after a normal switch-in) |
| `setSpecies` → raw `storedStats.spe` | every `clearVolatile` (switch-out, faint, construction), every `formeChange`, `transformInto` |

So a Pokémon dragged in by Roar/Whirlwind/Dragon Tail (gen≥5 `runSwitch` runs immediately,
`BA:153-155`, no `insertChoice`) or one that just changed forme / transformed carries its raw
Speed stat (no Trick Room inversion, no boosts, no Scarf, no paralysis) into every handler sort
until the next refresh. `runSwitch` sorts `getAllActive(true)` by this cache (`BA:181-183`,
`comparePriority` on Pokémon objects = by `.speed` desc, ties shuffled) and stores
`battle.speedOrder`.

### 3.4 HP

```text
damage(d, source, effect):            // P:1595-1605
  if !hp || isNaN(d) || d <= 0: return 0
  if 0<d<1: d = 1
  d = trunc(d); hp -= d
  if hp <= 0: d += hp; faint(source, effect)      // faint(): hp = 0, switchFlag = false, faintQueued = true, push faintQueue
  return d
heal(d):                              // P:1640-1652 (battle.heal wraps it, B:2265-2307)
  if !hp: false; d = trunc(d); if isNaN or d<=0: false; if hp >= maxhp: false
  hp += d; if hp > maxhp: d -= hp-maxhp; hp = maxhp; return d
sethp(d):                             // P:1655-1667 (Revival Blessing, Pain Split-likes)
  if !hp: 0; d = trunc(d); if d < 1: d = 1; d -= hp; hp += d; clamp to maxhp; return delta
updateMaxHp():                        // P:1498-1506 — called by formeChange(isPermanent) and faint regression
  nb = statModify(species.baseStats, set, 'hp'); if nb == baseMaxhp: return
  baseMaxhp = nb; newMax = nb                      // (×2 only under Dynamax)
  hp = hp <= 0 ? 0 : max(1, newMax - (maxhp - hp)) // damage taken is preserved
  maxhp = newMax; if hp: log('-heal', …, '[silent]')
```

`maxhp` changes at runtime only through `updateMaxHp`: Terapagos (90 → 95 Terastal → 160
Stellar base HP) and faint regression back. Wish stores `hp = source.maxhp / 2` as a float
(`M:20927`); `battle.heal` truncates before any handler (`B:2272-2273`), so storing
`floor(maxhp/2)` is exact.

`hurtThisTurn` is set to the post-hit `hp` whenever `spreadDamage` deals non-zero damage
(`B:2142`), including indirect damage (weather, status, recoil, Life Orb...). In this format
nothing reachable reads it (Assurance, Emergency Exit and Wimp Out are not generated), so it
is DROP; keep it only if non-generator teams are in scope.

### 3.5 Status and `statusState`

```text
setStatus(status, source=null, sourceEffect=null, ignoreImmunities=false):   // P:1684-1750
  if !hp: return false
  if !isActive && status: return false
  status = conditions.get(status)
  if battle.event: source ||= event.source; sourceEffect ||= battle.effect
  source ||= this
  if this.status == status.id:
      if sourceEffect?.status == this.status: log('-fail', this, status)
      elif sourceEffect?.status: log('-fail', source); attrLastMove('[still]')
      return false
  if !ignoreImmunities && status.id && !(source.hasAbility('corrosion') && status in {tox,psn}):
      if !runStatusImmunity(status.id == 'tox' ? 'psn' : status.id):
          if sourceEffect?.status: log('-immune', this)
          return false
  prev = (status, statusState)
  if status.id && !runEvent('SetStatus', this, source, sourceEffect, status): return that result
  this.status = status.id
  statusState = initEffectState({id: status.id, target: this})     // effectOrder++ if id
  if source: statusState.source = source
  if status.duration: statusState.duration = …                     // none of brn/par/slp/frz/psn/tox has one
  if status.durationCallback: …                                    // none
  if status.id && !singleEvent('Start', status, statusState, this, source, sourceEffect):
      (status, statusState) = prev; return false
  if status.id && !runEvent('AfterSetStatus', …): return false
  return true
trySetStatus(s, …) = setStatus(this.status || s, …)               // P:1669-1671
cureStatus(silent): if !hp || !status: false; log -curestatus; (slp & nightmare: unreachable); setStatus('')  // P:1674-1682
clearStatus():      same without the message                                                          // P:1755-1762
```

Note `setStatus('')` still allocates a fresh `statusState` with `id ''` and `effectOrder 0`
(no counter increment) and with `source` set (to `event.source` or self).

| Status | `statusState` keys beyond the standard | Notes |
|---|---|---|
| brn, par, frz, psn | `source` | no duration |
| slp | `source`, `startTime = random(2,5)` (2..4), `time = startTime` | `C:47-82`; `time--` in `onBeforeMove` (twice with Early Bird), cure at ≤0. Rest sets `time = startTime = 3` after `setStatus` (`M:14984-14987`) |
| tox | `source`, `stage` 0..15 | `stage = 0` in `onStart` and `onSwitchIn`; `++` (cap 15) in `onResidual` `C:138-160` |
| fnt | stale keys of the previous status survive | `checkFainted` assigns the string only (`B:2528-2537`) |

`statusState.source` is read by Sleep Clause Mod (`data/rulesets.ts:1392-1394`:
`!pokemon.statusState.source?.isAlly(pokemon)`), so it is CORE. Store `source: Option<PokemonId>`,
`time: u8`, `startTime: u8`, `stage: u8`, `effectOrder`. A Pokémon revived by Revival Blessing
gets `status = ''` without touching `statusState` (`B:2797`); harmless because nothing reads
`statusState` when `status` is empty.

### 3.6 Volatiles

```text
addVolatile(status, source=null, sourceEffect=null, linkedStatus=null):    // P:1969-2027
  status = conditions.get(status)
  if !hp && !status.affectsFainted: return false
  if linkedStatus && source && !source.hp: return false
  if battle.event: source ||= event.source; sourceEffect ||= battle.effect
  source ||= this
  if volatiles[status.id]:
      if !status.onRestart: return false
      return singleEvent('Restart', status, volatiles[id], this, source, sourceEffect)
  if !runStatusImmunity(status.id): if sourceEffect?.status: log('-immune'); return false
  r = runEvent('TryAddVolatile', this, source, sourceEffect, status); if !r: return r
  volatiles[id] = initEffectState({id, name: status.name, target: this})   // appended → insertion order
  if source: volatiles[id].source = source; volatiles[id].sourceSlot = source.getSlot()
  if sourceEffect: volatiles[id].sourceEffect = sourceEffect
  if status.duration: volatiles[id].duration = status.duration
  if status.durationCallback: volatiles[id].duration = durationCallback(this, source, sourceEffect)
  r = singleEvent('Start', status, volatiles[id], this, source, sourceEffect)
  if !r: delete volatiles[id]; return r                 // the effectOrder number stays consumed
  if linkedStatus && source:
      if !source.volatiles[linkedStatus]: source.addVolatile(linkedStatus, this, sourceEffect)
                                           source.volatiles[linkedStatus].linkedPokemon = [this]
                                           source.volatiles[linkedStatus].linkedStatus = status
      else: source.volatiles[linkedStatus].linkedPokemon.push(this)
      volatiles[id].linkedPokemon = [source]; volatiles[id].linkedStatus = linkedStatus
  return true
removeVolatile(status):                                 // P:2035-2046
  if !hp: return false                                  // !! fainted Pokémon cannot lose volatiles this way
  if !volatiles[id]: return false
  {linkedPokemon, linkedStatus} = volatiles[id]
  singleEvent('End', status, volatiles[id], this); delete volatiles[id]
  if linkedPokemon: removeLinkedVolatiles(linkedStatus, linkedPokemon)
removeLinkedVolatiles(linkedStatus, list): for p in list: d = p.volatiles[linkedStatus]; if d:
      d.linkedPokemon.splice(indexOf(this),1); if empty: p.removeVolatile(linkedStatus)    // P:2048-2058
```

Some code deletes volatiles directly without `End` (`delete pokemon.volatiles['stall']` in
`C:451`, `partiallytrapped` in `C:239`, Substitute removing `partiallytrapped` `M:18337`); this
still removes the key from the ordered map.

`copyVolatileFrom(pokemon, switchCause)` (Baton Pass = `'copyvolatile'`, Shed Tail =
`'shedtail'`) (`P:1246-1268`):

```text
clearVolatile()
if switchCause != 'shedtail': this.boosts = pokemon.boosts          // object handed over (old one is replaced below)
for i in pokemon.volatiles (order):
   if switchCause == 'shedtail' && i != 'substitute': continue
   if conditions.getByID(i).noCopy: continue
   volatiles[i] = initEffectState({...pokemon.volatiles[i], target: this})   // receiver not active ⇒ effectOrder = 0
   if volatiles[i].linkedPokemon: (re-point links to `this`)
pokemon.clearVolatile()
for i in volatiles: singleEvent('Copy', getVolatile(i), volatiles[i], this)
```

**Reachable volatiles and their state** (dynamic census ∪ static scan; `dur` = initial duration):

| Volatile id | Added by | dur | Extra state keys (type) | noCopy |
|---|---|---|---|---|
| `protect`, `spikyshield`, `banefulbunker`, `burningbulwark` | moves | 1 | — | |
| `stall` | Protect-family | 2 (reset to 2 on Restart) | `counter` ∈ {3,9,27,81,243,729} (u16, or u8 exponent) | |
| `wideguard` | — (side condition, see §4) | | | |
| `followme`, `ragepowder` | moves | 1 | — (their `onFoeRedirectTarget` uses `effectOrder`) | |
| `helpinghand` | move | 1 | `multiplier` = 1.5^n (store n) | |
| `flinch` | secondaries | 1 | — | |
| `roost` | move | 1 | `typeWas` — overwritten by every `getTypes` while active (`M:15455`) and re-read right after a fresh `getTypes` in `transformInto` ⇒ DERIVE (= types after Arceus `onType`) | |
| `mustrecharge` | Giga Impact (self) | 2 | — | |
| `twoturnmove` | Solar Beam, Meteor Beam, Electro Shot, Phantom Force, Shadow Force | 2 | `move` (MoveId) | |
| `<moveid>` (`solarbeam`, `meteorbeam`, `electroshot`, `phantomforce`, `shadowforce`) | `twoturnmove.onStart` `C:292` | 2 for Phantom/Shadow Force (their condition), none otherwise | `targetLoc` (i8) | |
| `gigatonhammer`, `bloodmoon` | `runMove` cantusetwice hint (`BA:267-269`), removed in the same `runMove` | — | — (empty condition) — XACT | |
| `choicelock` | Choice Band/Specs/Scarf | — | `move` (MoveId), `sourceEffect` (item, only for `disabledSource` REQ) | yes |
| `confusion` | moves, berries, Poison Puppeteer | — | `time` 1..5 (`random(min,6)`, min 3 for Axe Kick) | |
| `disable` | Disable, Cursed Body | 5 (−1 in some cases) | `move` (MoveId) | yes |
| `encore` | Encore | 3 (+1 in some cases) | `move` (MoveId) | yes |
| `taunt` | Taunt | 3 (+1 if target already moved) | — | |
| `throatchop` | Throat Chop | 2 | — | |
| `healblock` | Psychic Noise | `durationCallback` → 2 for Psychic Noise (`M:8286-8295`) | — | |
| `syrupbomb` | Syrup Bomb | 4 | `source` read | yes |
| `yawn` | Yawn | 2 | `source` read (sleep source for Sleep Clause) | |
| `leechseed` | Leech Seed | — | `sourceSlot` read (heal target) | |
| `saltcure`, `curse`, `noretreat`, `imprison` | moves | — | — | |
| `glaiverush` | Glaive Rush (self) | — | — | yes |
| `partiallytrapped` | Infestation | `random(5,7)` → 5..6 (Grip Claw absent) | `source`, `sourceEffect` (move, for messages), `boundDivisor` = 8 | |
| `trapped` + `trapper` | Spirit Shackle (`addVolatile('trapped', source, move, 'trapper')`) | — | `linkedPokemon: PokemonId[]`, `linkedStatus` | yes (both) |
| `substitute` | Substitute, Shed Tail | — | `hp` = `floor(maxhp/4)` of the user (u16) | |
| `charge` | Electromorphosis | — | — | |
| `flashfire` | Flash Fire | — | — | |
| `unburden` | Unburden | — | — | |
| `truant` | Truant | — | — | |
| `protosynthesis`, `quarkdrive` | ability / Booster Energy | — | `fromBooster` (bool), `bestStat` (atk..spe) | |
| `dynamax`, `commanding`, `commanded`, `lockedmove`, `attract`, `torment`, `gastroacid`, `embargo`, `smackdown`, `magnetrise`, `telekinesis`, `ingrain`, `aquaring`, `perishsong`, `destinybond`, `endure`, `focusenergy`, `laserfocus`, `dragoncheer`, `stockpile`, `minimize`, `defensecurl`, `rollout`, `furycutter`, `echoedvoice`, `uproar`, `skydrop`, `rolloutstorage`, `powder`, `electrify`, `tarshot`, `octolock`, `nightmare`, `beakblast`, `shelltrap`, `focuspunch`, `counter`, `mirrorcoat`, `lockon`, `foresight`, `miracleeye` | — | | | **unreachable** (§11) |

Census maximum: 5 volatiles on one Pokémon simultaneously; the theoretical maximum is ~20. Size
the container for the theoretical bound (see §10) or prove a tighter one.

### 3.7 Ability and `abilityState`

```text
setAbility(ability, source=null, sourceEffect=null, isFromFormeChange=false, isTransform=false):  // P:1908-1946
  if !hp: return false
  ability = abilities.get(ability); sourceEffect ||= battle.effect
  old = abilities.get(this.ability)
  if !isFromFormeChange:
     if ability.flags.cantsuppress || getAbility().flags.cantsuppress: return false   // also blocks Transform/Trace
  if !isFromFormeChange && !isTransform:
     r = runEvent('SetAbility', this, source, sourceEffect, ability); if !r: return r
  singleEvent('End', old, abilityState, this, source)
  this.ability = ability.id
  abilityState = initEffectState({id: ability.id, target: this})                   // effectOrder++ if active
  if sourceEffect && !isFromFormeChange && !isTransform: log '-ability' / '-activate' (Mummy/Lingering Aroma)
  if ability.id && (!isTransform || old.id != ability.id): singleEvent('Start', ability, abilityState, this, source)
  return old.id
ignoringAbility():                                       // P:858-877
  if !isActive: return true                             // gen ≥ 5
  if getAbility().flags.notransform && transformed: return true
  if getAbility().flags.cantsuppress: return false
  if volatiles.gastroacid: return true                  // unreachable
  (Ability Shield / Neutralizing Gas scan — unreachable; still costs a getAllActive walk)
  return false
hasAbility(x) = (ability ∈ x) && !ignoringAbility()     // P:1952-1959
```

Reachable `setAbility` callers: Trace (`A:5120-5152`, picks with `battle.sample`), `transformInto`
(`isTransform`), `formeChange(isPermanent)` (`isFromFormeChange`) for Tera Ogerpon/Terapagos,
Zero to Hero, Tera Shift, Shaymin-Sky freeze, Zacian/Zamazenta battle start. `cantsuppress`
abilities present: Zero to Hero, Disguise, Ice Face, Battle Bond, Shields Down, Tera Shift,
Comatose, Multitype.

`abilityState` is **replaced** (all custom keys gone) by `switchIn`, `setAbility`, `skillSwap`.
Stale keys on benched Pokémon are never read (inactive ⇒ `ignoringAbility()`), so Rust can
reset on those three events only. Custom keys reachable:

| Key | Owner | Type |
|---|---|---|
| `ending` | Air Lock, Cloud Nine (`A:97-101, 550-554`); read by `field.suppressingWeather` `F:110` | bool |
| `gluttony` | Gluttony (`A:1619-1628`); read by pinch berries `I:169…4082` | bool |
| `unnerved` | Unnerve, As One (Glastrier/Spectrier) | bool |
| `busted` | Disguise, Ice Face | bool |
| `protean`, `libero` | Protean, Libero (once per switch-in) | bool |
| `embodied` | Embody Aspect ×4 (Ogerpon tera forms) | bool |
| `resisted` | Tera Shell (`P:2221-2229`, written from `runEffectiveness`) | bool |
| `checkedBerserk` | Berserk | bool |
| `seek` | Trace | bool |
| `berryWeaken` | Ripen | bool |
| `counter` | Slow Start (5→0, deleted at 0) | u8 |
| `started` | deleted in `clearVolatile`, never set in gen 9 data | DROP |
| `choiceLock`, `fallen`, `boosts`, `berry`, `checkedAngerShell` | Gorilla Tactics, Supreme Overlord, Opportunist, Cud Chew, Anger Shell — absent | DROP |

`abilityState.effectOrder` is CORE (redirect sort, Dancer sort, SwitchIn sort).

### 3.8 Item and `itemState`

```text
setItem(item, source, effect):                 // P:1868-1889  (Trick, Harvest, Recycle, Bestow-likes)
  if !hp || !isActive: return false
  (pendingStaleness bookkeeping — DROP)
  oldItem = getItem(); oldState = itemState
  this.item = item.id
  itemState = initEffectState({id: item.id, target: this})       // effectOrder++ if item.id
  if oldItem.exists: singleEvent('End', oldItem, oldState, this)
  if item.id: singleEvent('Start', item, itemState, this, source, effect)
  return true
takeItem(source = this):                       // P:1851-1866  (Knock Off, Trick, Incinerate…)
  if !item: return undefined
  item = getItem()
  if runEvent('TakeItem', this, source, null, item):
     this.item = ''; old = itemState; clearEffectState(itemState)   // same object, keys wiped
     singleEvent('End', item, old, this); runEvent('AfterTakeItem', this, null, null, item); return item
  return false
useItem(source, sourceEffect):                 // P:1811-1849  (Focus Sash, White Herb, Power Herb, gems…)
  if (!hp && !getItem().isGem) || !isActive: false; if !item: false
  (item-told-to-eat-but-not-holding guard)
  if runEvent('UseItem', this, null, null, item):
     log '-enditem' …; if item.boosts: battle.boost(item.boosts, this, source, item)
     singleEvent('Use', item, itemState, this, source, sourceEffect)
     lastItem = item; item = ''; clearEffectState(itemState); usedItemThisTurn = true
     runEvent('AfterUseItem', this, null, null, item); return true
eatItem(force, source, sourceEffect):          // P:1768-1809  (berries)
  if !item: false; if (!hp && item ∉ {jabocaberry, rowapberry}) || !isActive: false
  (guard as above)
  if runEvent('UseItem') && (force || runEvent('TryEatItem')):
     log '-enditem' … '[eat]'; singleEvent('Eat', item, itemState, …); runEvent('EatItem', …)
     lastItem = item; item = ''; clearEffectState(itemState); usedItemThisTurn = true; ateBerry = true
     runEvent('AfterUseItem', …); return true
ignoringItem(isFling=false):                   // P:879-886
  if getItem().isPrimalOrb: false; if !isActive: true
  if volatiles.embargo || field.pseudoWeather.magicroom: true   // unreachable
  return !getItem().ignoreKlutz && hasAbility('klutz')         // Klutz absent → false
hasItem(x) = (item ∈ x) && !ignoringItem()
```

`itemState` reachable keys: `started` (Booster Energy `I:630`; deleted by `clearVolatile`), plus
`effectOrder`. White Herb's `boosts` is created and deleted inside one call (XACT).
Natural pre-battle mutation: none (Zacian keeps Rusted Sword).

### 3.9 Types (`P:2106-2146`)

```text
setType(newType, enforce=false):
  if !enforce:
     if 'Stellar' in newType: return false
     if species.num ∈ {493 (Arceus), 773}: return false        // gen ≥ 5
     if terastallized: return false
  types = array(newType); addedType = ''; knownType = true; apparentType = types.join('/')
  return true
addType(t): if terastallized: return false; addedType = t; return true      // unreachable callers
getTypes(excludeAdded=false, preterastallized=false):
  if !preterastallized && terastallized && terastallized != 'Stellar': return [terastallized]
  t = runEvent('Type', this, null, null, this.types)       // Arceus species onType (prio 1), Roost (prio −1)
  if !t.length: t.push('Normal')                           // JS aliasing hazard: if no handler returned a copy and
                                                           // this.types were empty this would mutate this.types (never happens here)
  return (!excludeAdded && addedType) ? t.concat(addedType) : t
hasType(x) = any(x ∈ getTypes())
```

Arceus `onType` (`C:850-864`, attached to the species because `dex-species.ts:529-536` copies
base-species conditions onto the Species): `if transformed || ability != 'multitype': return
types; else return [item.onPlate || 'Normal']`. Roost `onType` (`M:15453-15457`) records
`typeWas = types` and filters out Flying (the Roost condition refuses to start on a
terastallized user, `M:15445-15450`).

Reachable type changers: `setSpecies` (forme change/transform/switch-out), Protean/Libero
(`setType(move.type)` on PrepareHit, once per switch-in), Double Shock (Electric → `???`),
`transformInto` (`setType(target types pre-tera & pre-roost, enforce)`). Terastallized Pokémon
ignore non-enforced `setType`.

`apparentType` is display state; `knownType` is mostly request state but gates one `runEvent`
(see §3.1). `endTurn` (`B:1716-1728`), for each
non-terastallized active Pokémon: `seen = illusion || pokemon; real = seen.getTypes(true).join('/')`;
if `real != seen.apparentType` → log `-start <pokemon> typechange <real> [silent]` and
`seen.apparentType = real` (+ `typeadd` line if `addedType`). Note this writes the **Illusion
target's** (benched) `apparentType`, which is why `apparentType` must be per-Pokémon persistent.

### 3.10 Terastallization

* Per Pokémon: `teraType` (CONST), `terastallized` (CORE), `canTerastallize` (derived).
* Per side: implicit "used" flag — `terastallize()` sets `canTerastallize = null` on **every**
  Pokémon of the side (`BA:1932-1934`); `side.choice.terastallize` forbids a second tera in the
  same choice (`S:811-813`).
* `canTerastallize` model: `null` if side tera used; else `false` if `teraBlocked` (set by
  `transformInto` into Ogerpon/Terapagos while truthy, `P:1377`; cleared by `clearVolatile`,
  `P:1525`); else `teraType`. `actions.canTerastallize` (`BA:1911-1916`) returns `null` only for
  Z-crystal/mega holders (unreachable) so initial value = `teraType`.

```text
terastallize(pokemon):                                          // BA:1918-1953 (queued 'terastallize' action, order 106)
  if species.baseSpecies == 'Ogerpon' && teraType ∉ {Fire,Grass,Rock,Water} && (!illusion || illusion is Ogerpon):
      hint(…, false, side); return                              // generator never produces this
  if illusion && illusion.species.baseSpecies ∈ {Ogerpon, Terapagos}: singleEvent('End', Illusion, abilityState, pokemon)
  log('-terastallize', pokemon, teraType)
  terastallized = teraType
  for ally in side.pokemon: ally.canTerastallize = null
  addedType = ''; knownType = true; apparentType = teraType
  if species.baseSpecies == 'Ogerpon': formeChange(<base or battleOnly>+'tera' / 'tealtera', null, true)
  if species.name == 'Terapagos-Terastal': formeChange('Terapagos-Stellar', null, true)
  if species.baseSpecies == 'Morpeko' && !transformed && baseSpecies.id != species.id:
      formeRegression = true; baseSpecies = species; details = getUpdatedDetails()
  runEvent('AfterTerastallization', pokemon)                    // Teraform Zero
```

Faint deletes `terastallized` (`B:2572`) but the side's tera stays spent. Stellar:
`getTypes()` returns base types; `stellarBoostedTypes` records each type that already received
the one-time boost (`BA:1765-1771`, not recorded for Terapagos-Stellar); the 60-BP floor uses
it too (`BA:1647-1653`).

### 3.11 Species / forme changes / Transform / details

```text
formeChange(speciesId, source = battle.effect, isPermanent=false, abilitySlot='0', message):   // P:1427-1496
  raw = species.get(speciesId); sp = setSpecies(raw, source); if !sp: return false
  apparent = illusion ? illusion.species.name : sp.baseSpecies
  if isPermanent:
     baseSpecies = raw; details = getUpdatedDetails()
     log('detailschange', this, (illusion || this).details + (terastallized ? ', tera:T' : ''))
     updateMaxHp()
     if !source: formeRegression = true                                   // tera formes
     elif source.effectType == 'Item': canTerastallize = null; (mega/primal/ultra msgs); formeRegression = true   // unreachable
     elif source.effectType == 'Status': log('-formechange', this, sp.name, message)   // Shaymin-Sky frozen
  else:
     log('-formechange', this, (source is Ability ? sp.name : (illusion ? illusion.species.name : sp.name)), message[, '[from] ability: X'])
  if isPermanent && (!source || source.id ∉ {disguise, iceface}):
     if illusion && source: ability = ''                                  // Ogerpon/Terapagos tera vs Illusion
     ab = sp.abilities[abilitySlot] || sp.abilities['0']
     if source || !getAbility().flags.cantsuppress: setAbility(ab, null, null, isFromFormeChange=true)
     baseAbility = ab
  if terastallized: knownType = true; apparentType = terastallized
  return true
```

Reachable forme changes:

| Trigger | Call | Permanent? | Ability reset? | formeRegression |
|---|---|---|---|---|
| Tera Ogerpon (any mask) | `formeChange(ogerpon…tera, null, true)` | yes | Embody Aspect | yes |
| Tera Terapagos-Terastal | `formeChange('Terapagos-Stellar', null, true)` | yes | Teraform Zero | yes |
| Tera Morpeko-Hangry | direct `baseSpecies = species` | details only | — | yes |
| Tera Shift (switch-in) | `formeChange('Terapagos-Terastal', ability, true)` | yes | Tera Shell | no |
| Zero to Hero (switch-out) | `formeChange('Palafin-Hero', ability, true)` | yes | Zero to Hero | no |
| Disguise | `formeChange('Mimikyu-Busted', ability, true)` | yes | **no** (excluded id) | no |
| Ice Face | `formeChange('Eiscue'/'Eiscue-Noice', ability, true)` | yes | **no** | no |
| Shaymin-Sky frozen | `formeChange('Shaymin', frz, true)` | yes | Natural Cure | no |
| Shields Down | `formeChange('Minior-Meteor' / set.species)` | no | — | — |
| Hunger Switch | `formeChange('Morpeko'/'Morpeko-Hangry')` | no | — | — |
| Gulp Missile | `formeChange('cramorant…', move)` | no | — | — |
| Relic Song | `formeChange('Meloetta(-Pirouette)', move, false, '0', '[msg]')` | no | — | — |
| Zacian/Zamazenta `onBattleStart` | `setSpecies(Crowned)`, `baseSpecies = Crowned`, `details`, `setAbility(…, fromForme)`, `baseAbility`; Iron Head slot → Behemoth Blade/Bash (new MoveSlot object, `moveSlots = baseMoveSlots.slice()`) | yes | yes | no |

Faint regression (`B:2562-2579`): if `formeRegression && !transformed`, before `clearVolatile(false)`
set `baseSpecies = species(set.species)`, `baseAbility = set.ability`; after clearing, rebuild
`details`, log `detailschange … [silent]`, `updateMaxHp()`, `formeRegression = false`.

**`details` is derivable**: every writer sets `details = getUpdatedDetails()` immediately after
`species === baseSpecies` holds, so `details == getUpdatedDetails()` evaluated with
`species := baseSpecies`:
`name(baseSpecies) + (level==100 ? '' : ', L'+level) + (gender ? ', '+gender : '') + (shiny ? ', shiny' : '')`,
where `Greninja-Bond` and `Rockruff-Dusk` print their base species name (`P:536-542`). Protocol
"full details" (`P:544-554`) additionally substitutes the Illusion target's details **at the
Illusion target's level** (Illusion Level Mod) and appends `, tera:<type>`.

```text
transformInto(target, effect):                                   // P:1270-1380 (Transform move, Imposter)
  if target.fainted || this.illusion || target.illusion || target.volatiles.substitute
     || target.transformed || this.transformed || target is Eternamax
     || (target.species.baseSpecies ∈ {Ogerpon, Terapagos} && (this.terastallized || target.terastallized))
     || this.terastallized == 'Stellar': return false
  if !setSpecies(target.species, effect, isTransform=true): return false
  transformed = true; weighthg = target.weighthg
  types = target.getTypes(true, true)                            // runs Type event on target (refreshes roost.typeWas)
  setType(target.volatiles.roost ? target.volatiles.roost.typeWas : types, enforce=true)
  addedType = target.addedType; knownType = isAlly(target) && target.knownType; apparentType = target.apparentType
  storedStats = copy(target.storedStats)
  moveSlots = [for each target slot: {move, id, pp: min(5, move.pp), maxpp: same, target, disabled:false, used:false, virtual:true}]
  timesAttacked = target.timesAttacked
  boosts = copy(target.boosts)                                   // key order atk..evasion
  for v in [dragoncheer, focusenergy, gmaxchistrike, laserfocus]: removeVolatile(v)
  for v in same: if target has v: addVolatile(v) (+copy layers/hasDragonType)   // all unreachable in this format
  log('-transform', this, target[, '[from] ' + effect.fullname])
  if terastallized: knownType = true; apparentType = terastallized
  setAbility(target.ability, this, null, false, isTransform=true)   // fails silently vs cantsuppress (e.g. Zero to Hero)
  if species.baseSpecies ∈ {Ogerpon, Terapagos} && canTerastallize: canTerastallize = false
  return true
```

### 3.12 Move slots and PP

```ts
interface MoveSlot { id; move: string; pp; maxpp; target?; disabled: boolean|'hidden'; disabledSource?; used; virtual? }  // P:13-23
```

* `baseMoveSlots[i]` and `moveSlots[i]` are **the same object** whenever not transformed
  (`clearVolatile` does `baseMoveSlots.slice()`), so PP spent while active is permanent. Rust:
  store PP once in the base slots; when transformed, use a separate 4-slot overlay
  (`virtual=true`, `pp = maxpp = min(5, move.pp)`) that is discarded on switch-out.
* `pp` (u8, ≤ 64 for 40-PP moves ×8/5): `deductPP` (`P:888-902`, `used = true` even when pp is 0),
  Pressure (extra `deductPP`), Leppa Berry (absent), Spite (absent). `maxpp` is CONST per slot
  except the Zacian/Zamazenta replacement at battle start.
* `disabled` (`false | true | 'hidden'`) and `disabledSource` (REQ): reset to `false/''` for every
  active Pokémon at `endTurn` (`B:1691-1694`) and then recomputed by `runEvent('DisableMove')`
  and per-move `singleEvent('DisableMove')` + `cantusetwice` (`B:1695-1702`). Writers:
  `disableMove` (`P:1625-1637`) from Choice lock (`C:359`), Disable, Encore, Taunt, Throat Chop,
  Heal Block, Assault Vest, Imprison (`'hidden'`, `M:9507`), Gigaton Hammer/Blood Moon. `pp <= 0`
  also makes a move disabled in `getMoves` (`P:1021-1023`). `'hidden'` is reported as disabled
  except when restricting info for the last active Pokémon (`P:1025-1027`), but `chooseMove`
  always rejects it (`S:731-762`).
* `used`: set by `deductPP`, cleared in `switchIn` (`BA:138-140`); only Last Resort reads it
  (absent) → DROP.
* `target` per slot is CONST (`move.target`).

### 3.13 Switching, position, Illusion (state-relevant parts)

```text
switchIn(pokemon, pos, sourceEffect=null, isDrag=false):           // BA:62-160
  if !pokemon || pokemon.isActive: hint; return false
  old = side.active[pos]
  if old?.hp:
     old.beingCalledBack = true
     copyFlag = (sourceEffect && typeof sourceEffect.selfSwitch == 'string') ? selfSwitch : false
     if !old.skipBeforeSwitchOutEventFlag && !isDrag: runEvent('BeforeSwitchOut', old); eachEvent('Update')
     old.skipBeforeSwitchOutEventFlag = false
     if !runEvent('SwitchOut', old): return false
     if !old.hp: return 'pursuitfaint'
     singleEvent('End', old.getAbility(), old.abilityState, old)
     singleEvent('End', old.getItem(), old.itemState, old)
     queue.cancelAction(old)
     if copyFlag: pokemon.copyVolatileFrom(old, copyFlag)
     old.clearVolatile()
  if old:
     old.isActive = false; old.isStarted = false; old.usedItemThisTurn = false
     old.statsRaisedThisTurn = false; old.statsLoweredThisTurn = false
     old.position = pokemon.position
     if old.fainted: old.status = ''
     pokemon.position = pos; side.pokemon[pos] = pokemon; side.pokemon[old.position] = old
  pokemon.isActive = true; side.active[pos] = pokemon
  pokemon.activeTurns = 0; pokemon.activeMoveActions = 0
  for s in pokemon.moveSlots: s.used = false
  pokemon.abilityState = initEffectState({id: pokemon.ability, target: pokemon})   // effectOrder++
  pokemon.itemState    = initEffectState({id: pokemon.item,    target: pokemon})   // effectOrder++ iff item
  runEvent('BeforeSwitchIn', pokemon)                                              // Illusion picks its disguise here
  log(isDrag ? 'drag' : 'switch', pokemon, fullDetails[, '[from] ' + sourceEffect])
  pokemon.previouslySwitchedIn++
  if isDrag: runSwitch(pokemon)                    // gen ≥ 5: immediate
  else: queue.insertChoice({choice: 'runSwitch', pokemon})     // → pokemon.updateSpeed(); may call random(first,last+1)
  return true
runSwitch(pokemon):                                                  // BA:175-192
  switchersIn = [pokemon] + consecutive queued 'runSwitch' actions (shifted off the queue)
  all = getAllActive(includeFainted=true); speedSort(all)       // ties → shuffle
  battle.speedOrder = all.map(getFieldPositionValue)
  fieldEvent('SwitchIn', switchersIn)
  for p in switchersIn: if p.hp: p.isStarted = true; p.draggedIn = null
dragIn(side, pos): p = getRandomSwitchable(side)               // sample over side.pokemon[2..] non-fainted, current order
                   … runEvent('DragOut', old) … switchIn(p, pos, null, true)    // BA:162-174
```

Illusion (`A:2057-2071`): on `BeforeSwitchIn`, `illusion = null`, then scan `side.pokemon` from
the last index down to `position+1`; the first non-fainted one becomes the disguise unless the
user is terastallized and that one is Ogerpon/Terapagos (then no disguise, and the scan stops).
Cleared by Illusion `onEnd` when not `beingCalledBack` (logs `replace` + `-end Illusion` + one-time
hint), `onFaint`, `faintMessages`. Because the disguise depends on the *current* order of
`side.pokemon`, the order array is CORE.

### 3.14 Per-turn bookkeeping in `endTurn` (`B:1627-1805`)

```text
turn++; lastSuccessfulMoveThisTurn = null
(dynamax ending — unreachable; gen1 — unreachable)
for side in sides:
  for pokemon in side.active (fainted included; null skipped):
     moveThisTurn = ''; newlySwitched = false
     moveLastTurnResult = moveThisTurnResult; moveThisTurnResult = undefined
     if turn != 1: usedItemThisTurn = statsRaisedThisTurn = statsLoweredThisTurn = false; hurtThisTurn = null
     maybeDisabled = maybeLocked = false
     for s in moveSlots: s.disabled = false; s.disabledSource = ''
     runEvent('DisableMove', pokemon)
     for s in moveSlots: m = getActiveMove(s.id); singleEvent('DisableMove', m, null, pokemon)
                         if m.flags.cantusetwice && lastMove?.id == s.id: disableMove(lastMove.id)
     if getLastAttackedBy(): knownType = true
     prune attackedBy from the END: entry.source.isActive ? entry.thisTurn = false : splice(entry)
     if !terastallized: (apparentType sync, §3.9)
     trapped = maybeTrapped = false; runEvent('TrapPokemon', pokemon)
     if !knownType || dex.getImmunity('trapped', pokemon): runEvent('MaybeTrapPokemon', pokemon)
     for source in pokemon.foes(): for each ability slot of (source.illusion || source).species
         with name != source.ability (and not unreleased-H / banned):
         if knownType && !getImmunity('trapped', pokemon): continue
         singleEvent('FoeMaybeTrapPokemon', ability, {}, pokemon, source)
     if fainted: continue
     activeTurns++
  side.faintedLastTurn = side.faintedThisTurn; side.faintedThisTurn = null
maybeTriggerEndlessBattleClause(...)    // only the turn-limit part applies: turn > 1000 ⇒ tie; ≥500 warnings (LOG)
log('turn', turn); makeRequest('move')
```

Every `runEvent` above may consume PRNG on handler ties; they must run even though several only
feed request data.

### 3.15 Trapping and request-only flags

`trapped` is CORE because `chooseSwitch` rejects a switch when it is truthy (`S:985-997`) and
`getMoveRequestData` (`P:1083-1151`) sets `trapped = true` for hard-locked Pokémon
(`LockMove`: `mustrecharge` → `'recharge'`, `twoturnmove` → move id). Values: `true` (Spirit
Shackle `trapped` volatile → `tryTrap()`, partially trapped, No Retreat), `'hidden'` (Shadow Tag,
Arena Trap, Magnet Pull via `onFoeTrapPokemon → tryTrap(true)`; `tryTrap(true)` does not
downgrade an existing `true`, `P:1607-1612`). `maybeTrapped/maybeDisabled/maybeLocked` only shape
the request and `choice.cantUndo`.

`getMoveRequestData` side effects (run at every `makeRequest('move')` *and* again inside every
`chooseMove`, `S:570`): `trapped = true` if hard-locked; if `hardLocked || !isLastActive()` then
`maybeDisabled = maybeLocked = maybeTrapped = false`, else `maybeLocked ||= maybeDisabled`. With
the handler inventory of this format these calls consume no PRNG (no tied `Type`/`LockMove`
handlers), but they do mutate the flags above.

### 3.16 `attackedBy` — exact semantics and a bounded encoding

```ts
interface Attacker { source: Pokemon; damage: number; thisTurn: boolean; move?: ID; slot: PokemonSlot; damageValue?: number|boolean|undefined }  // P:25-32
gotAttacked(move, damage, source): attackedBy.push({source, damage: typeof damage=='number' ? damage : 0,
                                    move: move.id, thisTurn: true, slot: source.getSlot(), damageValue: damage})  // P:917-928
```

Pushed once per (move use, target) after all hits, only when `source !== target` (`BA:983-989`).
Readers:

| Reader | Needs |
|---|---|
| `getLastAttackedBy()` (`P:930-933`) — `endTurn` knownType, Berserk (`A:433`, uses `.damage`) | last entry |
| `getLastDamagedBy(true)` (`P:935-942`) — Comeuppance (`M:2686-2705`), Metal Burst-likes | last entry with `typeof damageValue === 'number'` and `!isAlly(source)`; uses `.damage`, `.thisTurn`, `.slot` |
| Avalanche (`M:913`) | ∃ entry with `source === target && damage > 0 && thisTurn` |

Pruning happens only in `endTurn` (entries whose source is not active are removed; the rest get
`thisTurn = false`), and the whole list is cleared by `clearVolatile`. The list is unbounded
(25 entries seen). Bounded exact encoding: keep **one record per distinct source**
`{src: PokemonId, last_seq, last_damage, last_slot, num_seq?, num_damage, num_slot, pos_dmg_this_turn}`
where `*_seq` come from a battle-global attack counter. Then: last entry = record with max
`last_seq`; last damaged-by = max `num_seq` among non-ally records; `thisTurn` ⇔
`seq > battle.seq_at_last_endTurn`; Avalanche = `rec(target).pos_dmg_this_turn`; pruning = drop
records whose source is inactive at `endTurn`, clear `pos_dmg_this_turn`. Correct because every
reader only looks at the latest qualifying entry per source and all entries of a source are
pruned together. For a guaranteed bound, allocate one slot for each of the **11 other stable PokemonIds**;
self-attacks are not appended at the move-execution call site. Six records is an observed/
practical estimate, not a proved limit: mid-turn replacement and forced switching can introduce
additional distinct sources before the next pruning pass. The raw list also has no length
bound; never substitute an empirical maximum for a safe allocation policy.

---------------------------------------------------------------------------------------------------

## 4. Side

Declaration `S:166-227`, constructor `S:229-289`.

| Field | Range | Mutated by | Read by | Rust |
|---|---|---|---|---|
| `battle`, `id` (`p1`/`p2`), `n` (0/1) | | ctor | | CONST |
| `name`, `avatar` | | `setPlayer` | protocol | CONST/LOG |
| `foe` | other side | `start()` `B:1932-1933` | everywhere | CONST |
| `allySide` | null | | | DROP |
| `team: PokemonSet[]` | 6 sets | ctor | `State` serialization, EBC | CONST |
| `pokemon: Pokemon[]` | permutation of 6 | `switchIn` swap, `swapPosition` (unreachable), `team` action (unreachable) | Illusion, `possibleSwitches`/`getRandomSwitchable` (order-sensitive `sample`), requests, switch choice indices, Sleep Clause scan, `terastallize` loop | CORE `[PokemonId; 6]` |
| `active: Pokemon[2]` | | `switchIn` | everywhere | DERIVE (= `pokemon[0..2]`) |
| `pokemonLeft` | 0..6 | ctor/`start` (=6), `faintMessages` `−1` `B:2557`, Revival Blessing `+1` `B:2786` | `checkWin`, `foePokemonLeft` (boost guard `B:2035`, Battle Bond), `possibleSwitches` | CORE u8 |
| `totalFainted` | 0..100 (cap) | `faintMessages` `B:2558` | Last Respects `M:10096` | CORE u8 |
| `faintedThisTurn`, `faintedLastTurn` | Pokemon\|null | `faintMessages` `B:2580`, `endTurn` `B:1773-1774` | only Retaliate (absent) | DROP (2 bits if paranoid) |
| `zMoveUsed`, `dynamaxUsed` | false / true | | | DROP |
| `sideConditions` | ordered map ≤4 seen | `addSideCondition` (`S:413-443`), `removeSideCondition` (`S:456-462`), Court Change, residual expiry, Defog-likes (Tidy Up, Mortal Spin, Rapid Spin remove own hazards) | hazards on SwitchIn, screens, Tailwind speed, Wide Guard | CORE |
| `slotConditions[2]` | ordered maps ≤1 seen | `addSlotCondition` (`S:464-495`), `removeSlotCondition` (`S:504-511`) | Wish, Revival Blessing | CORE |
| `activeRequest` | request JSON | `makeRequest`, `emitRequest`, `clearRequest`, `win` | `requestState` getter, `chooseMove` (`canDynamax`), `updateRequestForPokemon` | REQ (rebuild on demand) |
| `choice: Choice` | | `clearChoice`, `choose*` | `commitChoices` | harness/REQ staging (`actions`, `forcedSwitchesLeft`, `forcedPassesLeft`, `switchIns`, `terastallize`, `cantUndo`, `error`) |
| `lastMove`, `lastEnemyMove`, `lastSelectedMove*`, `lastEnemySelectedMove` | | gen 1 | | DROP |

```text
addSideCondition(status, source=null, sourceEffect=null):            // S:413-443
  source ||= battle.event?.target; (if source is a Side: source = source.active[0]); require source
  if sideConditions[id]: return status.onSideRestart ? singleEvent('SideRestart', …) : false
  sideConditions[id] = initEffectState({id, target: side, source, sourceSlot: source.getSlot(), duration: status.duration})  // effectOrder++
  if status.durationCallback: duration = durationCallback(side.active[0], source, sourceEffect)
  if !singleEvent('SideStart', …): delete; return false
  runEvent('SideConditionStart', side, source, status); return true
addSlotCondition(target, status, source=null, sourceEffect=null):    // S:464-495
  source ??= battle.event?.target; pos = target.position (if Pokemon)
  if slotConditions[pos][id]: return status.onRestart ? singleEvent('Restart', …) : false
  state = initEffectState({id, target: side, source, sourceSlot, isSlotCondition: true, duration})   // effectOrder++
  durationCallback as above
  if !singleEvent('Start', status, state, side.active[pos], source, sourceEffect): delete; return false
```

Reachable side conditions:

| Id | Set by | Duration | Extra keys |
|---|---|---|---|
| `reflect`, `lightscreen`, `auroraveil` | moves | 5, **8 with Light Clay** (in item pool) | — |
| `tailwind` | Tailwind | 4 | — |
| `wideguard` | Wide Guard | 1 | — (not swapped by Court Change) |
| `spikes` | Spikes, Ceaseless Edge | — | `layers` 1..3 |
| `toxicspikes` | Toxic Spikes, Toxic Debris | — | `layers` 1..2 |
| `stealthrock` | Stealth Rock, Stone Axe | — | — |
| `stickyweb` | Sticky Web | — | — |

Standard keys: `source` (PokemonId), `sourceSlot`, `duration`, `effectOrder` (CORE: hazards'
`onSwitchIn` order uses it). Unreachable: `safeguard, mist, luckychant, quickguard, craftyshield,
matblock, *pledge, gmax*`.

Reachable slot conditions:

| Id | Duration | Keys | Notes |
|---|---|---|---|
| `wish` | — (expires via `onResidual` when `getOverflowedTurnCount() > startingTurn`) | `hp` (= `source.maxhp/2`, float; store `floor`), `startingTurn` (`trunc(turn−1, 8)`, u8), `source` (name in message), `sourceSlot` (expiry looks up `getAtSlot(sourceSlot)`) | `M:20915-20946` |
| `revivalblessing` | 1 | — | presence drives the forced "switch" to a fainted ally (`S:932-978`, `B:2785-2802`, `B:2883-2889`) |

---------------------------------------------------------------------------------------------------

## 5. Field (`F:12-33`)

| Field | Range | Mutated by | Read by | Rust |
|---|---|---|---|---|
| `battle`, `id` | | | | CONST |
| `weather` | `'' raindance sunnyday sandstorm snowscape` | `setWeather` (`F:39-89`: Drizzle, Drought, Orichalcum Pulse, Sand Stream, Snow Warning, Rain Dance), `clearWeather` (`F:91-99`: residual expiry, Teraform Zero) | `effectiveWeather` (`F:101-104`, suppressed by an active non-ending Air Lock/Cloud Nine `F:106-116`), `isWeather`, weather handlers | CORE u8 |
| `weatherState` | `{source, sourceSlot, duration 5, effectOrder 0}` | as above | duration countdown; abilities re-setting weather fail if same weather (`F:45-53`, gen>5) | CORE (dur u8, source, sourceSlot) |
| `terrain` | `'' electricterrain grassyterrain psychicterrain` | `setTerrain` (`F:130-157`: Electric/Grassy/Psychic Surge, Hadron Engine, Seed Sower), `clearTerrain` | `effectiveTerrain` (`runEvent('TryTerrain')`), terrain handlers | CORE u8 |
| `terrainState` | `{source, sourceSlot, duration 5, effectOrder 0}` | | | CORE |
| `pseudoWeather` | ordered: `sleepclausemod` (permanent, `source null`, no duration) then optionally `trickroom` (`duration 5`, `source`, `sourceSlot`) | ctor `B:306`; Trick Room `addPseudoWeather` (`F:186-216`); a second Trick Room → `onFieldRestart` → `removePseudoWeather` (`F:223-230`) | `getActionSpeed`, `findFieldEventHandlers` | CORE: 1 bit + dur u8 + source u8 + sourceSlot u8 |

Weather/terrain `durationCallback`s check Damp/Heat/Smooth/Icy Rock and Terrain Extender —
none are in the item pool, so durations are always 5. Weather/terrain/pseudo-weather states
never increment `battle.effectOrder`.

---------------------------------------------------------------------------------------------------

## 6. Battle

Declaration `B:109-190`, constructor `B:191-316`.

| Field | Value / range | Mutated by | Rust |
|---|---|---|---|
| `id`, `debugMode` (false), `forceRandomChance` (null), `deserialized`, `strictChoices` | | ctor | CONST |
| `format`, `formatData`, `gameType`, `activePerHalf` (2), `dex`, `gen` (9), `ruleTable`, `actions`, `teamGenerator`, `NOT_FAIL…`, `send`, `trunc`, `clampIntRange`, `toID` | | ctor | CONST / code |
| `prngSeed` | starting seed | ctor | CONST (end-of-battle JSON only) |
| `prng` | Gen5RNG (4×u16) or SodiumRNG (32-byte seed, ChaCha20) — chosen by seed string prefix (`sim/prng.ts:58-68`) | every random call | CORE |
| `rated`, `reportExactHP` (false), `reportPercentages` (true after `onBegin`), `supportCancel` (true) | | ctor/`onBegin` | CONST |
| `queue.list: Action[]` | ≤ 7 seen | §6.1 | CORE |
| `faintQueue: {target, source, effect}[]` | ≤ 4 | `Pokemon.faint` push; `faintMessages` drain | XACT (empty at request boundaries; inside an action it is CORE) |
| `log`, `inputLog`, `messageLog`, `sentLogPos`, `sentEnd`, `sentRequests` | | `add`, I/O | LOG/IO |
| `requestState` | `'' move switch` (`teampreview` unreachable) | `makeRequest`, `clearRequest`, `commitChoices`, `turnLoop`, `win` | CORE u8 |
| `turn` | 0..1001 | `endTurn` | CORE u16 (Wish `startingTurn`, turn-limit tie) |
| `midTurn` | bool | `start` (true), `turnLoop` | CORE bit |
| `started`, `ended`, `winner` | | `start`, `win` | CORE |
| `effect`, `effectState`, `event`, `events` (null), `eventDepth` | | event dispatch | XACT call-stack |
| `activeMove`, `activePokemon`, `activeTarget` | | `setActiveMove`, `clearActiveMove`, many | XACT (null at boundaries because `runAction` always ends with `clearActiveMove()` `B:2835`) |
| `lastMove` | | `clearActiveMove(false)` | DROP (Copycat only) |
| `lastSuccessfulMoveThisTurn` | Option<MoveId> | `runMove` `BA:309`, `endTurn` (null) | CORE (Fusion Flare/Bolt `M:6363,6382`) |
| `lastMoveLine` | log index or −1 | `addMove` `B:3116`, `attrLastMove` `B:3121-3137` | LOG (never reset between turns; `attrLastMove`/`retargetLastMove` patch that log line) |
| `lastDamage`, `quickClawRoll` | | gen 1 / gen 2–3 | DROP |
| `effectOrder` | counter, 149 max in census (~7/turn) | `initEffectState` | CORE; u16 suffices unless > 65 increments/turn are sustained for 1000 turns — u32 recommended |
| `speedOrder: number[4]` | permutation of field positions | ctor (`[0,1,2,3]`), `runSwitch` `BA:183` | CORE [u8;4] (SwitchIn handler fractional speed `B:1012-1017`) |
| `hints: Set<string>` | | `hint(…, once=true)` | LOG — only the Illusion Level Mod hint uses `once` (`A:2085`) → 1 bit |

### 6.1 Persistent action-queue entries (`Q:24-108`, `Q:166-265`)

The queue survives mid-turn switch requests (`commitChoices` re-sorts only the new choices and
appends the old list, `B:3003-3017`), so its contents are CORE. Per action:

| Field | Type | Notes |
|---|---|---|
| `choice` | enum: `move, beforeTurnMove, priorityChargeMove, switch, instaswitch, revivalblessing, runSwitch, terastallize, beforeTurn, residual, start, pass, shift, event, megaEvo*, runDynamax, team` | reachable subset: move, switch, instaswitch, revivalblessing, runSwitch, terastallize, beforeTurn, residual, start |
| `order` | u16: team 1, start 2, instaswitch 3, beforeTurn 4, beforeTurnMove 5, revivalblessing 6, runSwitch 101, switch 103, megaEvo 104, runDynamax 105, terastallize 106, priorityChargeMove 107, move/shift/event 200, residual 300; `prioritizeAction` sets 3 | |
| `priority` | i8 + `fractionalPriority` (one decimal: 0, ±0.1, −0.1 for Mycelium Might; Quick Claw/Custap absent) | `getActionSpeed` (`B:2623-2667`) |
| `speed` | u16 (`getActionSpeed()` of the Pokémon; 1 for field actions) | recomputed on gen≥8 re-sorts |
| `pokemon`, `target` (switch), `originalTarget` | PokemonId | |
| `targetLoc` | i8 | random target resolved at `resolveAction` (`Q:268-273`) via `getRandomTarget` (may `sample`) |
| `moveid` / `move` (ActiveMove) | MoveId + mutated fields: `priority` (written by `getActionSpeed`, `B:2653`), `pranksterBoosted` (Prankster `A:3429`), `ignoreAbility` (Mycelium Might `A:2804`) | the same ActiveMove object is executed by `runMove` (`getActiveMove` returns ActiveMoves unchanged, `sim/dex.ts:316-322`) |
| `sourceEffect` | effect ref (self-switch move id, `Q:251-254`) | |
| `terastallize` | type | only on the move action that spawned a `terastallize` action |

---------------------------------------------------------------------------------------------------

## 7. Measured ranges

### 7.1 Census maxima (3 200 random-choice battles, ~20 turns each)

| Quantity | Max | Quantity | Max |
|---|---|---|---|
| volatiles on one Pokémon | 5 | side conditions on one side | 4 |
| slot conditions in one slot | 1 | pseudo-weathers | 2 |
| `attackedBy.length` | 25 | `battle.effectOrder` | 149 |
| `queue.list.length` | 7 | `stellarBoostedTypes.length` | 3 |
| `maxhp` | 645 | max stored stat | 391 |
| cached `speed` | 1790 | `weighthg` | 9500 |
| `timesAttacked` | 43 | `activeTurns` / `activeMoveActions` | 63 / 63 |
| `side.totalFainted` | 7 | `turn` | 85 |

Observed value sets: `switchFlag` ∈ {false, true, uturn, voltswitch, flipturn, partingshot,
batonpass, shedtail, revivalblessing}; `trapped` ∈ {false, true, 'hidden'}; `subFainted` ∈ {null,
false}; `draggedIn` always null; `volatileStaleness` always undefined; `addedType` always ''.

---------------------------------------------------------------------------------------------------

## 8. PRNG touch-points that live in state code

(Full list in the PRNG/event docs; these are the ones inside the functions specified here.)

| Site | Call |
|---|---|
| `speedSort` (`B:433-464`) inside every `runEvent`, `eachEvent`, `fieldEvent`, `queue.sort`, `runSwitch` | `prng.shuffle(list, start, end)` per tie group (only when ≥2 tied) |
| `queue.insertChoice` (`Q:398-401`) | `random(firstIndex, lastIndex+1)` when the new action ties with a range |
| slp `onStart` | `random(2,5)` |
| confusion `onStart` | `random(2 or 3, 6)` |
| partiallytrapped `durationCallback` | `random(5,7)` |
| stall `onStallMove` | `randomChance(1, counter)` |
| Trace `onUpdate`, `getRandomSwitchable` (Roar/Dragon Tail), `getRandomTarget`/`randomFoe` | `sample(list)` — list order = position order |
| Pokémon ctor gender | `sample(['M','F'])` — never with generator teams |

---------------------------------------------------------------------------------------------------

## 9. Protocol-only state summary (keep for log parity)

`name/fullname`, `gender`, `shiny` (CONST); `apparentType` (per Pokémon), `showCure`,
`illusion` (also CORE), `details` (derived), `battle.lastMoveLine`, `battle.hints` (1 bit),
`heroMessageDisplayed` (also gates a message only), `reportPercentages` (HP shown as
`ceil(100*hp/maxhp)`, 99 if that is 100 but `hp<maxhp`, `P:2075-2081`; secret side shows
`hp/maxhp`), `side.name`.

---------------------------------------------------------------------------------------------------

## 10. Suggested Rust layout and byte budget

Split per §3.2: 12 persistent `PokemonCore` records + 4 `ActiveSlot` records (reset on switch-out
exactly as `clearVolatile` does; Baton Pass/Shed Tail = keep the slot's volatiles minus
`noCopy` ids and set their `effectOrder` to 0; transform data dies with the slot). Use
format-local dense ids: species u16 (572 closed-scope records), ability u8 (207), item u8 (61 + none), move u16
(or u16 index into ~360 reachable + Struggle), type u8 (19 + `???`).

`PokemonCore` (×12):

| Item | Bytes |
|---|---|
| base_species u16, base_ability u8, ability u8 | 4 |
| ability_state: effect_order u32, flags u16 (ending, gluttony, unnerved, busted, protean, libero, embodied, resisted, checkedBerserk, seek, berryWeaken), slowstart counter u8 | 7 |
| item u8, item_state.effect_order u32 (+started bit in flags), last_item u8 | 6 |
| hp u16, maxhp u16 | 4 |
| status u8, status source u8, time u8, start_time u8, stage u8, status effect_order u32 | 9 |
| 4 base move slots × (pp u8, flags u8: disabled 2b, used 1b) (+ move id u16 if not CONST) | 8 (16) |
| position u8, illusion u8, terastallized u8, apparent_type [u8;2], switch_flag u8, last_move_target_loc i8 | 7 |
| active_turns u8, active_move_actions u8, times_attacked u8 | 3 |
| stellar_boosted u32 | 4 |
| flag word u32 (is_active, fainted, faint_queued, forme_regression, known_type, newly_switched, being_called_back, force_switch, skip_before_switch_out, stats_raised, stats_lowered, bond, hero_msg, sword, shield, tera_blocked, trapped×2, maybe_trapped, maybe_disabled, maybe_locked, show_cure×2) | 4 |
| **Total** | **≈56 (64 with move ids)** |

`ActiveSlot` (×4):

| Item | Bytes |
|---|---|
| species u16, types [u8;2], added_type u8, transformed bit | 6 |
| stored_stats [u16;5], weighthg u16, speed u16 | 14 |
| boosts [i8;7] | 7 |
| transform overlay: 4 × (move u16, pp u8, maxpp u8) | 16 |
| last_move u16, move_this_turn_result/move_last_turn_result (4 bits) (+ hurt_this_turn u16 if kept) | 3 (5) |
| volatiles: ordered small vector with 24 inline entries and a spill path; `VolEntry {id u8, duration u8, source u8, source_slot u8, a u16, b u16, effect_order u32}` = 12 B of packed payload (a/b carry move id, counter, time, hp, targetLoc, bestStat/fromBooster, link target, sourceEffect; effects needing more use a specialized payload) | 1 + 288 |
| attacked_by: `ArrayVec<AtkRec, 11>`; `AtkRec {src u8, last_seq u32, last_dmg u16, last_slot u8, num_seq u32, num_dmg u16, num_slot u8, pos_dmg_this_turn bit}` 16 B packed payload | 1 + 176 |
| **Total** | **≈512** before alignment/overflow metadata |

`Side` (×2): order [u8;6], pokemon_left u8, total_fainted u8, flags u8 (tera_used,
fainted_this/last_turn) = 9; side conditions `ArrayVec<{id u8, dur u8, layers u8, source u8,
source_slot u8, effect_order u32}, 9>` ≈ 82; slot conditions `[ArrayVec<{id u8, source u8,
source_slot u8, hp u16, starting_turn u8, effect_order u32}, 2>; 2]` ≈ 44 → **≈135 B** (≈70 B
with per-kind fixed slots + an insertion-order byte list).

`Field`: weather u8 + dur u8 + source u8 + source_slot u8; terrain same; trick room present/dur/
source/source_slot → **12 B** specialized payload, or roughly **16 B** with optional-target tags for the field states (§1.4).

`Battle` top level: PRNG 8 B (Gen5) or 32 B (Sodium); turn u16; effect_order u32;
attack_seq u32 + seq_at_last_endturn u32; speed_order [u8;4]; request_state u8; flags u8
(mid_turn, started, ended, winner×2, illusion_hint); last_successful_move u16; queue
16 inline actions × ~20 B = 320, with an overflow path (16 is not a proved bound) → **≈350–380 B** (+ protocol buffers, not counted).

Totals:

| Layout | Pokémon | Slots | Sides | Field | Battle | **Total** |
|---|---|---|---|---|---|---|
| reference payload (above) | 12×64 = 768 | 4×512 = 2 048 | 270 | 16 | 380 | **≈3.5 KB** |
| empirical hot payload (requires overflow storage; not a safe fixed bound) | 12×56 = 672 | 4×150 = 600 | 140 | 12 | 220 | **≈1.6 KB** |

A flat "every Pokémon carries its own volatile block" layout costs roughly
12×(64+512) + 666 ≈ 7.6 KB of payload. These estimates exclude Rust alignment, vector lengths/
capacities, overflow allocations, immutable team data, requests and protocol buffers; measure
`size_of` and snapshot sizes after implementation. They are design budgets, not a proved
minimum. Saturate `activeTurns` at 1, `activeMoveActions` at 2 and `timesAttacked` at 6 only
under the current closed-scope reader analysis (§3.1); wrapping u8 counters would be wrong.
A reference engine may retain full counters and the raw attackedBy list until equivalence of
the compact encoding has been checked.

---------------------------------------------------------------------------------------------------

## 11. Ignored (explicitly skipped branches)

* Team Preview (`runPickTeam`, `chooseTeam`, `team` action, `teampreview` request) — no rule.
* Dynamax/Gigantamax/Max moves (`dynamax` volatile, `getDynamaxRequest`, `maxMoveDisabled`,
  `baseMaxhp ≠ maxhp`, Eternamax `clearVolatile` exception, `endTurn` dynamax block).
* Z-moves (`zMoveUsed`, `canZMove`, `runZPower`, Z-Transform crit volatiles), Mega/Ultra Burst/
  Primal (`canMegaEvo*`, `runMegaEvo`, Item-source `formeChange` branch).
* Gen ≤ 8 branches: gen 1 `modifiedStats`/side `last*Move`, gen 2 `lastMoveEncore`/`draggedIn`/
  `quickClawRoll`, gen ≤4 `itemKnockedOff`/`lastItem` transfer, gen 4 Transform formes, gen ≤3
  faint handling, gen ≤5 weather durations, gen < 6 Hidden Power naming.
* Hidden Power (`hpType`, `hpPower`), Return/Frustration (`happiness`).
* Endless Battle Clause (`staleness`, `pendingStaleness`, `volatileStaleness`, berry-cycling
  loser check) — rule absent; only the turn-1000 tie and turn-500+ warnings remain.
* Multi battles / free-for-all (`allySide`, `sides[2..3]`, FFA retargeting, Court Change FFA
  rotation), triples (`shift`, center swap), Battle Royal targeting.
* Timers, chat, inactivity, `tiebreak`, `forceWin`, `undoChoice`/Cancel Mod UI behaviour
  (`choice.cantUndo` is REQ only), `/team` serialization (`sim/state.ts` is only a reference for
  which fields Showdown itself considers derived: it skips `side, battle, set, name, fullname, id,
  happiness, level, pokeball, baseMoveSlots` on Pokémon, `battle, team, pokemon, choice,
  activeRequest` on Side, `dex, gen, ruleTable, id, log, format, teamGenerator, field, sides,
  prng, hints, queue, actions…` on Battle — `sim/state.ts:39-50`).
* Unreachable content listed in §0.2 and the unreachable volatile row of §3.6: Commander
  (`commanding/commanded`, `getChoiceIndex` auto-pass for commanding), Neutralizing Gas
  (`ignoringAbility` scan, `abilityState.ending` for it), Ability Shield, Gastro Acid, Embargo,
  Magic Room, Wonder Room (`getStat`/`calculateStat` swaps), Gravity, Misty Terrain, Hail,
  Primordial Sea/Desolate Land/Delta Stream, Future Sight (`futuremove`), Healing Wish/Lunar
  Dance (`healreplacement`), Sky Drop (`isSkyDropped`), Ally Switch (`swapPosition`), Skill Swap
  (`skillSwap`), Outrage-family (`lockedmove`), Metronome-family callers, Conversion 2
  (`lastMoveUsed`), Belch (`ateBerry`), Retaliate (`faintedLastTurn`), Pickup
  (`usedItemThisTurn`), Supersweet Syrup (`syrupTriggered`), Forest's Curse/Trick-or-Treat
  (`addedType`), Power Trick/Guard Split/Power Split/Speed Swap/Heart Swap (`storedStats` edits),
  Grip Claw/Binding Band/Damp-Heat-Smooth-Icy Rock/Terrain Extender (durations), Cud Chew,
  Opportunist, Supreme Overlord, Gorilla Tactics, Emergency Exit/Wimp Out, Eject Button/Pack,
  Red Card, Utility Umbrella (`effectiveWeather` umbrella branch), Air Balloon, Mirror Herb,
  Klutz, Pursuit (`'pursuitfaint'`), Imposter/Transform crit-volatile copying.

---------------------------------------------------------------------------------------------------

## 12. Open questions

1. **Team source.** Everything marked DROP/unreachable assumes generator teams. If the fuzzer
   will use hand-built Gen 9 sets, re-run the census with those sets and promote fields
   accordingly (Commander, Neutralizing Gas, Wonder Room, Gravity, Future Sight are the big ones).
2. **What is compared.** If request JSON / `sideupdate` errors are part of the differential
   check, the REQ fields (`maybe*`, `disabledSource`, `baseStoredStats`,
   `pokeball`, `cantUndo`, `noCancel`) must be kept; otherwise they can go.
3. **`effectOrder` width.** u16 is very likely enough (≈7 increments/turn observed, turn cap
   1000) but a 1000-turn stall battle with constant Protect/hazards was not measured; u32 is the
   safe default.
4. **Volatile container bound.** Census max is 5, theoretical ~20. Use a small ordered vector
   with an overflow path until a hard bound is proved; per-kind fixed slots also need insertion order.
5. **`attackedBy` compaction** (§3.16) is argued correct but should be validated by running the
   exact `Vec<Attacker>` implementation side by side in debug builds.
6. **PRNG flavour.** Default Showdown seeds are now `sodium,<hex>` (ChaCha20, 32-byte state);
   `gen5` seeds are 8 bytes. Pick one for the fuzzer (state size and speed differ).


## 13. Verification notes (recovered state-model task)

Checked against the pinned source and ran 25 reproducible oracle probes:

```sh
node tools/oracle/check-state-model.mjs /home/aminaliu/src/pokemon-showdown
```

All 25 passed. They cover rules and format flags, generator defaults, party/field identity,
HP, move-slot aliasing and PP persistence, effectOrder allocation, clearEffectState retention,
mutable-target subOrder, numeric-zero versus failed attack records, ally filtering, unbounded
attack history, clearVolatile persistence/reset, Tera prohibition sentinels and both RNG
seed-state round trips. The probes are representative checks, not an exhaustive proof of
every field or mechanic. The existing 3,200-battle census was inspected from its saved
outputs, not rerun.

Corrections from this recovery: reconciled the old census with the final closed scope;
removed the blanket claim that EffectState.target is always implicit; replaced the six-source
attackedBy allocation suggestion with the safe eleven-other-Pokémon bound; marked queue/
volatile capacities as requiring overflow; revised the byte budget and explicitly excluded
alignment and allocator metadata; made saturation thresholds explicit. Source inspection
confirmed Trick Room's callbacks do not exercise the generic target-mutation case, so that
warning does not add a new reachable pseudo-weather.

The state-model authoring task is complete. Compact-layout equivalence and the other
workflow verification tasks remain implementation/review work, as identified in §12.
