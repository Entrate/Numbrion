# 01 — Showdown event system (spec for the Gen 9 Random Doubles engine)

Source of truth: `/home/aminaliu/src/pokemon-showdown` at commit `7332b60` (built to `dist/`).
Every `file:line` below is relative to that directory. Format: `gen9randomdoublesbattle`
(`config/formats.ts:165-172`, `gameType: 'doubles'`, `mod: 'gen9'`, ruleset
`['PotD', 'Obtainable', 'Species Clause', 'HP Percentage Mod', 'Cancel Mod', 'Illusion Level Mod', 'Sleep Clause Mod']`).
Gen 9 is the base dex (`data/`, with `data/scripts.ts` = `{ gen: 9 }`, so there are no script overrides), and
the format has no `battle`/`actions`/`field` overrides.

Notation: pseudo-code follows the TypeScript closely. "PRNG" means `battle.prng`. `random(a,b)` is
`PRNG.random(a,b)` = `floor(next() * (b-a) / 2^32) + a` and uses one `next()` per call (`sim/prng.ts:91-103`).
JS truthiness matters everywhere: falsy = `false, null, undefined, 0, -0, NaN, ''`.

Everything marked **QUIRK** is real Showdown behaviour that we verified empirically against
`dist/` (scripts were run in `/tmp`, not in the repo) and **must be reproduced**, even if it looks like a bug.

---

## 0. The core rules (read these first)

1. Handlers are **collected first, then sorted, then run**. The handler list is a snapshot. Handlers added
   during the event never run. Handlers whose effect was removed during the event still run in `runEvent`,
   because `runEvent` has no "is it still there" check. The only exceptions are the per-handler checks in §5.2.
   `fieldEvent` *does* recheck whether the effect is still there (§5.5).
2. Sorting uses selection sort. Each group of fully tied handlers is **shuffled with the PRNG**
   (`speedSort`, §4.4). This is the only place the event core consumes randomness. A handler takes part in the sort,
   and so in ties, **even if it will later be suppressed or skipped** (Gastro Acid, Mold Breaker, a Status
   mismatch, a fainted holder, or a duration-only entry with no callback). So you cannot pre-filter handlers
   before sorting.
3. `eachEvent` sorts the **active Pokémon** by speed and shuffles ties **whether or not any handler exists**.
   `eachEvent('Update')` runs very often, so speed ties between actives cost PRNG on every call.
4. A handler's return value replaces the relay variable unless it is `undefined`. A falsy relay value stops
   `runEvent`. `priorityEvent` stops on the first non-`undefined` value (§6).
5. Insertion order of handlers is part of the observable behaviour. It decides the pre-shuffle order of tie groups,
   and also the result of the two stable-sorted event families (§4.2, §4.3). Port the discovery order exactly (§2).

---

## 1. Data model

### 1.1 Effects and `effectType`

| Source | Object | `effectType` |
|---|---|---|
| `data/moves.ts` entry | `Move`/`ActiveMove` | `'Move'` |
| `data/abilities.ts` entry | `Ability` | `'Ability'` |
| `data/items.ts` entry | `Item` | `'Item'` |
| `data/conditions.ts` entry or `.condition` sub-object of a move/ability/item | `Condition` | `'Status'` (brn/par/slp/frz/psn/tox), `'Weather'` (8 weathers), `'Terrain'` (declared in the terrain moves' `condition`), otherwise `'Condition'` (`sim/dex-conditions.ts:648-652`) |
| Species | `Species` | `'Pokemon'` |
| Format / rules | `Format` | `'Format'` for the format itself. Rules use `'Rule'` / `'ValidatorRule'` from `data/rulesets.ts` |
| move `secondary`/`secondaries[i]`/`self` sub-objects | plain object passed as an effect to `singleEvent('Hit'/'AfterHit'…)` | `undefined` |

`dex.conditions.getByID(id)` (`sim/dex-conditions.ts:672-708`) resolves in this order: `''` → EMPTY_CONDITION
(`exists:false`); then `item:`/`ability:` prefixes (innate effects, **Ignored**); then **Rulesets** (returns the
`Format` object, which is how rule pseudo-weathers work); then `Conditions`; then the `.condition` of a Move,
then an Ability, then an Item with that id; then `recoil`/`drain` pseudo conditions; otherwise a non-existent
Condition.

Species handlers: `dex.species.get` copies every key of `Conditions[toID(baseSpecies)]` that the species object
doesn't already have (`sim/dex-species.ts:528-536`). In Gen 9 this gives Arceus (all formes) `onType` +
`onTypePriority: 1`, and Zacian/Zamazenta `onBattleStart`. Silvally is non-standard. Only these species have
handlers, and they are found through `pokemon.baseSpecies` (§2.2).

### 1.2 `EffectState` (`sim/pokemon.ts:34-39`)

`{ id: string, effectOrder: number, duration?: number, target?, source?, sourceSlot?, isSlotCondition?, ...free-form }`.
Handlers read and write it through `this.effectState`. That is the stored object itself, so mutations persist.

`initEffectState(obj, effectOrder?)` (`sim/battle.ts:3320-3330`):
```
if !obj.id: obj.id = ''
if effectOrder given:                  obj.effectOrder = effectOrder
elif obj.id && obj.target && (!(obj.target is Pokemon) || obj.target.isActive):
                                       obj.effectOrder = battle.effectOrder++   // global creation counter, starts at 0
else:                                  obj.effectOrder = 0                      // counter NOT advanced
```
`clearEffectState(state)` (`sim/battle.ts:3332-3343`) sets `id=''` and `effectOrder=0`, keeps `target`, and deletes
every other key.

Where states are created, and what `target` they start with. `target` matters for subOrder (§3) and for
`fieldEvent` (§5.5).

| State | Created at | initial `target` | advances counter? |
|---|---|---|---|
| `pokemon.statusState` | `setStatus` `sim/pokemon.ts:1732` | the Pokémon | yes if status id non-empty (status can only be set while active) |
| `pokemon.volatiles[id]` | `addVolatile` `sim/pokemon.ts:1999` (`{id,name,target}`) | the Pokémon | yes if holder active |
| `pokemon.abilityState` | switch-in `sim/battle-actions.ts:142`, `setAbility` `sim/pokemon.ts:1925`, Skill Swap `sim/battle.ts:1341-1342` | the Pokémon | yes if ability id non-empty and active |
| `pokemon.itemState` | switch-in `sim/battle-actions.ts:143`, `setItem` `sim/pokemon.ts:1883` | the Pokémon | yes if item non-empty and active (**no item → effectOrder 0**) |
| `pokemon.speciesState` | `sim/pokemon.ts:332` `{id}` | **none** | no (0) |
| `side.sideConditions[id]` | `sim/side.ts:426` | the Side | yes |
| `side.slotConditions[pos][id]` | `sim/side.ts:478` (`isSlotCondition: true`) | the **Side** | yes |
| `field.pseudoWeather[id]` | `sim/field.ts:200` `{id, source, sourceSlot, duration}` | **none** | no (0) |
| `field.weatherState` | `sim/field.ts:70` `{id}` | **none** | no (0) |
| `field.terrainState` | `sim/field.ts:141` | **none** | no (0) |
| `battle.formatData` | `sim/battle.ts:217` `{id: format.id}` | none | no (0) |
| temporary state (`singleEvent`/`runEvent` given `null`) | `initEffectState({})` | none | no (0) |

At battle start, the four lead switch-ins (p1a, p1b, p2a, p2b, in that order) take effectOrders for ability, then
item, per Pokémon. A Pokémon with no item takes no number. We verified 0..7 with 4 items.

**QUIRK, state.target mutation.** `runEvent` does `this.effectState.target = effectHolder` before every handler call
(`sim/battle.ts:906`). The consequences:
* Pseudo-weather/weather/terrain/format states get `target = Field` (or Battle) the first time `runEvent` dispatches
  one of their handlers. Before that, `target` is undefined. This changes the default subOrder of
  `'Condition'`-type pseudo-weathers from 2 to 5 (§3). We verified this with Gravity: subOrder 2 before the first
  `ModifyAccuracy` dispatch and 5 after it.
* Slot conditions have `target = Side`, but their effect holder is the Pokémon in the slot. After any `runEvent`
  dispatch (only `Swap`, for Healing Wish/Lunar Dance) `target` becomes that Pokémon. From then on their default
  subOrder is 2 instead of 3, **and `fieldEvent` skips them**, because its location check looks in
  `target.volatiles[id]` (§5.5). We verified that after `runEvent('Swap')`, Healing Wish no longer fires in
  `fieldEvent('SwitchIn')`.
* `singleEvent` and `fieldEvent` never mutate `target`.

### 1.3 Battle event context

| field | meaning | set by |
|---|---|---|
| `battle.effect` | effect whose handler is running (init `{id:''}`) | `singleEvent` and `runEvent` (per handler). Restored afterwards |
| `battle.effectState` | that handler's state (init `initEffectState({id:''})`) | same |
| `battle.event` | `{id, target, source, effect, modifier?}`, init `{id:''}` | `singleEvent`: `{id, target, source, effect: sourceEffect}` (**no `modifier`**). `runEvent`: `{id, target, source, effect: sourceEffect, modifier: 1}` for the whole loop. Restored afterwards |
| `battle.eventDepth` | nesting depth | `++` around the `singleEvent` callback and around the `runEvent` handler loop |

These are **observable**. Many mutators take their defaults from them. `setStatus`/`addVolatile` default
`source = battle.event.source` and `sourceEffect = battle.effect` (`sim/pokemon.ts:1693-1696`, `1977-1980`).
`setWeather`/`setTerrain` default `sourceEffect = battle.effect` and `source = battle.event.target`
(`sim/field.ts:41-42`, `132-133`). `addSideCondition`/`addSlotCondition`/`addPseudoWeather` default
`source = battle.event.target` (`sim/side.ts:416, 468`, `sim/field.ts:191`). `field.effectiveTerrain()` defaults its target to `battle.event.target` (`sim/field.ts:170`). The engine must track the same "current effect / current event" stack.

### 1.4 Handler record (`EventListener`, `sim/battle.ts:79-95`)

`{ effect, callback (fn | constant | undefined), state (EffectState|null), end (fn|null), endCallArgs?, effectHolder (Pokemon|Side|Field|Battle), target?, index?, order (number|false), priority, subOrder, effectOrder?, speed? }`

`callback` can be a **non-function constant**. In that case the "return value" is the constant
(`runEvent` `sim/battle.ts:911-913`, `singleEvent` `sim/battle.ts:644-648`). Gen 9 examples:
`onCriticalHit: false` (Battle Armor, Shell Armor), `onInvulnerability: false` (Phantom/Shadow Force condition),
`onLockMove: 'rollout'|'uproar'|'recharge'`, `onFractionalPriority: -0.1` (Stall, Lagging Tail),
`onNegateImmunity: false` (Ring Target), `onEat: false` (6 EV berries). `onPlate` (string) is **data only**.
It is read directly (`data/conditions.ts:857`, `sim/pokemon.ts:1368`, `data/moves.ts:9830`) and never dispatched.

---

## 2. Handler discovery

### 2.1 `getCallback(target, effect, callbackName)` (`sim/battle.ts:1022-1037`)
```
cb = effect[callbackName]
if cb === undefined && target is Pokemon && callbackName === 'onSwitchIn'
   && !effect.onAnySwitchIn && effect.effectType in {'Ability','Item'}:      // (+ innate 'ability:'/'item:' Status: Ignored)
    cb = effect.onStart                                                       // switch-in runs onStart
return cb
```
* The remap only applies to the exact name `onSwitchIn`, with a Pokémon target, and only when the effect has **no**
  `onSwitchIn` **and no** `onAnySwitchIn`. Commander, Pastel Veil and White Herb have `onAnySwitchIn` + `onStart`.
  Their `onStart` is *not* run at switch-in. Instead their `onAnySwitchIn` (collected for every active, §5.5) calls
  `onStart`/`onUpdate` on `this.effectState.target`.
* Priority keys are looked up with the *callback name*, so a remapped `onStart` uses `onSwitchInPriority` /
  `onSwitchInOrder` / `onSwitchInSubOrder`, never `onStartPriority`. Examples: Unnerve and As One have
  `onSwitchInPriority: 1`; Protosynthesis, Quark Drive and Booster Energy have `-2`.
* Abilities with their own `onSwitchIn` (Air Lock, Cloud Nine, Imposter, Neutralizing Gas, Tera Shift,
  Zero to Hero) run that instead. Their `onStart` (if any) runs only through `singleEvent('Start')` when the
  ability is gained (`setAbility`).

### 2.2 `findPokemonEventHandlers(pokemon, callbackName, getKey?)` (`sim/battle.ts:1101-1160`)
The order is fixed: **status → volatiles → ability → item → species → slot conditions**.
```
H = []
status = pokemon.getStatus()                       // EMPTY if ''
if (cb = getCallback(pokemon,status,name)) !== undefined || (getKey && pokemon.statusState[getKey]):
    H.push(resolvePriority({effect:status, callback:cb, state:pokemon.statusState, end:pokemon.clearStatus, effectHolder:pokemon}, name))
for id in pokemon.volatiles (JS insertion order; a deleted then re-added key goes to the end):
    st = pokemon.volatiles[id]; eff = conditions.getByID(id)
    if cb !== undefined || (getKey && st[getKey]): H.push(rp({eff, cb, st, end:pokemon.removeVolatile, holder:pokemon}))
ability = pokemon.getAbility():  same test with pokemon.abilityState, end: pokemon.clearAbility
item    = pokemon.getItem():     same test with pokemon.itemState,    end: pokemon.clearItem
species = pokemon.baseSpecies:   ONLY if cb !== undefined (no getKey test), state: pokemon.speciesState, end: () => {}
for id in side.slotConditions[pokemon.position] (insertion order):
    if cb !== undefined || (getKey && st[getKey]):
        H.push(rp({effect, cb, st, end: side.removeSlotCondition, endCallArgs:[side, pokemon, id], holder: pokemon}))
return H
```
`getKey` is only `'duration'`, and only from `fieldEvent('Residual')`. In that case, entries with a truthy `duration`
are collected **even with no callback**. These are "duration-only" handlers. They take part in sorting.

### 2.3 `findSideEventHandlers(side, name, getKey?, customHolder?)` (`sim/battle.ts:1220-1235`)
For `id in side.sideConditions` (insertion order): if `cb !== undefined || (getKey && st[getKey])`:
`{effect, cb, state: st, end: customHolder ? null : side.removeSideCondition, effectHolder: customHolder || side}`.

### 2.4 `findFieldEventHandlers(field, name, getKey?, customHolder?)` (`sim/battle.ts:1185-1218`)
Order: **pseudo-weathers (insertion order) → weather → terrain**.
`end` = `field.removePseudoWeather` / `field.clearWeather` / `field.clearTerrain`, or `null` when there is a customHolder.
`effectHolder = customHolder || field`. In this format, `field.pseudoWeather` always starts with `sleepclausemod`
(§9), so it is first in the pseudo-weather list.

### 2.5 `findBattleEventHandlers(name, getKey?, customHolder?)` (`sim/battle.ts:1162-1183`)
1. The Format itself: `getCallback(battle, format, name)`. `gen9randomdoublesbattle` has **no** callbacks
   (its only `on*` key is `onBegin: undefined`), so this adds nothing.
2. Custom handlers in `battle.events[name]` added by `Battle.onEvent` (`sim/battle.ts:1253-1291`). They are pushed
   **without `resolvePriority`**. They keep the `{priority, order, subOrder}` given to `onEvent` (default 0 /
   false / 0), have `state = formatData` when the target is a Format, and have no speed. This format never calls
   `onEvent`, so `battle.events` is always `null`. Documented only for completeness.

### 2.6 `findEventHandlers(target, eventName, source?)`: the prefix system (`sim/battle.ts:1039-1099`)
```
if target is Array:                                       // used by TryHit, TryHitSide, AfterMoveSecondary, DamagingHit
    H = []
    for (i, p) in target.entries():
        cur = findEventHandlers(p, eventName, source)     // full discovery per target, including source/field/battle
        for h in cur: h.target = p; h.index = i
        H = H.concat(cur)
    return H
shouldBubbleDown = target is Side
prefixed = eventName not in ['BeforeTurn','Update','Weather','WeatherChange','TerrainChange']
H = []
if target is Pokemon && (target.isActive || source?.isActive):
    H = findPokemonEventHandlers(target, 'on'+E)
    if prefixed:
        for a in target.alliesAndSelf():                  // = side.allies(): side.active, non-null, hp > 0, slot order. INCLUDES target itself (if hp>0)
            H.push(...fPEH(a, 'onAlly'+E)); H.push(...fPEH(a, 'onAny'+E))
        for f in target.foes():                           // = foe side .allies(): non-null, hp > 0, slot order (both foes in doubles)
            H.push(...fPEH(f, 'onFoe'+E));  H.push(...fPEH(f, 'onAny'+E))
    target = target.side
if source && prefixed:                                    // source = the runEvent source only if it is a Pokemon
    H.push(...fPEH(source, 'onSource'+E))                 // NO isActive check on the source
if target is Side:
    for side in battle.sides (p1, then p2):
        if shouldBubbleDown:                              // only when the original target was a Side
            for active in side.active:                    // NO hp/fainted filter here
                if side === target || side === target.allySide: H += fPEH(active, 'on'+E)
                elif prefixed:                                   H += fPEH(active, 'onFoe'+E)
                if prefixed:                                     H += fPEH(active, 'onAny'+E)
        if side.n < 2 || !side.allySide:                  // always true with 2 players
            if side === target || side === target.allySide: H.push(...fSEH(side, 'on'+E))
            elif prefixed:                                   H.push(...fSEH(side, 'onFoe'+E))
            if prefixed:                                     H.push(...fSEH(side, 'onAny'+E))
H.push(...findFieldEventHandlers(field, 'on'+E))
H.push(...findBattleEventHandlers('on'+E))
return H
```
Consequences that are easy to get wrong:
* **"Ally" includes self.** A Pokémon's `onAllyX` fires for X events on itself as well as on its partner. `onAnyX`
  fires for events on any active, including itself. `onFoeX` fires for events on either foe; there is no
  adjacency in doubles.
* A target with hp 0 that has not fainted yet still gets its own `on` handlers (if `isActive`). Its own
  `onAlly`/`onAny` are dropped because `allies()` filters `hp`.
* If neither target nor source is active, the target's own handlers **and all side handlers** are skipped (target
  is never converted to its side). Only source, field and battle handlers remain.
* A target that is neither a Pokémon, a Side nor an array (Battle, or Field, e.g. `runEvent('TryTerrain', field)`)
  only gets source, field and battle handlers. A falsy target becomes the Battle (`sim/battle.ts:778`).
* The only `runEvent` with a Side target in this format is `SideConditionStart` (`sim/side.ts:441`).
  There, actives of the target side get `onSideConditionStart`, foes get `onFoeSideConditionStart`, and everyone gets
  `onAnySideConditionStart`. Fainted actives are included.
* Insertion order for a Pokémon target T on side S (T at slot k):
  `T.on* | S.active[0].onAlly*, S.active[0].onAny*, S.active[1].onAlly*, S.active[1].onAny* | foe[0].onFoe*, foe[0].onAny*, foe[1].onFoe*, foe[1].onAny* | source.onSource* | p1 side (on*/onFoe*, then onAny*) | p2 side (same) | field (pseudo-weathers, weather, terrain) | battle`.
  Each `fPEH` block is in status/volatiles/ability/item/species/slot order. The same Pokémon's handlers are
  re-collected once per prefix.
* `prefixed=false` events (`BeforeTurn, Update, Weather, WeatherChange, TerrainChange`) only collect
  `T.on*`, the target side's `on*` side conditions, and field/battle `on*`.

---

## 3. `resolvePriority(h, callbackName)` (`sim/battle.ts:954-1020`)
Runs at **collection time**. It snapshots `speed`.
```
h.order    = effect[callbackName+'Order']    || false     // 0 → false
h.priority = effect[callbackName+'Priority'] || 0
h.subOrder = effect[callbackName+'SubOrder'] || 0
if !h.subOrder:                                           // default subOrder by effect kind
    h.subOrder = { Condition:2, Weather:5, Format:5, Rule:5, Ruleset:5, Ability:7, Item:8 }[effect.effectType] || 0
    if effect.effectType == 'Condition':
        if h.state?.target is Side:  h.subOrder = h.state.isSlotCondition ? 3 : 4
        elif h.state?.target is Field: h.subOrder = 5
    elif effect.effectType == 'Ability':
        if effect.name in ['Poison Touch','Perish Body']: h.subOrder = 6
        elif effect.name == 'Stall': h.subOrder = 9
if callbackName.endsWith('SwitchIn') || callbackName.endsWith('RedirectTarget'):
    h.effectOrder = h.state?.effectOrder                  // otherwise effectOrder stays undefined (= 0 in compare)
if h.effectHolder has getStat (i.e. is a Pokemon):
    p = h.effectHolder
    h.speed = p.speed                                     // CACHED speed (see §3.1)
    if effect is Ability 'Magic Bounce' && callbackName == 'onAllyTryHitSide':
        h.speed = p.getStat('spe', true, true)            // raw storedStats.spe: no boosts, no events, NO Trick Room inversion
    if callbackName.endsWith('SwitchIn'):
        h.speed -= battle.speedOrder.indexOf(p.getFieldPositionValue()) / (activePerHalf*2)   // /4 in doubles
// non-Pokemon holders: speed stays undefined (= 0 in compare)
```
Resulting default subOrders:

| holder / kind | default subOrder |
|---|---|
| Status (`effectType 'Status'`) | **0** (not in the table) |
| volatile (`'Condition'`, state.target = Pokémon) | 2 |
| slot condition (`'Condition'`, state.target = Side) | 3, or **2 after the QUIRK mutation** (§1.2) |
| side condition | 4 |
| pseudo-weather (`'Condition'`) | **2 until first dispatched by runEvent, then 5** (§1.2 QUIRK) |
| weather (`'Weather'`) | 5 |
| terrain (`'Terrain'`) | **0** (not in the table) |
| Format / Rule (`sleepclausemod`) | 5 |
| Ability | 7 (Poison Touch, Perish Body 6; Stall 9) |
| Item | 8 |
| Move (onEffect / move handlers), species (`'Pokemon'`), ValidatorRule | 0 |

An explicit `…SubOrder` key always wins. Only `onResidualSubOrder`, `onFieldResidualSubOrder` and
`onSideResidualSubOrder` are used in Gen 9 data (Appendix A).

`getFieldPositionValue()` = `side.n + 2*position` (`sim/pokemon.ts:526-529`): p1a=0, p2a=1, p1b=2, p2b=3.
`battle.speedOrder` starts as `[0,1,2,3]` (`sim/battle.ts:263-266`). `runSwitch` sets it before every
`fieldEvent('SwitchIn')` (§5.6). `endsWith('SwitchIn')` is a suffix test, so it also covers
`onBeforeSwitchIn` (runEvent) and `onAnySwitchIn`. For BeforeSwitchIn the *previous* `speedOrder` is used.

### 3.1 The `pokemon.speed` cache (cross-reference: speed/queue doc)
`pokemon.speed` is a cached number that is **not** recomputed per event.
`updateSpeed()` sets `speed = getActionSpeed()` = `trunc(TR ? 10000 - getStat('spe') : getStat('spe'), 13)`
(`sim/pokemon.ts:556-558`, `641-649`). `getStat('spe')` runs `runEvent('ModifyBoost')` + `runEvent('ModifySpe')`,
which can themselves consume PRNG through ties.
`updateSpeed` is called from: `commitChoices` (`sim/battle.ts:2997`, all actives), the residual action
(`sim/battle.ts:2818`, all actives, *before* `fieldEvent('Residual')`), after each action when the next queued
action is a move (`sim/battle.ts:2917-2919`, all actives), and `queue.insertChoice` for the inserted Pokémon
(`sim/battle-queue.ts:379`). `setSpecies` sets `speed = storedStats.spe` with no modifiers and no TR
(`sim/pokemon.ts:1418`). It is reached via `clearVolatile` (construction, switch-out, faint), forme changes and
Transform.

---

## 4. Ordering

### 4.1 `comparePriority(a, b)` (default; `sim/battle.ts:408-415`)
```
ord(x) = x.order || 4294967296        // false/0/undefined → last
return (ord(a) - ord(b))                               // order ASC
    || ((b.priority||0) - (a.priority||0))             // priority DESC
    || ((b.speed||0)    - (a.speed||0))                // speed DESC
    || ((a.subOrder||0) - (b.subOrder||0))             // subOrder ASC
    || ((a.effectOrder||0) - (b.effectOrder||0))       // effectOrder ASC
    || 0
```
(JS `||` chaining. A NaN term is falsy and falls through. Speeds can be fractional only for `…SwitchIn`, in
multiples of 1/4, which is exact in f64.)
Note that speed is compared **before** subOrder. Field/side handlers in `runEvent` have speed 0, so for equal
order and priority they come after every Pokémon-held handler. subOrder only breaks ties within equal speed.
When `speedSort` is used on Pokémon objects (`runSwitch`, Neutralizing Gas, …), only `.speed` exists, so it is a
speed-DESC sort.

### 4.2 `compareRedirectOrder` (used by `priorityEvent`; `sim/battle.ts:417-423`)
```
(b.priority||0)-(a.priority||0)  ||  (b.speed||0)-(a.speed||0)
|| (a.effectHolder?.abilityState && b.effectHolder?.abilityState ? a.effectHolder.abilityState.effectOrder - b.effectHolder.abilityState.effectOrder : 0)
|| 0
```
It ignores `order` and `subOrder`. It is used with JS `Array.prototype.sort` (**stable**), so ties keep insertion
order. **No PRNG.**

### 4.3 `compareLeftToRightOrder` (`sim/battle.ts:425-430`)
`ord(a)-ord(b) || (b.priority||0)-(a.priority||0) || (a.index||0)-(b.index||0) || 0`, used with a stable `sort`.
It ignores speed and subOrder. **No PRNG.** It is used for `runEvent` when the event id is
`Invulnerability`, `TryHit`, `DamagingHit` or `EntryHazard` (`sim/battle.ts:793-794`). That applies to single-target
calls too (index undefined → 0). `EntryHazard` is never dispatched in this codebase.

### 4.4 `speedSort(list, cmp = comparePriority)` (`sim/battle.ts:433-464`), the only PRNG consumer in the core
```
if list.length < 2: return                         // no PRNG
sorted = 0
while sorted + 1 < list.length:
    next = [sorted]
    for i in sorted+1 .. list.length-1:
        d = cmp(list[next[0]], list[i])             // compare against the FIRST member of the current best group
        if d < 0: continue
        if d > 0: next = [i]
        if d === 0: next.push(i)                    // (d NaN → ignored)
    for k in 0 .. next.length-1:                    // move the group to the front, in index order
        idx = next[k]
        if idx != sorted + k: swap(list[sorted+k], list[idx])     // this also permutes non-selected elements!
    if next.length > 1:
        prng.shuffle(list, sorted, sorted + next.length)
    sorted += next.length
```
`PRNG.shuffle(items, start, end)` (`sim/prng.ts:150-158`), a forward Fisher–Yates:
```
while start < end - 1:
    j = prng.random(start, end)      // one next()
    if j != start: swap(items[start], items[j])
    start++
```
**PRNG cost:** each tie group of size k ≥ 2 costs exactly k−1 `next()` calls. Groups are processed from the front of
the sorted order to the back. The swap step changes the order of the unsorted remainder, so the pre-shuffle order
of later tie groups depends on it. Port the algorithm literally; do not swap in another sort.

Verified examples of PRNG consumption that has no visible effect, which must still be mirrored:
* `runEvent('ModifyDamage')` against a side with **Reflect + Light Screen**. Both are `onAnyModifyDamage` side
  conditions with order false, priority 0, speed 0 and subOrder 4, so they tie and cost 1 PRNG call.
* `fieldEvent('Residual')` with a Pokémon holding the **`protect` (duration 1) and `stall` (duration 2)** volatiles.
  Both are duration-only, order false, same holder speed, subOrder 2, so they tie and cost 1 call.
* `eachEvent(...)` with two actives at equal `speed`: 1 call per `eachEvent`, even with zero handlers.

---

## 5. Dispatchers

### 5.1 `singleEvent(eventid, effect, state, target, source?, sourceEffect?, relayVar?, customCallback?)` (`sim/battle.ts:575-657`)
This runs exactly one effect's handler, with no discovery and no sorting.
```
if eventDepth >= 8: add 'message' lines; throw "Stack overflow"
if log.length - sentLogPos > 1000: add 'message' lines; throw "Infinite loop"
hasRelayVar = true
if relayVar === undefined: relayVar = true; hasRelayVar = false          // NOTE: null IS a relay var here
if effect.effectType == 'Status' && target is Pokemon && target.status !== effect.id: return relayVar
if eventid == 'SwitchIn' && effect.effectType == 'Ability' && effect.flags.breakable && suppressingAbility(target): return relayVar
if eventid not in {'Start','TakeItem','SetAbility'} && effect.effectType == 'Item' && target is Pokemon && target.ignoringItem(): return relayVar
if eventid != 'End' && effect.effectType == 'Ability' && target is Pokemon && target.ignoringAbility(): return relayVar
if effect.effectType == 'Weather' && eventid not in {'FieldStart','FieldResidual','FieldEnd'} && field.suppressingWeather(): return relayVar
cb = customCallback || effect['on'+eventid]
if cb === undefined: return relayVar
save (effect, effectState, event)
battle.effect = effect; battle.effectState = state || initEffectState({}); battle.event = {id:eventid, target, source, effect:sourceEffect}
eventDepth++
args = [target, source, sourceEffect]; if hasRelayVar: args.unshift(relayVar)
ret = (typeof cb == 'function') ? cb.apply(battle, args) : cb
eventDepth--; restore (effect, effectState, event)
return ret === undefined ? relayVar : ret
```
(The `debug` lines that the suppression branches would print are emitted only in debug mode; see Ignored.)

### 5.2 `runEvent(eventid, target?, source?, sourceEffect?, relayVar?, onEffect?, fastExit?)` (`sim/battle.ts:762-941`)
```
if eventDepth >= 8: throw "Stack overflow" (after 'message' lines)
if !target: target = battle
effectSource = (source is Pokemon) ? source : null
H = findEventHandlers(target, eventid, effectSource)
if onEffect:                                                          // BasePower, Damage, ModifyTarget call sites
    if !sourceEffect: throw
    cb = sourceEffect['on'+eventid]
    if cb !== undefined:
        if target is Array: throw
        H.unshift(resolvePriority({effect: sourceEffect, callback: cb, state: initEffectState({}), end: null, effectHolder: target}, 'on'+eventid))
if eventid in ['Invulnerability','TryHit','DamagingHit','EntryHazard']: H.sort(compareLeftToRightOrder)   // stable, no PRNG
elif fastExit: H.sort(compareRedirectOrder)                                                              // stable, no PRNG
else: speedSort(H)                                                                                       // PRNG on ties
hasRelayVar = 1; args = [target, source, sourceEffect]
if relayVar === undefined || relayVar === null: relayVar = true; hasRelayVar = 0                       // null is NOT a relay var here
else: args.unshift(relayVar)
parent = battle.event; battle.event = {id:eventid, target, source, effect:sourceEffect, modifier:1}; eventDepth++
targetRelayVars = []
if target is Array: targetRelayVars = Array.isArray(relayVar) ? relayVar /*same array object*/ : target.map(_ => true)
for h in H:
    if h.index !== undefined:                                          // array target
        if !targetRelayVars[h.index] && !(targetRelayVars[h.index] === 0 && eventid == 'DamagingHit'): continue
        if h.target: args[hasRelayVar] = h.target; battle.event.target = h.target    // event.target is NOT restored between handlers
        if hasRelayVar: args[0] = targetRelayVars[h.index]
    eff = h.effect; holder = h.effectHolder
    if eff.effectType == 'Status' && holder.status !== eff.id: continue
    if eff.effectType == 'Ability' && eff.flags.breakable && suppressingAbility(holder): continue          // Mold Breaker (see §7.4 dead code)
    if eventid not in {'Start','SwitchIn','TakeItem'} && eff.effectType == 'Item' && holder is Pokemon && holder.ignoringItem(): continue
    elif eventid != 'End' && eff.effectType == 'Ability' && holder is Pokemon && holder.ignoringAbility(): continue
    if (eff.effectType == 'Weather' || eventid == 'Weather') && eventid not in {'Residual','End'} && field.suppressingWeather(): continue
    if typeof h.callback == 'function':
        save (effect, effectState); battle.effect = eff; battle.effectState = h.state || initEffectState({})
        battle.effectState.target = holder                             // QUIRK §1.2
        ret = h.callback.apply(battle, args)
        restore (effect, effectState)
    else: ret = h.callback                                             // constant handler
    if ret !== undefined:
        relayVar = ret
        if !relayVar || fastExit:
            if h.index !== undefined:
                targetRelayVars[h.index] = relayVar
                if targetRelayVars.every(v => !v): break
            else: break
        if hasRelayVar: args[0] = relayVar
eventDepth--
if typeof relayVar == 'number' && relayVar === Math.abs(Math.floor(relayVar)):      // non-negative integer
    relayVar = battle.modify(relayVar, battle.event.modifier)                         // applies chainModify() accumulation
battle.event = parent
return (target is Array) ? targetRelayVars : relayVar
```
Notes:
* Each per-handler check runs **at dispatch time**, after the earlier handlers have run. A handler that becomes
  suppressed mid-event is skipped, and one that becomes un-suppressed runs.
* For an **array target**, a truthy non-`undefined` return does **not** update `targetRelayVars[i]`. It only
  updates `relayVar`/`args[0]`, which the next handler overwrites with `targetRelayVars[index]`. So only falsy
  returns matter per target. A falsy return blocks every later handler for that target. The loop only breaks once
  every target is falsy.
* Array discovery repeats source, field and battle handlers once per target (§2.6), so e.g. a field effect's
  `onTryHit` runs once per target.
* `fastExit` (priorityEvent) breaks on the first non-`undefined` return, including truthy ones.
* With `onEffect`, the effect's own handler is unshifted and then sorted like the others. Its holder is the event
  `target` (for `BasePower` that is the *attacker*: `runEvent('BasePower', source, target, move, bp, true)`,
  `sim/battle-actions.ts:1637`). Its speed is that Pokémon's speed. Its priority is
  `sourceEffect['on'+E+'Priority']`. It goes through the same per-handler checks, which use the *holder*
  (e.g. an Ability `sourceEffect` of `Damage` is ability-suppression-checked against the damaged Pokémon).

### 5.3 `priorityEvent(eventid, target, source?, effect?, relayVar?, onEffect?)` (`sim/battle.ts:947-952`)
`= runEvent(eventid, target, source, effect, relayVar, onEffect, true)`. Call sites: `RedirectTarget`
(`sim/pokemon.ts:830`), `LockMove` (`sim/pokemon.ts:950`), `SemiLockMove` (`sim/pokemon.ts:960`),
`WeatherModifyDamage` (`sim/battle-actions.ts:1733`).

### 5.4 `eachEvent(eventid, effect?, relayVar?)` (`sim/battle.ts:469-480`)
```
actives = getAllActive()                       // p1 slots, then p2 slots; non-null && !fainted (hp-0-but-not-yet-fainted INCLUDED)
if !effect && battle.effect: effect = battle.effect      // battle.effect is always an object, so a missing effect becomes the current effect
speedSort(actives, (a, b) => b.speed - a.speed)          // PRNG on equal cached speeds, ALWAYS (even with no handlers)
for p in actives: runEvent(eventid, p, null, effect, relayVar)
if eventid == 'Weather': eachEvent('Update')             // gen >= 7
```
Call sites: `Update` (`sim/battle-actions.ts:83, 960, 996`, `sim/battle.ts:2849, 2865`; `2915` is gen<5),
`BeforeTurn` (`sim/battle.ts:2813`), `WeatherChange` (`sim/field.ts:87, 97`, `data/abilities.ts:98, 102, 551, 555`),
`TerrainChange` (`sim/field.ts:155, 165`), `Weather` (each weather's `onFieldResidual`, `data/conditions.ts:506…744`).

### 5.5 `fieldEvent(eventid, targets?)`: only `'Residual'` and `'SwitchIn'` (`sim/battle.ts:488-572`)
```
name = 'on'+eventid; getKey = (eventid == 'Residual') ? 'duration' : undefined
H = findFieldEventHandlers(field, 'onField'+eventid, getKey)                      // holder field, end = remove/clear fns
for side in sides (p1, p2):
    if side.n < 2 || !side.allySide: H = H.concat(findSideEventHandlers(side, 'onSide'+eventid, getKey))   // holder side
    for active in side.active:
        if !active: continue                                                     // fainted actives are NOT skipped here
        if eventid == 'SwitchIn': H = H.concat(fPEH(active, 'onAny'+eventid))    // for ALL actives, before the targets filter
        if targets && !targets.includes(active): continue
        H = H.concat(fPEH(active, name, getKey))
        H = H.concat(findSideEventHandlers(side, name, undefined, active))       // per-active copy: holder = active, end = null, no getKey
        H = H.concat(findFieldEventHandlers(field, name, undefined, active))     // per-active copy: holder = active, end = null
        H = H.concat(findBattleEventHandlers(name, getKey, active))              // (none in this format)
speedSort(H)
while H.length:
    h = H.shift(); eff = h.effect
    if h.effectHolder is Pokemon && h.effectHolder.fainted && !h.state?.isSlotCondition: continue
    if eventid == 'Residual' && h.end && h.state?.duration:
        h.state.duration--
        if h.state.duration == 0:
            (h.end).call(...(h.endCallArgs || [h.effectHolder, eff.id]))       // e.g. removeVolatile.call(pokemon, id)
            if ended: return
            continue                                                           // NO residual callback and NO faintMessages on the expiry turn
    // still-present check
    if h.state?.target is Pokemon:
        loc = (eff is Ability && !state.id.startsWith('ability:')) ? target.abilityState
            : (eff is Item && !state.id.startsWith('item:')) ? target.itemState
            : (eff is Status) ? target.statusState
            : target.volatiles[eff.id]                     // species states and MUTATED slot-condition states land here → mismatch → skipped (QUIRK)
        if loc !== h.state: continue
    elif h.state?.target is Side && !h.state.isSlotCondition:
        if target.sideConditions[eff.id] !== h.state: continue
    elif h.state?.target is Field:                         // only once the target was set by a runEvent dispatch (§1.2)
        loc = eff is Weather ? field.weatherState : eff is Terrain ? field.terrainState : field.pseudoWeather[eff.id]
        if loc !== h.state: continue
    // (state.target undefined → no check: a never-runEvent-dispatched pseudo-weather/weather/terrain removed earlier in this fieldEvent still runs)
    evid = eventid
    if h.effectHolder has sideConditions: evid = 'Side'+eventid
    if h.effectHolder has pseudoWeather:  evid = 'Field'+eventid
    if h.callback: singleEvent(evid, eff, h.state, h.effectHolder, null, null, undefined, h.callback)
    faintMessages()
    if ended: return
```
* `end` functions are unbound methods called with `this = effectHolder`, plus `endCallArgs` for slot conditions:
  `clearStatus.call(pokemon)`, `removeVolatile.call(pokemon, id)`, `clearAbility/clearItem.call(pokemon)`,
  `removeSideCondition.call(side, id)`, `removeSlotCondition.call(side, pokemon, id)`,
  `removePseudoWeather.call(field, id)`, `clearWeather.call(field)`, `clearTerrain.call(field)`.
  Per-active copies of side/field handlers and battle handlers have `end = null`, so they never decrement.
* Duration bookkeeping therefore happens once, on the holder that owns `end`. That is the volatile/status/slot
  holder Pokémon, the side (via `onSideResidual*` keys) or the field (via `onFieldResidual*` keys). This is why
  Reflect/Tailwind/Trick Room/terrains have `onSideResidualOrder`/`onFieldResidualOrder` keys with no callback.
* Dispatch goes through `singleEvent` with the custom callback, so the singleEvent suppression rules apply with
  `eventid` = `Residual`/`SwitchIn`/`SideResidual`/`FieldResidual`/`SideSwitchIn`/`FieldSwitchIn`. Example: an item's
  remapped `onStart` at switch-in **is** blocked by `ignoringItem()`, because 'SwitchIn' is not exempt in
  `singleEvent`. Breakable switch-in abilities are blocked by an active Mold Breaker move (drag-in during the move).
  For `onAnySwitchIn`, the `target` passed is the **holder**, not the Pokémon that switched in.
* There are no `onFieldSwitchIn`/`onSideSwitchIn`/`onFoeSwitchIn`/`onAllySwitchIn` handlers in Gen 9 data.

### 5.6 `runSwitch(pokemon)` prelude (`sim/battle-actions.ts:174-191`)
```
switchersIn = [pokemon] + every immediately following queued 'runSwitch' action's pokemon (shifted off the queue)
allActive = getAllActive(true)            // includes fainted actives
speedSort(allActive)                      // comparePriority on Pokemon objects = speed DESC; PRNG on ties
battle.speedOrder = allActive.map(p => p.getFieldPositionValue())
fieldEvent('SwitchIn', switchersIn)
```
The fractional speed (`- index/4`, §3) makes every Pokémon's switch-in handlers keep this pre-sorted order with
no second shuffle. Within one Pokémon, ties are broken by subOrder and then effectOrder (creation order). That is
how multiple hazards on a side resolve in the order they were set.

---

## 6. Return values and the relay variable

| handler returns | `runEvent` | `priorityEvent` | `singleEvent` |
|---|---|---|---|
| `undefined` / no return | relayVar unchanged, next handler | same | returns relayVar (or `true` if none passed) |
| truthy `v` (incl. `true`, objects, non-zero numbers, non-empty strings) | relayVar = v; `args[0] = v` if hasRelayVar; continue | relayVar = v; **stop** | returns v |
| `false` (FAIL) | relayVar = false; **stop** | stop | returns false |
| `null` (SILENT_FAIL) | relayVar = null; **stop** | stop | returns null |
| `0` (HIT_SUBSTITUTE) | relayVar = 0; **stop** | stop | returns 0 |
| `''` (NOT_FAIL) | relayVar = ''; **stop** | stop | returns '' |

Constants on `Battle`: `NOT_FAIL = ''`, `HIT_SUBSTITUTE = 0`, `FAIL = false`, `SILENT_FAIL = null`
(`sim/battle.ts:272-275`). Their meaning comes from the callers:
* `false` = loud failure (the caller prints `-fail`/"But it failed!"). `null` = silent failure (the handler printed its
  own message). `undefined`/`true` = success / keep going.
* `''` (NOT_FAIL) stops the chain and is falsy, but callers treat it as "not a failure":
  `sim/battle-actions.ts:507` (`moveThisTurnResult = null`), `:594` (`return hitResult === NOT_FAIL`), and
  `:646` (`TryHit` results that are not NOT_FAIL are coerced to `|| false`). It is used 34 times in `data/moves.ts`.
* `0` from `TryPrimaryHit` (Substitute, `data/moves.ts:18370`) means "the substitute took the hit"
  (`sim/battle-actions.ts:1053`).
* Merging per-target results across hits uses `combineResults` (`sim/battle-actions.ts:1548-1563`): priority
  by `typeof` `undefined < string('') < object(null) < boolean < number`; numbers add. That belongs to the move
  pipeline doc.

`relayVar` passing: `runEvent` only passes a relay variable when it is neither `undefined` nor `null`. Otherwise
handlers get `(target, source, effect)` and the hidden relay starts at `true`. `singleEvent` passes it whenever it
is not `undefined` (so a `null` *is* passed as the first argument).

### 6.1 Modifiers (`sim/battle.ts:2309-2347, 2385-2389`)
* `chainModify(n, d=1)`: `prev = trunc(event.modifier*4096)`, `next = trunc(n*4096/d)`,
  `event.modifier = ((prev*next + 2048) >> 12) / 4096`. `>>` is the JS **int32** shift; use i32 wrapping
  semantics. `n` may be `[num, den]`.
* At the end of `runEvent`, a non-negative integer relayVar becomes `modify(relayVar, event.modifier)`:
  `m = trunc(mod*4096)`, `result = trunc((trunc(v*m) + 2047) / 4096)`, where `trunc(x) = x >>> 0` (u32 wrap).
  With `modifier = 1` this is the identity for `v < 2^20`. It is applied even after an early stop, and for *every*
  numeric event (priorities, damage, stats, weights…). Negative or fractional results are left alone.
* `finalModify(v)` = `modify(v, event.modifier)` then `event.modifier = 1`. It is used by paralysis `onModifySpe`
  (`data/conditions.ts:33`).
* `singleEvent`'s event object has **no** `modifier`. `chainModify` there computes `trunc(NaN)` = 0, so the modifier
  becomes 0, and it is never applied. It is harmless but must not crash.

---

## 7. Suppression

### 7.1 `pokemon.ignoringAbility()` (`sim/pokemon.ts:858-877`)
```
if !isActive: return true                                   // gen >= 5
if getAbility().flags.notransform && transformed: return true
if getAbility().flags.cantsuppress: return false
if volatiles.gastroacid: return true                        // Gastro Acid
if hasItem('Ability Shield') || ability == 'neutralizinggas': return false
for p in battle.getAllActive():                             // non-fainted actives, p1 then p2
    if p.ability == 'neutralizinggas' && !p.volatiles.gastroacid && !p.transformed && !p.abilityState.ending && !this.volatiles.commanding:
        return true                                          // Neutralizing Gas
return false
```
### 7.2 `pokemon.ignoringItem(isFling=false)` (`sim/pokemon.ts:879-886`)
```
if getItem().isPrimalOrb: return false                      // (no primal orbs in Gen 9)
if !isActive: return true
if volatiles.embargo || field.pseudoWeather.magicroom: return true     // Embargo, Magic Room
if isFling: return hasAbility('klutz')
return !getItem().ignoreKlutz && hasAbility('klutz')        // Klutz
```
`hasItem(x)` = id match `&& !ignoringItem()`. `hasAbility(x)` = id match `&& !ignoringAbility()`. Keep the
short-circuit order: it is what prevents infinite recursion (Ability Shield has `ignoreKlutz`).
Both checks look at the holder's **current** ability/item, not the handler's effect. A handler from an item that was
removed mid-event still runs in `runEvent` unless the holder is now item-ignoring.

### 7.3 `suppressingAbility(target)` (Mold Breaker family; `sim/battle.ts:365-368`)
`activePokemon?.isActive && activePokemon !== target && activeMove?.ignoreAbility && !target?.hasItem('Ability Shield')`
(the gen<8 self-clause is ignored). It applies only to abilities with `flags.breakable`.

### 7.4 Where each check applies

| check | `singleEvent` (target) | `runEvent` (effectHolder) | `fieldEvent` |
|---|---|---|---|
| Status no longer current | yes | yes | via singleEvent + location check |
| Mold Breaker (`breakable` ability) | **only `eventid=='SwitchIn'`** | every event | SwitchIn only |
| `ignoringItem()` for Item effects | all except `Start`, `TakeItem`, `SetAbility` | all except `Start`, `SwitchIn`, `TakeItem` | via singleEvent (Residual and SwitchIn suppressed) |
| `ignoringAbility()` for Ability effects | all except `End` | all except `End` | via singleEvent |
| `suppressingWeather()` (Air Lock/Cloud Nine) | `effectType=='Weather'` except `FieldStart/FieldResidual/FieldEnd` | `effectType=='Weather'` **or `eventid=='Weather'`** (so ALL `onWeather` handlers, e.g. Ice Body/Dry Skin, are skipped), except `Residual`/`End` | via singleEvent |
| holder fainted | – | (only implicitly via `isActive`/`hp` filters in discovery) | skipped unless slot condition |

`field.suppressingWeather()` (`sim/field.ts:106-116`): any non-null, non-fainted active with
`!ignoringAbility() && getAbility().suppressWeather && !abilityState.ending`.

Dead code: in `runEvent` (`sim/battle.ts:840-876`) the `if (!effect.num) { AttackingEvents … }` branch can never run,
because the outer condition already requires `flags.breakable` and the first inner `if (effect.flags['breakable'])`
always `continue`s. Implement only the "breakable → skip" behaviour.

Suppression checks never dispatch events and never touch the PRNG.

---

## 8. Limits

* `eventDepth >= 8` at the start of `singleEvent` or `runEvent` adds four `|message|` lines and **throws**
  (`sim/battle.ts:580-587`, `770-777`). `eachEvent`/`fieldEvent` don't count, but every `runEvent` they make does.
* `log.length - sentLogPos > 1000` in `singleEvent` adds message lines and throws "Infinite loop" (`sim/battle.ts:588-594`).
  `sentLogPos` advances only in `sendUpdates()` (`sim/battle.ts:3265-3273`).
* For differential fuzzing, treat either throw as "Showdown crashed". The Rust engine should assert or report the
  same condition rather than diverge silently.

---

## 9. Format and rule handlers in this format

`ruleTable` iteration order (Map insertion order, verified):
`potd, obtainable, obtainablemoves, obtainableabilities, obtainableformes, evlimit, obtainablemisc, -unreleased, -tag:unobtainable, -nonexistent, speciesclause, hppercentagemod, cancelmod, illusionlevelmod, sleepclausemod`.

1. **Registering rules as pseudo-weather** (`sim/battle.ts:293-307`, in the Battle constructor, before sides exist).
   For each rule not starting with `+*-!`: if `dex.formats.get(rule)` exists and has an *own key* starting with
   `on` that is not one of `onBegin, onTeamPreview, onBattleStart, onValidateRule, onValidateTeam, onChangeSet, onValidateSet`,
   then `field.addPseudoWeather(rule)`. In this format **only `sleepclausemod`** qualifies, through `onSetStatus`.
   Obtainable has `onValidateTeam` and Obtainable Misc has `onChangeSet`, both excluded.
   Effect: `field.pseudoWeather = { sleepclausemod: {id, source: null, sourceSlot: undefined, duration: undefined, effectOrder: 0} }`.
   `addPseudoWeather` also runs `singleEvent('FieldStart')` (no callback → true) and
   `runEvent('PseudoWeatherChange', battle, null, rule)` (no handlers exist yet). There is no log output and the
   PRNG and effectOrder counter are untouched.
2. **`onBegin`** (`sim/battle.ts:1948-1953`, in `start()` after `|gen|`/`|tier|`): `format.onBegin` (undefined),
   then each rule in the order above. Output:
   `|rule|Species Clause: Limit one of each Pokémon`, `|rule|HP Percentage Mod: HP is shown in percentages`
   (+ `reportPercentages = true`, which has no further effect in Gen ≥7 because `getHealth` already uses
   percentages, `sim/pokemon.ts:2075`), Cancel Mod sets `supportCancel = true` (no log; affects request `noCancel` /
   `cantUndo`, `sim/battle.ts:1462, 3062`), `|rule|Illusion Level Mod: Illusion disguises the Pokémon's true level`
   (behaviour lives in `getFullDetails`, `sim/pokemon.ts:546-549`), `|rule|Sleep Clause Mod: Limit one foe put to sleep`.
   PotD prints only if `global.Config?.potd` is set. That never happens when the sim runs as a library, so nothing
   is printed. The Obtainable family, EV Limit and `-…` bans have no `onBegin`.
3. **`onBattleStart`**: none of these rules or the format define it (`sim/battle.ts:2686-2691`). Separately, the
   `start` action calls `singleEvent('BattleStart', conditions.getByID(pokemon.species.id), pokemon.speciesState, pokemon)`
   for **every** Pokémon of p1 then p2 (`sim/battle.ts:2682-2684`). Only `zacian`/`zamazenta` (Condition ids) have
   handlers. Note that it looks up the condition by the *species* id, so e.g. `zaciancrowned` has none.
4. **Sleep Clause Mod `onSetStatus`** (`data/rulesets.ts:1379-1403`):
   ```
   onSetStatus(status, target, source):
     if source?.isAlly(target): return
     if status.id == 'slp':
       for p in target.side.pokemon:                                   // party order
         if p.hp && p.status == 'slp' && !p.statusState.source?.isAlly(p):
           add('-message', 'Sleep Clause Mod activated.'); hint("Sleep Clause Mod prevents players from putting more than one of their opponent's Pokémon to sleep at a time")
           return false
   ```
   It is collected as a field handler in `runEvent('SetStatus', …)` (`sim/pokemon.ts:1724`): holder = field, speed 0,
   priority 0, order false, subOrder 5 ('Rule'). It sorts after every Pokémon-held `onSetStatus`/`onAllySetStatus`/…,
   after side conditions (subOrder 4, e.g. Safeguard), and after terrains (subOrder 0: Electric/Misty Terrain). A
   falsy return from any of those stops the chain before Sleep Clause can print. `hint(msg)` is called without
   `once`, so `|-hint|…` is printed **every** time the clause triggers (`sim/battle.ts:3069-3079`).
5. Species Clause / Obtainable `onValidateTeam`, Obtainable Misc `onChangeSet`: validator-only, **never run by the sim**.
   Random-battle teams are not validated by `Battle`.

---

## 10. Quirk and porting checklist

1. Collect, then sort, then run. Snapshot speeds at collection time. Recheck suppression at dispatch time.
2. Reproduce discovery insertion order exactly (§2). The selection sort's swap pattern + shuffle depends on it.
3. Suppressed, skipped, duration-only and fainted-holder handlers still take part in sorting and ties.
4. `eachEvent` shuffles speed-tied actives every call. `runSwitch` shuffles speed-tied actives (fainted included)
   before every switch-in batch.
5. `TryHit`/`Invulnerability`/`DamagingHit` use a stable left-to-right sort, and `priorityEvent` uses a stable
   redirect sort. Neither consumes PRNG.
6. The `runEvent` mutation `state.target = holder` changes pseudo-weather subOrder (2 → 5) and breaks slot
   conditions after a `Swap` (subOrder 3 → 2, skipped by `fieldEvent`).
7. Terrain and Status handlers have default subOrder 0. Weather and Rule have 5.
8. Magic Bounce's `onAllyTryHitSide` uses raw `storedStats.spe` (no Trick Room) as its sort speed.
9. Item handlers are suppressed in `singleEvent('SwitchIn')`/`('Residual')` but not in `runEvent('SwitchIn')`.
   Ability handlers are suppressed everywhere except `End`. Mold Breaker only affects `singleEvent` for `SwitchIn`.
10. `eventid == 'Weather'` suppresses *all* handlers under Air Lock/Cloud Nine, not just weather effects.
11. Array-target events only propagate falsy per-target results.
12. Numeric non-negative integer results of every `runEvent` pass through `modify(…, event.modifier)`.
13. On a Residual expiry turn, the effect's `end` runs, its residual callback does **not**, and `faintMessages()`
    is **not** called for that entry.
14. `effectOrder` is a global creation counter. It only advances for states with an id and a target (an active
    Pokémon, a Side). It only influences ordering for `…SwitchIn` callbacks, and through
    `abilityState.effectOrder` in `compareRedirectOrder`.
15. Debug lines are emitted only when `battle.debugMode` is set. Run the reference Showdown with debug off (this format
    has no `debug: true`), or the protocol logs will differ.

---

## 11. Dispatch call-site inventory (which event is fired how, and from where)

Legend: `R` = `runEvent`, `R+` = `runEvent(..., onEffect=true)`, `S` = `singleEvent`, `P` = `priorityEvent`,
`E` = `eachEvent`, `F` = `fieldEvent`. "data" counts calls made from inside handlers in `data/{moves,abilities,items,conditions}.ts`.
The dynamic names `'Modify'+stat` (`sim/pokemon.ts:634`, `sim/battle-actions.ts:1695-1696`) are expanded.
`fieldEvent` additionally dispatches `SideResidual`/`FieldResidual`/`SideSwitchIn`/`FieldSwitchIn` to side and field
holders (§5.5). Events only used by ignored mechanics: `AfterMega` (Mega), `Drive`/`Memory` (non-standard items).
`EntryHazard` appears only in the sort list (`sim/battle.ts:793`) and is never fired.

| event | sim call sites | data call sites |
|---|---|---|
| Accuracy | R battle-actions.ts:732; R battle-actions.ts:930 | — |
| AfterBoost | R battle.ts:2087 | — |
| AfterEachBoost | R battle.ts:2080 | — |
| AfterFaint | R battle.ts:2605 | — |
| AfterHit | S battle-actions.ts:1116 | — |
| AfterMega | R battle-actions.ts:1901 | — |
| AfterMove | S battle-actions.ts:311; R battle-actions.ts:312 | — |
| AfterMoveSecondary | S battle-actions.ts:808; R battle-actions.ts:809 | — |
| AfterMoveSecondarySelf | S battle-actions.ts:536; R battle-actions.ts:537 | S×1 |
| AfterSetStatus | R pokemon.ts:1746 | — |
| AfterSubDamage | — | S×1 R×1 |
| AfterTakeItem | R pokemon.ts:1862 | — |
| AfterTerastallization | R battle-actions.ts:1953 | — |
| AfterUseItem | R pokemon.ts:1805; R pokemon.ts:1845 | R×4 |
| Attract | — | R×1 |
| BasePower | R+ battle-actions.ts:1637 | — |
| BattleStart | S battle.ts:2683 | — |
| BeforeFaint | R battle.ts:2555 | — |
| BeforeMove | R battle-actions.ts:255 | — |
| BeforeSwitchIn | R battle-actions.ts:144 | — |
| BeforeSwitchOut | R battle-actions.ts:81; R battle.ts:2896 | — |
| BeforeTurn | E battle.ts:2813 | — |
| ChangeBoost | R battle.ts:2036 | — |
| ChargeMove | — | R×16 |
| CheckShow | S battle.ts:2768 | — |
| Copy | S pokemon.ts:1266 | — |
| CriticalHit | R battle-actions.ts:1633 | — |
| Damage | R+ battle.ts:2125 | — |
| DamagingHit | R battle-actions.ts:1112; R battle-actions.ts:1120 | — |
| DeductPP | R battle-actions.ts:476 | — |
| DisableMove | R battle.ts:1695; S battle.ts:1698 | — |
| DragOut | R battle-actions.ts:169; R battle-actions.ts:1347 | R×1 |
| Drive | — | R×1 |
| Eat | S pokemon.ts:1785 | S×5 |
| EatItem | R pokemon.ts:1786 | R×5 S×1 |
| Effectiveness | S pokemon.ts:2215; R pokemon.ts:2216 | — |
| EmergencyExit | R battle-actions.ts:529; R battle-actions.ts:539; R battle-actions.ts:1007; R battle-actions.ts:1122; R battle-actions.ts:1383; R battle.ts:2867; R battle.ts:2872 | — |
| End | S battle-actions.ts:103; S battle-actions.ts:104; S battle-actions.ts:300; S battle-actions.ts:1926; S battle.ts:1337; S battle.ts:1338; S battle.ts:2560; S battle.ts:2561; S pokemon.ts:1861; S pokemon.ts:1884; S pokemon.ts:1923; S pokemon.ts:2040; S side.ts:508 | S×7 |
| Faint | R battle.ts:2559 | — |
| FieldEnd | S field.ts:94; S field.ts:162; S field.ts:227 | — |
| FieldRestart | S field.ts:198 | — |
| FieldStart | S field.ts:82; S field.ts:150; S field.ts:210 | — |
| Flinch | — | R×1 |
| FoeMaybeTrapPokemon | S battle.ts:1759 | — |
| FractionalPriority | R battle-queue.ts:249 | — |
| Heal | R battle.ts:2305 | — |
| Hit | S battle-actions.ts:1268; R battle-actions.ts:1272 | — |
| HitField | S battle-actions.ts:1258 | — |
| HitProtect | R battle.ts:1306 | — |
| HitSide | S battle-actions.ts:1263 | — |
| Immunity | R pokemon.ts:2280 | — |
| Invulnerability | R battle-actions.ts:626 | R×2 |
| LockMove | P pokemon.ts:950 | — |
| MaybeTrapPokemon | R battle.ts:1733 | — |
| Memory | — | R×1 |
| ModifyAccuracy | R battle-actions.ts:708; R battle-actions.ts:928 | — |
| ModifyAtk | R pokemon.ts:634; R battle-actions.ts:1695-1696 | — |
| ModifyBoost | R battle-actions.ts:712; R battle-actions.ts:716; R battle-actions.ts:910; R battle-actions.ts:919; R pokemon.ts:581; R pokemon.ts:618 | — |
| ModifyCritRatio | R battle-actions.ts:1611 | — |
| ModifyDamage | R battle-actions.ts:1813 | — |
| ModifyDef | R pokemon.ts:634; R battle-actions.ts:1695-1696 | — |
| ModifyMove | S battle-actions.ts:431; R battle-actions.ts:439 | — |
| ModifyPriority | S battle.ts:2649; R battle.ts:2650 | — |
| ModifySTAB | R battle-actions.ts:1776 | — |
| ModifySecondaries | R battle-actions.ts:1330 | — |
| ModifySpA | R pokemon.ts:634; R battle-actions.ts:1695-1696 | — |
| ModifySpD | R pokemon.ts:634; R battle-actions.ts:1695-1696 | — |
| ModifySpe | R pokemon.ts:634 | — |
| ModifySpecies | R pokemon.ts:1388 | — |
| ModifyTarget | R+ battle-actions.ts:416 | — |
| ModifyType | S battle-actions.ts:395; S battle-actions.ts:403; R battle-actions.ts:404; S battle-actions.ts:430; R battle-actions.ts:438 | — |
| ModifyWeight | R pokemon.ts:686 | — |
| MoveAborted | R battle-actions.ts:257 | — |
| MoveFail | S battle-actions.ts:527 | — |
| NegateImmunity | R pokemon.ts:2249 | — |
| OverrideAction | R battle-actions.ts:228 | — |
| PrepareHit | S battle-actions.ts:587; R battle-actions.ts:588; S battle-actions.ts:820; R battle-actions.ts:821 | R×1 |
| PseudoWeatherChange | R field.ts:214 | — |
| RedirectTarget | P pokemon.ts:830 | — |
| Residual | F battle.ts:2820 | — |
| Restart | S pokemon.ts:1985; S side.ts:476 | — |
| SemiLockMove | P pokemon.ts:960 | — |
| SetAbility | R battle.ts:1324; R battle.ts:1326; R pokemon.ts:1920 | — |
| SetStatus | R pokemon.ts:1724 | — |
| SetWeather | R field.ts:55 | — |
| SideConditionStart | R side.ts:441 | — |
| SideEnd | S side.ts:459 | — |
| SideRestart | S side.ts:424 | — |
| SideStart | S side.ts:437 | — |
| StallMove | — | R×10 |
| Start | S battle.ts:1346; S battle.ts:1347; S pokemon.ts:1739; S pokemon.ts:1886; S pokemon.ts:1943; S pokemon.ts:2009; S side.ts:490 | S×1 |
| Swap | R battle.ts:1608; R battle.ts:1609 | S×2 |
| SwitchIn | F battle-actions.ts:184 | — |
| SwitchOut | R battle-actions.ts:87 | — |
| TakeItem | R pokemon.ts:1856 | S×12 |
| TerrainChange | E field.ts:155; E field.ts:165 | S×2 |
| TrapPokemon | R battle.ts:1731 | — |
| Try | S battle-actions.ts:586; S battle-actions.ts:819 | — |
| TryAddVolatile | R pokemon.ts:1994 | — |
| TryBoost | R battle.ts:2038 | — |
| TryEatItem | R pokemon.ts:1781 | — |
| TryHeal | R battle.ts:2275 | R×11 |
| TryHit | R battle-actions.ts:640; S battle-actions.ts:1035 | R×5 |
| TryHitField | R battle-actions.ts:832; S battle-actions.ts:1031 | — |
| TryHitSide | R battle-actions.ts:834; R battle-actions.ts:839; S battle-actions.ts:1033 | — |
| TryImmunity | S battle-actions.ts:669 | — |
| TryMove | S battle-actions.ts:486; R battle-actions.ts:488 | — |
| TryPrimaryHit | R battle-actions.ts:1133 | — |
| TryTerrain | R field.ts:171 | — |
| Type | R pokemon.ts:2142 | — |
| Update | E battle-actions.ts:83; E battle-actions.ts:960; E battle-actions.ts:996; E battle.ts:478; E battle.ts:2849; E battle.ts:2865; E battle.ts:2915 | S×1 |
| Use | S pokemon.ts:1839 | — |
| UseItem | R pokemon.ts:1780; R pokemon.ts:1822 | — |
| UseMoveMessage | S battle-actions.ts:494 | — |
| Weather | — | E×8 |
| WeatherChange | E field.ts:87; E field.ts:97 | E×4 S×3 R×3 |
| WeatherModifyDamage | P battle-actions.ts:1733 | — |

---

## 12. Event-key table: every `on[A-Z]\w*` key in `data/{moves,abilities,items,conditions}.ts`

How this was produced: it was generated from the compiled dex (`dist/`) by walking every entry of
Moves/Abilities/Items/Conditions plus their nested `condition`, `secondary`/`secondaries[]` and `self` objects.
Each such object counts as one "effect".
* `#all` counts effects in the data files, including `isNonstandard` entries.
* `#gen9` counts entries without `isNonstandard`. A nested object inherits its parent's legality. All 35
  `conditions.ts` entries count as Gen 9.
* `#RD` is an approximation of what Random Doubles can reach. Moves = every `movepool` in
  `data/random-battles/gen9/doubles-sets.json` + Struggle. Abilities = set abilities + abilities of in-battle formes
  (no Mega/Primal/Gmax). Items = item names appearing as string literals in `data/random-battles/gen9/teams.ts` +
  species `requiredItem(s)`. Move `condition`s count as reachable when the move is in the pool **or** a reachable
  ability/item/move creates that condition id (e.g. Electric Surge → `electricterrain`, Cursed Body → `disable`).
  Excluded conditions: dynamax, silvally, hail, gem, rolloutstorage, futuremove, healreplacement, commanded,
  commanding, lockedmove.
  Treat `#RD` as a guide. **The engine should implement every `#gen9` handler**, because the fuzzer may feed
  arbitrary legal teams.
* The `…Priority/Order/SubOrder` columns list the values used by Gen 9-legal effects, as `value×count`. Blank means
  default (order false, priority 0, subOrder from §3).
* Rows are callback keys (= `'on' + prefix + event`). "base event (prefix)" shows how `findEventHandlers` reaches
  the key. Some keys are reachable in two ways. `onFoeMaybeTrapPokemon` is both the `Foe`-prefixed
  `MaybeTrapPokemon` and the literal `singleEvent('FoeMaybeTrapPokemon')` (`sim/battle.ts:1759`).
  `onAnySwitchIn` is only collected by `fieldEvent('SwitchIn')`.
* Keys that are **not events**: `onPlate` (data), and the `…Priority/Order/SubOrder` keys themselves. Watch out for
  name collisions. `onModifyPriority` and `onFractionalPriority` *are* events, and their priority keys are
  `onModifyPriorityPriority` (unused) and `onFractionalPriorityPriority` (Quick Claw/Custap −2, Quick Draw/Mycelium
  Might −1). `onFractionalPriority: -0.1` on Stall and Lagging Tail is a constant handler, not a priority key.
* `onSideResidual` has no callback anywhere. It appears only as `onSideResidualOrder/SubOrder` keys, which order the
  duration-only side-condition entries in `fieldEvent('Residual')`. Likewise most `onFieldResidualOrder/SubOrder`
  entries have no callback (pseudo-weathers and terrains), and the weathers have real `onFieldResidual` callbacks.
* Not in this table: the single rule handler, Sleep Clause Mod's `onSetStatus` (§9.4, `data/rulesets.ts:1387`).
  There are no Format handlers.

Summary: 162 keys (161 handler keys + `onSideResidual`). 153 are used by at least one Gen 9-legal effect, and 120 by
at least one RD-reachable effect. 815 Gen 9-legal effects (436 RD-reachable) have at least one handler.

| callback key | base event (prefix) | dispatched via | #all | #gen9 | #RD | `…Priority` values (gen9) | `…Order` values (gen9) | `…SubOrder` values (gen9) |
|---|---|---|---|---|---|---|---|---|
| `onAccuracy` | Accuracy | runEvent | 3 | 2 | 1 |  |  |  |
| `onAfterBoost` | AfterBoost | runEvent | 3 | 3 | 1 |  |  |  |
| `onAfterEachBoost` | AfterEachBoost | runEvent | 2 | 2 | 2 |  |  |  |
| `onAfterHit` | AfterHit | singleEvent | 8 | 8 | 6 |  |  |  |
| `onAfterMove` | AfterMove | singleEvent, runEvent | 7 | 6 | 1 |  |  |  |
| `onAfterMoveSecondary` | AfterMoveSecondary | singleEvent, runEvent | 9 | 9 | 3 | 2×1 |  |  |
| `onAfterMoveSecondarySelf` | AfterMoveSecondarySelf | singleEvent, runEvent | 8 | 7 | 4 | -1×1 |  |  |
| `onAfterSetStatus` | AfterSetStatus | runEvent | 2 | 2 | 2 | -1×1 |  |  |
| `onAfterSubDamage` | AfterSubDamage | singleEvent, runEvent | 12 | 8 | 7 |  |  |  |
| `onAfterTerastallization` | AfterTerastallization | runEvent | 1 | 1 | 1 |  |  |  |
| `onAfterUseItem` | AfterUseItem | runEvent | 1 | 1 | 1 |  |  |  |
| `onAllyAfterUseItem` | Ally+AfterUseItem | runEvent | 1 | 1 | 0 |  |  |  |
| `onAllyBasePower` | Ally+BasePower | runEvent+onEffect | 3 | 3 | 2 | 22×3 |  |  |
| `onAllyFaint` | Ally+Faint | runEvent | 2 | 2 | 0 |  |  |  |
| `onAllyModifyAtk` | Ally+ModifyAtk | runEvent | 1 | 1 | 0 | 3×1 |  |  |
| `onAllyModifySpD` | Ally+ModifySpD | runEvent | 1 | 1 | 0 | 4×1 |  |  |
| `onAllySetStatus` | Ally+SetStatus | runEvent | 3 | 3 | 1 |  |  |  |
| `onAllyTryAddVolatile` | Ally+TryAddVolatile | runEvent | 3 | 3 | 2 |  |  |  |
| `onAllyTryBoost` | Ally+TryBoost | runEvent | 1 | 1 | 1 |  |  |  |
| `onAllyTryHitSide` | Ally+TryHitSide | runEvent, singleEvent | 5 | 3 | 3 |  |  |  |
| `onAnyAccuracy` | Any+Accuracy | runEvent | 1 | 1 | 1 |  |  |  |
| `onAnyAfterMega` | Any+AfterMega | runEvent | 4 | 4 | 1 |  |  |  |
| `onAnyAfterMove` | Any+AfterMove | singleEvent, runEvent | 4 | 4 | 1 |  |  |  |
| `onAnyAfterSetStatus` | Any+AfterSetStatus | runEvent | 1 | 1 | 1 |  |  |  |
| `onAnyAfterTerastallization` | Any+AfterTerastallization | runEvent | 2 | 2 | 0 |  |  |  |
| `onAnyBasePower` | Any+BasePower | runEvent+onEffect | 3 | 2 | 0 | 20×2 |  |  |
| `onAnyDamage` | Any+Damage | runEvent+onEffect | 1 | 1 | 0 |  |  |  |
| `onAnyDragOut` | Any+DragOut | runEvent | 1 | 0 | 0 |  |  |  |
| `onAnyFaint` | Any+Faint | runEvent | 1 | 1 | 1 | 1×1 |  |  |
| `onAnyInvulnerability` | Any+Invulnerability | runEvent | 2 | 1 | 1 | 1×1 |  |  |
| `onAnyModifyAccuracy` | Any+ModifyAccuracy | runEvent | 1 | 1 | 0 | -1×1 |  |  |
| `onAnyModifyAtk` | Any+ModifyAtk | runEvent | 1 | 1 | 1 |  |  |  |
| `onAnyModifyBoost` | Any+ModifyBoost | runEvent | 1 | 1 | 1 |  |  |  |
| `onAnyModifyDamage` | Any+ModifyDamage | runEvent | 4 | 4 | 4 |  |  |  |
| `onAnyModifyDef` | Any+ModifyDef | runEvent | 1 | 1 | 1 |  |  |  |
| `onAnyModifySpA` | Any+ModifySpA | runEvent | 1 | 1 | 1 |  |  |  |
| `onAnyModifySpD` | Any+ModifySpD | runEvent | 1 | 1 | 1 |  |  |  |
| `onAnyPrepareHit` | Any+PrepareHit | singleEvent, runEvent | 1 | 0 | 0 |  |  |  |
| `onAnyPseudoWeatherChange` | Any+PseudoWeatherChange | runEvent | 1 | 1 | 0 |  |  |  |
| `onAnyRedirectTarget` | Any+RedirectTarget | priorityEvent | 2 | 2 | 2 |  |  |  |
| `onAnySetStatus` | Any+SetStatus | runEvent | 1 | 1 | 0 |  |  |  |
| `onAnySetWeather` | Any+SetWeather | runEvent | 3 | 3 | 0 |  |  |  |
| `onAnySwitchIn` | Any+SwitchIn | fieldEvent(SwitchIn) only | 6 | 6 | 1 | -4×1, -3×2, -2×2 |  |  |
| `onAnyTryMove` | Any+TryMove | singleEvent, runEvent | 1 | 1 | 0 |  |  |  |
| `onAnyTryPrimaryHit` | Any+TryPrimaryHit | runEvent | 1 | 1 | 0 |  |  |  |
| `onAttract` | Attract | runEvent | 1 | 1 | 0 | -100×1 |  |  |
| `onBasePower` | BasePower | runEvent+onEffect | 106 | 94 | 59 | 6×4, 7×1, 9×1, 10×1, 14×1, 15×45, 16×2, 19×5, 21×5, 23×8, 24×1, 30×1 |  |  |
| `onBattleStart` | BattleStart | singleEvent | 2 | 2 | 2 |  |  |  |
| `onBeforeMove` | BeforeMove | runEvent | 21 | 17 | 12 | -1×1, 1×1, 2×1, 3×1, 5×1, 6×2, 7×1, 8×1, 9×1, 10×2, 11×1, 100×2 |  |  |
| `onBeforeSwitchIn` | BeforeSwitchIn | runEvent | 1 | 1 | 1 |  |  |  |
| `onBeforeSwitchOut` | BeforeSwitchOut | runEvent | 1 | 1 | 0 | -1×1 |  |  |
| `onBeforeTurn` | BeforeTurn | eachEvent | 1 | 1 | 0 |  |  |  |
| `onChangeBoost` | ChangeBoost | runEvent | 3 | 3 | 2 |  |  |  |
| `onChargeMove` | ChargeMove | runEvent | 1 | 1 | 1 |  |  |  |
| `onCheckShow` | CheckShow | singleEvent | 1 | 1 | 1 |  |  |  |
| `onCopy` | Copy | singleEvent | 3 | 2 | 0 |  |  |  |
| `onCriticalHit` | CriticalHit | runEvent | 5 | 4 | 3 |  |  |  |
| `onDamage` | Damage | runEvent+onEffect | 17 | 14 | 9 | -40×2, -30×1, -20×1, -10×1, 1×3 |  |  |
| `onDamagingHit` | DamagingHit | runEvent | 44 | 43 | 23 |  | 1×6, 2×1 |  |
| `onDeductPP` | DeductPP | runEvent | 1 | 1 | 1 |  |  |  |
| `onDisableMove` | DisableMove | runEvent, singleEvent | 12 | 11 | 6 |  |  |  |
| `onDragOut` | DragOut | runEvent | 6 | 6 | 0 | 1×2, 2×3 |  |  |
| `onDrive` | Drive | runEvent | 4 | 0 | 0 |  |  |  |
| `onEat` | Eat | singleEvent | 77 | 53 | 5 |  |  |  |
| `onEatItem` | EatItem | runEvent, singleEvent | 3 | 3 | 2 |  |  |  |
| `onEffectiveness` | Effectiveness | singleEvent, runEvent | 8 | 7 | 4 | -2×1, -1×1 |  |  |
| `onEmergencyExit` | EmergencyExit | runEvent | 2 | 2 | 0 |  |  |  |
| `onEnd` | End | singleEvent | 55 | 49 | 27 |  |  |  |
| `onFaint` | Faint | runEvent | 4 | 2 | 1 |  |  |  |
| `onFieldEnd` | FieldEnd | singleEvent | 18 | 16 | 11 |  |  |  |
| `onFieldResidual` | FieldResidual | fieldEvent(Residual) | 8 | 8 | 7 |  | 1×8, 27×8 | 1×1, 2×1, 5×1, 6×1, 7×4 |
| `onFieldRestart` | FieldRestart | singleEvent | 4 | 4 | 1 |  |  |  |
| `onFieldStart` | FieldStart | singleEvent | 21 | 18 | 11 |  |  |  |
| `onFlinch` | Flinch | runEvent | 1 | 1 | 0 |  |  |  |
| `onFoeAfterBoost` | Foe+AfterBoost | runEvent | 2 | 2 | 0 |  |  |  |
| `onFoeBeforeMove` | Foe+BeforeMove | runEvent | 2 | 1 | 1 | 4×1 |  |  |
| `onFoeBeforeSwitchOut` | Foe+BeforeSwitchOut | runEvent | 1 | 0 | 0 |  |  |  |
| `onFoeDisableMove` | Foe+DisableMove | runEvent, singleEvent | 1 | 1 | 1 |  |  |  |
| `onFoeMaybeTrapPokemon` | FoeMaybeTrapPokemon / Foe+MaybeTrapPokemon | singleEvent, runEvent | 3 | 3 | 3 |  |  |  |
| `onFoeRedirectTarget` | Foe+RedirectTarget | priorityEvent | 3 | 2 | 2 | 1×2 |  |  |
| `onFoeTrapPokemon` | Foe+TrapPokemon | runEvent | 4 | 3 | 3 |  |  |  |
| `onFoeTryEatItem` | Foe+TryEatItem | runEvent | 3 | 3 | 3 |  |  |  |
| `onFoeTryMove` | Foe+TryMove | singleEvent, runEvent | 3 | 3 | 2 |  |  |  |
| `onFractionalPriority` | FractionalPriority | runEvent | 7 | 6 | 2 | -2×2, -1×2 |  |  |
| `onHit` | Hit | singleEvent, runEvent | 174 | 98 | 46 |  |  |  |
| `onHitField` | HitField | singleEvent | 6 | 4 | 2 |  |  |  |
| `onHitProtect` | HitProtect | runEvent | 1 | 0 | 0 |  |  |  |
| `onHitSide` | HitSide | singleEvent | 4 | 3 | 1 |  |  |  |
| `onImmunity` | Immunity | runEvent | 15 | 14 | 7 |  |  |  |
| `onInvulnerability` | Invulnerability | runEvent | 7 | 7 | 2 |  |  |  |
| `onLockMove` | LockMove | priorityEvent | 7 | 5 | 2 |  |  |  |
| `onMaybeTrapPokemon` | MaybeTrapPokemon | runEvent | 1 | 1 | 0 | -10×1 |  |  |
| `onMemory` | Memory | runEvent | 17 | 0 | 0 |  |  |  |
| `onModifyAccuracy` | ModifyAccuracy | runEvent | 7 | 6 | 0 | -2×1, -1×3, 10×1 |  |  |
| `onModifyAtk` | ModifyAtk | runEvent | 25 | 23 | 20 | 1×3, 5×19 |  |  |
| `onModifyBoost` | ModifyBoost | runEvent | 2 | 0 | 0 |  |  |  |
| `onModifyCritRatio` | ModifyCritRatio | runEvent | 11 | 6 | 1 |  |  |  |
| `onModifyDamage` | ModifyDamage | runEvent | 6 | 6 | 3 |  |  |  |
| `onModifyDef` | ModifyDef | runEvent | 8 | 7 | 5 | 2×1, 6×5, 10×1 |  |  |
| `onModifyMove` | ModifyMove | singleEvent, runEvent | 61 | 54 | 35 | -5×2, -2×1, -1×4, 1×4 |  |  |
| `onModifyPriority` | ModifyPriority | singleEvent, runEvent | 4 | 4 | 4 |  |  |  |
| `onModifySTAB` | ModifySTAB | runEvent | 1 | 1 | 1 |  |  |  |
| `onModifySecondaries` | ModifySecondaries | runEvent | 2 | 2 | 2 |  |  |  |
| `onModifySpA` | ModifySpA | runEvent | 22 | 20 | 16 | 1×2, 5×17 |  |  |
| `onModifySpD` | ModifySpD | runEvent | 6 | 5 | 5 | 1×1, 2×1, 6×2, 10×1 |  |  |
| `onModifySpe` | ModifySpe | runEvent | 23 | 21 | 12 | -101×1 |  |  |
| `onModifyTarget` | ModifyTarget | runEvent+onEffect | 2 | 2 | 0 |  |  |  |
| `onModifyType` | ModifyType | singleEvent, runEvent | 22 | 15 | 11 | -1×5, 1×1 |  |  |
| `onModifyWeight` | ModifyWeight | runEvent | 3 | 3 | 1 | 1×1 |  |  |
| `onMoveAborted` | MoveAborted | runEvent | 4 | 3 | 2 |  |  |  |
| `onMoveFail` | MoveFail | singleEvent | 7 | 4 | 1 |  |  |  |
| `onNegateImmunity` | NegateImmunity | runEvent | 3 | 1 | 0 |  |  |  |
| `onOverrideAction` | OverrideAction | runEvent | 1 | 1 | 1 |  |  |  |
| `onPlate` | — (data field, never dispatched) |  | 34 | 17 | 17 |  |  |  |
| `onPrepareHit` | PrepareHit | singleEvent, runEvent | 23 | 19 | 9 |  |  |  |
| `onRedirectTarget` | RedirectTarget | priorityEvent | 3 | 2 | 0 | -1×2 |  |  |
| `onResidual` | Residual | fieldEvent | 54 | 45 | 24 | -100×1 | 3×1, 4×1, 5×7, 6×1, 7×1, 8×1, 9×2, 10×1, 12×1, 13×2, 14×1, 15×1, 16×1, 17×1, 18×1, 22×1, 23×1, 24×1, 25×1, 28×11, 29×9 | 1×2, 2×8, 3×6, 4×2 |
| `onRestart` | Restart | singleEvent | 15 | 11 | 3 |  |  |  |
| `onSetAbility` | SetAbility | runEvent | 1 | 1 | 0 |  |  |  |
| `onSetStatus` | SetStatus | runEvent | 15 | 15 | 10 |  |  |  |
| `onSideConditionStart` | SideConditionStart | runEvent | 2 | 2 | 1 |  |  |  |
| `onSideEnd` | SideEnd | singleEvent | 14 | 9 | 4 |  |  |  |
| `onSideResidual` | SideResidual | fieldEvent(Residual) | 0 | 0 | 0 |  | 26×9 | 1×1, 2×1, 3×1, 4×1, 5×1, 7×1, 8×1, 9×1, 10×1 |
| `onSideRestart` | SideRestart | singleEvent | 2 | 2 | 2 |  |  |  |
| `onSideStart` | SideStart | singleEvent | 23 | 15 | 9 |  |  |  |
| `onSourceAccuracy` | Source+Accuracy | runEvent | 2 | 2 | 0 |  |  |  |
| `onSourceAfterFaint` | Source+AfterFaint | runEvent | 8 | 7 | 6 |  |  |  |
| `onSourceBasePower` | Source+BasePower | runEvent+onEffect | 2 | 2 | 1 | 17×1 |  |  |
| `onSourceDamagingHit` | Source+DamagingHit | runEvent | 2 | 2 | 2 |  |  |  |
| `onSourceInvulnerability` | Source+Invulnerability | runEvent | 1 | 1 | 0 | 1×1 |  |  |
| `onSourceModifyAccuracy` | Source+ModifyAccuracy | runEvent | 4 | 4 | 3 | -2×2, -1×2 |  |  |
| `onSourceModifyAtk` | Source+ModifyAtk | runEvent | 4 | 4 | 3 | 5×1, 6×3 |  |  |
| `onSourceModifyDamage` | Source+ModifyDamage | runEvent | 34 | 33 | 10 | -1×1 |  |  |
| `onSourceModifySecondaries` | Source+ModifySecondaries | runEvent | 1 | 1 | 0 |  |  |  |
| `onSourceModifySpA` | Source+ModifySpA | runEvent | 4 | 4 | 3 | 5×4 |  |  |
| `onSourceTryHeal` | Source+TryHeal | runEvent | 1 | 1 | 0 |  |  |  |
| `onSourceTryPrimaryHit` | Source+TryPrimaryHit | runEvent | 19 | 2 | 1 |  |  |  |
| `onStallMove` | StallMove | runEvent | 1 | 1 | 1 |  |  |  |
| `onStart` | Start | singleEvent | 169 | 146 | 87 |  |  |  |
| `onSwap` | Swap | runEvent, singleEvent | 2 | 2 | 0 |  |  |  |
| `onSwitchIn` | SwitchIn | fieldEvent | 17 | 14 | 10 | -2×8, -1×8, 1×4, 2×2 |  |  |
| `onSwitchOut` | SwitchOut | runEvent | 3 | 3 | 3 |  |  |  |
| `onTakeItem` | TakeItem | runEvent, singleEvent | 181 | 28 | 28 |  |  |  |
| `onTerrainChange` | TerrainChange | eachEvent, singleEvent | 6 | 6 | 1 |  |  |  |
| `onTrapPokemon` | TrapPokemon | runEvent | 9 | 8 | 3 | -11×2, -10×1 |  |  |
| `onTry` | Try | singleEvent | 41 | 37 | 14 |  |  |  |
| `onTryAddVolatile` | TryAddVolatile | runEvent | 12 | 12 | 8 |  |  |  |
| `onTryBoost` | TryBoost | runEvent | 16 | 16 | 10 | 1×1, 2×1 |  |  |
| `onTryEatItem` | TryEatItem | runEvent | 13 | 11 | 3 | -1×1 |  |  |
| `onTryHeal` | TryHeal | runEvent | 3 | 2 | 1 | 1×1 |  |  |
| `onTryHit` | TryHit | runEvent, singleEvent | 79 | 59 | 42 | 1×3, 3×5, 4×3 |  |  |
| `onTryImmunity` | TryImmunity | singleEvent | 10 | 7 | 3 |  |  |  |
| `onTryMove` | TryMove | singleEvent, runEvent | 25 | 19 | 9 | -2×1, 1×2 |  |  |
| `onTryPrimaryHit` | TryPrimaryHit | runEvent | 1 | 1 | 1 | -1×1 |  |  |
| `onType` | Type | runEvent | 3 | 3 | 2 | -1×1, 1×2 |  |  |
| `onUpdate` | Update | eachEvent, singleEvent | 54 | 43 | 16 |  |  |  |
| `onUse` | Use | singleEvent | 3 | 3 | 1 |  |  |  |
| `onUseItem` | UseItem | runEvent | 1 | 1 | 0 |  |  |  |
| `onUseMoveMessage` | UseMoveMessage | singleEvent | 1 | 0 | 0 |  |  |  |
| `onWeather` | Weather | eachEvent | 6 | 6 | 4 |  |  |  |
| `onWeatherChange` | WeatherChange | eachEvent, singleEvent, runEvent | 4 | 4 | 2 |  |  |  |
| `onWeatherModifyDamage` | WeatherModifyDamage | priorityEvent | 5 | 4 | 4 |  |  |  |

---

## Appendix A. Every Gen 9 priority / order / subOrder key, grouped by callback

`O` = `<cb>Order`, `P` = `<cb>Priority`, `S` = `<cb>SubOrder`, `-` = absent (O → false, P → 0, S → default from §3).
Within each callback, groups are listed in sort order (order ASC, priority DESC, subOrder ASC).
`†` = not in the RD pool approximation (still Gen 9-legal). Labels: `M:` move, `MC:` move condition, `A:` ability,
`AC:` ability condition, `I:` item, `IC:` item condition, `C:` `conditions.ts` entry.
"(no callback; duration-only)" entries take part in `fieldEvent('Residual')` only because they have a `duration`
(§2.2). They decrement and expire with no callback. Use this list to order Residual exactly. In one Residual
pass, all `onResidual*`, `onSideResidual*` and `onFieldResidual*` handlers are sorted together in a single
`speedSort` (Pokémon-held entries carry the holder's speed; side and field holders have speed 0).

- `onAfterMoveSecondary`:
  - O=- P=2 S=-: I:ejectbutton†
- `onAfterMoveSecondarySelf`:
  - O=- P=-1 S=-: I:shellbell†
- `onAfterSetStatus`:
  - O=- P=-1 S=-: I:lumberry
- `onAllyBasePower`:
  - O=- P=22 S=-: A:battery†, A:powerspot, A:steelyspirit
- `onAllyModifyAtk`:
  - O=- P=3 S=-: A:flowergift†
- `onAllyModifySpD`:
  - O=- P=4 S=-: A:flowergift†
- `onAnyBasePower`:
  - O=- P=20 S=-: A:darkaura†, A:fairyaura†
- `onAnyFaint`:
  - O=- P=1 S=-: A:soulheart
- `onAnyInvulnerability`:
  - O=- P=1 S=-: A:noguard
- `onAnyModifyAccuracy`:
  - O=- P=-1 S=-: A:victorystar†
- `onAnySwitchIn`:
  - O=- P=-2 S=-: A:commander†, I:whiteherb
  - O=- P=-3 S=-: A:opportunist†, I:mirrorherb†
  - O=- P=-4 S=-: I:ejectpack†
- `onAttract`:
  - O=- P=-100 S=-: I:destinyknot†
- `onBasePower`:
  - O=- P=30 S=-: A:technician
  - O=- P=24 S=-: A:rivalry†
  - O=- P=23 S=-: A:aerilate†, A:galvanize, A:ironfist, A:normalize†, A:pixilate, A:reckless, A:refrigerate†, I:punchingglove†
  - O=- P=21 S=-: A:analytic†, A:sandforce, A:sheerforce, A:supremeoverlord†, A:toughclaws
  - O=- P=19 S=-: A:flareboost†, A:megalauncher, A:sharpness, A:strongjaw, A:toxicboost
  - O=- P=16 S=-: I:muscleband†, I:wiseglasses†
  - O=- P=15 S=-: I:adamantcrystal, I:adamantorb†, I:blackbelt†, I:blackglasses†, I:charcoal†, I:cornerstonemask, I:dracoplate, I:dragonfang†, I:dreadplate, I:earthplate, I:fairyfeather†, I:fistplate, I:flameplate, I:griseouscore, I:griseousorb†, I:hardstone†, I:hearthflamemask, I:icicleplate, I:insectplate, I:ironplate, I:lustrousglobe, I:lustrousorb, I:magnet, I:meadowplate, I:metalcoat†, I:mindplate, I:miracleseed†, I:mysticwater, I:nevermeltice†, I:pixieplate, I:poisonbarb†, I:sharpbeak†, I:silkscarf, I:silverpowder, I:skyplate, I:softsand†, I:souldew, I:spelltag†, I:splashplate, I:spookyplate, I:stoneplate, I:toxicplate, I:twistedspoon†, I:wellspringmask, I:zapplate
  - O=- P=14 S=-: C:gem†
  - O=- P=10 S=-: MC:helpinghand
  - O=- P=9 S=-: MC:charge
  - O=- P=7 S=-: A:punkrock
  - O=- P=6 S=-: MC:electricterrain, MC:grassyterrain, MC:mistyterrain†, MC:psychicterrain
- `onBeforeMove`:
  - O=- P=100 S=-: MC:chillyreception†, MC:glaiverush
  - O=- P=11 S=-: C:mustrecharge
  - O=- P=10 S=-: C:frz, C:slp
  - O=- P=9 S=-: A:truant
  - O=- P=8 S=-: C:flinch
  - O=- P=7 S=-: MC:disable
  - O=- P=6 S=-: MC:gravity†, MC:throatchop
  - O=- P=5 S=-: MC:taunt
  - O=- P=3 S=-: C:confusion
  - O=- P=2 S=-: MC:attract†
  - O=- P=1 S=-: C:par
  - O=- P=-1 S=-: MC:destinybond†
- `onBeforeSwitchOut`:
  - O=- P=-1 S=-: C:dynamax†
- `onDamage`:
  - O=- P=1 S=-: A:disguise, A:iceface, A:poisonheal
  - O=- P=-10 S=-: MC:endure†
  - O=- P=-20 S=-: M:falseswipe†
  - O=- P=-30 S=-: A:sturdy
  - O=- P=-40 S=-: I:focusband†, I:focussash
- `onDamagingHit`:
  - O=1 P=- S=-: A:aftermath, A:electromorphosis, A:innardsout†, A:ironbarbs†, A:roughskin, A:windpower†
  - O=2 P=- S=-: I:rockyhelmet
- `onDragOut`:
  - O=- P=2 S=-: C:commanded†, C:commanding†, C:dynamax†
  - O=- P=1 S=-: A:guarddog†, A:suctioncups†
- `onEffectiveness`:
  - O=- P=-1 S=-: C:deltastream
  - O=- P=-2 S=-: MC:tarshot†
- `onFieldResidual`:
  - O=1 P=- S=-: C:deltastream, C:desolateland, C:hail†, C:primordialsea, C:raindance, C:sandstorm, C:snowscape, C:sunnyday
  - O=27 P=- S=1 (no callback; duration-only): MC:trickroom
  - O=27 P=- S=2 (no callback; duration-only): MC:gravity†
  - O=27 P=- S=5 (no callback; duration-only): MC:wonderroom†
  - O=27 P=- S=6 (no callback; duration-only): MC:magicroom†
  - O=27 P=- S=7 (no callback; duration-only): MC:electricterrain, MC:grassyterrain, MC:mistyterrain†, MC:psychicterrain
- `onFoeBeforeMove`:
  - O=- P=4 S=-: MC:imprison
- `onFoeRedirectTarget`:
  - O=- P=1 S=-: MC:followme, MC:ragepowder
- `onFractionalPriority`:
  - O=- P=-1 S=-: A:myceliummight, A:quickdraw†
  - O=- P=-2 S=-: I:custapberry, I:quickclaw†
- `onMaybeTrapPokemon`:
  - O=- P=-10 S=-: I:shedshell†
- `onModifyAccuracy`:
  - O=- P=10 S=-: A:wonderskin†
  - O=- P=-1 S=-: A:sandveil†, A:snowcloak†, A:tangledfeet†
  - O=- P=-2 S=-: I:brightpowder†
- `onModifyAtk`:
  - O=- P=5 S=-: A:blaze, A:defeatist†, A:dragonsmaw, A:guts, A:hugepower, A:hustle, A:orichalcumpulse, A:overgrow, A:purepower, A:rockypayload, A:slowstart, A:stakeout, A:steelworker†, A:swarm, A:torrent, A:transistor, AC:flashfire, AC:protosynthesis, AC:quarkdrive
  - O=- P=1 S=-: A:gorillatactics†, I:choiceband, I:lightball
- `onModifyDef`:
  - O=- P=10 S=-: C:snowscape
  - O=- P=6 S=-: A:furcoat, A:grasspelt†, A:marvelscale†, AC:protosynthesis, AC:quarkdrive
  - O=- P=2 S=-: I:eviolite
- `onModifyMove`:
  - O=- P=1 S=-: A:propellertail, A:stalwart, A:stancechange†, I:punchingglove†
  - O=- P=-1 S=-: A:battlebond, A:stench†, I:kingsrock†, I:razorfang†
  - O=- P=-2 S=-: A:serenegrace
  - O=- P=-5 S=-: A:mindseye, A:scrappy
- `onModifySpA`:
  - O=- P=5 S=-: A:blaze, A:defeatist†, A:dragonsmaw, A:hadronengine, A:minus†, A:overgrow, A:plus†, A:rockypayload, A:solarpower, A:stakeout, A:steelworker†, A:swarm, A:torrent, A:transistor, AC:flashfire, AC:protosynthesis, AC:quarkdrive
  - O=- P=1 S=-: I:choicespecs, I:lightball
- `onModifySpD`:
  - O=- P=10 S=-: C:sandstorm
  - O=- P=6 S=-: AC:protosynthesis, AC:quarkdrive
  - O=- P=2 S=-: I:eviolite
  - O=- P=1 S=-: I:assaultvest
- `onModifySpe`:
  - O=- P=-101 S=-: C:par
- `onModifyType`:
  - O=- P=1 S=-: A:normalize†
  - O=- P=-1 S=-: A:aerilate†, A:galvanize, A:liquidvoice, A:pixilate, A:refrigerate†
- `onModifyWeight`:
  - O=- P=1 S=-: A:heavymetal
- `onRedirectTarget`:
  - O=- P=-1 S=-: MC:counter†, MC:mirrorcoat†
- `onResidual`:
  - O=3 P=- S=-: C:futuremove†
  - O=4 P=- S=-: MC:wish
  - O=5 P=- S=1: MC:firepledge†
  - O=5 P=- S=2: MC:grassyterrain
  - O=5 P=- S=3: A:healer, A:hydration, A:shedskin
  - O=5 P=- S=4: I:blacksludge†, I:leftovers
  - O=6 P=- S=-: MC:aquaring†
  - O=7 P=- S=-: MC:ingrain†
  - O=8 P=- S=-: MC:leechseed
  - O=9 P=- S=-: C:psn, C:tox
  - O=10 P=- S=-: C:brn
  - O=12 P=- S=-: MC:curse
  - O=13 P=- S=-: C:partiallytrapped, MC:saltcure
  - O=14 P=- S=-: MC:syrupbomb
  - O=15 P=- S=- (no callback; duration-only): MC:taunt
  - O=16 P=- S=-: MC:encore
  - O=17 P=- S=- (no callback; duration-only): MC:disable
  - O=18 P=- S=- (no callback; duration-only): MC:magnetrise†
  - O=22 P=- S=- (no callback; duration-only): MC:throatchop
  - O=23 P=- S=- (no callback; duration-only): MC:yawn
  - O=24 P=- S=-: MC:perishsong†
  - O=25 P=- S=- (no callback; duration-only): MC:roost
  - O=28 P=- S=1: MC:uproar†
  - O=28 P=- S=2: A:baddreams, A:cudchew†, A:harvest, A:moody†, A:pickup†, A:slowstart, A:speedboost
  - O=28 P=- S=3: I:flameorb, I:stickybarb†, I:toxicorb
  - O=29 P=- S=-: A:hungerswitch, A:opportunist†, A:powerconstruct†, A:schooling†, A:shieldsdown, A:zenmode†, I:ejectpack†, I:mirrorherb†, I:whiteherb
  - O=- P=-100 S=-: C:dynamax†
- `onSideResidual`:
  - O=26 P=- S=1 (no callback; duration-only): MC:reflect
  - O=26 P=- S=2 (no callback; duration-only): MC:lightscreen
  - O=26 P=- S=3 (no callback; duration-only): MC:safeguard†
  - O=26 P=- S=4 (no callback; duration-only): MC:mist†
  - O=26 P=- S=5 (no callback; duration-only): MC:tailwind
  - O=26 P=- S=7 (no callback; duration-only): MC:waterpledge†
  - O=26 P=- S=8 (no callback; duration-only): MC:firepledge†
  - O=26 P=- S=9 (no callback; duration-only): MC:grasspledge†
  - O=26 P=- S=10 (no callback; duration-only): MC:auroraveil
- `onSourceBasePower`:
  - O=- P=17 S=-: A:dryskin
- `onSourceInvulnerability`:
  - O=- P=1 S=-: MC:lockon†
- `onSourceModifyAccuracy`:
  - O=- P=-1 S=-: A:compoundeyes, A:hustle
  - O=- P=-2 S=-: I:widelens, I:zoomlens†
- `onSourceModifyAtk`:
  - O=- P=6 S=-: A:heatproof†, A:purifyingsalt, A:thickfat
  - O=- P=5 S=-: A:waterbubble
- `onSourceModifyDamage`:
  - O=- P=-1 S=-: A:ripen
- `onSourceModifySpA`:
  - O=- P=5 S=-: A:heatproof†, A:purifyingsalt, A:thickfat, A:waterbubble
- `onSwitchIn`:
  - O=- P=2 S=-: A:neutralizinggas†, A:terashift
  - O=- P=1 S=- (callback = onStart via getCallback remap): A:asoneglastrier, A:asonespectrier, A:klutz†, A:unnerve
  - O=- P=-1 S=- (callback = onStart via getCallback remap): A:mimicry†, A:schooling†, A:shieldsdown, I:electricseed†, I:grassyseed†, I:mistyseed†, I:psychicseed†, I:roomservice†
  - O=- P=-2 S=- (callback = onStart via getCallback remap): A:costar†, A:flowergift†, A:forecast†, A:hospitality, A:iceface, A:protosynthesis, A:quarkdrive, I:boosterenergy
- `onTrapPokemon`:
  - O=- P=-10 S=-: I:shedshell†
  - O=- P=-11 S=-: C:commanded†, C:commanding†
- `onTryBoost`:
  - O=- P=2 S=-: A:guarddog†
  - O=- P=1 S=-: I:clearamulet
- `onTryEatItem`:
  - O=- P=-1 S=-: A:ripen
- `onTryHeal`:
  - O=- P=1 S=-: I:bigroot†
- `onTryHit`:
  - O=- P=4 S=-: MC:psychicterrain, MC:quickguard†, MC:wideguard
  - O=- P=3 S=-: MC:banefulbunker, MC:burningbulwark, MC:protect, MC:silktrap†, MC:spikyshield
  - O=- P=1 S=-: A:magicbounce, A:overcoat, A:sapsipper
- `onTryMove`:
  - O=- P=1 S=-: C:desolateland, C:primordialsea
  - O=- P=-2 S=-: IC:metronome†
- `onTryPrimaryHit`:
  - O=- P=-1 S=-: MC:substitute
- `onType`:
  - O=- P=1 S=-: C:arceus, C:silvally†
  - O=- P=-1 S=-: MC:roost

---

## Ignored (present in the source, deliberately not specified)

* **Dynamax / Gigantamax**: `dynamax` condition (`onResidualPriority -100`, `onDragOutPriority 2`, …), `runDynamax`,
  `maxguard` checks in moves, `dynamaxEnding` in `endTurn` (`sim/battle.ts:1630-1640`).
* **Z-moves**: Z-crystal `onTakeItem`, `healreplacement` slot condition (Z-Memento etc., `sim/battle-actions.ts:1515`).
* **Mega Evolution / Primal**: `runEvent('AfterMega')` (`sim/battle-actions.ts:1901`) and all `onAnyAfterMega` handlers,
  which are never fired. Mega stones' `onTakeItem: false`. `isPrimalOrb` in `ignoringItem`. Red/Blue Orb `onSwitchIn`.
* **gen < 9 branches**: `getCallback` `gen >= 5` test; `ignoringAbility`/`ignoringItem` `gen >= 5`;
  `suppressingAbility` `|| gen < 8`; `eachEvent` `Weather → Update` `gen >= 7` (always true here);
  `sim/battle.ts:2915` `gen < 5` Update; gen-4 `'event'` queue actions (`sim/battle.ts:2751-2753`, `data/mods/gen4/items.ts`);
  gen ≤ 3 faint handling (`sim/battle.ts:2584-2597`); Gen-1 `modifyStat` in `setSpecies`.
* **Team Preview**: `onTeamPreview` (`sim/battle.ts:1982-1996`). Not in this ruleset.
* **Timers, chat, inactivity, `|t:|` timestamps**: outside the event system.
* **Multi battles / free-for-all / triples**: `side.n >= 2` / `allySide` branches in `findEventHandlers` and
  `fieldEvent`, `activeTeam()` multi branch, FFA branch of `side.foes()`, `adjacentFoes` for `activePerHalf > 2`,
  `getSlot` position offsets.
* **Innate effects**: `item:`/`ability:` prefixed condition ids (`dex-conditions.ts:680-685`), the Status branch of
  `getCallback`, and the `startsWith('ability:'/'item:')` branches of the `fieldEvent` location check.
* **Custom handlers** (`Battle.onEvent`, `battle.events`): never registered by this format (§2.5).
* **Format/battle/actions/field script overrides**: none for gen9 / this format.
* **Debug output**: `battle.debug()` lines (`|debug|…`) exist only in debug mode. `forceRandomChance` is debug-only.
* **Twisted Dimension Mod** (`getActionSpeed`), **Hackmons** branch of `FoeMaybeTrapPokemon` scanning: not in this ruleset.
* **Dead code**: the `!effect.num` "AttackingEvents" branch in `runEvent` (§7.4).
* **`Drive` / `Memory` events and `onDrive`/`onMemory` handlers**: all non-standard in Gen 9 (Techno Blast and Multi-Attack users absent).
* **Validator-only rule hooks**: `onValidateTeam`, `onValidateSet`, `onValidateRule`, `onChangeSet`.

## Open questions

1. **RD reachability is approximate.** Items are scraped from string literals in `teams.ts`, and conditions from
   literal ids in reachable handlers. The exact item/ability/move universe belongs to the team-generation doc.
   Recommendation: implement every `#gen9` handler.
2. **Speed cache timing** (§3.1) affects handler order and PRNG ties. The exact list of `updateSpeed()` calls and the
   `setSpecies` reset belongs to the speed/turn-order doc, and both docs must agree.
3. **Slot-condition `target` mutation** (§1.2): it can only be triggered by `runEvent('Swap')`, i.e. Ally Switch
   with Healing Wish or Lunar Dance pending. Neither move is in the RD pool, but both are Gen 9-legal. We reproduce
   it as-is. Flagging it for review in case the fuzzer should exclude it.
4. **Copied volatiles' effectOrder**: `copyVolatileFrom` (`sim/pokemon.ts:1253`) runs while the incoming Pokémon is
   not yet `isActive` (`sim/battle-actions.ts:114` vs `:135`), so copied volatiles get `effectOrder 0`. This only
   matters for `…SwitchIn` tie-breaks (Baton Pass/Shed Tail). Worth a targeted differential test.
5. **Throws** (depth ≥ 8, more than 1000 unsent log lines): how should the fuzz harness classify these? We propose
   "reference crashed → discard the case".
