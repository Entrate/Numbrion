# 03 — Move execution, damage, switching and core state mutators (Gen 9 Random Doubles)

Oracle: `~/src/pokemon-showdown` @ `7332b60`. All `file:line` refs are relative to that repo
(`sim/…`, `data/…`). Format: `gen9randomdoublesbattle` (`config/formats.ts:165`): `mod: gen9`
(= base data, `data/scripts.ts` has no overrides), `gameType: doubles`, rules `PotD, Obtainable,
Species Clause, HP Percentage Mod, Cancel Mod, Illusion Level Mod, Sleep Clause Mod`.
Constants for this format: `gen = 9`, `activePerHalf = 2`, `gameType = 'doubles'`, 2 sides.

The pseudo-code mirrors the TypeScript. Where a step says "PRNG" it consumes from the battle PRNG;
anything that may consume PRNG is marked **[PRNG]**. Event calls are written
`runEvent(Name, target, source, effect, relayVar[, onEffect][, fastExit])` and
`singleEvent(Name, effect, state, target, source, sourceEffect, relayVar)` with the exact argument
order used by Showdown (argument order matters: handlers receive them positionally). Event
dispatch, handler collection, sorting and relay semantics belong to the event-system spec; this
document only states *which* events fire, *when*, and with *which* arguments.

Behaviour was spot-checked against the built oracle (`dist/`) with a PRNG-tracing harness; quirks
called "verified" below were observed in real logs.

---------------------------------------------------------------------------------------------------

## 0. Cross-cutting primitives

### 0.1 Integer helpers (`sim/dex.ts:365`, `lib/utils.ts:320`, `sim/battle.ts:2309-2400`)

```
trunc(num, bits=0):   bits ? ToUint32(num) % 2**bits : ToUint32(num)     // JS `num >>> 0`
                      // ToUint32 truncates toward zero then wraps mod 2^32. Inputs here are >= 0.
clampIntRange(num, min?, max?):
    if typeof num != number: num = 0
    num = floor(num)                       // NOTE: floor, not trunc
    if min defined && num < min: num = min
    if max defined && num > max: num = max
    return num

modify(value, numerator, denominator=1):               // "pokeRound" multiply by a fraction
    if numerator is [n, d]: (numerator, denominator) = (n, d)
    modifier = trunc(numerator * 4096 / denominator)    // f64 math then trunc
    return trunc((trunc(value * modifier) + 2048 - 1) / 4096)   // rounds .5 DOWN

chain(prev, next):                        // not used in this pipeline directly
    p = trunc(prev*4096) (or trunc(p0*4096/p1) for [p0,p1]); n likewise
    return ((p * n + 2048) >> 12) / 4096

chainModify(numerator, denominator=1):    // only valid inside a runEvent handler
    previousMod = trunc(event.modifier * 4096)
    if numerator is [n, d]: (numerator, denominator) = (n, d)
    nextMod = trunc(numerator * 4096 / denominator)
    event.modifier = ((previousMod * nextMod + 2048) >> 12) / 4096
    // returns undefined → handler returning `this.chainModify(...)` returns undefined,
    // i.e. it does NOT change the relay value; it only changes event.modifier.

finalModify(relayVar): relayVar = modify(relayVar, event.modifier); event.modifier = 1; return relayVar

randomizer(baseDamage):                   // sim/battle.ts:2395   [PRNG]
    return trunc(trunc(baseDamage * (100 - random(16))) / 100)
```

**runEvent's implicit final modifier** (`sim/battle.ts:932-937`): after all handlers ran, if the
final relay value is a number and `relayVar === Math.abs(Math.floor(relayVar))` (i.e. a
non-negative *integer*), it is replaced by `modify(relayVar, event.modifier)`. Every
`chainModify` therefore takes effect only here, once, using the 4096-fixed-point chained modifier.
Non-integer relay values (e.g. 1.5 STAB, float multiaccuracy accuracy) are returned untouched and
**the accumulated modifier is silently dropped**.

Stat-stage multipliers (`sim/pokemon.ts:560-594` `calculateStat`, `:596-639` `getStat`):
`boostTable = [1, 1.5, 2, 2.5, 3, 3.5, 4]`; `b >= 0 → floor(stat * table[b])`,
`b < 0 → floor(stat / table[-b])`. Because all quotients are exact or far from integers this is
identical to integer math: `b>=0: (stat*(2+b))/2`, `b<0: (stat*2)/(2-b)` (integer division).
`calculateStat(stat, boost, modifier=1, statUser)` first runs
`runEvent('ModifyBoost', statUser||this, null, null, {[stat]: boost})` (Unaware etc.), clamps to
±6, applies the table, then `modify(stat, modifier||1)` (identity for 1). Wonder Room swaps the
*stored* def/spd before boosts.

### 0.2 PRNG surface used in this document (`sim/prng.ts:91-160`, `sim/battle.ts:346-358`)

- `random(n)` → `floor(r * n / 2^32)` in `[0,n)`; `random(m,n)` → `floor(r*(n-m)/2^32)+m`.
  One raw 32-bit draw per call.
- `randomChance(num, den)` → `random(den) < num`. **Always draws**, even for `randomChance(1,1)`
  or `randomChance(100,100)`. `num` may be a non-integer (compared as f64).
- `sample(arr)` → `arr[random(arr.length)]`. **Draws even when `arr.length === 1`.** Throws on
  empty (callers guard).
- `shuffle(arr, start, end)` (used by `speedSort` on ties) → `for (i = start; i < end - 1; i++)
  { j = random(i, end); if (j != i) swap(i, j) }` → a tie group of size k draws k-1 times.
- `speedSort(list, cmp)` (`sim/battle.ts:433`) is a selection sort that shuffles every group of
  tied elements → **any tie draws**. Relevant call sites in this doc: `eachEvent('Update')`
  (ties on `pokemon.speed`), `runSwitch`'s sort of all actives, `fieldEvent('SwitchIn')`, and every
  `runEvent` whose handler list (after priority/speed/subOrder/effectOrder) has ties — except
  `Invulnerability`, `TryHit`, `DamagingHit`, `EntryHazard` (left-to-right comparator, no PRNG) and
  `priorityEvent`s (`fastExit`, `compareRedirectOrder` via `Array.sort`, no PRNG).
- `Battle.forceRandomChance` is a test hook, always `null` here.

#### PRNG site index (this document's scope)

| Site | Call | When |
|---|---|---|
| `Battle.getRandomTarget` → `Side.randomFoe` | `sample(aliveFoes)` | target type ∉ {self, all, allySide, allyTeam, adjacentAllyOrSelf, adjacentAlly} and ≥1 foe with hp>0 (§1.3) |
| `Battle.getRandomTarget` (adjacentAlly) | `sample(adjacentAllies)` | ≥1 alive ally |
| Triggers of the two above | — | `BattleQueue.resolveAction` (targetLoc 0), `getActionSpeed` → `getTarget`, `runMove` → `getTarget`, OverrideAction, `useMoveInner` (target undefined / target-type changed), `getMoveTargets` (fainted foe), twoturnmove |
| `hitStepAccuracy` | `randomChance(acc, 100)` | per surviving target, `acc !== true` |
| `hitStepMoveHitLoop` multihit `[2,5]` | `sample(20-elt table)` then Loaded Dice `random(2)` if `<4` | §4.9 |
| `hitStepMoveHitLoop` multihit 10 + Loaded Dice | `random(7)` | |
| `hitStepMoveHitLoop` multiaccuracy | `randomChance(acc, 100)` | hits ≥2 |
| `getDamage` crit | `randomChance(1, [0,24,8,2,1][ratio])` | `move.willCrit === undefined && ratio>0` |
| `modifyDamage` | `random(16)` | every damage calc reaching the formula |
| `selfDrops` | `random(100)` | `!isSecondary && self.boosts` (even when `self.chance` undefined) |
| `secondaries` | `random(100)` per (target × secondary) | after `ModifySecondaries` filtering |
| `dragIn` | `sample(possibleSwitches)` | before any other check |
| `eachEvent('Update')` | shuffle on speed ties | twice per hitting move (§4.9) + after every action |
| `runSwitch` | `speedSort(allActive)` + `fieldEvent('SwitchIn')` | every switch batch |
| `BattleQueue.insertChoice` | `random(first, last+1)` | tie range when inserting `runSwitch` |
| `getConfusionDamage` | `random(16)` | confusion self-hit |
| Conditions (spec'd in conditions doc, listed for completeness) | par `randomChance(1,4)`, frz `randomChance(1,5)`, confusion `random(2,6)` start / `randomChance(33,100)`, slp `random(2,5)` start, lockedmove `random(2,4)` start, stall `randomChance(1,counter)`, partiallytrapped `random(5,7)` | |

### 0.3 Result conventions (`sim/battle.ts:181-184`, `sim/battle-actions.ts:1548`)

`NOT_FAIL = ''` (failed but "not a failure", e.g. Protect), `HIT_SUBSTITUTE = 0`, `FAIL = false`
(loud failure), `SILENT_FAIL = null`. `undefined` = success/no-op. Truthiness matters everywhere;
the Rust port needs a tagged union `Undefined | NotFail | Null | Bool(b) | Num(n)` for these relay
values (number 0 is distinct from false).

```
combineResults(left, right):                         // battle-actions.ts:1548
    prio = ['undefined', 'string'(NOT_FAIL), 'object'(null), 'boolean', 'number']
    if prio.index(typeof left) > prio.index(typeof right): return left
    if left && !right && right !== 0:                   return left
    if typeof left == number && typeof right == number: return left + right
    return right
```
Examples: `(true,false)→true`, `(false,true)→true`, `(0,true)→0`, `(5,3)→8`, `(undefined,null)→null`,
`(null,false)→false`, `('',undefined)→''`.

### 0.4 Logging primitives (`sim/battle.ts:3069-3144`)

- `add(...parts)`: if no part is a function → push `'|' + parts.join('|')`. Pokemon parts stringify
  via `Pokemon.toString()` = `isActive ? getSlot() + fullname.slice(2) : fullname` → `"p2a: Name"`
  (active) or `"p2: Name"` (inactive / fainted-and-processed); Illusion uses the illusion's fullname.
  Effects stringify to `.name` (e.g. `[from] ${move}` → `[from] U-turn`).
- If any part is a function (`pokemon.getHealth`, `pokemon.getFullDetails`) → `addSplit(side, secret,
  shared)`: pushes `|split|p1`, then the secret line, then the shared line. `getHealth`
  (`pokemon.ts:2060`): secret `hp/maxhp[ status]`, shared `ceil(100*hp/maxhp)/100` with 100→99 if
  `hp<maxhp`; fainted → `0 fnt` in both. So every `-damage`/`-heal`/`switch`/`drag` produces 3 lines.
- `addMove(...)`: like `add` but records `lastMoveLine = log.length` (index of the line pushed).
- `attrLastMove(...args)` (`:3121`): if `lastMoveLine < 0` no-op. If the recorded line starts with
  `|-anim|` and args include `[still]` → that line is **deleted** and `lastMoveLine = -1`. Else if args
  include `[still]` → field 4 (the target) of the move line is blanked (`|move|p1a: X|Move||...`).
  Then `'|' + args.join('|')` is appended to that line.
- `retargetLastMove(p)` (`:3139`): replaces field 4 of the recorded line with `p.toString()`.
- `hint(text, once?, side?)`: pushes `|-hint|text` (or split per side); dedup only when `once`.
- `debug(...)`: only logged when `debugMode` (false in normal battles) — ignore.

### 0.5 ActiveMove identity

`dex.getActiveMove(m)` (`sim/dex.ts:316`): if `m` already has numeric `hit` it is returned **as is**
(same object); else deep-clone the dex Move and set `hit = 0`. The queue resolves `action.move =
getActiveMove(moveid)` (`battle-queue.ts:172,261`), and `runMove`, `useMoveInner`, `spreadMoveHit`
all call `getActiveMove` on it → **one mutable ActiveMove object flows through the whole use**
(except: OverrideAction replacement, and callers like Sleep Talk that pass a string). Mutated
fields used below: `hit, lastHit, spreadHit, smartTarget, totalDamage, moveHitData{slot→{crit,
typeMod, bypassProtect}}, hitTargets, selfDropped, stellarBoosted, hasSheerForce, sourceEffect,
ignoreAbility, ignoreImmunity, priority, pranksterBoosted, isExternal, type, category, target,
secondaries (deep-cloned, so Serene Grace mutation is per-use), self, multihit, multiaccuracy, flags`.
Dex Move defaults (`sim/dex-moves.ts:470-509`): `critRatio = Number(data.critRatio) || 1`
(**default 1**), `ignoreImmunity = data.ignoreImmunity ?? (category === 'Status')`,
`secondaries = data.secondaries || [data.secondary]`, `priority = data.priority || 0`.
`getMoveHitData(move)` (`pokemon.ts:706`) lazily creates `move.moveHitData[target.getSlot()] =
{crit:false, typeMod:0, bypassProtect:false}` keyed by slot string (`"p2a"`).

`setActiveMove(move, pokemon, target)` sets `battle.activeMove/activePokemon/activeTarget`
(`activeTarget = target || pokemon`). `clearActiveMove(failed)`: if `activeMove`: unless `failed`,
`battle.lastMove = activeMove`; then null all three.

`suppressingSecondaries()` = `activeMove?.hasSheerForce && activePokemon?.hasAbility('sheerforce')`.

---------------------------------------------------------------------------------------------------

## 1. Targeting

### 1.1 Location model (doubles) — `pokemon.ts:770-789`, `battle.ts:2403-2439`

- `getLocOf(t)` = `±(t.position + 1)`: negative if `t` is on the caller's side, positive otherwise.
- `getAtLoc(loc)`: `loc < 0` → own side, `loc > 0` → foe side; returns `side.active[|loc|-1]`
  (may be a fainted Pokémon still sitting in the slot).
- `validTargetLoc(loc, source, targetType)`: `loc == 0 → true`; `|loc| > 2 → false`;
  `isSelf = (loc == source loc)`, `isFoe = loc > 0`; in doubles **every foe slot is adjacent** and
  the ally slot is adjacent, self is not. Result:

| targetType | valid locs (doubles) |
|---|---|
| `normal`, `randomNormal`, `scripted` | either foe, ally (not self) |
| `adjacentAlly` | ally only |
| `adjacentAllyOrSelf` | ally or self |
| `adjacentFoe` | either foe |
| `any` | anything but self |
| everything else (`self`, `all*`, `*Side`, `allies`, `allyTeam`, `foeSide`) | **false** for any non-zero loc |

`validTarget(t, source, type) = validTargetLoc(source.getLocOf(t), source, type)` (used by
Follow Me / Rage Powder / Lightning Rod / Storm Drain redirect handlers).

Adjacency helpers (`pokemon.ts:716-746`, `side.ts:369-403`), doubles:
- `side.allies(all=false)` = `side.active` non-null, and (unless `all`) `hp > 0`. Order = slot order.
- `side.foes(all)` = `foe.allies(all)`. `pokemon.foes(all)` = `side.foes(all)`.
- `adjacentFoes()` = `side.foes()` (alive foes, slot order). `alliesAndSelf()` = `side.allies()`.
- `adjacentAllies()` = alive allies ≠ self, filtered by `isAdjacent` (false if either fainted).
- `isAlly(p)` = same side. `randomFoe()` = `foes()` empty ? `null` : `sample(foes())` **[PRNG]**.

### 1.2 `Battle.getTarget(pokemon, move, targetLoc, originalTarget?)` — `battle.ts:2441`

```
move = dex.moves.get(move)                         // dex data (static target type)
tracksTarget = move.tracksTarget
if pokemon.hasAbility(['stalwart','propellertail']): tracksTarget = true
if tracksTarget && originalTarget?.isActive: return originalTarget
if move.smartTarget:                               // Dragon Darts
    cur = pokemon.getAtLoc(targetLoc)
    return cur && !cur.fainted ? cur : getRandomTarget(pokemon, move)          [PRNG]
selfLoc = pokemon.getLocOf(pokemon)
if move.target in ['adjacentAlly','any','normal'] && targetLoc == selfLoc
   && !volatiles.twoturnmove && !volatiles.iceball && !volatiles.rollout:
    return move.flags.futuremove ? pokemon : null
if move.target != 'randomNormal' && validTargetLoc(targetLoc, pokemon, move.target):
    target = pokemon.getAtLoc(targetLoc)
    if target?.fainted:
        if target.isAlly(pokemon):
            if move.target == 'adjacentAllyOrSelf': return pokemon      // (gen != 5)
            return target                          // fainted ally: do NOT retarget
    if target && !target.fainted: return target
return getRandomTarget(pokemon, move)                                           [PRNG]
```
Note: for every non-choosable target type (`allAdjacent`, `allAdjacentFoes`, `allies`, `foeSide`,
`randomNormal`, …) `validTargetLoc` is false, so `getTarget` **always** falls through to
`getRandomTarget` and draws if the type isn't self-like. `getTarget` is therefore **not pure**:
it is called by `getActionSpeed` (queue sorting, every re-sort after each action in gen ≥ 8),
by `runMove`, and by `beforeTurnMove`; each call may draw.

### 1.3 `Battle.getRandomTarget(pokemon, move)` — `battle.ts:2494`

```
move = dex.moves.get(move)
if move.target in ['self','all','allySide','allyTeam','adjacentAllyOrSelf']: return pokemon
if move.target == 'adjacentAlly':
    allies = pokemon.adjacentAllies()
    return allies.length ? sample(allies) : null                                [PRNG]
// singles / triples branches ignored
return pokemon.side.randomFoe() || pokemon.side.foe.active[0]                   [PRNG if ≥1 alive foe]
```
So `normal`, `any`, `adjacentFoe`, `randomNormal`, `scripted`, `allAdjacent`, `allAdjacentFoes`,
`foeSide`, **and `allies`** (e.g. Howl, Lunar Blessing, Jungle Healing) draw `random(#aliveFoes)`.
If no foe is alive: no draw, returns `foe.active[0]` (fainted).

Where the queue sets the target (`battle-queue.ts:258-269`): for every `move` action,
`if (!action.targetLoc) { t = getRandomTarget(...) [PRNG]; if (t) action.targetLoc = getLocOf(t) }`
then `action.originalTarget = getAtLoc(action.targetLoc)`; then `getActionSpeed(action)` →
`getTarget(...)` [PRNG]. Verified order at turn start: for each side in order, each action:
resolveAction draw, then getActionSpeed draw.

### 1.4 `Pokemon.getMoveTargets(move, target)` — `pokemon.ts:791`

```
targets = []
switch move.target:
  case 'all','foeSide','allySide','allyTeam':
      if !move.target.startsWith('foe'):  targets.push(...alliesAndSelf())      // alive, slot order
      if !move.target.startsWith('ally'): targets.push(...foes(true))           // incl. 0-hp foes
      if targets.length && !targets.includes(target): retargetLastMove(targets.last)
  case 'allAdjacent':
      targets.push(...adjacentAllies())       // then falls through
  case 'allAdjacentFoes':
      targets.push(...adjacentFoes())          // alive foes, slot order
      if targets.length && !targets.includes(target): retargetLastMove(targets.last)
  case 'allies':
      targets = alliesAndSelf()
  default:                                     // single-target types
      selectedTarget = target
      if !target || (target.fainted && !target.isAlly(this)):     // (ffa clause ignored)
          possible = getRandomTarget(this, move)                                [PRNG]
          if !possible: return {targets:[], pressureTargets:[]}
          target = possible
      if !move.tracksTarget:                   // activePerHalf > 1
          target = priorityEvent('RedirectTarget', this, this, move, target)
      if move.smartTarget:
          targets = getSmartTargets(target, move); target = targets[0]
      else targets.push(target)
      if target.fainted && !move.flags.futuremove: return {targets:[], pressureTargets:[]}
      if selectedTarget !== target: retargetLastMove(target)
pressureTargets = targets
if move.target == 'foeSide': pressureTargets = []
if move.flags.mustpressure: pressureTargets = foes()    // Tera Blast etc.
return {targets, pressureTargets}
```
- `RedirectTarget` is a `priorityEvent` on the user (fastExit, `compareRedirectOrder`: priority, then
  speed, then abilityState.effectOrder; no PRNG; first non-undefined return wins). Handlers: Follow Me
  and Rage Powder volatiles (`onFoeRedirectTarget`, priority 1; return the holder if
  `validTarget(holder, user, move.target)` and not Sky-Dropped; Rage Powder additionally ignores
  Grass/Overcoat/Safety Goggles users; both set `smartTarget = false`), Spotlight (priority 2),
  Counter/Mirror Coat volatiles (`onRedirectTarget` on the user, priority -1), Lightning Rod / Storm Drain
  (`onAnyRedirectTarget`, `data/abilities.ts:2353,4646`: Electric/Water moves; they log
  `|-activate|<holder>|ability: Lightning Rod` only if the redirect changes the target). Note
  `move.tracksTarget` here is the ActiveMove flag (Stalwart/Propeller Tail set it in ModifyMove;
  Snipe Shot has it in data) → those moves skip redirection entirely.
- **Fainted ally targets are not retargeted** but still go through RedirectTarget (verified:
  Follow Me on the other side captured a Pollen Puff aimed at a fainted ally). If the final target is
  fainted → empty targets → `[notarget]` (§3 step 14).
- `getSmartTargets(target, move)` (`pokemon.ts:757`): `t2 = target.adjacentAllies()[0]`;
  if `!t2 || t2 === this || !t2.hp` → `smartTarget = false`, `[target]`; else if `!target.hp` →
  `smartTarget = false`, `[t2]`; else `[target, t2]`.

---------------------------------------------------------------------------------------------------

## 2. `BattleActions.runMove` — the outer move caller (`battle-actions.ts:210-354`)

Called by `runAction` for `choice: 'move'` (`battle.ts:2709`: skipped if `!pokemon.isActive ||
pokemon.fainted`) with `(action.move, pokemon, action.targetLoc, {sourceEffect: action.sourceEffect,
originalTarget: action.originalTarget})`; by Dancer (externalMove) and indirectly by Instruct
(which queues a prioritized `move` action).

```
runMove(moveOrMoveName, pokemon, targetLoc, {sourceEffect, externalMove, originalTarget}):
 1  pokemon.activeMoveActions++
 2  target = battle.getTarget(pokemon, moveOrMoveName, targetLoc, originalTarget)      [PRNG]
 3  baseMove = getActiveMove(moveOrMoveName)          // same object as action.move
 4  priority = baseMove.priority; pranksterBoosted = baseMove.pranksterBoosted
 5  if baseMove.id != 'struggle' && !externalMove:
        changed = runEvent('OverrideAction', pokemon, target, baseMove)               // Encore
        if changed && changed !== true:
            baseMove = getActiveMove(changed)          // fresh clone
            baseMove.priority = priority
            if pranksterBoosted: baseMove.pranksterBoosted = pranksterBoosted
            target = getRandomTarget(pokemon, baseMove)                               [PRNG]
 6  move = baseMove; move.isExternal = externalMove
 7  setActiveMove(move, pokemon, target)
 8  willTryMove = runEvent('BeforeMove', pokemon, target, move)                       [PRNG in handlers]
    if !willTryMove:
        runEvent('MoveAborted', pokemon, target, move)    // twoturnmove removes itself here
        clearActiveMove(true)
        pokemon.moveThisTurnResult = willTryMove          // false or null
        return
 9  if move.flags.cantusetwice && pokemon.lastMove?.id == move.id: pokemon.addVolatile(move.id)
10  if move.beforeMoveCallback && move.beforeMoveCallback(pokemon, target, move):
        clearActiveMove(true); pokemon.moveThisTurnResult = false; return
11  pokemon.lastDamage = 0
12  if !externalMove:
        lockedMove = pokemon.getLockedMove()   // priorityEvent('LockMove', pokemon); true→null
        if !lockedMove:
            if !pokemon.deductPP(baseMove, null, target) && move.id != 'struggle':
                add('cant', pokemon, 'nopp', move)          // |cant|p1a: X|nopp|Move Name
                clearActiveMove(true); pokemon.moveThisTurnResult = false; return
        else:
            sourceEffect = dex.conditions.get('lockedmove')   // → "[from] lockedmove" on move line
        pokemon.moveUsed(move, targetLoc)
13  noLock = externalMove && !pokemon.volatiles.lockedmove
14  moveDidSomething = useMove(baseMove, pokemon, {target, sourceEffect})
15  battle.lastSuccessfulMoveThisTurn = moveDidSomething ? battle.activeMove?.id : null
16  if battle.activeMove: move = battle.activeMove
17  singleEvent('AfterMove', move, null, pokemon, target, move)
    runEvent('AfterMove', pokemon, target, move)        // lockedmove ends at duration 1, etc.
18  if move.flags.cantusetwice && pokemon.removeVolatile(move.id):
        add('-hint', `Some effects can force a Pokemon to use ${move.name} again in a row.`)
19  DANCER (§2.4)
20  if noLock && pokemon.volatiles.lockedmove: delete pokemon.volatiles.lockedmove   // raw delete
21  battle.faintMessages()
22  battle.checkWin()
```
Notes:
- Step 12 is skipped for `externalMove` (Dancer): no PP, no `moveUsed` (lastMove unchanged).
- `deductPP(move, amount=null, target)` (`pokemon.ts:888`): `slot = getMoveData(move)` (by id);
  none → return 0; `slot.used = true`; `slot.pp == 0` → return 0; `amount ||= 1`; `slot.pp -= amount`;
  if negative: `amount += slot.pp; slot.pp = 0`; return amount.
- `moveUsed(move, targetLoc)` (`pokemon.ts:904`): `lastMove = move; lastMoveTargetLoc = targetLoc;
  moveThisTurn = move.id`.
- `moveThisTurnResult` semantics: `undefined` not moved, `false` failed, `null` "did nothing /
  couldn't act" (Stomping Tantrum/Lash Out/Truant read it).

### 2.1 BeforeMove handlers (order matters for PRNG)

`runEvent('BeforeMove', pokemon, target, move)` — handlers on the user, sorted by
`onBeforeMovePriority` desc (then speed/subOrder; same-holder ties at equal priority and subOrder
shuffle **[PRNG]**). The first handler returning `false`/`null` stops the event.

| prio | effect | behaviour (data file:line) |
|---|---|---|
| 100 | Glaive Rush, Chilly Reception, Grudge, Rage volatiles | cleanup only (`data/moves.ts:6670,2412,7912,14573`) |
| 11 | `mustrecharge` | `cant\|recharge`, remove `mustrecharge` & `truant`, return **null** (`conditions.ts:364`) |
| 10 | `slp` | `time--` (twice with Early Bird); `time <= 0` → `cureStatus()`, continue; else `cant\|slp`, return false unless `move.sleepUsable` (`conditions.ts:66`) |
| 10 | `frz` | defrost-flag move (not Burn Up w/o Fire) → continue; **`randomChance(1,5)`** → cure; else `cant\|frz` false (`conditions.ts:96`) |
| 9 | Truant | (`abilities.ts:5190`) |
| 8 | `flinch` | `cant\|flinch`, `runEvent('Flinch')` (Steadfast), false (`conditions.ts:198`) |
| 7 | Disable | |
| 6 | Gravity, Heal Block, Throat Chop | |
| 5 | Taunt | |
| 3 | `confusion` | `time--`; 0 → remove (`-end\|…\|confusion`), continue; else `-activate\|p\|confusion`; **`randomChance(33,100)`** false→continue; true→ self-hit: `activeTarget = pokemon`, `d = getConfusionDamage(pokemon, 40)`, `damage(d, pokemon, pokemon, {id:'confused', effectType:'Move', type:'???'})` → `\|-damage\|p\|hp\|[from] confusion`, return false (`conditions.ts:179`) |
| 2 | Attract | |
| 1 | `par` | **`randomChance(1,4)`** → `cant\|par`, false (`conditions.ts:39`) |
| 0 | `choicelock` (Condition), Gorilla Tactics (Ability) | choice-lock failure: `addMove('move', p, move.name)`, `[still]`, `-fail`, false |
| -1 | Destiny Bond volatile | |

`getConfusionDamage(pokemon, 40)` (`battle-actions.ts:1837`):
```
atk = pokemon.calculateStat('atk', pokemon.boosts.atk); def = pokemon.calculateStat('def', pokemon.boosts.def)
base = trunc(trunc(trunc(trunc(2*level/5 + 2) * 40 * atk) / def) / 50) + 2
d = trunc(base, 16); d = randomizer(d)  [PRNG random(16)]; return max(1, d)
```
(no crit, no STAB, no type, no modifiers).

### 2.2 Locked / charge / recharge plumbing

- `getLockedMove()` = `priorityEvent('LockMove', pokemon)`; `true` → null. Providers:
  `lockedmove.onLockMove → effectState.move`, `twoturnmove.onLockMove → effectState.move`,
  `mustrecharge.onLockMove = 'recharge'` (`conditions.ts:253-378`).
- Choice side (`side.ts:675-689`): a locked Pokémon's choice becomes
  `{choice:'move', moveid: lockedMove, targetLoc: volatiles[lockedMove]?.targetLoc || lastMoveTargetLoc || 0}`.
- **Recharge** (Giga Impact, Hyper Beam…): move data `self: {volatileStatus: 'mustrecharge'}`
  applied in `selfDrops` (§4.12) only if at least one target was not `false`; `onStart` logs
  `|-mustrecharge|p`. Next turn the forced `recharge` action is cancelled in BeforeMove (prio 11):
  `|cant|p|recharge`, returns null. Duration 2.
- **Two-turn moves** (Solar Beam, Electro Shot, Meteor Beam, Phantom Force, Shadow Force …):
  the move's `onTryMove` (singleEvent `TryMove`, §3 step 17):
  ```
  if attacker.removeVolatile(move.id): return            // 2nd turn: proceed normally
  add('-prepare', attacker, move.name)
  [Electro Shot/Meteor Beam: boost({spa:1}, attacker, attacker, move)]
  if weather shortcut (Solar*: sun via attacker.effectiveWeather(undefined, true); Electro Shot: rain):
      attrLastMove('[still]'); addMove('-anim', attacker, move.name, defender); return
  if !runEvent('ChargeMove', attacker, defender, move): return   // Power Herb: useItem, [still], -anim, false
  attacker.addVolatile('twoturnmove', defender); return null
  ```
  `twoturnmove.onStart(attacker, defender, effect)` (`conditions.ts:291`): `effectState.move =
  effect.id`; `attacker.addVolatile(effect.id)` (Phantom/Shadow Force's condition makes the user
  invulnerable via `onInvulnerability: false`); `targetLoc = attacker.lastMoveTargetLoc`; if the
  move was called by another move and target type ≠ self: if `defender.fainted` →
  `defender = sample(attacker.foes(true))` **[PRNG]**; `targetLoc = getLocOf(defender)`; store
  `volatiles[effect.id].targetLoc`; `attrLastMove('[still]')`; `runEvent('PrepareHit', attacker,
  defender, effect)`. Turn-1 log (verified): `|move|p1a: X|Solar Beam||[still]` then
  `|-prepare|p1a: X|Solar Beam`. Turn 2: PP not deducted (locked), move line carries
  `|[from] lockedmove`. `useMoveInner` returns `null` on the charge turn (TryMove null).
- **Rampage** (`lockedmove`, via `self.volatileStatus`): `onStart` draws `random(2,4)`
  (`trueDuration`), duration 2; `onRestart` resets duration to 2 while `trueDuration >= 2`;
  `onAfterMove` removes at duration 1; `onEnd` adds confusion when `trueDuration <= 1`
  (`|-start|p|confusion|[fatigue]`); `onResidual` drops it when asleep. Protect/Wide Guard delete the
  volatile (raw `delete`) if `duration === 2`. (No rampage move is in this format's set pool, but
  Instruct/Dancer/Struggle paths keep the code reachable in principle.)

### 2.3 Struggle

Choice: chosen when a Pokémon has no usable moves (`side.ts:701`), `targetLoc` 0 → random target
(`randomNormal`) **[PRNG]**. In `runMove`, OverrideAction is skipped and `deductPP` returning 0 is
tolerated. In `useMoveInner`, `singleEvent('ModifyMove')` sets `type = '???'` and logs
`|-activate|p|move: Struggle` **before** the `|move|` line (verified). Accuracy `true`, `'???'`
type → no immunity, effectiveness 0, no STAB. Recoil (§4.14): `max(1, round(baseMaxhp/4))` via
`directDamage` → `|-damage|p|hp|[from] recoil` (lowercase). Recoil is also applied when hitting a
Substitute (inside Substitute's TryPrimaryHit).

### 2.4 Dancer (`battle-actions.ts:319-345`)

```
if move.flags.dance && moveDidSomething && !move.isExternal:
    dancers = [p for p in getAllActive() if p != pokemon && p.hasAbility('dancer') && !p.isSemiInvulnerable()]
    dancers.sort((a,b) => (a.storedStats.spe - b.storedStats.spe) || (b.abilityState.effectOrder - a.abilityState.effectOrder))
        // JS stable sort, ascending stored Speed; tie → higher effectOrder first. No PRNG.
    targetOf1stDance = battle.activeTarget
    for dancer in dancers:
        if battle.faintMessages(): break
        if dancer.fainted: continue
        add('-activate', dancer, 'ability: Dancer')
        t = (!targetOf1stDance.isAlly(dancer) && pokemon.isAlly(dancer)) ? targetOf1stDance : pokemon
        runMove(move.id, dancer, dancer.getLocOf(t), {sourceEffect: Dancer ability, externalMove: true})
```

---------------------------------------------------------------------------------------------------

## 3. `useMove` / `useMoveInner` — the inner move caller (`battle-actions.ts:365-544`)

`useMove` is used directly by callers (Sleep Talk, Metronome-likes, Magic Bounce re-use, etc.).

```
useMove(move, pokemon, options):
    pokemon.moveThisTurnResult = undefined
    old = pokemon.moveThisTurnResult                      // undefined
    r = useMoveInner(move, pokemon, options)
    if old === pokemon.moveThisTurnResult: pokemon.moveThisTurnResult = r
    return r

useMoveInner(moveOrMoveName, pokemon, {target, sourceEffect}):
 1  if !sourceEffect && battle.effect.id: sourceEffect = battle.effect     // calling effect, if any
    if sourceEffect && sourceEffect.id in ['instruct','custapberry']: sourceEffect = null
 2  move = getActiveMove(moveOrMoveName); pokemon.lastMoveUsed = move
 3  (Z / Max branches ignored)
 4  if battle.activeMove:
        move.priority = battle.activeMove.priority
        if !move.hasBounced: move.pranksterBoosted = battle.activeMove.pranksterBoosted
 5  baseTarget = move.target
    rv = runEvent('ModifyTarget', pokemon, target, move, {target}, onEffect=true)   // Comeuppance, Metal Burst
    if rv.target !== undefined: target = rv.target
 6  if target === undefined: target = getRandomTarget(pokemon, move)            [PRNG]
 7  if move.target in ['self','allies']: target = pokemon
 8  if sourceEffect: move.sourceEffect = sourceEffect.id; move.ignoreAbility = sourceEffect.ignoreAbility
        // NB overwrites a data-defined ignoreAbility with the source's (usually undefined)
 9  moveResult = false
    setActiveMove(move, pokemon, target)
10  singleEvent('ModifyType', move, null, pokemon, target, move, move)
    singleEvent('ModifyMove', move, null, pokemon, target, move, move)
    if baseTarget !== move.target: target = getRandomTarget(pokemon, move)       [PRNG]
11  move = runEvent('ModifyType', pokemon, target, move, move)
    move = runEvent('ModifyMove', pokemon, target, move, move)
    if baseTarget !== move.target: target = getRandomTarget(pokemon, move)       [PRNG]
        // compares to the ORIGINAL baseTarget: if step 10 changed the target type, this draws AGAIN
        // (e.g. Expanding Force in Psychic Terrain, Tera Starstorm on Terapagos-Stellar).
12  if !move || pokemon.fainted: return false
13  movename = move.name ('Hidden Power' for hiddenpower)
    attrs = sourceEffect ? `|[from] ${sourceEffect.fullname}` : ''
    addMove('move', pokemon, movename, `${target}${attrs}`)
        // |move|p1a: User|Move Name|p2b: Target[|[from] lockedmove | [from] ability: Dancer | [from] move: Sleep Talk]
        // target null → literally the string "null"
14  if !target:
        attrLastMove('[notarget]'); add('-fail', pokemon); return false
15  {targets, pressureTargets} = pokemon.getMoveTargets(move, target)           [PRNG] [log edits]
    if targets.length: target = targets.last
16  callerMoveForPressure = (sourceEffect && sourceEffect.pp) ? sourceEffect : null
    if !sourceEffect || callerMoveForPressure:          // lockedmove/Dancer/… → no Pressure
        extraPP = 0
        for source in pressureTargets:
            ppDrop = runEvent('DeductPP', source, pokemon, move)   // Pressure returns 1 vs non-allies
            if ppDrop !== true: extraPP += ppDrop || 0
        if extraPP > 0: pokemon.deductPP(callerMoveForPressure || moveOrMoveName, extraPP)
17  r = singleEvent('TryMove', move, null, pokemon, target, move)               // move's own onTryMove (charge moves…)
    if r: r = runEvent('TryMove', pokemon, target, move)                          // Armor Tail, Dazzling, Queenly Majesty
                                                                                  // (onFoeTryMove), Damp (onAnyTryMove)
    if !r: return r                                                              // null (charge turn) or false
18  singleEvent('UseMoveMessage', move, null, pokemon, target, move)
19  if move.ignoreImmunity === undefined: move.ignoreImmunity = (move.category == 'Status')
20  if move.selfdestruct == 'always': battle.faint(pokemon, pokemon, move)    // Explosion: queued FIRST
21  damage = false
    if move.target in ['all','foeSide','allySide','allyTeam']:
        damage = tryMoveHit(targets, pokemon, move)                             // §3.1
        if damage === NOT_FAIL: pokemon.moveThisTurnResult = null
        if damage || damage === 0 || damage === undefined: moveResult = true
    else:
        if !targets.length:
            attrLastMove('[notarget]'); add('-fail', pokemon); return false
            // verified: |move|p1a: X|Coaching|p1: Pikachu|[notarget]  then  |-fail|p1a: X
        moveResult = trySpreadMoveHit(targets, pokemon, move)                    // §4
22  if move.selfBoost && moveResult: moveHit(pokemon, pokemon, move, move.selfBoost, false, true)
        // Scale Shot / Clanging Scales: once, after all hits, no roll
23  if !pokemon.hp: battle.faint(pokemon, pokemon, move)                        // no-op if already queued
24  if !moveResult:
        originalHp = pokemon.hp
        singleEvent('MoveFail', move, null, target, pokemon, move)              // HJK crash, Steel Beam
        if pokemon !== target && move.category != 'Status':
            runEvent('EmergencyExit', pokemon, pokemon, undefined, originalHp)
        return false
25  if !battle.suppressingSecondaries() && !move.flags.futuremove:
        originalHp = pokemon.hp
        singleEvent('AfterMoveSecondarySelf', move, null, pokemon, target, move)
        runEvent('AfterMoveSecondarySelf', pokemon, target, move)               // Life Orb recoil, Shell Bell…
        if pokemon !== target && move.category != 'Status':
            runEvent('EmergencyExit', pokemon, pokemon, undefined, originalHp)
26  return true
```

### 3.1 `tryMoveHit` — field/side moves only (`battle-actions.ts:813-849`)

```
tryMoveHit(targets, pokemon, move):
    target = targets[0]
    setActiveMove(move, pokemon, targets[0])
    hit = singleEvent('Try', move, null, pokemon, target, move)
          && singleEvent('PrepareHit', move, {}, target, pokemon, move)
          && runEvent('PrepareHit', pokemon, target, move)
    if !hit: if hit === false: add('-fail', pokemon); attrLastMove('[still]')
             return false
    if move.target == 'all': hit = runEvent('TryHitField', target, pokemon, move)
    else:                    hit = runEvent('TryHitSide', target, pokemon, move)   // Magic Bounce (ally side)
    if !hit: if hit === false: add('-fail', pokemon); attrLastMove('[still]')
             return false
    return moveHit(target, pokemon, move)             // ONE target: targets[0]
```
`targets[0]` is: `allySide/allyTeam/all` → first alive ally-or-self (slot order); `foeSide` →
`foe.active[0]` (even at 0 hp). The side/field effect is applied via that Pokémon (`target.side`).

---------------------------------------------------------------------------------------------------

## 4. `trySpreadMoveHit` and the hit steps (`battle-actions.ts:546-1013`)

Used for every move whose target type is not `all/foeSide/allySide/allyTeam` (including
single-target moves).

```
trySpreadMoveHit(targets, pokemon, move, notActive=false):
    if targets.length > 1 && !move.smartTarget: move.spreadHit = true
        // decided BEFORE any filtering: a spread move keeps 0.75x even if only one target survives
    steps = [InvulnerabilityEvent, TryHitEvent, TypeImmunity, TryImmunity, Accuracy,
             BreakProtect, StealBoosts, MoveHitLoop]          // gen 9 order
    if notActive: setActiveMove(move, pokemon, targets[0])    // future moves only
    hit = singleEvent('Try', move, null, pokemon, targets[0], move)
          && singleEvent('PrepareHit', move, {}, targets[0], pokemon, move)
          && runEvent('PrepareHit', pokemon, targets[0], move)          // Protean/Libero; Protect's stall roll
    if !hit:
        if hit === false: add('-fail', pokemon); attrLastMove('[still]')
        return hit === NOT_FAIL                                        // '' → true
    atLeastOneFailure = false
    for step in steps:
        hitResults = step(targets, pokemon, move)
        if !hitResults: continue                                       // steps 5,6 return undefined
        targets = targets.filter((_, i) => hitResults[i] || hitResults[i] === 0)
        atLeastOneFailure ||= hitResults.some(v => v === false)
        if move.smartTarget && atLeastOneFailure: move.smartTarget = false
        if !targets.length: break
    move.hitTargets = targets
    moveResult = !!targets.length
    if !moveResult && !atLeastOneFailure: pokemon.moveThisTurnResult = null
    if move.spreadHit: attrLastMove('[spread] ' + targets.map(t => t.getSlot()).join(','))
        // verified: all targets protected → trailing "|[spread] " (empty list, trailing space)
    return moveResult
```
Note the different argument orders: `Try` (target=user, source=targets[0]),
`PrepareHit` singleEvent (target=targets[0], source=user), `PrepareHit` runEvent (target=user,
source=targets[0]). Protect's `onPrepareHit` = `!!queue.willAct() && runEvent('StallMove', user)`
— the stall roll `randomChance(1, counter)` happens only if another action remains queued.

### 4.1 Step 0 — `hitStepInvulnerabilityEvent` (`:617`)
```
if move.id == 'helpinghand': return all true
for each (i, target):
    if target.volatiles.commanding: r = false                         // Commander: unreachable here
    elif move.id == 'toxic' && pokemon.hasType('Poison'): r = true
    else r = runEvent('Invulnerability', target, pokemon, move)       // left-to-right, no PRNG
        // Phantom/Shadow Force volatile: false; No Guard (onAnyInvulnerability prio 1): returns 0 → kept
    if r === false:
        if move.smartTarget: move.smartTarget = false
        else: if !move.spreadHit: attrLastMove('[miss]'); add('-miss', pokemon, target)
```

### 4.2 Step 1 — `hitStepTryHitEvent` (`:639`)
```
hitResults = runEvent('TryHit', targets, pokemon, move)    // ARRAY target; left-to-right order:
    // handlers sorted by (order, priority desc, target index asc). Move's OWN onTryHit is NOT here.
    // Protect (vol, prio 3), Wide Guard (side cond, prio 4), Quick Guard, absorb abilities
    // (Volt/Water Absorb, Lightning Rod, Storm Drain, Sap Sipper, Flash Fire, Earth Eater, Well-Baked Body,
    //  Wind Rider, Motor Drive, Dry Skin), Bulletproof, Soundproof, Good as Gold, Telepathy, Magic Bounce…
if !hitResults.includes(true) && hitResults.includes(false):
    add('-fail', pokemon); attrLastMove('[still]')
for i: if hitResults[i] !== NOT_FAIL: hitResults[i] = hitResults[i] || false   // null/0/undefined → false
return hitResults
```
Protect returns `NOT_FAIL` (target dropped, not a "failure") after logging
`|-activate|<target>|move: Protect` (unless `move.smartTarget`, in which case it sets
`smartTarget = false` silently). Absorb abilities return `null` (→ `false` = failure, but no `-fail`
because the `-fail` check runs before normalisation and `null` is not `false`).

### 4.3 Step 2 — `hitStepTypeImmunity` (`:650`)
```
if move.ignoreImmunity === undefined: move.ignoreImmunity = (move.category == 'Status')
for i: hitResults[i] = targets[i].runImmunity(move, !move.smartTarget)
```
`runImmunity(move, message)` (`pokemon.ts:2236`):
```
if move.ignoreImmunity === true or move.ignoreImmunity[move.type]: return true
type = move.type; if !type || type == '???': return true
negate = !runEvent('NegateImmunity', this, type)                 // Foresight-likes
notImmune = type == 'Ground' ? isGrounded(negate) : (negate || dex.getImmunity(type, this))
if notImmune: return true
if message:
    if notImmune === null (Levitate): add('-immune', this, '[from] ability: Levitate')
    else add('-immune', this)
return false
```
`isGrounded(negate)` (`pokemon.ts:2148`): Gravity/Ingrain/Smack Down/Iron Ball → true; Flying
type (not negated, not ???+Roost) → false; Levitate (not Mold-Broken) → **null**; Magnet Rise /
Telekinesis → false; else `item !== 'airballoon'`. `dex.getImmunity(type, target)` uses
`target.getTypes()` (Tera type when terastallized) and `typechart[defType].damageTaken[type] === 3`.

### 4.4 Step 3 — `hitStepTryImmunity` (`:662`)
```
for (i, target):
    if move.flags.powder && target !== pokemon && !dex.getImmunity('powder', target):  // Grass
        add('-immune', target); r = false
    elif !singleEvent('TryImmunity', move, {}, target, pokemon, move):
        add('-immune', target); r = false
    elif move.pranksterBoosted && pokemon.hasAbility('prankster') && !target.isAlly(pokemon)
         && !dex.getImmunity('prankster', target):                                        // Dark
        if target.illusion || !(move.status && !dex.getImmunity(move.status, target)):
            hint("Since gen 7, Dark is immune to Prankster moves.")
        add('-immune', target); r = false
    else r = true
```

### 4.5 Step 4 — `hitStepAccuracy` (`:686`) **[PRNG]**
```
for (i, target):
    battle.activeTarget = target
    accuracy = move.accuracy                       // number or true
    if move.ohko:
        if !target.isSemiInvulnerable():
            accuracy = 30
            if move.ohko == 'Ice' && !pokemon.hasType('Ice'): accuracy = 20      // Sheer Cold
            if pokemon.level >= target.level && (move.ohko === true || !target.hasType(move.ohko)):
                accuracy += pokemon.level - target.level
            else:
                add('-immune', target, '[ohko]'); r = false; continue
        // (if semi-invulnerable: accuracy stays move.accuracy; Invulnerability already filtered)
    else:
        accuracy = runEvent('ModifyAccuracy', target, pokemon, move, accuracy)   // BEFORE stages
        if accuracy !== true:
            boost = 0
            if !move.ignoreAccuracy:
                b = runEvent('ModifyBoost', pokemon, null, null, {...pokemon.boosts})
                boost = clampIntRange(b.accuracy, -6, 6)
            if !move.ignoreEvasion:
                b = runEvent('ModifyBoost', target, null, null, {...target.boosts})
                boost = clampIntRange(boost - b.evasion, -6, 6)
            if boost > 0: accuracy = trunc(accuracy * (3 + boost) / 3)
            elif boost < 0: accuracy = trunc(accuracy * 3 / (3 - boost))
    if move.alwaysHit || (move.id == 'toxic' && pokemon.hasType('Poison'))
       || (move.target == 'self' && move.category == 'Status' && !target.isSemiInvulnerable()):
        accuracy = true
    else:
        accuracy = runEvent('Accuracy', target, pokemon, move, accuracy)        // No Guard → true
    if accuracy !== true && !randomChance(accuracy, 100):                      [PRNG random(100)]
        if move.smartTarget: move.smartTarget = false
        else: if !move.spreadHit: attrLastMove('[miss]'); add('-miss', pokemon, target)
        if !move.ohko && pokemon.hasItem('blunderpolicy') && pokemon.useItem(): boost({spe:2}, pokemon)
        r = false; continue
    r = true
```
- `ModifyAccuracy` event target = defender, source = attacker: `onModifyAccuracy` on the defender
  (Sand Veil, Snow Cloak, BrightPowder, Lax Incense, Tangled Feet), `onSourceModifyAccuracy` on the
  attacker (Compound Eyes, Hustle, Wide Lens, Zoom Lens), `onAnyModifyAccuracy` (Victory Star),
  plus Gravity (field). `Accuracy` event: No Guard (`onAnyAccuracy` → true), Thunder/Hurricane/Blizzard
  set `accuracy = true` in their own ModifyMove instead.
- `accuracy: 100` moves **do roll** (`random(100) < 100` always true but consumes). Verified.
- Rolls happen for all targets, in target order, before any damage of the move.
- Log on single-target miss: `|move|…|Target|[miss]` attribute + `|-miss|p_user|p_target`.
  Spread miss: only `|-miss|…` per missed target (no `[miss]` attr).

### 4.6 Step 5 — `hitStepBreakProtect` (`:751`) (Feint, Hyperspace Fury/Hole, Phantom/Shadow Force)
```
if move.breaksProtect:
    for target in targets:
        broke = false
        for id in ['banefulbunker','burningbulwark','kingsshield','obstruct','protect','silktrap','spikyshield']:
            if target.removeVolatile(id): broke = true            // Detect uses the 'protect' volatile
        for id in ['craftyshield','matblock','quickguard','wideguard']:
            if target.side.removeSideCondition(id): broke = true
        if broke:
            if move.id == 'feint': add('-activate', target, 'move: Feint')
            else add('-activate', target, `move: ${move.name}`, '[broken]')
            delete target.volatiles.stall                          // raw delete, no End event
return undefined
```
Only reachable when TryHit didn't stop it: these moves bypass Protect because
`checkMoveBypassesProtect` / Protect's handler lets them through (`flags.protect` absent or
`HitProtect` false).

### 4.7 Step 6 — `hitStepStealBoosts` (Spectral Thief; unobtainable in Gen 9 — Ignored, returns undefined).

### 4.8 `afterMoveSecondaryEvent(targets, pokemon, move)` (`:807`)
`singleEvent('AfterMoveSecondary', move, null, targets[0], pokemon, move)`;
`runEvent('AfterMoveSecondary', targets, pokemon, move)` (array; Red Card, Eject Button, Pickpocket,
Anger Shell, frz `thawsTarget`…).

### 4.9 Step 7 — `hitStepMoveHitLoop` (`:850-1013`)
```
damage = targets.map(_ => 0)
move.totalDamage = 0; pokemon.lastDamage = 0
targetHits = move.multihit || 1
if targetHits is array:
    if targetHits == [2,5]:                                               // only array form in gen 9 data
        targetHits = sample([2,2,2,2,2,2,2,3,3,3,3,3,3,3,4,4,4,5,5,5])   [PRNG random(20)]
        if targetHits < 4 && pokemon.hasItem('loadeddice'):
            targetHits = 5 - random(2)                                    [PRNG]
    else: targetHits = random(targetHits[0], targetHits[1] + 1)          [PRNG] (unreachable)
if targetHits == 10 && pokemon.hasItem('loadeddice'): targetHits -= random(7)   [PRNG] Population Bomb
targetHits = floor(targetHits)
// Skill Link (ModifyMove) already replaced [2,5] with 5 and deleted multiaccuracy → no draw.
// Loaded Dice ModifyMove deletes multiaccuracy.
nullDamage = true; moveDamage = []
isSleepUsable = move.sleepUsable || dex.moves.get(move.sourceEffect).sleepUsable
targetsCopy = targets.slice()
for hit = 1 .. targetHits:
    if damage.includes(false): break
    if hit > 1 && pokemon.status == 'slp' && !isSleepUsable: break
    if targets.every(t => !t?.hp): break
    move.hit = hit; move.lastHit = (hit == targetHits)
    if move.smartTarget && targets.length > 1:
        targetsCopy = [targets[hit-1]]; damage = [damage[hit-1]]          // NB damage becomes length 1
    else targetsCopy = targets.slice()
    target = targetsCopy[0]
    if target && typeof move.smartTarget == 'boolean':                  // Dragon Darts (true OR false)
        if hit > 1: addMove('-anim', pokemon, move.name, target)          // moves lastMoveLine!
        else retargetLastMove(target)
    if target && move.multiaccuracy && hit > 1:                          // Triple Axel, Population Bomb
        accuracy = move.accuracy
        boostTable = [1, 4/3, 5/3, 2, 7/3, 8/3, 3]                        // f64
        if accuracy !== true:
            if !move.ignoreAccuracy:
                b = clampIntRange(runEvent('ModifyBoost', pokemon, null, null, {...pokemon.boosts}).accuracy, -6, 6)
                accuracy = b > 0 ? accuracy * boostTable[b] : accuracy / boostTable[-b]
            if !move.ignoreEvasion:
                b = clampIntRange(runEvent('ModifyBoost', target, null, null, {...target.boosts}).evasion, -6, 6)
                if b > 0: accuracy /= boostTable[b] elif b < 0: accuracy *= boostTable[-b]
        accuracy = runEvent('ModifyAccuracy', target, pokemon, move, accuracy)   // AFTER stages here;
            // modifier only applied if accuracy is a non-negative integer (see §0.1)
        if !move.alwaysHit:
            accuracy = runEvent('Accuracy', target, pokemon, move, accuracy)
            if accuracy !== true && !randomChance(accuracy, 100): break     [PRNG] (silent: no -miss)
    [moveDamageThisHit, targetsCopy] = spreadMoveHit(targetsCopy, pokemon, move, move)   // §5
    if move.smartTarget: moveDamage.push(...moveDamageThisHit) else moveDamage = moveDamageThisHit
    if !moveDamage.some(v => v !== false): break
    nullDamage = false
    for (i, md) in moveDamage:
        if move.smartTarget && i != hit-1: continue
        damage[i] = (md === true || !md) ? 0 : md
        move.totalDamage += damage[i]
    eachEvent('Update')                                                   [PRNG on speed ties]
    if !pokemon.hp && targets.length == 1: hit++; break                  // user fainted (Rough Skin…)
// `hit` is now (#hits performed + 1)
if hit == 1: return damage.fill(false)
if nullDamage: damage.fill(false)
faintMessages(false, false, !pokemon.hp)          // processes faints NOW (before -hitcount/recoil)
if move.multihit && typeof move.smartTarget != 'boolean': add('-hitcount', targets[0], hit - 1)
if move.totalDamage: applyRecoilDamage(move.totalDamage, move, pokemon)        // §4.14
if move.smartTarget: targetsCopy = targets.slice()
for (i, target) in targetsCopy:
    if target && pokemon !== target:
        target.gotAttacked(move, moveDamage[i], pokemon)
        if typeof moveDamage[i] == number: target.timesAttacked += move.smartTarget ? 1 : hit - 1
if move.ohko && !targets[0].hp: add('-ohko')
if !damage.some(v => !!v || v === 0): return damage
eachEvent('Update')                                                       [PRNG on speed ties]
if !suppressingSecondaries():
    afterMoveSecondaryEvent(targetsCopy.filter(t => !!t), pokemon, move)   // excludes false AND null(sub)
    for (i, d) in damage:
        cur = targets.length == 1 ? move.totalDamage : d
        if typeof cur == number && targets[i].hp:
            runEvent('EmergencyExit', targets[i], pokemon, undefined, (targets[i].hurtThisTurn || 0) + cur)
return damage
```
Important consequences:
- The returned `damage` array is the step result used by `trySpreadMoveHit`'s filter: entries that
  are `false` (failed) or `undefined` drop out of `move.hitTargets`; `0` stays.
- Dragon Darts quirk (port literally): after hit 2 the array is `[undefined, d2]` (length 2, hole at
  0), so the first target is filtered out of `hitTargets`.
- `-hitcount` is not printed for Dragon Darts even when it fell back to single-target.
- `gotAttacked(move, dmg, src)` (`pokemon.ts:917`) pushes `{source, damage: number-or-0, move: id,
  thisTurn: true, slot: src.getSlot(), damageValue: dmg}` onto `attackedBy`.
- Verified Population Bomb (Loaded Dice) sequence: accuracy `random(100)` → `random(7)` →
  per hit (crit `random(24)`, `random(16)`), loop stops when the user faints to Rough Skin;
  `|faint|` precedes `|-hitcount|`.

---------------------------------------------------------------------------------------------------

## 5. `spreadMoveHit` — one hit against a target list (`battle-actions.ts:1014-1126`)

Also used (via `moveHit`) for secondaries (`isSecondary`), self effects (`isSelf`), selfBoost, and
side/field moves. `moveHit(targets, pokemon, move, moveData, isSecondary, isSelf)` = wrap single
target in array, call `spreadMoveHit`, return `[0][0]` with `true → undefined` (`:1359`).

```
spreadMoveHit(targets, pokemon, move, hitEffect?, isSecondary=false, isSelf=false):
    target = targets[0]
    damage = targets.map(_ => true)
    move = getActiveMove(move)                    // same object
    hitResult = true
    moveData = hitEffect || move                  // the HitEffect: move itself / secondary / self / selfBoost
    if move.target == 'all' && !isSelf:
        hitResult = singleEvent('TryHitField', moveData, {}, target || null, pokemon, move)
    elif move.target in ['foeSide','allySide','allyTeam'] && !isSelf:
        hitResult = singleEvent('TryHitSide', moveData, {}, target || null, pokemon, move)
    elif target:
        hitResult = singleEvent('TryHit', moveData, {}, target, pokemon, move)   // move's own onTryHit, per hit
    if !hitResult:
        if hitResult === false: add('-fail', pokemon); attrLastMove('[still]')
        return [[false], targets]

    // 0. Substitute
    if !isSecondary && !isSelf && move.target not in ['all','allyTeam','allySide','foeSide']:
        for (i, t) in targets: if t: damage[i] = runEvent('TryPrimaryHit', t, pokemon, moveData)
    for i:
        if damage[i] === HIT_SUBSTITUTE (0): damage[i] = true; targets[i] = null     // null = "hit the sub"
        if targets[i] && isSecondary && !moveData.self: damage[i] = true
        if !damage[i]: targets[i] = false
    // 1. damage numbers
    damage = getSpreadDamage(damage, targets, pokemon, move, moveData, isSecondary, isSelf)   [PRNG]
    for i: if damage[i] === false: targets[i] = false
    // 2. apply damage
    damage = battle.spreadDamage(damage, targets, pokemon, move)            // §6.1 (logs -damage, drain)
    for i: if damage[i] === false: targets[i] = false
    // 3. effects / onHit
    damage = runMoveEffects(damage, targets, pokemon, move, moveData, isSecondary, isSelf)    // §5.3
    for i: if !damage[i] && damage[i] !== 0: targets[i] = false
    activeTarget = battle.activeTarget
    // 4. self effects
    if moveData.self && !move.selfDropped: selfDrops(targets, pokemon, move, moveData, isSecondary)   [PRNG]
    // 5. secondaries
    if moveData.secondaries: secondaries(targets, pokemon, move, moveData, isSelf)                  [PRNG]
    battle.activeTarget = activeTarget
    // 6. phazing
    if moveData.forceSwitch: damage = forceSwitch(damage, targets, pokemon, move)
    for i: if !damage[i] && damage[i] !== 0: targets[i] = false
    damagedTargets = [t for (i,t) if typeof damage[i] == number && t]; damagedDamage = matching damage
    pokemonOriginalHP = pokemon.hp
    if damagedDamage.length && !isSecondary && !isSelf:
        runEvent('DamagingHit', damagedTargets, pokemon, move, damagedDamage)    // array, left-to-right
            // Rough Skin, Static (randomChance(3,10)), Flame Body, Effect Spore, Cotton Down, Rocky Helmet,
            // frz thaw by Fire, Justified, Stamina, Weak Armor… Fires even for targets at 0 hp (not yet fainted).
        if moveData.onAfterHit && pokemon.hp:
            for t in damagedTargets: singleEvent('AfterHit', moveData, {}, t, pokemon, move)
                // onAfterHit moves: Knock Off, Covet, Thief, Rapid Spin, Mortal Spin, Ice Spinner,
                // Ceaseless Edge, Stone Axe
        runEvent('EmergencyExit', pokemon, undefined, undefined, pokemonOriginalHP)
    return [damage, targets]
```
Target-slot sentinel values after step 0: `Pokemon` (normal), `null` (hit a Substitute: skipped
by damage/effects, **but self-drops and secondaries still iterate it**), `false` (failed).
For a `null` (substitute) entry, `secondaries()` still rolls `random(100)` for each secondary with
`ModifySecondaries` run on the *battle* (`runEvent` target null → battle), so Shield Dust etc. don't
filter; the resulting `moveHit(null, …)` applies only the secondary's `self` part (Power-Up Punch
boosts through a sub; flinch etc. do nothing).

### 5.1 Substitute (`data/moves.ts`, `substitute.condition.onTryPrimaryHit`, priority -1)
If `target !== source && !move.flags.bypasssub && !move.infiltrates`:
`d = getDamage(source, target, move)` **[PRNG crit+roll]**; `!d && d !== 0` → `-fail`, `[still]`,
return null; `d = min(d, sub.hp)`; `sub.hp -= d`; `source.lastDamage = d`; if `sub.hp <= 0`:
(`-ohko` if OHKO) `target.removeVolatile('substitute')` → `|-end|p|Substitute|`; else
`|-activate|p|move: Substitute|[damage]`; if `d`: `applyRecoilDamage(d, move, source)`; if drain:
`heal(ceil(d*drain0/drain1), source, target, 'drain')` (**ceil**, unlike normal drain);
`singleEvent('AfterSubDamage', move, …)`, `runEvent('AfterSubDamage', target, source, move, d)`;
return `HIT_SUBSTITUTE`. Because the hit loop then sees damage `0`, `move.totalDamage` stays 0 and
recoil is not applied twice.

### 5.2 `getSpreadDamage` (`:1137`)
```
for (i, target) in targets:
    if !target: continue
    battle.activeTarget = target
    damage[i] = undefined
    cur = getDamage(source, target, moveData)
    if cur === false || cur === null:
        // (dead code: the '-fail' branch requires damage[i] === false, but it was just set to undefined)
        damage[i] = false; continue
    damage[i] = cur                      // number, or undefined for status / bp-0 moves
```
For secondaries/self/selfBoost `moveData` has no basePower → `getDamage` returns `undefined`
without drawing.

### 5.3 `runMoveEffects` (`:1175-1305`)
```
didAnything = damage.reduce(combineResults)
for (i, target) in targets:
    if target === false: continue
    hitResult = undefined; didSomething = undefined
    if target:                                            // null (sub) skips this block
        if moveData.boosts && !target.fainted:
            hitResult = battle.boost(moveData.boosts, target, source, move, isSecondary, isSelf)
            didSomething = combineResults(didSomething, hitResult)
        if moveData.heal && !target.fainted:
            if target.hp >= target.maxhp:
                add('-fail', target, 'heal'); attrLastMove('[still]')     // |move|p|Recover||[still] ; |-fail|p|heal
                damage[i] = combineResults(damage[i], false); didAnything = combineResults(didAnything, null); continue
            amount = target.baseMaxhp * heal[0] / heal[1]
            d = battle.heal(Math.round(amount), target, source, move)
            if !d && d !== 0:
                if d !== null: add('-fail', source); attrLastMove('[still]')
                damage[i] = combineResults(damage[i], false); didAnything = combineResults(didAnything, null); continue
            didSomething = true
        if moveData.status:
            hitResult = target.trySetStatus(moveData.status, source, moveData.ability || move)
            if !hitResult && move.status:          // primary status move failing
                damage[i] = combineResults(damage[i], false); didAnything = combineResults(didAnything, null); continue
            didSomething = combineResults(didSomething, hitResult)
        if moveData.forceStatus: didSomething = combine(didSomething, target.setStatus(forceStatus, source, move))
        if moveData.volatileStatus: didSomething = combine(.., target.addVolatile(volatileStatus, source, move))
        if moveData.sideCondition:  didSomething = combine(.., target.side.addSideCondition(sideCondition, source, move))
        if moveData.slotCondition:  didSomething = combine(.., target.side.addSlotCondition(target, slotCondition, source, move))
        if moveData.weather:        didSomething = combine(.., field.setWeather(weather, source, move))
        if moveData.terrain:        didSomething = combine(.., field.setTerrain(terrain, source, move))
        if moveData.pseudoWeather:  didSomething = combine(.., field.addPseudoWeather(pseudoWeather, source, move))
        if moveData.forceSwitch:    didSomething = combine(.., !!battle.canSwitch(target.side))
        if move.target == 'all' && !isSelf:
            if moveData.onHitField: didSomething = combine(.., singleEvent('HitField', moveData, {}, target, source, move))
        elif move.target in ['foeSide','allySide'] && !isSelf:
            if moveData.onHitSide: didSomething = combine(.., singleEvent('HitSide', moveData, {}, target.side, source, move))
        else:
            if moveData.onHit: didSomething = combine(.., singleEvent('Hit', moveData, {}, target, source, move))
            if !isSelf && !isSecondary: runEvent('Hit', target, source, move)
    if moveData.selfdestruct == 'ifHit' && damage[i] !== false: battle.faint(source, source, move)
    if moveData.selfSwitch:
        if battle.canSwitch(source.side) && !source.volatiles.commanded: didSomething = true
        else didSomething = combineResults(didSomething, false)
    if didSomething === undefined: didSomething = true
    damage[i] = combineResults(damage[i], didSomething === null ? false : didSomething)
    didAnything = combineResults(didAnything, didSomething)
if !didAnything && didAnything !== 0 && !moveData.self && !moveData.selfdestruct:
    if !isSelf && !isSecondary && didAnything === false: add('-fail', source); attrLastMove('[still]')
elif move.selfSwitch && source.hp && !source.volatiles.commanded:
    source.switchFlag = move.id                           // U-turn, Volt Switch, Flip Turn, Parting Shot,
                                                          // Baton Pass, Shed Tail, Chilly Reception, Revival Blessing
return damage
```
`canSwitch(side)` = number of non-fainted Pokémon in `side.pokemon[active.length..]`.

### 5.4 `selfDrops` (`:1306`) **[PRNG]**
```
for target in targets:
    if target === false: continue
    if moveData.self && !move.selfDropped:
        if !isSecondary && moveData.self.boosts:
            roll = random(100)                                                    [PRNG — always]
            if moveData.self.chance === undefined || roll < moveData.self.chance:
                moveHit(source, source, move, moveData.self, isSecondary, true)
            if !move.multihit: move.selfDropped = true          // so spread moves roll ONCE
        else:
            moveHit(source, source, move, moveData.self, isSecondary, true)   // volatileStatus (mustrecharge,
                                                                               // lockedmove); once per target
```
Close Combat / Draco Meteor / Make It Rain / Stellar Tera Blast (`self` set in ModifyMove) all draw
`random(100)` even though they have no chance (verified). Multi-hit moves with `self.boosts`
roll every hit. Sheer Force deletes `move.self` together with `secondaries`.

### 5.5 `secondaries` (`:1325`) **[PRNG]**
```
for target in targets:
    if target === false: continue                  // fainted-but-not-processed targets STILL roll (verified)
    secs = runEvent('ModifySecondaries', target, source, moveData, moveData.secondaries.slice())
        // Shield Dust / Covert Cloak (target-side): keep only entries with .self
    for s in secs:
        roll = random(100)                                                       [PRNG — one per entry]
        if s.chance === undefined || roll < s.chance:     // (gen<=8 overflow ignored)
            moveHit(target, source, move, s, true, isSelf)
```
Serene Grace doubles `secondary.chance` (and `self.chance`) in ModifyMove (priority -2) on the
per-use clone. Sheer Force (`abilities.ts`, ModifyMove) deletes `move.secondaries` and `move.self`
and sets `hasSheerForce` when the move has secondaries and no `hasSheerForceBoost` → **no rolls at
all** and `suppressingSecondaries()` becomes true (skips AfterMoveSecondary[Self], e.g. Life Orb
recoil). A secondary hit (`moveHit(target, …, s, isSecondary=true)`) runs `TryHit` singleEvent on
the secondary object, skips Substitute, damage is `undefined`, then `runMoveEffects` applies
`s.boosts` (with `isSecondary` → no `-boost …|0` spam), `s.status` (trySetStatus with
`sourceEffect = move`), `s.volatileStatus` (flinch/confusion), `s.onHit`; `s.self` via selfDrops
without a roll. No `Hit`/`DamagingHit` events for secondaries.

### 5.6 `forceSwitch` (`:1342`) — Roar, Whirlwind, Dragon Tail, Circle Throw
```
for (i, target):
    if target && target.hp > 0 && source.hp > 0 && battle.canSwitch(target.side):
        r = runEvent('DragOut', target, source, move)          // Suction Cups, Guard Dog, Ingrain, Commanding
        if r: target.forceSwitchFlag = true
        elif r === false && move.category == 'Status':
            add('-fail', source); attrLastMove('[still]'); damage[i] = false
return damage
```
The actual drag happens after the whole action (§7.4).

### 5.7 Recoil / drain / crash (`:1368`)
```
applyRecoilDamage(dealt, move, pokemon):
    if move.struggleRecoil: r = clampIntRange(round(pokemon.baseMaxhp / 4), 1)
    elif move.mindBlownRecoil || move.chloroblastRecoil: r = round(pokemon.maxhp / 2)
    elif move.recoil: r = clampIntRange(round(dealt * recoil[0] / recoil[1]), 1)
    else return null
    hpBefore = pokemon.hp
    if move.struggleRecoil: battle.directDamage(r, pokemon, pokemon, {id: 'strugglerecoil'})  // "[from] recoil"
    else: battle.damage(r, pokemon, pokemon, move.mindBlownRecoil ? conditions.get(move.name) : 'recoil')
        // 'recoil' → Condition{name:'Recoil'} → "|-damage|p|hp|[from] Recoil" (capital R, verified)
        // Steel Beam: conditions.get('Steel Beam') → nonexistent Condition named 'steelbeam' → "[from] steelbeam"
    runEvent('EmergencyExit', pokemon, pokemon, undefined, hpBefore)
```
`round` = JS `Math.round` (half up). Called once per move with `move.totalDamage` (sum over hits and
targets), after `faintMessages` (so the target's `|faint|` precedes the recoil `-damage`; verified).
Rock Head / Magic Guard block via `Damage` event handlers. Steel Beam on failure: `onMoveFail`
deals `round(maxhp/2)`. High Jump Kick crash is its own `onMoveFail`.
Drain is in `spreadDamage` (§6.1).

---------------------------------------------------------------------------------------------------

## 5A. `getDamage` and `modifyDamage` — the damage formula (`battle-actions.ts:1572-1829`)

```
getDamage(source, target, move, suppressMessages=false):
 1  if typeof move == string: move = getActiveMove(move)
    if typeof move == number: (BP-only typeless physical pseudo-move; unused here)
 2  if !target.runImmunity(move, !suppressMessages): return false
 3  if move.ohko: return target.maxhp
 4  if move.damageCallback: return move.damageCallback(source, target)   // Super Fang, Endeavor, Final Gambit, Ruination…
 5  if move.damage == 'level': return source.level                        // Night Shade
    elif move.damage: return move.damage
 6  category = battle.getCategory(move)            // move.category || 'Physical' (ActiveMove → its current category)
 7  basePower = move.basePower
    if move.basePowerCallback: basePower = move.basePowerCallback(source, target, move)
    if !basePower: return basePower === 0 ? undefined : basePower       // status moves → undefined, NO draws
    basePower = clampIntRange(basePower, 1)                               // floor, min 1
 8  critRatio = runEvent('ModifyCritRatio', source, target, move, move.critRatio || 0)  // default 1
    critRatio = clampIntRange(critRatio, 0, 4); critMult = [0, 24, 8, 2, 1]
 9  hitData = target.getMoveHitData(move)
    hitData.crit = move.willCrit || false
    if move.willCrit === undefined && critRatio:
        hitData.crit = randomChance(1, critMult[critRatio])             [PRNG random(24|8|2|1)]
    if hitData.crit: hitData.crit = runEvent('CriticalHit', target, null, move)   // Battle/Shell Armor → false
10  basePower = runEvent('BasePower', source, target, move, basePower, onEffect=true)   // after crit roll
    if !basePower: return 0
    basePower = clampIntRange(basePower, 1)
11  (Max-move BP=0 hack ignored)
12  dexMove = dex.moves.get(move.id)                                      // STATIC data
    if source.terastallized
       && (source.terastallized == 'Stellar' ? !source.stellarBoostedTypes.includes(move.type)
                                              : source.hasType(move.type))
       && basePower < 60 && dexMove.priority <= 0 && !dexMove.multihit
       && !((move.basePower == 0 || move.basePower == 150) && move.basePowerCallback):
        basePower = 60                                                     // Tera 60-BP floor
13  level = source.level
    attacker = move.overrideOffensivePokemon == 'target' ? target : source     // Foul Play
    defender = move.overrideDefensivePokemon == 'source' ? source : target
    isPhysical = move.category == 'Physical'
    attackStat = move.overrideOffensiveStat || (isPhysical ? 'atk' : 'spa')    // Body Press 'def'
    defenseStat = move.overrideDefensiveStat || (isPhysical ? 'def' : 'spd')   // Psyshock 'def'
    atkBoosts = attacker.boosts[attackStat]; defBoosts = defender.boosts[defenseStat]
    ignoreNegOff = !!move.ignoreNegativeOffensive; ignorePosDef = !!move.ignorePositiveDefensive
    if hitData.crit: ignoreNegOff = true; ignorePosDef = true
    if move.ignoreOffensive || (ignoreNegOff && atkBoosts < 0): atkBoosts = 0
    if move.ignoreDefensive || (ignorePosDef && defBoosts > 0): defBoosts = 0   // Sacred Sword etc set ignoreDefensive
    attack  = attacker.calculateStat(attackStat, atkBoosts, 1, source)    // ModifyBoost on SOURCE (Unaware)
    defense = defender.calculateStat(defenseStat, defBoosts, 1, target)   // ModifyBoost on TARGET
    eventStat = category == 'Physical' ? 'Atk' : 'SpA'
    attack  = runEvent('Modify' + eventStat, source, target, move, attack)       // on the USER even for Foul Play
    defense = runEvent('Modify' + {def:'Def', spd:'SpD'}[defenseStat], target, source, move, defense)
14  baseDamage = trunc(trunc(trunc(trunc(2*level/5 + 2) * basePower * attack) / defense) / 50)
    return modifyDamage(baseDamage, source, target, move, suppressMessages)

modifyDamage(baseDamage, pokemon, target, move, suppressMessages):
 1  if !move.type: move.type = '???'
    type = move.type
 2  baseDamage += 2
 3  if move.spreadHit: baseDamage = modify(baseDamage, 0.75)               // modifier 3072
    (Parental Bond branch ignored)
 4  baseDamage = priorityEvent('WeatherModifyDamage', pokemon, target, move, baseDamage)
        // field weather handler: rain Water ×1.5 / Fire ×0.5, sun Fire ×1.5 / Water ×0.5 (Hydro Steam ×1.5
        // in sun via attacker.effectiveWeather), checks DEFENDER.effectiveWeather() (Utility Umbrella);
        // suppressed by Cloud Nine/Air Lock. chainModify → applied by runEvent's final modify.
 5  isCrit = target.getMoveHitData(move).crit
    if isCrit: baseDamage = trunc(baseDamage * (move.critModifier || 1.5))   // plain trunc, not modify()
 6  baseDamage = randomizer(baseDamage)                                   [PRNG random(16)]
 7  if type != '???':                                                     // STAB
        isSTAB = move.forceSTAB || pokemon.hasType(type) || pokemon.getTypes(false, true).includes(type)
        stab = isSTAB ? 1.5 : 1
        if pokemon.terastallized == 'Stellar':
            if !pokemon.stellarBoostedTypes.includes(type) || move.stellarBoosted:
                stab = isSTAB ? 2 : [4915, 4096]
                move.stellarBoosted = true                                // keeps the boost for later hits/targets
                if pokemon.species.name != 'Terapagos-Stellar': pokemon.stellarBoostedTypes.push(type)
            // NO ModifySTAB event for Stellar
        else:
            if pokemon.terastallized == type && pokemon.getTypes(false, true).includes(type): stab = 2
            stab = runEvent('ModifySTAB', pokemon, target, move, stab)    // Adaptability: 2→2.25, else→2
                                                                          // (only if forceSTAB or hasType(type))
        baseDamage = modify(baseDamage, stab)                              // 1.5→6144, 2→8192, 2.25→9216, [4915,4096]
 8  typeMod = clampIntRange(target.runEffectiveness(move), -6, 6)
    target.getMoveHitData(move).typeMod = typeMod
    if typeMod > 0: if !suppressMessages: add('-supereffective', target)
                    repeat typeMod: baseDamage *= 2
    if typeMod < 0: if !suppressMessages: add('-resisted', target)
                    repeat -typeMod: baseDamage = trunc(baseDamage / 2)
 9  if isCrit && !suppressMessages: add('-crit', target)                 // AFTER the effectiveness line
10  if pokemon.status == 'brn' && move.category == 'Physical' && !pokemon.hasAbility('guts') && move.id != 'facade':
        baseDamage = modify(baseDamage, 0.5)
11  baseDamage = runEvent('ModifyDamage', pokemon, target, move, baseDamage)     // final modifiers (below)
12  (Z/Max bypassProtect 0.25 / '-zbroken' ignored)
13  if !baseDamage: return 1
14  return trunc(baseDamage, 16)                                          // % 65536, after the min-1 check
```
Message order per target (verified): `-supereffective`/`-resisted`, then `-crit`; for spread moves
all targets' calc messages come first (target order), then all `-damage` lines.
Draw order per target: crit (`random(24)` normally), then roll (`random(16)`); targets in order.

`runEffectiveness(move)` (`pokemon.ts:2208`): if `this.terastallized && move.type == 'Stellar'` →
1; else sum over `getTypes()` of `singleEvent('Effectiveness', move, null, this, type, move,
dex.getEffectiveness(move, type))` then `runEvent('Effectiveness', this, type, move, that)`
(Freeze-Dry, Scrappy-like, Tera Shell…). Tera Shell (Terapagos-Terastal at full HP, not
suppressed): resets on `move.hit == 1`; returns -1 and logs `|-activate|p|ability: Tera Shell` once.

Event targets for the stat/damage modifier events (who holds the handler):
- `ModifyAtk/ModifySpA` target = attacker (`onModifyAtk` on attacker; `onSourceModifyAtk` on defender
  — e.g. Thick Fat, Heatproof-likes; `onAnyModifyAtk` — Ruin abilities).
- `ModifyDef/ModifySpD` target = defender (`onModifyDef`; `onSourceModifyDef` on attacker; `onAny…`).
- `ModifyDamage` target = attacker, source = defender: `onModifyDamage` on the **attacker**
  (Life Orb ×5324/4096, Expert Belt, Metronome item, Tinted Lens, Sniper, Neuroforce);
  `onSourceModifyDamage` on the **defender** (Multiscale, Shadow Shield, Filter, Solid Rock, Prism
  Armor, Fluffy, Ice Scales, Punk Rock, resist berries; Ripen prio -1); `onAnyModifyDamage` from
  any active/side (Reflect / Light Screen / Aurora Veil: ×2732/4096 in doubles if not crit and not
  infiltrating; Friend Guard ×0.75). Handler order = speedSort (priority, holder speed, subOrder:
  Condition 2 < side condition 4 < Ability 7 < Item 8, effectOrder) — ties draw. Because `chainModify`
  rounds at each step (`(a*b+2048)>>12`), the order changes results; port the order exactly.
- `BasePower` target = attacker, source = defender, `onEffect=true` (the move's own `onBasePower`
  joins the sorted list).

---------------------------------------------------------------------------------------------------

## 6. Core state mutators and the log lines they emit

### 6.1 `Battle.spreadDamage(damage[], targets[], source, effect, instafaint=false)` (`battle.ts:2095`)
```
if !targets: return [0]
if typeof effect == string || !effect: effect = conditions.getByID(effect || '')   // 'recoil'→Recoil, 'drain'→Drain
for (i, cur) in damage:
    target = targets[i]; td = cur
    if !(td || td === 0): ret[i] = td; continue            // undefined/false/null pass through
    if !target || !target.hp: ret[i] = 0; continue          // includes null (substitute)
    if !target.isActive: ret[i] = false; continue
    if td !== 0: td = clampIntRange(td, 1)                  // floor, min 1 (td is never `true` here: the
                                                            // only `true` entries are sub hits, whose target is null)
    if effect.id != 'strugglerecoil':
        if effect.effectType == 'Weather' && !target.runStatusImmunity(effect.id): ret[i] = 0; continue
        td = runEvent('Damage', target, source, effect, td, onEffect=true)   // Sturdy, Focus Sash, Disguise,
                                                                          // False Swipe (move onDamage), Magic Guard…
        if !(td || td === 0): ret[i] = (cur === true ? undefined : td); continue
    if td !== 0: td = clampIntRange(td, 1)
    ret[i] = td = target.damage(td, source, effect)           // Pokemon.damage, may queue faint
    if td !== 0: target.hurtThisTurn = target.hp
    if source && effect.effectType == 'Move': source.lastDamage = td
    name = effect.fullname == 'tox' ? 'psn' : effect.fullname
    switch effect.id:
      'partiallytrapped': add('-damage', t, t.getHealth, '[from] ' + t.volatiles.partiallytrapped.sourceEffect.fullname, '[partiallytrapped]')
      'powder':           add('-damage', t, t.getHealth, '[silent]')
      'confused':         add('-damage', t, t.getHealth, '[from] confusion')
      default:
        if effect.effectType == 'Move' || !name: add('-damage', t, t.getHealth)
        elif source && (source !== t || effect.effectType == 'Ability'):
                          add('-damage', t, t.getHealth, `[from] ${name}`, `[of] ${source}`)
        else:             add('-damage', t, t.getHealth, `[from] ${name}`)
    if td && effect.effectType == 'Move' && effect.drain && source:       // DRAIN, right after this -damage
        heal(Math.round(td * drain[0] / drain[1]), source, target, 'drain')
if instafaint: (gen<=2 only paths; with instafaint=true: for targets now at 0 hp → faintMessages(true)) — unused here
return ret
```
`Battle.damage(d, target, source, effect, instafaint)` = defaults from the current event
(`target ||= event.target`, `source ||= event.source`, `effect ||= battle.effect`), then
`spreadDamage([d],[target],…)[0]`. Typical residual/item damage: `damage(baseMaxhp/16)` → clamp
floors it.

`Pokemon.damage(d, source, effect)` (`pokemon.ts:1595`): `if !hp || isNaN(d) || d <= 0 → 0`;
`0<d<1 → 1`; `d = trunc(d)`; `hp -= d`; if `hp <= 0`: `d += hp` (actual damage dealt), `faint(source,
effect)`; return d.

`Battle.directDamage(d, target, source, effect)` (`battle.ts:2211`): event defaults; `!target?.hp →
0`; `!d → 0`; `d = clampIntRange(d, 1)`; no `Damage` event; `d = target.damage(d, source, effect)`;
log `strugglerecoil → '-damage', t, hp, '[from] recoil'`, `confusion → '[from] confusion'`, else plain
`'-damage', t, hp`; `if target.fainted: faint(target)` (never true here). Used by Substitute's HP
cost (`directDamage(maxhp/4)` → plain `-damage`), Shed Tail, Struggle recoil.

### 6.2 Healing (`battle.ts:2265`, `pokemon.ts:1640-1667`)
```
Battle.heal(d, target, source, effect):
    event defaults; if effect == 'drain': effect = conditions.getByID('drain')
    if d && d <= 1: d = 1
    d = trunc(d)
    d = runEvent('TryHeal', target, source, effect, d)      // Big Root (drain ×1.3), Heal Block, Liquid Ooze
    if !d: return d                                         // 0 → no log, returns 0
    if !target?.hp: return false; if !target.isActive: return false
    if target.hp >= target.maxhp: return false
    final = target.heal(d, source, effect)
    switch effect?.id:
      'leechseed','rest': add('-heal', t, hp, '[silent]')
      'drain':            add('-heal', t, hp, '[from] drain', `[of] ${source}`)   // source = drained target
      'wish':             (no log)
      default: if !effect: break
               Move → add('-heal', t, hp)
               source && source !== t → add('-heal', t, hp, `[from] ${effect.fullname}`, `[of] ${source}`)
               else → add('-heal', t, hp, `[from] ${effect.fullname}`)
    runEvent('Heal', target, source, effect, final)
    return final
Pokemon.heal(d): if !hp → false; d = trunc(d); NaN/≤0 → false; hp >= maxhp → false;
                 hp += d; if hp > maxhp: d -= hp - maxhp; hp = maxhp; return d
Pokemon.sethp(d): if !hp → 0; d = trunc(d); d < 1 → 1; delta = d - hp; hp += delta; clamp to maxhp; return delta
```
Move heal (Recover etc.): `heal(Math.round(baseMaxhp * h0 / h1), target, source, move)`.

### 6.3 Boosts (`battle.ts:2024`, `pokemon.ts:1210-1244`)
```
Battle.boost(boost, target, source, effect, isSecondary=false, isSelf=false):
    if battle.event: target ||= event.target; source ||= event.source; effect ||= battle.effect
    if !target?.hp: return 0
    if !target.isActive: return false
    if !target.side.foePokemonLeft(): return false
    boost = runEvent('ChangeBoost', target, source, effect, {...boost})     // Contrary, Simple
    boost = target.getCappedBoost(boost)
    boost = runEvent('TryBoost', target, source, effect, {...boost})        // Clear Body, Mirror Armor…
    success = null; boosted = isSecondary
    for name in boost (insertion order of the object):
        cur = {[name]: boost[name]}
        by = target.boostBy(cur)                       // applies & returns delta
        msg = '-boost'
        if boost[name] < 0 || target.boosts[name] == -6: msg = '-unboost'; by = -by
        if by:
            success = true
            switch effect?.id:
              'bellydrum','angerpoint': add('-setboost', target, 'atk', target.boosts.atk, '[from] ' + effect.fullname)
              default:
                if !effect: break
                if effect.effectType == 'Move': add(msg, target, name, by)
                elif effect.effectType == 'Item': add(msg, target, name, by, '[from] item: ' + effect.name)
                else:
                    if effect.effectType == 'Ability' && !boosted: add('-ability', target, effect.name, 'boost'); boosted = true
                    add(msg, target, name, by)
            runEvent('AfterEachBoost', target, source, effect, cur)
        elif effect?.effectType == 'Ability': if isSecondary || isSelf: add(msg, target, name, by)
        elif !isSecondary && !isSelf: add(msg, target, name, by)      // "|-boost|p|atk|0" = "won't go higher"
    runEvent('AfterBoost', target, source, effect, boost)
    if success: if any >0: target.statsRaisedThisTurn = true; if any <0: target.statsLoweredThisTurn = true
    return success                                                     // true or null
getCappedBoost(b): for each key with non-zero b[k]: capped[k] = clamp(boosts[k] + b[k], -6, 6) - boosts[k]
boostBy(b): b = getCappedBoost(b); for k: boosts[k] += b[k]; return last delta
setBoost(b): boosts[k] = b[k] for each key
```
Boost key iteration order is the object's insertion order (data order, e.g. `{def:-1, spd:-1}`).
At top level (no enclosing event) `battle.effect` is `{id:''}` (truthy, not Move/Item/Ability) →
plain `-boost` lines.

### 6.4 Status (`pokemon.ts:1669-1762`)
```
trySetStatus(status, source, sourceEffect) = setStatus(this.status || status, source, sourceEffect)
setStatus(status, source=null, sourceEffect=null, ignoreImmunities=false):
    if !hp: return false
    if !isActive && status: return false
    status = conditions.get(status)
    if battle.event: source ||= event.source; sourceEffect ||= battle.effect
    if !source: source = this
    if this.status == status.id:
        if sourceEffect?.status == this.status: add('-fail', this, this.status)
        elif sourceEffect?.status: add('-fail', source); attrLastMove('[still]')
        return false
    if !ignoreImmunities && status.id && !(source?.hasAbility('corrosion') && status.id in ['tox','psn']):
        if !runStatusImmunity(status.id == 'tox' ? 'psn' : status.id):
            if sourceEffect?.status: add('-immune', this)
            return false
    prev = (status, statusState)
    if status.id:
        r = runEvent('SetStatus', this, source, sourceEffect, status)    // Sleep Clause Mod, Misty Terrain, Safeguard…
        if !r: return r
    this.status = status.id
    statusState = initEffectState({id: status.id, target: this})       // allocates effectOrder if active
    if source: statusState.source = source
    if status.duration: statusState.duration = status.duration
    if status.durationCallback: statusState.duration = status.durationCallback(this, source, sourceEffect)
    if status.id && !singleEvent('Start', status, statusState, this, source, sourceEffect):
        restore prev; return false                                       // brn/par/…: '-status' line; slp draws random(2,5)
    if status.id && !runEvent('AfterSetStatus', this, source, sourceEffect, status): return false
    return true
cureStatus(silent=false): if !hp || !status → false
    add('-curestatus', this, this.status, silent ? '[silent]' : '[msg]')
    if status == 'slp' && removeVolatile('nightmare'): add('-end', this, 'Nightmare', '[silent]')
    setStatus(''); return true
clearStatus(): same without the -curestatus line
runStatusImmunity(type, message): fainted → false; !type → true;
    !dex.getImmunity(type, this) → (message: '-immune') false
    imm = runEvent('Immunity', this, null, null, type); !imm → (message && imm !== null: '-immune') false
    true
```
Status `onStart` logs (`conditions.ts`): `|-status|p|brn` (+`[from] item: Flame Orb` /
`[from] ability: X|[of] src`); slp from a move: `|-status|p|slp|[from] move: Spore` (verified).

### 6.5 Volatiles and side/slot conditions
```
addVolatile(status, source=null, sourceEffect=null, linkedStatus=null):   // pokemon.ts:1969
    status = conditions.get(status)
    if !hp && !status.affectsFainted: return false
    if linkedStatus && source && !source.hp: return false
    if battle.event: source ||= event.source; sourceEffect ||= battle.effect
    if !source: source = this
    if volatiles[status.id]:
        if !status.onRestart: return false
        return singleEvent('Restart', status, volatiles[id], this, source, sourceEffect)
    if !runStatusImmunity(status.id):
        if sourceEffect?.status: add('-immune', this)
        return false
    r = runEvent('TryAddVolatile', this, source, sourceEffect, status); if !r: return r
    volatiles[id] = initEffectState({id, name: status.name, target: this})   // effectOrder++ (target active)
    if source: .source = source; .sourceSlot = source.getSlot()
    if sourceEffect: .sourceEffect = sourceEffect
    if status.duration: .duration = status.duration
    if status.durationCallback: .duration = status.durationCallback(this, source, sourceEffect)
    r = singleEvent('Start', status, volatiles[id], this, source, sourceEffect)
    if !r: delete volatiles[id]; return r
    (linked volatile bookkeeping for Leech-Seed-like links)
    return true
removeVolatile(status): if !hp → false; if absent → false
    singleEvent('End', status, volatiles[id], this); delete volatiles[id]; removeLinkedVolatiles(...); true
side.addSideCondition(status, source, sourceEffect):        // side.ts:413
    existing → !onSideRestart ? false : singleEvent('SideRestart', ...)
    sideConditions[id] = initEffectState({id, target: side, source, sourceSlot, duration})
    durationCallback(side.active[0], source, sourceEffect)
    !singleEvent('SideStart', ...) → delete, false
    runEvent('SideConditionStart', side, source, status); true
side.removeSideCondition(id): absent → false; singleEvent('SideEnd', …); delete; true
side.addSlotCondition(target, status, source, sourceEffect): analogous with 'Restart'/'Start'
    (Start's target = side.active[slot]); isSlotCondition = true
```

### 6.6 Faint, faint processing, win (`pokemon.ts:1581`, `battle.ts:2539-2621,1524`)
```
Pokemon.faint(source=null, effect=null):
    if fainted || faintQueued: return 0
    d = hp; hp = 0; switchFlag = false; faintQueued = true
    battle.faintQueue.push({target: this, source, effect}); return d
Battle.faint(p, source, effect) = p.faint(source, effect)

faintMessages(lastFirst=false, forceCheck=false, checkWin=true):
    if ended: return
    length = faintQueue.length
    if !length: if forceCheck && this.checkWin(): return true; return false
    if lastFirst: move last entry to front
    while faintQueue.length:
        left = faintQueue.length; fd = faintQueue.shift(); p = fd.target
        if !p.fainted && runEvent('BeforeFaint', p, fd.source, fd.effect):
            add('faint', p)                                                // |faint|p2a: Name
            if p.side.pokemonLeft: p.side.pokemonLeft--
            if p.side.totalFainted < 100: p.side.totalFainted++
            runEvent('Faint', p, fd.source, fd.effect)                    // Destiny Bond, Aftermath…
            singleEvent('End', p.getAbility(), p.abilityState, p)
            singleEvent('End', p.getItem(), p.itemState, p)
            if p.formeRegression && !p.transformed: p.baseSpecies = species(set.species); p.baseAbility = set.ability
            p.clearVolatile(false)                                         // keeps switchFlag/forceSwitchFlag
            p.fainted = true; p.illusion = null; p.isActive = false; p.isStarted = false
            delete p.terastallized
            if p.formeRegression: p.details = getUpdatedDetails(); add('detailschange', p, p.details, '[silent]');
                                  p.updateMaxHp(); p.formeRegression = false
            p.side.faintedThisTurn = p
            if faintQueue.length >= left: checkWin = true
    if checkWin && this.checkWin(fd): return true
    if fd && length: runEvent('AfterFaint', fd.target, fd.source, fd.effect, length)   // ONCE, last entry
    return false

checkWin(fd):
    if every side has pokemonLeft == 0: win(fd ? fd.target.side : null); return true   // gen>4: last fainted's side wins
    for side in sides: if !side.foePokemonLeft(): win(side); return true
win(side): add(''); add('win', side.name) or add('tie'); ended = true; requestState = ''
checkFainted(): for each active with fainted: status = 'fnt'; switchFlag = true
```
Order matters: Explosion queues the user first (§3 step 20), so `|faint|` for the user precedes the
targets' (verified). `hitStepMoveHitLoop` processes faints with `checkWin = !pokemon.hp`.

### 6.7 Items used inside this pipeline (`pokemon.ts:1768-1866`)
`useItem(source, sourceEffect)`: `(!hp && !gem) || !isActive → false`; `!item → false`; event
defaults; `runEvent('UseItem', this, null, null, item)` → log `|-enditem|p|Item` (Red Card:
`[of] source`; Gems: `[from] gem|[move] <activeMove.name>`), `item.boosts → boost(...)`,
`singleEvent('Use', item, …)`, `lastItem = item; item = ''; clearEffectState(itemState);
usedItemThisTurn = true; runEvent('AfterUseItem', ...)`. `eatItem` similar with
`TryEatItem`, `|-enditem|p|Berry|[eat]`, `Eat`/`EatItem` events, `ateBerry = true`.

---------------------------------------------------------------------------------------------------

## 7. Switching (`battle-actions.ts:62-192`, `battle.ts:2765-2910`)

### 7.1 `switchIn(pokemon, pos, sourceEffect=null, isDrag=false)`
```
if !pokemon || pokemon.isActive: hint("A switch failed because the Pokémon trying to switch in is already in."); return false
side = pokemon.side; oldActive = side.active[pos]
if oldActive?.hp:                                            // unfainted outgoing
    oldActive.beingCalledBack = true
    switchCopyFlag = typeof sourceEffect?.selfSwitch == 'string' ? sourceEffect.selfSwitch : false  // 'copyvolatile' | 'shedtail'
    if !oldActive.skipBeforeSwitchOutEventFlag && !isDrag:
        runEvent('BeforeSwitchOut', oldActive)
        eachEvent('Update')                                                      [PRNG on ties]
    oldActive.skipBeforeSwitchOutEventFlag = false
    if !runEvent('SwitchOut', oldActive): return false        // Regenerator, Natural Cure, Zero to Hero…
    if !oldActive.hp: return 'pursuitfaint'                   // unreachable in gen 9
    singleEvent('End', oldActive.getAbility(), oldActive.abilityState, oldActive)
    singleEvent('End', oldActive.getItem(), oldActive.itemState, oldActive)
    queue.cancelAction(oldActive)                             // removes ALL its queued actions
    if switchCopyFlag: pokemon.copyVolatileFrom(oldActive, switchCopyFlag)
    oldActive.clearVolatile()
if oldActive:
    oldActive.isActive = false; isStarted = false; usedItemThisTurn = false
    statsRaisedThisTurn = false; statsLoweredThisTurn = false
    oldActive.position = pokemon.position
    if oldActive.fainted: oldActive.status = ''
    pokemon.position = pos
    side.pokemon[pokemon.position] = pokemon                  // PARTY ORDER SWAP
    side.pokemon[oldActive.position] = oldActive
pokemon.isActive = true; side.active[pos] = pokemon
pokemon.activeTurns = 0; pokemon.activeMoveActions = 0
for slot in pokemon.moveSlots: slot.used = false
pokemon.abilityState = initEffectState({id: pokemon.ability, target: pokemon})   // effectOrder++
pokemon.itemState    = initEffectState({id: pokemon.item,    target: pokemon})   // effectOrder++
runEvent('BeforeSwitchIn', pokemon)                          // Illusion setup
add(isDrag ? 'drag' : 'switch', pokemon, pokemon.getFullDetails [, `[from] ${sourceEffect}`])
    // split: |switch|p1b: Pikachu|Pikachu, M|232/232|[from] U-turn   then shared …|100/100|[from] U-turn
    // details: species[, L<lvl>][, M|F][, shiny][, tera:Type]; Illusion shows the disguise (Illusion Level Mod)
pokemon.previouslySwitchedIn++
if isDrag: runSwitch(pokemon)                                  // immediately
else queue.insertChoice({choice: 'runSwitch', pokemon})       // order 101; tie range → random(first,last+1) [PRNG]
return true
```
Party order matters: `possibleSwitches(side)` iterates `side.pokemon[active.length..]` in current
order, so it changes the outcome of `getRandomSwitchable`'s `sample`.

`copyVolatileFrom(old, cause)` (`pokemon.ts:1246`): `this.clearVolatile()`; unless `shedtail`:
`this.boosts = old.boosts` (same object); for each volatile id of `old` (insertion order): if
`shedtail && id != 'substitute'` skip; if `conditions[id].noCopy` skip; `this.volatiles[id] =
initEffectState({...old.volatiles[id], target: this})` — the incoming Pokémon is not active yet, so
`effectOrder` is reset to 0; fix linkedPokemon links; then `old.clearVolatile()`; then
`singleEvent('Copy', volatile, state, this)` for each copied volatile.

`clearVolatile(includeSwitchFlags=true)` (`pokemon.ts:1508`): boosts all 0; `moveSlots =
baseMoveSlots.slice()`; `transformed = false`; `ability = baseAbility`; hidden power reset;
`canTerastallize === false → teraType`; remove linked volatiles; `volatiles = {}`; switch flags
false (if requested); `lastMove = lastMoveUsed = null`, `moveThisTurn = ''`,
`moveLastTurnResult = moveThisTurnResult = undefined`, `lastDamage = 0`, `attackedBy = []`,
`hurtThisTurn = null`, `newlySwitched = true`, `beingCalledBack = false`,
`volatileStaleness = undefined`, delete `abilityState.started`/`itemState.started`,
`setSpecies(baseSpecies)` (reverts temporary formes/types/stats).

### 7.2 `runSwitch(pokemon)`
```
switchersIn = [pokemon]
while queue.peek()?.choice == 'runSwitch': switchersIn.push(queue.shift().pokemon)
allActive = getAllActive(true)                     // includes fainted-in-slot
speedSort(allActive)                               // by pokemon.speed; ties shuffle [PRNG]
battle.speedOrder = allActive.map(a => a.getFieldPositionValue())   // side.n + 2*position
fieldEvent('SwitchIn', switchersIn)                // hazards, abilities, items, onAnySwitchIn …  [PRNG on ties]
for p in switchersIn: if p.hp: p.isStarted = true; p.draggedIn = null
```
`fieldEvent` handler details (fractional speeds from `speedOrder`, effectOrder ties for
`*SwitchIn`) are in the event-system spec.

### 7.3 `dragIn(side, pos)` **[PRNG]**
```
pokemon = battle.getRandomSwitchable(side)        // sample(possibleSwitches) — draws FIRST
if !pokemon || pokemon.isActive: return false
oldActive = side.active[pos]; if !oldActive: throw
if !oldActive.hp: return false
if !runEvent('DragOut', oldActive): return false
return switchIn(pokemon, pos, null, true)         // no BeforeSwitchOut, 'drag' line, runSwitch now
```
Verified: `|drag|p2b: Eevee|Eevee, F|272/272` (no `[from]`), preceded by `random(1)` when only
one bench Pokémon remains.

### 7.4 Where switches are executed (`battle.ts:2825-2910`, end of every `runAction`)
1. **Phazing**: for each side, each active (slot order): if `forceSwitchFlag`: `if hp: dragIn(side,
   position)`; `forceSwitchFlag = false`. (Runs after the whole move incl. AfterMove and Dancer.)
2. `clearActiveMove()`; `faintMessages()`; return if ended.
3. If the queue is empty → `checkFainted()` (fainted actives get `switchFlag = true`); if the next
   action is `instaswitch` → return (no request yet).
4. `eachEvent('Update')` **[PRNG on ties]**; EmergencyExit checks (residual/runSwitch).
5. For each side with an active `switchFlag`: if `!canSwitch(side)` clear the flags (Revival Blessing
   exception); else for each active with `hp && switchFlag && switchFlag !== 'revivalblessing' &&
   !skipBeforeSwitchOutEventFlag`: `runEvent('BeforeSwitchOut', p)`; set
   `skipBeforeSwitchOutEventFlag = true`; `faintMessages()`.
6. If any side still needs a switch → `makeRequest('switch')`; the turn pauses. The chosen switches
   come back as `instaswitch` actions (order 3) prepended to the remaining queue; `resolveAction`
   sets `action.sourceEffect = dex.moves.get(switchFlag)` when `switchFlag` is a move id string
   (U-turn etc.) and clears the flag. `switchIn(target, pokemon.position, sourceEffect)` then copies
   volatiles for `copyvolatile`/`shedtail` and logs `[from] <Move Name>` (verified
   `[from] U-turn`, `[from] Shed Tail`).
7. Otherwise (gen ≥ 8) if the next action is a move: `updateSpeed()` all, recompute
   `getActionSpeed` for every queued action (→ `getTarget` draws, §1.2), `queue.sort()` (ties draw).

Self-switch flow summary: `runMoveEffects` sets `source.switchFlag = move.id` → after the action,
step 5/6 → request → `instaswitch` → `switchIn(..., sourceEffect=Move)` → queued `runSwitch`.

---------------------------------------------------------------------------------------------------

## 8. Terastallization (`battle-actions.ts:1911-1954`)

Queued as a separate action `{choice:'terastallize', order: 106}` before the move
(`battle-queue.ts:228`) when the move choice had `terastallize` and the Pokémon isn't already
terastallized; executed by `runAction` → `actions.terastallize(pokemon)`.
```
canTerastallize(p) = (p.getItem().zMove || p.canMegaEvo || gen != 9) ? null : p.teraType
terastallize(p):
    if p.species.baseSpecies == 'Ogerpon' && p.teraType not in [Fire,Grass,Rock,Water]
       && (!p.illusion || p.illusion.species.baseSpecies == 'Ogerpon'):
        hint("If Ogerpon Terastallizes into a type other than Fire, Grass, Rock, or Water, the game crashes.", false, p.side)
        return
    if p.illusion && p.illusion.species.baseSpecies in ['Ogerpon','Terapagos']:
        singleEvent('End', Illusion ability, p.abilityState, p)
    type = p.teraType
    add('-terastallize', p, type)                   // |-terastallize|p1b: Maushold|Stellar
    p.terastallized = type
    for ally in p.side.pokemon: ally.canTerastallize = null
    p.addedType = ''; p.knownType = true; p.apparentType = type
    if Ogerpon: formeChange(<battleOnly or id> + ('tealtera' if base 'ogerpon' else 'tera'), null, true)
    if p.species.name == 'Terapagos-Terastal': formeChange('Terapagos-Stellar', null, true)
    if Morpeko && !transformed && baseSpecies.id != species.id:
        formeRegression = true; baseSpecies = species; details = getUpdatedDetails()
    runEvent('AfterTerastallization', p)
```
Effects elsewhere: `getTypes()` returns `[teraType]` unless Stellar (`pokemon.ts:2138`);
`getTypes(false, true)` returns pre-tera types; `setType`/`addType` fail while terastallized;
`getFullDetails` appends `, tera:Type`; tera STAB/Stellar/60-BP floor in §5A; Tera Blast
(`data/moves.ts` terablast: type = teraType, Physical if `getStat('atk',false,true) >
getStat('spa',false,true)`, Stellar: BP 100, `self = {boosts:{atk:-1, spa:-1}}`, logs
`[anim] Tera Blast <Type>` attr in onPrepareHit); `faintMessages` deletes `terastallized`.

---------------------------------------------------------------------------------------------------

## 9. Worked PRNG traces (from the oracle, for difftest sanity)

- Earthquake (p1a) with ally Rotom (Levitate) and two foes: `getTarget` `random(2)` →
  `|move|p1a: Garchomp|Earthquake|p2b: Amoonguss|[spread] p2a,p2b`, `|-immune|p1b: Rotom|[from]
  ability: Levitate` (type step) → `random(100)`×2 (accuracy, p2a then p2b) → p2a: `random(24)`,
  `random(16)`, `|-supereffective|p2a…` → p2b: `random(24)`, `random(16)` → split `-damage` ×2.
- Rock Slide with one foe missing: `random(100)`×2 → `|-miss|p2a: Tyranitar|p1b: Rotom` →
  hit target crit+roll → `-damage` → flinch secondary `random(100)` (per hit target only).
- Close Combat: acc → crit → roll → `-damage` → selfDrops `random(100)` → `-unboost def 1`,
  `-unboost spd 1` → DamagingHit (Rough Skin `-damage … [from] ability: Rough Skin|[of] p1a…`).
- Protect fail on consecutive use: `|move|p1b: Pikachu|Protect||[still]` → `random(3)` → `|-fail|p1b: Pikachu`.
- Fake Out KO: acc, crit, roll, `-damage 0 fnt`, flinch `random(100)` (target at 0 hp still
  rolls), Static `random(10)` in DamagingHit, then `|faint|`.

---------------------------------------------------------------------------------------------------

## 10. Ignored (not reachable in gen9randomdoublesbattle) — explicitly skipped branches

- **Z-moves**: `zMove` options in `runMove`/`useMoveInner`, `getZMove`, `getActiveZMove`, `canZMove`,
  `runZPower`, `Z_MOVES`, `-zpower`, `[zeffect]` boost/heal paths, `isZ` checks, Z-Weather Ball.
- **Dynamax / Max moves**: `maxMove` options, `getMaxMove`, `getActiveMaxMove`, `MAX_MOVES`,
  `runDynamax`, `dynamax` volatile checks (OHKO `target.volatiles.dynamax`, LockMove), Max-move
  BP=0 hack (`getDamage` step 11), `bypassProtect` ×0.25 and `-zbroken`, G-Max effects.
- **Mega / Ultra Burst**: `canMegaEvo`, `canUltraBurst`, `runMegaEvo[X|Y]`, gen-7 mega re-sort.
- **Gen < 9 branches**: step-order swaps (gen ≤ 6, gen 4), gen-4 `selfdestruct` timing, gen-4
  `lastMove` copy & gen ≤ 4 `lastItem` transfer in `switchIn`, gen-2 `draggedIn`, gen ≤ 5/6 crit
  tables and 2× crit, gen < 5 heal flooring, gen ≤ 4 drain, gen-1 `lastDamage`/Substitute
  `directDamage`, gen ≤ 3 faint skipping, gen-5 min-damage, gen ≤ 4 Explosion defense halving, gen ≤ 8
  secondary overflow (`% 256`), gen-4 sleep multi-hit, gen < 5 `DamagingHit` placement, `-notarget`
  (gen < 5), gen ≤ 4 Pursuit switch continuation, gen-2 `moveUsed` special cases.
- **Parental Bond** (`multihitType === 'parentalbond'` modifier).
- **Free-for-all** (spread ×0.5, FFA hazard `TryHitSide` array, FFA `getTarget`/`validTargetLoc`/`foes`).
- **Multi battles / triples** (`allySide`, `side.n >= 2`, `getAtLoc` side+2, `activePerHalf > 2`
  branches in `getRandomTarget`/`adjacentFoes`/`isAdjacent`, `win` "A & B").
- **Singles** branches of `getRandomTarget` (`gameType === 'singles'`).
- **Pursuit** (`'pursuitfaint'`, tracksTarget via Pursuit) — move not in Gen 9.
- **Spectral Thief** (`hitStepStealBoosts`, `-clearpositiveboost`) — Past.
- **Mind Blown** (Past). Chloroblast (not in this format's pool; code path documented anyway).
- **Commander** (`commanding`/`commanded` checks) — ability not in the format's set pool.
- **Champions mod** branches (`getHealth` champions percentage, Curse target override in
  `resolveAction`) and **Mega Sol** (`effectiveWeather`, `WeatherModifyDamage` priority 1).
- **Future moves** (`futuremove` slot condition, `notActive` in `trySpreadMoveHit`) — no Future
  Sight/Doom Desire in the pool (but `flags.futuremove` checks are harmless).
- Team preview, timers, chat, `inactive`, `forceRandomChance`, `debug` log lines (debugMode off),
  `instafaint` paths of `spreadDamage`, `resetRNG`.

---------------------------------------------------------------------------------------------------

## 11. Gotchas / open questions for implementers

1. **Hidden PRNG in targeting**: every `getTarget`/`getRandomTarget` for spread, `allies`, `foeSide`,
   `randomNormal` moves draws `random(#aliveFoes)` — at queue resolution, at every
   `getActionSpeed` (turn start + re-sort after each action), and again in `runMove`. Missing any of
   these desyncs immediately. Verified in traces.
2. `sample()` and `randomChance()` always draw (length-1 arrays, 100% accuracy, `randomChance(1,1)`
   for crit stage 3+).
3. `eachEvent('Update')` inside the hit loop (twice per hitting move) shuffles speed ties → draws.
   The architecture rule "never skip an eachEvent" is load-bearing here.
4. `[spread] ` with an empty slot list (trailing space) when every target was protected/immune.
5. Multiaccuracy (hits ≥ 2) uses f64 accuracy; `ModifyAccuracy`'s chained modifier is dropped
   whenever the f64 isn't an exact non-negative integer (runEvent rule). Reproduce with f64.
6. `useMoveInner` re-runs `getRandomTarget` twice if the singleEvent `ModifyMove` changes
   `move.target` (Expanding Force/Tera Starstorm) — two draws.
7. Dragon Darts' `damage` array hole (`[undefined, d2]`) and the missing `-hitcount`.
8. Fainted (hp 0, unprocessed) targets still receive secondary rolls, `DamagingHit` (Static etc.
   roll) and `AfterMoveSecondary`; they are filtered only by `hp` checks inside handlers.
9. Recoil `-damage` comes after the target's `|faint|` (faintMessages runs before recoil);
   Life Orb recoil comes later still (AfterMoveSecondarySelf, after `AfterMoveSecondary`).
10. Substitute drain uses `ceil`; normal drain uses `round`; heal amounts `round`; residual damage
    floors (`clampIntRange`); boosts from move data are applied in object-key order.
11. OPEN: handler-order tie-breaking inside `runEvent` (`ModifyDamage`, `BasePower`, `ModifyAtk`, …)
    when two handlers tie on priority/speed/subOrder (e.g. two Pokémon with equal `speed`
    contributing modifiers) — draws via `shuffle`, and the resulting chain-rounding differs by order.
    The event-system spec must define this; this doc assumes it.
12. OPEN: `Pokemon.speed` used by `eachEvent`/handler sorting is the cached value from
    `updateSpeed()` (not recomputed mid-move). Confirm the cache refresh points in the turn-loop spec.
13. OPEN: the exact set of `TryHit`/`TryMove`/`ModifyMove` handlers reachable in this format is a
    data question for the abilities/items/moves specs; this doc only fixes where they fire.

