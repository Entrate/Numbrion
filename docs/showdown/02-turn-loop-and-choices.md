# 02 — Battle lifecycle, turn loop, requests and choices (Gen 9 Random Doubles)

Oracle: `~/src/pokemon-showdown` at commit `7332b60`. All `file:line` references are
into that checkout, in the TypeScript sources (`sim/`, `data/`, `config/`). Line
numbers are of the TS file, not `dist/`.

Scope: format `gen9randomdoublesbattle` (`config/formats.ts:165-172`):

```ts
{ name: "[Gen 9] Random Doubles Battle", mod: 'gen9', gameType: 'doubles', team: 'random',
  bestOfDefault: true,
  ruleset: ['PotD', 'Obtainable', 'Species Clause', 'HP Percentage Mod', 'Cancel Mod',
            'Illusion Level Mod', 'Sleep Clause Mod'] }
```

This document covers: construction, `setPlayer`, `start`, the `start` action, slots and
positions, requests (exact JSON), choice parsing and validation, `commitChoices`, the
`BattleQueue`, `runAction` for every action type, `turnLoop`, `endTurn`, fainting,
win/tie, and forced switches. Move execution (`runMove`/`useMove`), damage, the event
system internals (`runEvent`, handler collection, `resolvePriority`) and protocol
message formatting are covered by other documents. When this doc says
"`runEvent(X)` (may draw)", it means the handler list is ordered with `speedSort`,
which calls `prng.shuffle` when handlers tie (see the events doc). Never skip such
a call, even if no handler does anything.

Conventions:

* `B` = the Battle, `S` = a Side, `P` = a Pokemon.
* PRNG primitives (`sim/prng.ts`). Each one consumes exactly one `rng.next()` per call
  to `random`:
  * `random()`: float in [0, 1).
  * `random(n)`: `floor(next * n / 2^32)`.
  * `random(m, n)`: `floor(next * (n - m) / 2^32) + m`. Note `if (from) from = Math.floor(from)`,
    so `random(0, n)` behaves like `random(n)`.
  * `randomChance(a, b)`: `random(b) < a`. One draw.
  * `sample(arr)`: `arr[random(arr.length)]`. **This draws even when `arr.length === 1`.**
    It throws on an empty array.
  * `shuffle(arr, start, end)`: Fisher-Yates from the front:
    `while (start < end - 1) { j = random(start, end); swap(arr[start], arr[j]) if j != start; start++ }`.
    That is `end - start - 1` draws.
* All draws in this document use `B.prng`. The random-team generator uses its own
  separate PRNG instance (see 2.2), so team generation never consumes battle PRNG.
* `|t:|<unix seconds>` log lines are wall-clock and non-deterministic. The differential
  harness must strip or normalize them. Where they are emitted is documented below,
  because their position is still part of the log structure.
* Debug mode is off for this format (`format.debug` is falsy). Run the oracle without
  `debug: true` so no `|debug|` lines are emitted. `B.debug(...)` calls are no-ops.

---------------------------------------------------------------------------------------

## 1. Format facts that affect the turn loop

Computed rule table (`Dex.formats.getRuleTable(format)`), in iteration order:

```
potd, obtainable, obtainablemoves, obtainableabilities, obtainableformes, evlimit,
obtainablemisc, -unreleased, -tag:unobtainable, -nonexistent, speciesclause,
hppercentagemod, cancelmod, illusionlevelmod, sleepclausemod
```

* `ruleTable.pickedTeamSize = null`, so there is no team preview. `onChooseTeam = null`.
  `maxTeamSize = 6`, `minTeamSize = 2` apply to the validator only. The battle never
  validates.
* `gameType = 'doubles'`, `activePerHalf = 2` (`sim/battle.ts:228-230`), `format.playerCount = 2`.
* Rules with `onBegin` (called in `start()` in rule-table order, `sim/battle.ts:1947-1952`):
  * `potd` (`data/rulesets.ts:483-491`): `if (global.Config?.potd) add('rule', "Pokemon of the Day: " + species.name)`.
    In a bare `dist/` sim, `global.Config` is undefined, so this does nothing. The oracle
    harness must not define `Config.potd`. PotD otherwise only affects random team
    generation (`data/random-battles/gen9/teams.ts:1766-1867`), which we bypass by
    supplying explicit teams. **Engine: treat PotD as a no-op.**
  * `speciesclause` → `|rule|Species Clause: Limit one of each Pokémon`
  * `hppercentagemod` → `|rule|HP Percentage Mod: HP is shown in percentages`; sets `B.reportPercentages = true`
  * `cancelmod` → sets `B.supportCancel = true`. No log line.
  * `illusionlevelmod` → `|rule|Illusion Level Mod: Illusion disguises the Pokémon's true level`
  * `sleepclausemod` → `|rule|Sleep Clause Mod: Limit one foe put to sleep`
  * `obtainable*`, `evlimit` have no `onBegin` (validator only).
* Rules that are added as **pseudo-weather** in the constructor (`sim/battle.ts:297-310`):
  every rule-table key without a `+*-!` prefix whose Format object has any `on*` key
  other than `onBegin, onTeamPreview, onBattleStart, onValidateRule, onValidateTeam,
  onChangeSet, onValidateSet`. For this format that is **only `sleepclausemod`** (it
  has `onSetStatus`). So `field.pseudoWeather = { sleepclausemod: {id, source: null,
  sourceSlot: undefined, duration: undefined, effectOrder: 0} }` exists from
  construction onward. It emits no log line. It takes part in handler collection for
  every event as a field condition.
* No `endlessbattleclause`. The hard turn limit (tie when turn > 1000) and its warnings
  still apply (section 9.2).
* `B.reportExactHP = false`, `B.supportCancel = true` (after `onBegin`),
  `B.reportPercentages = true` (after `onBegin`).
* `bestOfDefault` is a server/room feature. Ignored.
* `dex.data.Scripts` for gen9 is `{gen: 9}` (`data/scripts.ts`). There are no
  battle/side/pokemon/queue/actions/field overrides. `format.battle/side/pokemon/queue/actions/field`
  are all absent. `dex.currentMod === 'base'`.

---------------------------------------------------------------------------------------

## 2. Construction

### 2.1 `new Battle(options)` (`sim/battle.ts:191-316`)

Order of side effects (only what matters for us):

```ts
log = []
add('t:', floor(Date.now()/1000))                 // log line 1: |t:|<ts>
format = Dex.formats.get(formatid, true); dex = Dex.forFormat(format); gen = 9
ruleTable = dex.formats.getRuleTable(format)
// (Scripts functions copied onto battle: none for gen9)
debugMode = format.debug || !!options.debug       // false
forceRandomChance = null                         // only in debug mode
strictChoices = !!options.strictChoices          // false: invalid choices do not throw
formatData = initEffectState({id: format.id})    // effectOrder 0 (no target)
gameType = 'doubles'
field = new Field(this)                          // weather/terrain states: initEffectState({id:''}) -> effectOrder 0
sides = [null, null]
activePerHalf = 2
prng = options.prng || new PRNG(options.seed || undefined)   // no seed => random sodium seed
prngSeed = prng.startingSeed
rated = options.rated || !!options.rated
reportExactHP = !!format.debug (false); reportPercentages = false; supportCancel = false
queue = new BattleQueue(this); actions = new BattleActions(this); faintQueue = []
inputLog = []; messageLog = []; sentLogPos = 0; sentEnd = false
requestState = ''; turn = 0; midTurn = false; started = false; ended = false
effect = {id: ''}; effectState = initEffectState({id: ''}); event = {id: ''}; events = null; eventDepth = 0
activeMove = activePokemon = activeTarget = null
lastMove = null; lastMoveLine = -1; lastSuccessfulMoveThisTurn = null; lastDamage = 0
effectOrder = 0; quickClawRoll = false
speedOrder = [0, 1, 2, 3]                        // for i < activePerHalf*2
teamGenerator = null; hints = new Set()
send = options.send || noop
inputLog.push(`>start ` + JSON.stringify({formatid, seed: prngSeed, rated?}))   // after optional >version lines
add('gametype', 'doubles')                       // |gametype|doubles
for rule of ruleTable.keys(): maybe field.addPseudoWeather(rule)   // only sleepclausemod, see section 1
for side of ['p1','p2','p3','p4']: if options[side] setPlayer(side, options[side])
```

`Field.addPseudoWeather('sleepclausemod')` (`sim/field.ts:186-216`): `source` stays null
because `battle.event.target` is undefined. It creates the state with
`initEffectState({id, source: null, sourceSlot: undefined, duration: undefined})`
(effectOrder 0, since there is no target). It calls `singleEvent('FieldStart')`
(no handler, returns true) and then `runEvent('PseudoWeatherChange', null, null, status)`
(no handlers, empty list, no draw).

`initEffectState(obj, effectOrder?)` (`sim/battle.ts:3320-3330`): if `effectOrder` is
given, use it. Otherwise, if `obj.id && obj.target && (target is not a Pokemon ||
target.isActive)`, set `obj.effectOrder = B.effectOrder++`. Otherwise set it to 0.
**`B.effectOrder` is a global counter that must be ported exactly.** It is consumed by
ability/item states on switch-in (2.7) and by every side/slot/volatile/status state
created on an active target.

### 2.2 `setPlayer(slot, options)` (`sim/battle.ts:3224-3258`) and team input

```ts
slotNum = parseInt(slot[1]) - 1
if (!sides[slotNum]) {
  team = getTeam(options)                                  // 2.3
  side = new Side(options.name || `Player ${slotNum+1}`, this, slotNum, team)   // 2.4, may DRAW (gender)
  if (options.avatar) side.avatar = `${options.avatar}`
  sides[slotNum] = side
} else { ...edit name/avatar; throw if options.team... }    // not used
if (options.team && typeof options.team !== 'string') options.team = Teams.pack(options.team)
   // packs the *mutated* set objects (Pokemon ctor normalizes name/level/evs/ivs)
inputLog.push(`>player ${slot} ` + JSON.stringify(options))
add('player', side.id, side.name, side.avatar, options.rating || '')   // |player|p1|Name|avatar|rating
if (sides.every(Boolean) && !started) start()
```

`getTeam(options)` (`sim/battle.ts:3163-3180`):
* If `options.team` is a string, `Teams.unpack(team)`. If it is an array, use it as is
  (by reference, it gets mutated).
* Otherwise, random generation: `options.seed ||= PRNG.generateSeed()`, then
  `Teams.getGenerator(format, options.seed)` (`sim/teams.ts:628-649`, resolves to
  `data/random-battles/gen9/teams.ts`, which owns its own `PRNG.get(seed)`) and
  `.getTeam(options)`. **This never touches `B.prng`.** For differential fuzzing,
  always pass explicit teams. Team generation is a separate, out-of-scope concern.

The battle performs **no team validation**. The only checks are: an unknown species
throws (`sim/pokemon.ts:322-324`), an empty move list throws (`:348-350`), unknown
moves are silently dropped (`:352` `if (!move.id) continue`), more than 24 Pokemon are
ignored (`sim/side.ts:303`), and level/EV/IV/happiness values are clamped.

### 2.3 Packed team format (`sim/teams.ts:120-345`)

Pokemon are separated by `]`. Fields are separated by `|`:

```
NAME|SPECIES|ITEM|ABILITY|MOVES|NATURE|EVS|GENDER|IVS|SHINY|LEVEL|HAPPINESS,HPTYPE,POKEBALL,GIGANTAMAX,DYNAMAXLEVEL,TERATYPE
```

`pack` (`:120-205`):
* name: `set.name || set.species`.
* species: `packName(species)`, or `''` if `packName(name) === packName(species)`. Here
  `packName` strips everything except `[A-Za-z0-9]`.
* item, ability: `packName`. moves: `packName` each, joined with `,`. nature: raw.
* evs: `hp,atk,def,spa,spd,spe` with `''` for 0. If all six are empty, the field is `''`.
* gender: raw or `''`.
* ivs: `''` for 31 or undefined. If all six are empty, the field is `''`.
* shiny: `S` or `''`. level: `''` if 100 or unset. happiness: `''` if undefined or 255.
* The trailing misc group `,HPTYPE,POKEBALL,G,DYNLEVEL,TERATYPE` is appended to the
  happiness field only if any of `pokeball/hpType/gigantamax/dynamaxLevel!=10/teraType`
  is set.

`unpack` (`:215-345`): accepts a JSON array string (it packs it first). Then, per
Pokemon (up to 24), with any missing `|` returning `null`:
* `species = unpackName(field, Dex.species) || name`.
* `item = unpackName(field, Dex.items)`.
* `ability`: if the field is one of `'' '0' '1' 'H' 'S'`, it becomes
  `species.abilities[field || '0'] || (field === '' ? '' : '!!!ERROR!!!')`.
  Otherwise `unpackName(field, Dex.abilities)`.
* `moves = field.split(',', 24).map(unpackName(_, Dex.moves))`.
* `nature = unpackName(field, Dex.natures)`.
* `evs`: only set if the field is non-empty: `Number(x) || 0` each.
* `gender`: only set if non-empty.
* `ivs`: only set if non-empty: `x === '' ? 31 : Number(x) || 0`.
* `shiny = true` if non-empty. `level = parseInt` if non-empty.
* misc: if present, `happiness = misc[0] ? Number : 255`, `hpType = misc[1] || ''`,
  `pokeball = unpackName(misc[2]||'', Dex.items)`, `gigantamax = !!misc[3]`,
  `dynamaxLevel = misc[4] ? Number : 10`, `teraType = misc[5]`.
* `unpackName(name, table)` returns `table.get(name).name` if it exists. Otherwise it
  returns a respaced guess.

The engine can take sets in its own binary form. What matters is that the resulting
`PokemonSet` fields equal what Showdown sees after `unpack` and after the Pokemon
constructor normalizes them.

### 2.4 `new Side(name, battle, n, team)` (`sim/side.ts:229-286`)

```ts
id = ['p1','p2','p3','p4'][n]; n = n; name; avatar = ''
team = team; pokemon = []
for set of team: addPokemon(set)       // new Pokemon(set, this); P.position = pokemon.length; push
active = [null, null]                  // doubles
pokemonLeft = pokemon.length
faintedLastTurn = faintedThisTurn = null; totalFainted = 0
zMoveUsed = false; dynamaxUsed = (gen !== 8)   // true
sideConditions = {}; slotConditions = [{}, {}]
activeRequest = null
choice = {cantUndo:false, error:'', actions:[], forcedSwitchesLeft:0, forcedPassesLeft:0,
          switchIns:new Set(), zMove:false, mega:false, ultra:false, dynamax:false, terastallize:false}
lastMove = lastEnemyMove = null; lastSelectedMove = 'nomove'; lastSelectedMoveSlot = 0  // gen1 only
foe = null; allySide = null            // foe set in start()
```

Pokemon are constructed in team order. For p1 this happens during `setPlayer('p1')`,
and for p2 during `setPlayer('p2')`, both before `start()`.

### 2.5 `new Pokemon(set, side)` (`sim/pokemon.ts:309-506`)

Exact order (only state that matters):

```ts
side, battle; m = {}
baseSpecies = dex.species.get(set.species || set.name)   // throw if !exists
set = set; species = baseSpecies
if (set.name === set.species || !set.name) set.name = baseSpecies.baseSpecies   // e.g. "Toxtricity-Low-Key" -> "Toxtricity"
speciesState = initEffectState({id: species.id})          // effectOrder 0
name = set.name.substr(0, 20); fullname = `${side.id}: ${name}`
set.level = clampIntRange(set.adjustLevel || set.level || 100, 1, 9999); level = set.level
gender = {M:'M',F:'F',N:'N'}[set.gender] || species.gender || B.sample(['M','F'])   // *** PRNG DRAW if set has no valid gender and species is dual-gendered ***
if (gender === 'N') gender = ''
happiness = typeof set.happiness === 'number' ? clamp(0,255) : 255
pokeball = toID(set.pokeball) || 'pokeball'
dynamaxLevel = number ? clamp(0,10) : 10; gigantamax = set.gigantamax || false
baseMoveSlots = []; moveSlots = []; ppUps = []
for moveid of set.moves:
  move = dex.moves.get(moveid); if (!move.id) continue
  if (move.id === 'hiddenpower' && move.type !== 'Normal') { set.hpType ||= move.type; move = dex.moves.get('hiddenpower') }
  ppUps_i = (move.noPPBoosts || move.id === 'trumpcard') ? 0 : 3
  basePP = calculatePP(move, ppUps_i)                     // noPPBoosts ? move.pp : move.pp * (5+ppUps)/5  -> pp*8/5
  baseMoveSlots.push({move: move.name, id: move.id, pp: basePP, maxpp: basePP, target: move.target,
                      disabled: false, disabledSource: '', used: false})
  ppUps.push(ppUps_i)
position = 0
details = getUpdatedDetails()      // "<species.name>[, L<level> if != 100][, <gender> if != ''][, shiny]"
                                   // species.name 'Greninja-Bond'/'Rockruff-Dusk' -> baseSpecies
status = ''; statusState = initEffectState({}); volatiles = {}; showCure = undefined
set.evs ||= {all 0}; set.ivs ||= {all 31}
for stat in {hp,atk,def,spe,spa,spd}: if (!set.evs[stat]) set.evs[stat] = 0;
                                      if (!set.ivs[stat] && set.ivs[stat] !== 0) set.ivs[stat] = 31
evs clamped to [0,255]; ivs clamped to [0,31]
hpType = set.hpType || dex.getHiddenPower(ivs).type; hpPower = ...; baseHpType/baseHpPower = same
storedStats = {atk:0,...}; boosts = {atk..evasion: 0}
baseAbility = ability = toID(set.ability); abilityState = initEffectState({id: ability, target: this})  // effectOrder 0 (not active)
item = toID(set.item); itemState = initEffectState({id: item, target: this})                       // effectOrder 0
lastItem = ''; usedItemThisTurn = false; ateBerry = false; itemKnockedOff = false
trapped = maybeTrapped = maybeDisabled = maybeLocked = false
illusion = null; transformed = false; fainted = false; faintQueued = false; subFainted = null
formeRegression = false
types = baseTypes = baseSpecies.types; addedType = ''; knownType = true; apparentType = types.join('/')
teraType = set.teraType || types[0]
switchFlag = false; forceSwitchFlag = false; skipBeforeSwitchOutEventFlag = false
draggedIn = null; newlySwitched = false; beingCalledBack = false
lastMove = null; lastMoveEncore = null; lastMoveUsed = null; moveThisTurn = ''
statsRaisedThisTurn = statsLoweredThisTurn = false; hurtThisTurn = null; lastDamage = 0
attackedBy = []; timesAttacked = 0
isActive = false; activeTurns = 0; activeMoveActions = 0; previouslySwitchedIn = 0
truantTurn = bondTriggered = heroMessageDisplayed = swordBoost = shieldBoost = syrupTriggered = false
stellarBoostedTypes = []; isStarted = false; duringMove = false
weighthg = 1; speed = 0
canMegaEvo = actions.canMegaEvo(this)          // null for every gen-9 random set (no mega stones / Rayquaza rule)
canUltraBurst = actions.canUltraBurst(this)    // null
canGigantamax = baseSpecies.canGigantamax || null
canTerastallize = actions.canTerastallize(this)   // (item.zMove || canMegaEvo || gen!=9) ? null : teraType
maxhp = baseMaxhp = hp = 0
clearVolatile()                                // sim/pokemon.ts:1508, includes setSpecies(baseSpecies)
hp = maxhp
```

`clearVolatile(includeSwitchFlags = true)` (`sim/pokemon.ts:1508-1560`) resets boosts,
`moveSlots = baseMoveSlots.slice()` (a **shallow** copy, so the slot objects are shared
with `baseMoveSlots`), transformed, ability, hpType, and restores
`canTerastallize === false → teraType`. It removes linked volatiles, sets
`volatiles = {}`, and resets switch flags if asked. It resets lastMove, lastMoveEncore,
lastMoveUsed, moveThisTurn, moveLastTurnResult, moveThisTurnResult, lastDamage,
attackedBy, hurtThisTurn. It sets `newlySwitched = true`, `beingCalledBack = false`,
`volatileStaleness = undefined`, deletes `abilityState.started` and `itemState.started`,
and calls `setSpecies(baseSpecies)`.

`setSpecies(raw)` (`sim/pokemon.ts:1387-1421`):
`species = runEvent('ModifySpecies', this, null, source, raw)`. The handler list is
empty at construction because the Pokemon is not active, and the field/battle have
no such handler. Then `setType(species.types, true)`, `apparentType = raw.types.join('/')`,
`addedType`, `knownType = true`, `weighthg`, and `stats = spreadModify(species.baseStats, set)`.
If `species.maxHP` is set, `stats.hp = maxHP`. If `!maxhp`, set `baseMaxhp = maxhp = hp = stats.hp`.
Then `baseStoredStats = stats` (unless transform), copy into `storedStats`, and
`speed = storedStats.spe`.

Stat formula (`sim/battle.ts:2350-2376`, `tr(x) = x >>> 0`, `tr(x,16) = (x>>>0) % 65536`):

```
hp    = tr(tr(2*base + iv + tr(ev/4) + 100) * level / 100 + 10)
other = tr(tr(2*base + iv + tr(ev/4)) * level / 100 + 5)
nature plus  : other = tr(tr(other*110, 16) / 100)
nature minus : other = tr(tr(other*90, 16) / 100)
```

Iterate stats in `hp, atk, def, spa, spd, spe` order (`for in baseStats`). Integer
floor division is exactly equivalent here. An unknown or empty nature is neutral.

### 2.6 `start()` (`sim/battle.ts:1909-1973`)

```ts
started = true
sides[1].foe = sides[0]; sides[0].foe = sides[1]
add('gen', 9)                                  // |gen|9
add('tier', format.name)                       // |tier|[Gen 9] Random Doubles Battle
if (rated) { if (rated === 'Rated battle') rated = true; add('rated', typeof rated === 'string' ? rated : '') }
format.onBegin?.()                             // none
for rule of ruleTable.keys(): if no '+*-!' prefix: dex.formats.get(rule).onBegin?.()   // see section 1
if (sides.some(s => !s.pokemon[0])) throw
if (debugMode) checkEVBalance()                // off
// customRules: none
runPickTeam()                                  // no onTeamPreview, pickedTeamSize null -> nothing
queue.addChoice({choice: 'start'})             // order 2, speed 1, no draw
midTurn = true
if (!requestState) turnLoop()
```

### 2.7 The `start` action and initial switch-ins

`turnLoop()` (section 8.1) emits `|` and `|t:|...`. Since `midTurn` is already true, it
does not add `beforeTurn`/`residual`. It shifts the `start` action and runs it
(`sim/battle.ts:2674-2707`):

```ts
for side of sides: { if (side.pokemonLeft) side.pokemonLeft = side.pokemon.length; add('teamsize', side.id, side.pokemon.length) }
add('start')
for P of getAllPokemon():   // p1 team order then p2 team order
  singleEvent('BattleStart', dex.conditions.getByID(P.species.id), P.speciesState, P)
  // only 'zacian' / 'zamazenta' conditions have onBattleStart (data/conditions.ts:880-940):
  // with Rusted Sword/Shield they setSpecies to Crowned, rewrite baseSpecies/details/ability,
  // and replace Iron Head with Behemoth Blade/Bash in baseMoveSlots (no log line).
format.onBattleStart?.(); rules' onBattleStart?.()      // none here
for side of sides:
  for i in 0..side.active.length:                         // i = 0, 1
    if (!side.pokemonLeft) { side.active[i] = side.pokemon[i]; fainted = true; hp = 0 }   // forfeited before start; ignore
    else actions.switchIn(side.pokemon[i], i)             // p1a, p1b, p2a, p2b in that order
midTurn = true
// then the common runAction post-processing (8.3)
```

`switchIn(P, pos)` with no old active (section 8.4) sets `isActive`, `side.active[pos]`,
fresh `abilityState`/`itemState` (each `initEffectState` with an active target, so
`B.effectOrder++` twice per Pokemon: p1a gets 0/1, p1b 2/3, p2a 4/5, p2b 6/7). It runs
`runEvent('BeforeSwitchIn', P)` (may draw), logs `|switch|...`, and then calls
`queue.insertChoice({choice:'runSwitch', pokemon: P})`.
**`insertChoice` draws `random(first, last+1)` when the new runSwitch ties in speed
with an already queued runSwitch** (7.5). So speed-tied leads consume PRNG here.

Post-processing of the `start` action: no phazing, no faint, and the queue is not empty
(it holds the runSwitches). `start` is excluded from the gen≥5 `Update`/`EmergencyExit`
block, there are no switches, and there is no re-sort because peek is `runSwitch`.
Then `runSwitch` (one action that absorbs all 4, 8.4) runs. Its post-processing does
`eachEvent('Update')` (may draw on speed ties) and
`runEvent('EmergencyExit', firstSwitcher, ..., originalHP)`. Then the queue is empty,
so `endTurn()` starts turn 1.

Observed log skeleton (timestamps elided):

```
|t:|...
|gametype|doubles
|player|p1|A||
|player|p2|B||
|gen|9
|tier|[Gen 9] Random Doubles Battle
|rule|Species Clause: Limit one of each Pokémon
|rule|HP Percentage Mod: HP is shown in percentages
|rule|Illusion Level Mod: Illusion disguises the Pokémon's true level
|rule|Sleep Clause Mod: Limit one foe put to sleep
|
|t:|...
|teamsize|p1|6
|teamsize|p2|6
|start
|split|p1
|switch|p1a: Ho-Oh|Ho-Oh, L70|264/264
|switch|p1a: Ho-Oh|Ho-Oh, L70|100/100
... (p1b, p2a, p2b the same way)
... (SwitchIn effects: Intimidate, weather, terrain, ...)
|turn|1
```

PRNG at startup, in order: (a) gender samples during `setPlayer` (p1 then p2, team
order), only for sets without a valid gender; (b) `BeforeSwitchIn` handler ties;
(c) `insertChoice` tie draws for runSwitch; (d) `runSwitch` `speedSort(allActive)`;
(e) `fieldEvent('SwitchIn')` handler `speedSort` and whatever the handlers draw;
(f) `eachEvent('Update')`; (g) `endTurn` events (section 9). With 4 distinct speeds
and no tied handlers, startup draws nothing (verified).

---------------------------------------------------------------------------------------

## 3. Slots and positions in doubles

* `side.active: [Pokemon, Pokemon]`. Slot `a` is position 0 and slot `b` is position 1.
  After the start action the entries are never null. **A fainted Pokemon stays in
  `side.active[i]`** (with `fainted = true`, `isActive = false`, `hp = 0`) until it is
  replaced, or forever if there is no replacement.
* `side.pokemon` is the party in **current order**. `P.position` is its index in
  `side.pokemon`, and positions `0..1` are the active slots. `switchIn` swaps
  party order: the incoming Pokemon takes index `pos`, and the outgoing one takes the
  incoming Pokemon's old index (`sim/battle-actions.ts:121-133`). `switch N` choices
  and request `side.pokemon[]` use this current order. Example: p1 = [Pikachu,
  Snorlax, Gengar]. Pikachu U-turns into `switch 3`, giving [Gengar, Snorlax, Pikachu].
* `getSlot()` (`sim/pokemon.ts:520-524`): `side.id + 'abcdef'[position]` → `p1a p1b p2a p2b`.
* `getFieldPositionValue()` (`:526-529`): `side.n + 2*position` → p1a=0, p2a=1, p1b=2, p2b=3.
* Relative locations (`getLocOf`, `:784-789`): `loc(target) = target.position + 1`,
  negated if the target is on the viewer's side. From p1: p1a=-1, p1b=-2, p2a=+1, p2b=+2.
* `getAtLoc(loc)` (`:770-778`): `loc < 0 ? ownSide.active[-loc-1] : foeSide.active[loc-1]`.
  `getAtLoc(0)` returns `foe.active[-1]`, which is `undefined`.
* Adjacency (`isAdjacent`, `:741-746`): with `activePerHalf <= 2`, two Pokemon are
  adjacent iff `this !== other` and neither is `fainted`. In doubles all foes are
  adjacent, and so is the ally.
* `side.allies()` = active entries that are non-null and have `hp > 0` (`sim/side.ts:391-397`).
  `side.foes()` = `foe.allies()`. `P.adjacentFoes()` = `side.foes()` in doubles.
  These filter on `hp`, not `fainted`, so a Pokemon at 0 HP that is still in the
  faint queue is already excluded.
* `getAllActive(includeFainted=false)` (`sim/battle.ts:1369-1379`): p1 slots then p2
  slots, skipping null and (unless asked) fainted.
* `getAtSlot('p2b')` (`:1615-1621`) is used by slot-tracking effects.
* `swapPosition(P, newPos, attrs)` (`:1592-1613`), used by Ally Switch
  (`data/moves.ts:330`): returns false if `newPos !== 1` and the target is null or
  fainted. Otherwise it logs `|swap|<P>|<newPos>|<attrs>`, swaps
  `side.pokemon`/`side.active` entries and positions, then
  `runEvent('Swap', target, P)` and `runEvent('Swap', P, target)`. Queued actions keep
  their `pokemon` reference and their relative `targetLoc`, so targets now refer to the
  new occupants.

`validTargetLoc(targetLoc, source, targetType)` (`sim/battle.ts:2403-2435`), specialized
for doubles (`numSlots = 2`, `sourceLoc = -(source.position+1)`):

```
if targetLoc == 0: return true
if |targetLoc| > 2: return false
isSelf = (sourceLoc == targetLoc); isFoe = targetLoc > 0
isAdjacent = targetLoc > 0 ? true            // |-(3-t) - sourceLoc| <= 1 holds for every combo in doubles
                           : |targetLoc - sourceLoc| == 1   // i.e. the ally slot
switch targetType:
  'randomNormal','scripted','normal': return isAdjacent           // either foe or the ally (never self)
  'adjacentAlly':        return isAdjacent && !isFoe              // ally only
  'adjacentAllyOrSelf':  return (isAdjacent && !isFoe) || isSelf  // ally or self
  'adjacentFoe':         return isAdjacent && isFoe               // either foe
  'any':                 return !isSelf                           // foes or ally
  default:               return false                             // self, all*, foeSide, allySide, allyTeam, ...
```

Validity does not depend on whether the Pokemon at that location is fainted.

Choosable target types (`CHOOSABLE_TARGETS`, `sim/battle-actions.ts:3`, `targetTypeChoices`
`:1544`): `normal, any, adjacentAlly, adjacentAllyOrSelf, adjacentFoe`.

---------------------------------------------------------------------------------------

## 4. Requests

### 4.1 State variables

* `B.requestState: '' | 'move' | 'switch' | 'teampreview'` (only `''`, `move` and `switch` occur here).
* `S.activeRequest: ChoiceRequest | null`. `S.requestState` is a getter
  (`sim/side.ts:295-300`): `''` if there is no request or it is a `wait` request,
  `'switch'` if `forceSwitch`, otherwise `'move'`.
* `B.midTurn`: true from `start()` until the end of `turnLoop` after `endTurn`. It is
  false while a `move` request is pending (between turns) and true while a mid-turn
  `switch` request is pending. It decides whether `turnLoop` adds
  `beforeTurn`/`residual` (8.1). It also stays true across end-of-turn faint
  replacement requests, because those happen before `endTurn`.

### 4.2 `makeRequest(type?)` (`sim/battle.ts:1381-1412`)

```ts
if (type) { requestState = type; for side: side.clearChoice() }   // clearChoice reads B.requestState (5.7)
else type = requestState
for side: side.activeRequest = null
// (teampreview branch ignored)
requests = getRequests(type)
for i: sides[i].activeRequest = requests[i]
sentRequests = false
if (sides.every(s => s.isChoiceDone())) throw "Choices are done immediately after a request"
```

Note that the final `isChoiceDone()` calls run `getChoiceIndex()`, which **auto-pushes
`pass` actions** for fainted/commanding slots (move request) or slots without
`switchFlag` (switch request) into `S.choice.actions` (5.2).

`clearRequest()` (`:1414-1420`): `requestState = ''`, then for each side
`activeRequest = null` and `clearChoice()`.

### 4.3 `getRequests(type)` (`sim/battle.ts:1422-1469`)

```ts
requests = [null, null]
switch (type) {
case 'switch':
  for i, side: if (!side.pokemonLeft) continue
    switchTable = side.active.map(P => !!P?.switchFlag)
    if (switchTable.some(Boolean)) requests[i] = { forceSwitch: switchTable, side: side.getRequestData() }
  break
default /* 'move' */:
  for i, side: if (!side.pokemonLeft) continue
    activeData = side.active.map(P => P?.getMoveRequestData())    // ALL slots, fainted included; mutates P (4.4)
    requests[i] = { active: activeData, side: side.getRequestData() }
    // side.allySide: multi only
}
multipleRequestsExist = requests.filter(Boolean).length >= 2
for i: if (requests[i]) { if (!supportCancel || !multipleRequestsExist) requests[i].noCancel = true }
       else requests[i] = { wait: true, side: sides[i].getRequestData() }
```

`supportCancel` is true, so `noCancel: true` appears only when exactly one side
has a real request. That happens with a mid-turn U-turn, or with end-of-turn
replacements on one side only. `wait` requests never carry `noCancel`.

**JSON key order** is insertion order:
* move request: `{"active":[...],"side":{...}}` plus an optional trailing `"noCancel":true`.
  An `"update":true` key is appended later if the request is re-emitted after an
  "Unavailable choice" (5.8).
* switch request: `{"forceSwitch":[bool,bool],"side":{...}}` plus an optional `"noCancel":true`.
* wait request: `{"wait":true,"side":{...}}`.

The request is sent as `sideupdate` `p1\n|request|<JSON>` by `Side.emitRequest`
(`sim/side.ts:521-525`) from `B.sendUpdates()` (`sim/battle.ts:3265-3305`), and only if
the log has grown since the last send. Requests are not part of the battle log.

### 4.4 `Pokemon.getMoveRequestData()` (`sim/pokemon.ts:1083-1151`). Note that it **mutates** state.

```ts
lockedMove = getLockedMove()            // priorityEvent('LockMove', this) -> id | null  (sorted with compareRedirectOrder: NO draw)
hardLocked = !!lockedMove
if (lockedMove) this.trapped = true     // *** mutation ***
else lockedMove = this.maybeLocked ? null : getSemiLockedMove(true)    // priorityEvent('SemiLockMove'); null in gen 9
isLastActive = this.isLastActive()      // isActive && no unfainted active at a higher position on own side
canSwitchIn = B.canSwitch(side) > 0     // any unfainted Pokemon at party index >= 2
moves = getMoves(lockedMove, isLastActive)
if (!moves.length) { moves = [{move:'Struggle', id:'struggle', target:'randomNormal', disabled:false}]; lockedMove = 'struggle' }
data = { moves }
if (hardLocked || !isLastActive) {
  maybeDisabled = false; maybeLocked = false; maybeTrapped = false      // *** mutations ***
  if (hardLocked || canSwitchIn) { if (this.trapped) data.trapped = true }   // trapped may be true or 'hidden'
} else {
  maybeLocked = maybeLocked || maybeDisabled
  if (maybeDisabled) data.maybeDisabled = true
  if (maybeLocked) data.maybeLocked = true
  if (canSwitchIn) { if (trapped === true) data.trapped = true; else if (maybeTrapped) data.maybeTrapped = true }
}
if (!lockedMove) {
  // canMegaEvo/X/Y, canUltraBurst, canZMove, canDynamax/maxMoves: never set in gen 9 random doubles
  if (canTerastallize) data.canTerastallize = canTerastallize   // the tera type string
}
return data
```

`getMoves(lockedMove, restrictData)` (`sim/pokemon.ts:964-1043`):
* `lockedMove === 'recharge'` returns `[{move:'Recharge', id:'recharge'}]`.
* Another locked move returns `[{move: slot.move, id: slot.id}]` for the matching slot,
  or `[{move: dex name, id}]` if there is none.
* Otherwise, for each `moveSlot` in order:
  `{move: name, id, pp, maxpp, target, disabled}`. Here `target` is `moveSlot.target`,
  except `curse` becomes `'self'` unless the user has the Ghost type, `pollenpuff`
  becomes `'adjacentFoe'` while Heal Blocked, and `terastarstorm` becomes
  `'allAdjacentFoes'` for Terapagos-Stellar. `disabled = moveSlot.disabled`, or `true`
  when `pp <= 0`. The value `'hidden'` becomes `!restrictData`. If **no** move is
  enabled, `getMoves` returns `[]`. That turns into the Struggle entry above, and the
  normal entries are not sent at all.
* (`hiddenpower`, `return`, `frustration` name decorations are gen-9-unreachable.)

Key order in a move entry: `move, id, pp, maxpp, target, disabled`. Locked and
recharge entries have only `move, id`. `disabledSource` appears only after a request
update (5.8). Key order in `data`: `moves`, then
`trapped` or `maybeDisabled, maybeLocked, trapped|maybeTrapped`, then `canTerastallize`.

**Fainted actives still get an entry.** Example: p2a fainted with no replacement shows
`{"moves":[...full list...],"canTerastallize":"Normal"}`. `isLastActive` is false
because the Pokemon is not active, and LockMove has no handlers for an inactive
Pokemon.

### 4.5 `Side.getRequestData()` / `Pokemon.getSwitchRequestData()` (`sim/side.ts:357-367`, `sim/pokemon.ts:1153-1190`)

```ts
{ name: side.name, id: side.id, pokemon: side.pokemon.map(P => P.getSwitchRequestData()) }   // current party order
P entry (key order):
{ ident: fullname,                       // "p1: Name" (never the slot form)
  details: P.details,                    // NOT the illusion / tera-decorated one
  condition: getHealth().secret,         // "hp/maxhp[ status]" or "0 fnt"
  active: P.position < side.active.length,   // true for a fainted Pokemon still in a slot
  stats: {atk, def, spa, spd, spe} from baseStoredStats,
  moves: P.moves (current moveSlots ids),
  baseAbility, item, pokeball,
  ability,                               // gen > 6
  commanding: !!volatiles.commanding && !fainted,
  reviving: isActive && !!side.slotConditions[position].revivalblessing,
  teraType, terastallized: terastallized || '' }
```

### 4.6 Example (verified)

```json
{"active":[{"moves":[{"move":"Brave Bird","id":"bravebird","pp":24,"maxpp":24,"target":"any","disabled":false},
 {"move":"Tailwind","id":"tailwind","pp":24,"maxpp":24,"target":"allySide","disabled":false}, ...],
 "canTerastallize":"Ground"}, {...}],
 "side":{"name":"A","id":"p1","pokemon":[{"ident":"p1: Ho-Oh","details":"Ho-Oh, L70","condition":"264/264",
 "active":true,"stats":{"atk":223,"def":167,"spa":195,"spd":257,"spe":167},
 "moves":["bravebird","tailwind","recover","sacredfire"],"baseAbility":"regenerator","item":"sitrusberry",
 "pokeball":"pokeball","ability":"regenerator","commanding":false,"reviving":false,"teraType":"Ground",
 "terastallized":""}, ...]}}
```

Mid-turn U-turn: `{"forceSwitch":[true,false],"side":{...},"noCancel":true}` for the
U-turner's side, and `{"wait":true,"side":{...}}` for the other side.

Locked Outrage: `{"moves":[{"move":"Outrage","id":"outrage"}],"trapped":true}`.

---------------------------------------------------------------------------------------

## 5. Choices

### 5.1 `Battle.choose(sideid, input)` (`sim/battle.ts:2962-2981`)

```ts
side = getSide(sideid)
if (!side.choose(input)) { if (!side.choice.error) side.emitChoiceError(`Unknown error for choice: ${input}. ...`); return false }
if (!side.isChoiceDone()) { side.emitChoiceError(`Incomplete choice: ${input} - missing other pokemon`); return false }
if (allChoicesDone()) commitChoices()
return true
```

`allChoicesDone()` (`:3058-3067`): counts the sides for which `isChoiceDone()` is true.
(It would set `cantUndo` if `!supportCancel`, which does not apply here.) It returns
`count >= sides.length`. A side with a `wait` request is always done.

`makeChoices(...inputs)` (`:2983-2994`) is a test helper. With inputs it calls
`sides[i].choose(input)` for each truthy input (not `B.choose`), then
`commitChoices()` unconditionally. With no inputs, it calls `side.autoChoose()` for
every side and then `commitChoices()`.

BattleStream input (`sim/battle-stream.ts:76-100`): `>start {json}` constructs the
battle, `>player p1 {json}` calls `setPlayer`, `>p1 <choice>` calls `B.choose`, and
`>p1 undo` calls `undoChoice`. After every write, `battle.sendUpdates()` runs.

### 5.2 `Side.choose(input)` (`sim/side.ts:1185-1301`)

```ts
if (!requestState) return emitChoiceError(B.ended ? "Can't do anything: The game is over" : "Can't do anything: It's not your turn")
if (choice.cantUndo) return emitChoiceError("Can't undo: A trapping/disabling effect would cause undo to leak information")
clearChoice()                                                    // every call starts from scratch
choiceStrings = input.startsWith('team ') ? [input] : input.split(',')
if (choiceStrings.length > active.length /*2*/) return emitChoiceError(`Can't make choices: You sent choices for ${n} Pokémon, but this is a doubles game!`)
for choiceString of choiceStrings:
  [choiceType, data] = splitFirst(choiceString.trim(), ' '); data = data.trim()   // splitFirst: no space -> [str, '']
  if (choiceType === 'testfight') { choiceType = 'move'; data = 'testfight' }
  switch (choiceType):
  case 'move':
    targetLoc = undefined; event = ''
    loop:
      if (/\s(?:-|\+)?[1-3]$/.test(data) && toID(data) !== 'conversion2') {
        if (targetLoc !== undefined) return error   // "Conflicting arguments for \"move\": <original>"
        targetLoc = parseInt(data.slice(-2)); data = data.slice(0, -2).trim()
      } else if data.endsWith(' mega')        -> event 'mega',        strip 5
      else if data.endsWith(' megax')         -> 'megax', strip 6
      else if data.endsWith(' megay')         -> 'megay', strip 6
      else if data.endsWith(' zmove')         -> 'zmove', strip 6
      else if data.endsWith(' ultra')         -> 'ultra', strip 6
      else if data.endsWith(' dynamax')       -> 'dynamax', strip 8
      else if data.endsWith(' gigantamax')    -> 'dynamax', strip 11
      else if data.endsWith(' max')           -> 'dynamax', strip 4
      else if data.endsWith(' terastal')      -> 'terastallize', strip 9
      else if data.endsWith(' terastallize')  -> 'terastallize', strip 13
      else break
      // each event branch: if (event) return error (conflicting); NOTE no trim after the event strips
    if (!chooseMove(data, targetLoc, event)) return false; break
  case 'switch': if (!chooseSwitch(data)) return false; break
  case 'shift':  if (data) return emitChoiceError(`Unrecognized data after "shift": ${data}`); if (!chooseShift()) return false; break
  case 'team':   if (!chooseTeam(data)) return false; break
  case 'pass': case 'skip': if (data) return emitChoiceError(`Unrecognized data after "pass": ${data}`); if (!choosePass()) return false; break
  case 'auto': case 'default': if (!autoChoose()) return false; break
  default: emitChoiceError(`Unrecognized choice: ${choiceString}`); break   // NOTE: does NOT return; continues parsing
return !choice.error
```

Notes:
* A target token is ` 1`, ` 2`, ` 3`, ` -1`..` -3` or ` +1`..` +3` at the end, matched
  repeatedly from the right. `0` is not a target token (`move tackle 0` turns into
  `tackle0`, which matches no move). `3` and `-3` parse but fail `validTargetLoc`.
* Unrecognized choice types do not abort. `"foo, move 1 1"` reports the error and still
  runs `chooseMove` for index 0. The side's `choice.actions` can then be partially
  filled while `choice.error` is set. `isChoiceDone()` does **not** look at `error`.
  If a later `B.choose` by the other side completes `allChoicesDone()`, this
  partial choice is committed. Example: slot 0 is fainted and the input is
  `"move 1 1, foo"`. It auto-passes slot 0 and fills slot 1, so it counts as done.
  Port this literally if invalid inputs are fuzzed.
* `choice.cantUndo` is only ever set by `maybeTrapped` switches, `maybeLocked`
  locked/struggle moves, and singles-only `maybeDisabled`, so it is rare.

### 5.3 Auto-pass: `getChoiceIndex(isPass?)` and `choosePass()` (`sim/side.ts:1303-1357`)

```ts
getChoiceIndex(isPass):
  index = choice.actions.length
  if (!isPass) switch (requestState) {
    case 'move':   while (index < active.length && (active[index].fainted || active[index].volatiles.commanding)) { choosePass(); index++ }
    case 'switch': while (index < active.length && !active[index].switchFlag) { choosePass(); index++ }
  }
  return index

choosePass():
  index = getChoiceIndex(true); if (index >= active.length) return false
  P = active[index]
  switch (requestState) {
    case 'switch': if (P.switchFlag) { if (!choice.forcedPassesLeft) return emitChoiceError(`Can't pass: You need to switch in a Pokémon to replace ${P.name}`); choice.forcedPassesLeft-- } break
    case 'move':   if (!P.fainted && !P.volatiles.commanding) return emitChoiceError(`Can't pass: Your ${P.name} must make a move (or switch)`); break
    default: return emitChoiceError("Can't pass: Not a move or switch request")
  }
  choice.actions.push({choice: 'pass'})
  return true
```

### 5.4 `chooseMove(moveText?, targetLoc = 0, event = '')` (`sim/side.ts:552-849`). Checks run in this exact order:

```ts
if (requestState !== 'move') return err(`Can't move: You need a ${requestState} response`)
index = getChoiceIndex()                                   // auto-passes fainted/commanding slots
if (index >= active.length) return err("Can't move: You sent more choices than unfainted Pokémon.")
autoChoose = !moveText; P = active[index]
request = P.getMoveRequestData()                           // *** called again: same mutations as 4.4 ***
moveSlot = undefined; moveid = ''; targetType = ''
if (autoChoose) moveText = 1
if (number or /^[0-9]+$/):                                 // 1-based index into request.moves (may be [locked] or [Struggle])
  i = Number(moveText) - 1
  if (i < 0 || i >= request.moves.length || !request.moves[i]) return err(`Can't move: Your ${P.name} doesn't have a move ${i+1}`)
  moveSlot = i; moveid = request.moves[i].id; targetType = request.moves[i].target   // undefined for locked/recharge entries
else:
  moveid = toID(moveText); if (moveid.startsWith('hiddenpower')) moveid = 'hiddenpower'
  first i with request.moves[i].id === moveid: moveSlot = i; targetType = move.target || 'normal'   // locked entries -> 'normal'
  (maxMoves / canZMove name lookups: never present in gen 9)
  if (!targetType && moveid !== 'testfight') return err(`Can't move: Your ${P.name} doesn't have a move matching ${moveid}`)
moves = P.getMoves()                                       // no lockedMove, restrictData=false: 'hidden' => disabled
if (autoChoose):
  for i, m of request.moves: if (m.disabled) continue; if (i < moves.length && m.id === moves[i].id && moves[i].disabled) continue
                              moveid = m.id; moveSlot = i; targetType = m.target; break
move = dex.moves.get(moveid)
// Z: zMove = event==='zmove' ? actions.getZMove(move,P) : undefined   -> always undefined in gen 9
if (event === 'zmove' && !zMove) return err(`Can't move: ${P.name} can't use ${move.name} as a Z-move`)
// Max: maxMove = (event==='dynamax' || P.volatiles.dynamax) ? actions.getMaxMove(move,P) : undefined
//      NOTE: getMaxMove returns a real Max move in gen 9, so targetType becomes the max move's target
if (event === 'dynamax' && !maxMove) return err(`... can't use ${move.name} as a Max Move`)
if (maxMove) targetType = dex.moves.get(maxMove).target
// target validation
if (autoChoose || moveid === 'testfight') targetLoc = 0
else if (targetTypeChoices(targetType)) {
  if (!targetLoc && active.length >= 2) return err(`Can't move: ${move.name} needs a target`)
  if (!B.validTargetLoc(targetLoc, P, targetType)) return err(`Can't move: Invalid target for ${move.name}`)
} else if (targetLoc) return err(`Can't move: You can't choose a target for ${move.name}`)
// locked / struggle
lockedMove = P.getLockedMove() || P.getSemiLockedMove()
if (lockedMove) {
  loc = P.lastMoveTargetLoc || 0; if (P.volatiles[lockedMove]?.targetLoc) loc = P.volatiles[lockedMove].targetLoc
  if (P.maybeLocked) choice.cantUndo = true
  choice.actions.push({choice:'move', pokemon:P, targetLoc: loc, moveid: lockedMove}); return true   // event (tera) silently dropped
}
// (gen 1 branch skipped)
if (!moves.length) { if (P.maybeLocked) cantUndo = true; push({choice:'move', pokemon:P, moveid:'struggle'}); return true }  // targetLoc undefined; tera dropped
if (moveid === 'testfight') {...}                          // client helper, ignored
else if (maxMove) { if (P.maxMoveDisabled(move)) return err(`... ${maxMove.name} is disabled`) }   // skips normal disabled check
else if (!zMove) {
  isEnabled = false; disabledSource = ''
  for m of moves: if (m.id !== moveid) continue; if (!m.disabled) { isEnabled = true; break } else if (m.disabledSource) disabledSource = m.disabledSource
  // NB: getMoves() entries never carry disabledSource, so disabledSource stays ''
  if (!isEnabled) { if (autoChoose) throw; return err(`Can't move: ${P.name}'s ${move.name} is disabled`, {pokemon:P, update: req => {
      updated = updateDisabledRequest(P, req)
      for m of req.moves: if (m.id === moveid) { if (!m.disabled) { m.disabled = true; updated = true }
                                                 if (m.disabledSource !== disabledSource) { m.disabledSource = disabledSource; updated = true }; break }
      return updated }}) }
}
// mega/megax/megay/ultra: canMegaEvo etc. are null -> "can't mega evolve" / "can't mega evolve X/Y" / "can't ultra burst" errors
dynamax = event === 'dynamax'; canDynamax = activeRequest.active[active.indexOf(P)].canDynamax   // undefined
if (dynamax && (choice.dynamax || !canDynamax)) {
  if (P.volatiles.dynamax) dynamax = false
  else return err("Can't move: Dynamaxing doesn't outside of Gen 8.")   // gen !== 8
}
terastallize = event === 'terastallize'
if (terastallize && !P.canTerastallize) return err(`Can't move: ${P.name} can't Terastallize.`)
if (terastallize && choice.terastallize) return err("Can't move: You can only Terastallize once per battle.")   // also blocks the 2nd active this turn
(gen !== 9 check)
choice.actions.push({choice:'move', pokemon:P, targetLoc, moveid, moveSlot,
                     mega:false, megax:false, megay:false, zmove:undefined, maxMove:undefined,
                     terastallize: terastallize ? P.teraType : undefined})
(cantUndo for maybeDisabled: singles only)
if (terastallize) choice.terastallize = true
return true
```

Gotchas (verified):
* Locked move chosen **by name** (`move outrage`): the request entry has no `target`, so
  `targetType = 'normal'`. That gives "Outrage needs a target" in doubles.
  `move 1` or `move outrage 1` work, and the locked branch then replaces `targetLoc`
  with the stored location.
* Spread, self and field moves (`allAdjacent`, `allAdjacentFoes`, `self`, `all`,
  `foeSide`, `allySide`, `randomNormal`, `scripted`, ...) must be sent **without** a
  target. Choosable moves must be sent **with** one in doubles.
* A target location is valid even if the Pokemon there is fainted (for example,
  targeting the fainted ally slot with a `normal` move).

### 5.5 `chooseSwitch(slotText?)` (`sim/side.ts:915-1023`)

```ts
if (requestState !== 'move' && requestState !== 'switch') return err(`Can't switch: You need a ${requestState} response`)
index = getChoiceIndex()                    // auto-pass (move: fainted/commanding; switch: slots without switchFlag)
if (index >= active.length) return err(requestState === 'switch'
     ? "Can't switch: You sent more switches than Pokémon that need to switch"
     : "Can't switch: You sent more choices than unfainted Pokémon")
P = active[index]
if (!slotText) {
  if (requestState !== 'switch') return err("Can't switch: You need to select a Pokémon to switch in")
  if (slotConditions[P.position].revivalblessing) { slot = 0; while (!pokemon[slot].fainted) slot++ }
  else { if (!choice.forcedSwitchesLeft) return choosePass()
         slot = active.length; while (choice.switchIns.has(slot) || pokemon[slot].fainted) slot++ }
} else slot = parseInt(slotText) - 1
if (isNaN(slot) || slot < 0) {               // name or species-id lookup
  slot = -1; for i, mon: if (slotText.toLowerCase() === mon.name.toLowerCase() || toID(slotText) === mon.species.id) { slot = i; break }
  if (slot < 0) return err(`Can't switch: You do not have a Pokémon named "${slotText}" to switch to`)
}
if (slot >= pokemon.length) return err(`Can't switch: You do not have a Pokémon in slot ${slot+1} to switch to`)
else if (slot < active.length && !slotConditions[P.position].revivalblessing) return err("Can't switch: You can't switch to an active Pokémon")
else if (choice.switchIns.has(slot)) return err(`Can't switch: The Pokémon in slot ${slot+1} can only switch in once`)
target = pokemon[slot]
if (slotConditions[P.position].revivalblessing) {
  if (!target.fainted) return err("Can't switch: You have to pass to a fainted Pokémon")
  choice.forcedSwitchesLeft = clampIntRange(forcedSwitchesLeft - 1, 0)
  P.switchFlag = false                       // *** mutation at choice time ***
  push({choice:'revivalblessing', pokemon:P, target}); return true
}
if (target.fainted) return err("Can't switch: You can't switch to a fainted Pokémon")
if (requestState === 'move') {
  if (P.trapped) return err("Can't switch: The active Pokémon is trapped", {pokemon:P, update: req => {
      updated = false; if (req.maybeTrapped) { delete req.maybeTrapped; updated = true }
      if (!req.trapped) { req.trapped = true; updated = true }; return updated }})
  else if (P.maybeTrapped) choice.cantUndo = true
} else /* switch */ { if (!choice.forcedSwitchesLeft) throw "Player somehow switched too many Pokemon"; choice.forcedSwitchesLeft-- }
choice.switchIns.add(slot)
push({choice: requestState === 'switch' ? 'instaswitch' : 'switch', pokemon:P, target})
return true
```

The **party index** `slot` is 0-based into the current `side.pokemon` order. Switching
to a Pokemon that is already queued to come in by an earlier, still-unexecuted action
is not checked here. The executed `switchIn` refuses it (8.4).

### 5.6 `chooseShift` / `chooseTeam` (error paths only)

* `shift`: if `index >= active.length`, "Can't shift: You do not have a Pokémon in
  slot N". Otherwise, if the request is not a move request, "...only shift during a
  move phase". Otherwise "Can't shift: You can only shift to the center in triples".
* `team ...`: "Can't choose for Team Preview: You're not in a Team Preview phase".

### 5.7 `autoChoose`, `isChoiceDone`, `clearChoice`, undo

```ts
autoChoose():  // sim/side.ts:1360-1378
  if (requestState === 'switch') { i = 0; while (!isChoiceDone()) { if (!chooseSwitch()) throw; if (++i > 10) throw } }
  else if (requestState === 'move') { i = 0; while (!isChoiceDone()) { if (!chooseMove()) throw; if (++i > 10) throw } }
  return true
  // move: first enabled move of each slot, targetLoc 0 (so resolveAction picks a random target, 7.2)
  // switch: lowest-index unfainted bench Pokemon not already chosen; extra flagged slots pass

isChoiceDone():  // :539-550
  if (!requestState) return true
  if (choice.forcedSwitchesLeft) return false
  getChoiceIndex()                           // auto-pass trailing slots
  return choice.actions.length >= active.length

clearChoice():  // :1118-1140
  forcedSwitches = forcedPasses = 0
  if (B.requestState === 'switch') {
    canSwitchOut = active.filter(P => P?.switchFlag).length
    canSwitchIn  = pokemon.slice(active.length).filter(P => P && !P.fainted).length
    forcedSwitches = min(canSwitchOut, canSwitchIn); forcedPasses = canSwitchOut - forcedSwitches
  }
  choice = {cantUndo:false, error:'', actions:[], forcedSwitchesLeft: forcedSwitches, forcedPassesLeft: forcedPasses,
            switchIns:new Set(), zMove:false, mega:false, ultra:false, dynamax:false, terastallize:false}
```

Example: both actives fainted and one bench Pokemon left. That gives
`forceSwitch:[true,true]`, `forcedSwitchesLeft=1`, `forcedPassesLeft=1`. Valid
inputs are `"switch 3, pass"` and `"pass, switch 3"`. `"switch 3"` alone is
"Incomplete choice". `"pass, pass"` gives "Can't pass: You need to switch in ...".
`"switch 3, switch 3"` gives "...can only switch in once".

`undoChoice(sideid)` (`sim/battle.ts:3030-3056`) with Cancel Mod: if the side has a
request and `!cantUndo`, re-run `updateDisabledRequest` for each chosen move,
`clearChoice()`, and re-emit the request if anything changed. This is not needed for
fuzzing unless `undo` inputs are generated.

### 5.8 Errors: `emitChoiceError(message, update?)` (`sim/side.ts:527-537`)

```ts
choice.error = message
updated = update ? updateRequestForPokemon(update.pokemon, update.update) : null   // req = activeRequest.active[P.position]; returns update(req) ?? true
type = `[${updated ? 'Unavailable' : 'Invalid'} choice]`
send sideupdate `${id}\n|error|${type} ${message}`
if (updated) emitRequest(activeRequest, true)      // sets activeRequest.update = true and re-sends the (mutated) request
if (strictChoices) throw
return false
```

`updateDisabledRequest(P, req)` (`sim/side.ts:851-904`) in doubles: if `P.maybeLocked`,
clear it and delete `req.maybeLocked`. If `P.maybeDisabled` (gen ≥ 4), clear it,
delete `req.maybeDisabled`, and mark every move with `getMoveData(id).disabled`
truthy as `disabled: true`. If every move is now disabled or is Struggle, clear
`canTerastallize` (`req.canTerastallize = undefined`, which drops the key from JSON).
It returns `updated`.

Choice errors **never touch the battle log or the PRNG**. They only produce
sideupdates. (The getMoveRequestData mutations in 5.4 do touch state.)

### 5.9 `getChoice()` for the input log (`sim/side.ts:323-351`)

`move <moveid>[ <+N|-N> if targetLoc != 0][ terastallize]`, `switch <target.position+1>`,
`pass`, joined with `, `. `commitChoices` pushes `>p1 <getChoice()>` for each side whose
`getChoice()` is non-empty (`sim/battle.ts:3009-3012`). Note `+1` rather than `1` for
foe targets.

---------------------------------------------------------------------------------------

## 6. `commitChoices()` (`sim/battle.ts:2996-3028`)

```ts
updateSpeed()                                   // for P of getAllActive(): P.speed = P.getActionSpeed()   (events may draw)
oldQueue = queue.list; queue.clear()            // non-empty only when resuming mid-turn
if (!allChoicesDone()) throw
for side: if (side.getChoice()) inputLog.push(`>${side.id} ${side.getChoice()}`)
for side of sides (p1, p2): side.commitChoices()  // = queue.addChoice(side.choice.actions): resolveAction each action
                                                  //   in slot order and push (7.2). DRAWS happen here, p1 slot a,
                                                  //   p1 slot b, p2 slot a, p2 slot b. Messages emitted here
                                                  //   (Quick Claw / Custap / Quick Draw) land BEFORE the turn's "|" line.
clearRequest()                                  // requestState = ''; activeRequest = null; clearChoice() on both sides
queue.sort()                                    // speedSort(list): sorts ONLY the new actions (DRAWS on ties)
queue.list.push(...oldQueue)                    // remaining mid-turn actions keep their order, after the new ones
requestState = ''; for side: activeRequest = null
turnLoop()
if (log.length - sentLogPos > 500) sendUpdates()   // affects only the LINE LIMIT guard, see section 13
```

The PRNG order for a normal turn start is therefore:
`updateSpeed` events, then resolve p1a, p1b, p2a, p2b, then `queue.sort` shuffles,
then `turnLoop` (`beforeTurn` insert, which never draws in practice, then the actions).

---------------------------------------------------------------------------------------

## 7. The BattleQueue (`sim/battle-queue.ts`)

### 7.1 Action records

All actions are plain objects in `queue.list: Action[]`. These fields matter:

| field | meaning |
|---|---|
| `choice` | `move, beforeTurnMove, priorityChargeMove, switch, instaswitch, revivalblessing, runSwitch, terastallize, beforeTurn, residual, start, pass, shift, event, team, megaEvo*, runDynamax` |
| `order` | lower first. Assigned by `resolveAction`. May be mutated (prioritizeAction sets 3, Quash sets 201) |
| `priority` | higher first. Move: base + modifiers + `fractionalPriority`. Others: unset (treated as 0) |
| `fractionalPriority` | move only, from `runEvent('FractionalPriority')` |
| `speed` | higher first. `P.getActionSpeed()` for actions with a pokemon, else 1 |
| `pokemon` | actor (for switch: the Pokemon leaving; for runSwitch: the one that came in) |
| `target` | switch target Pokemon |
| `targetLoc`, `originalTarget` | move targeting |
| `move` | ActiveMove (shared object between a move and its beforeTurnMove sub-action) |
| `moveid`, `terastallize`, `sourceEffect` | ... |
| `subOrder`, `effectOrder` | never set on actions (compare as 0) |

Order table (`resolveAction`, `:174-195`):

```
team 1, start 2, instaswitch 3, beforeTurn 4, beforeTurnMove 5, revivalblessing 6,
runSwitch 101, switch 103, megaEvo/X/Y 104, runDynamax 105, terastallize 106,
priorityChargeMove 107, shift 200, move/event 200 (default), residual 300
(any other choice throws "Unexpected orderless action")
```

### 7.2 `resolveAction(action, midTurn = false)` (`sim/battle-queue.ts:166-277`). This is the exact order of work.

```ts
if (action.choice === 'pass') return []
actions = [action]
if (!action.side && action.pokemon) action.side = action.pokemon.side
if (!action.move && action.moveid) action.move = dex.getActiveMove(action.moveid)     // fresh deep clone
if (!action.order) action.order = ORDERS[action.choice] ?? 200
if (!midTurn) {
  if (action.choice === 'move') {
    if (!maxMove && !zmove && action.move.beforeTurnCallback)          // Counter, Mirror Coat (gen 9)
      actions.unshift(...resolveAction({choice:'beforeTurnMove', pokemon, move: action.move, targetLoc: action.targetLoc}))
    (mega/megax/megay: never)
    if (action.terastallize && !action.pokemon.terastallized)
      actions.unshift(...resolveAction({choice:'terastallize', pokemon}))
    (runDynamax: never)
    if (!maxMove && !zmove && action.move.priorityChargeCallback)       // Focus Punch, Chilly Reception, Shell Trap, Beak Blast
      actions.unshift(...resolveAction({choice:'priorityChargeMove', pokemon, move: action.move}))   // NB: no targetLoc passed
    action.fractionalPriority = runEvent('FractionalPriority', pokemon, null, action.move, 0)   // relayVar 0 (see note)
  } else if (action.choice === 'switch' || action.choice === 'instaswitch') {
    if (typeof action.pokemon.switchFlag === 'string') action.sourceEffect = dex.moves.get(action.pokemon.switchFlag)   // e.g. U-turn
    action.pokemon.switchFlag = false
  }
}
if (action.move) {                                   // move, beforeTurnMove, priorityChargeMove
  action.move = dex.getActiveMove(action.move)       // same object if already an ActiveMove
  if (!action.targetLoc) {                           // 0 or undefined
    target = B.getRandomTarget(action.pokemon, action.move)          // MAY DRAW (7.4)
    if (target) action.targetLoc = action.pokemon.getLocOf(target)
  }
  action.originalTarget = action.pokemon.getAtLoc(action.targetLoc)
}
B.getActionSpeed(action)                             // MAY DRAW (7.3)
return actions                                       // e.g. [priorityChargeMove, terastallize, beforeTurnMove, move]
```

Draw order within one `move` resolve:
1. beforeTurnMove sub-resolve (its random target, then its speed).
2. terastallize sub-resolve (speed events).
3. priorityChargeMove sub-resolve (random target, since `targetLoc` is undefined, then speed).
4. `FractionalPriority` (handler ties, Quick Claw `randomChance(1,5)`, Quick Draw `randomChance(3,10)`).
5. Main random target.
6. `getActionSpeed` (getTarget random, ModifyPriority handler ties, speed stat events).

Note on `FractionalPriority`: the handler's first argument is the relayVar, which is `0`,
**not** the move's priority. So Quick Claw's `priority <= 0` test is always true for the
first handler, and Quick Claw rolls even for priority moves such as Protect
(verified). Quick Claw logs `|-activate|<P>|item: Quick Claw` at that moment, which is
before the turn's `|` / `|t:|` lines. Custap Berry is eaten at that moment too.

### 7.3 `getActionSpeed(action)` (`sim/battle.ts:2623-2667`)

```ts
if (action.choice === 'move') {
  move = action.move                               // (zmove / maxMove substitution: never)
  priority = dex.moves.get(move.id).priority       // base data priority, not the ActiveMove's
  target = getTarget(action.pokemon, action.move, action.targetLoc)                 // MAY DRAW (7.4), no originalTarget
  priority = singleEvent('ModifyPriority', move, null, action.pokemon, target, null, priority)   // e.g. Grassy Glide
  priority = runEvent('ModifyPriority', action.pokemon, target, move, priority)     // Prankster, Gale Wings, Triage...; may draw
  action.priority = priority + action.fractionalPriority
  action.move.priority = priority                  // gen > 5: mutates the ActiveMove
}
action.speed = action.pokemon ? action.pokemon.getActionSpeed() : 1
```

`Pokemon.getActionSpeed()` (`sim/pokemon.ts:641-649`):
`speed = getStat('spe', false, false)`. That applies boosts via
`runEvent('ModifyBoost')` and then `runEvent('ModifySpe')` (both may draw), and caps at
10000. If `field.pseudoWeather.trickroom` exists, `speed = 10000 - speed`. It returns
`trunc(speed, 13)` = `(speed >>> 0) % 8192`. It is computed even for fainted or
inactive Pokemon. Their own handlers are not collected because they are not
active, but field and battle handlers still are.

### 7.4 Target resolution draws: `getRandomTarget` and `getTarget` (`sim/battle.ts:2441-2526`)

```ts
getRandomTarget(P, move):
  if move.target in ['self','all','allySide','allyTeam','adjacentAllyOrSelf']: return P          // no draw
  if move.target === 'adjacentAlly': allies = P.adjacentAllies(); return allies.length ? sample(allies) : null   // draw iff ally alive
  // doubles (activePerHalf == 2):
  return P.side.randomFoe() || P.side.foe.active[0]     // randomFoe: foes = foe.allies() (hp>0); empty -> null (no draw); else sample(foes) (DRAW, even if 1)

getTarget(P, move, targetLoc, originalTarget?):
  move = dex.moves.get(move)
  tracksTarget = move.tracksTarget || P.hasAbility(['stalwart','propellertail'])
  if (tracksTarget && originalTarget?.isActive) return originalTarget       // getActionSpeed never passes originalTarget
  if (move.smartTarget) { cur = P.getAtLoc(targetLoc); return cur && !cur.fainted ? cur : getRandomTarget(P, move) }   // Dragon Darts
  selfLoc = P.getLocOf(P)
  if (['adjacentAlly','any','normal'].includes(move.target) && targetLoc === selfLoc &&
      !P.volatiles.twoturnmove && !P.volatiles.iceball && !P.volatiles.rollout)
    return move.flags.futuremove ? P : null
  if (move.target !== 'randomNormal' && validTargetLoc(targetLoc, P, move.target)) {
    T = P.getAtLoc(targetLoc)               // targetLoc 0 -> undefined
    if (T?.fainted) { if (T.isAlly(P)) return (move.target === 'adjacentAllyOrSelf') ? P : T }   // fainted foe falls through
    if (T && !T.fainted) return T
  }
  return getRandomTarget(P, move)
```

Consequences:

| move target | resolveAction (targetLoc 0) | getTarget in getActionSpeed |
|---|---|---|
| `normal/any/adjacentFoe` with a chosen live target | no draw | no draw |
| same, chosen target is a fainted **foe** | — | draw (randomFoe) |
| same, chosen target is a fainted **ally** | — | no draw (returns the fainted ally) |
| `normal` etc. via autoChoose (loc 0) | draw | (now has a loc) usually no draw |
| `allAdjacent`, `allAdjacentFoes`, `foeSide`, `scripted` | draw | `allAdjacent*`/`foeSide`: **draw every time** (validTargetLoc false). `scripted`: no draw if the target is alive |
| `randomNormal` (Outrage, Struggle, ...) | draw | **draw every time** |
| `self`, `all`, `allySide`, `allyTeam` | no draw | no draw (returns P) |
| `adjacentAllyOrSelf` | no draw | no draw |
| `adjacentAlly` (Helping Hand...) via loc 0 | draw if an ally is alive | draw if an ally is alive |

`getActionSpeed` runs at resolve time and again at **every gen-8+ re-sort** (8.3, step 9),
for every queued action that has a `pokemon`. That includes the queued actions of
fainted or benched Pokemon. So, for example, a pending Heat Wave consumes one
`sample` per re-sort.

### 7.5 Ordering: `comparePriority`, `speedSort` (`sim/battle.ts:408-467`)

```ts
comparePriority(a, b) =      // negative => a first
     (a.order || 4294967296) - (b.order || 4294967296)
  || (b.priority || 0) - (a.priority || 0)
  || (b.speed || 0) - (a.speed || 0)
  || (a.subOrder || 0) - (b.subOrder || 0)
  || (a.effectOrder || 0) - (b.effectOrder || 0)
  || 0

speedSort(list, cmp = comparePriority):          // selection sort with tie shuffles; port EXACTLY
  if (list.length < 2) return
  sorted = 0
  while (sorted + 1 < list.length) {
    nextIndexes = [sorted]
    for (i = sorted + 1; i < list.length; i++) {
      delta = cmp(list[nextIndexes[0]], list[i])
      if (delta < 0) continue
      if (delta > 0) nextIndexes = [i]
      if (delta === 0) nextIndexes.push(i)
    }
    for (i = 0; i < nextIndexes.length; i++) {
      index = nextIndexes[i]
      if (index !== sorted + i) swap(list[sorted + i], list[index])
    }
    if (nextIndexes.length > 1) prng.shuffle(list, sorted, sorted + nextIndexes.length)   // k-1 draws for k tied
    sorted += nextIndexes.length
  }
```

The swaps move non-minimal elements around, so the relative order of the remaining
elements (and therefore later shuffles) depends on this exact algorithm.

`eachEvent(eventid)` (`sim/battle.ts:469-480`): `actives = getAllActive()`, then
`speedSort(actives, (a,b) => b.speed - a.speed)` using the **cached** `P.speed`, then
`runEvent(eventid, P, null, effect)` for each Pokemon in order.

### 7.6 Queue API

```ts
shift()/peek()/push()/unshift()/clear()   // trivial; peek() = list[0]
addChoice(choices):                       // resolve each (midTurn=false) and PUSH at the end, no sorting
sort():                                   // B.speedSort(list)
insertChoice(choices, midTurn = false):   // sim/battle-queue.ts:369-403
  if array: for c: insertChoice(c)  (midTurn NOT forwarded); return
  if (choice.pokemon) choice.pokemon.updateSpeed()              // events, may draw
  actions = resolveAction(choice, midTurn)                       // may draw
  firstIndex = lastIndex = null
  for i, cur of list:
    c = comparePriority(actions[0], cur)
    if (c <= 0 && firstIndex === null) firstIndex = i
    if (c < 0) { lastIndex = i; break }
  if (firstIndex === null) list.push(...actions)
  else { if (lastIndex === null) lastIndex = list.length
         index = firstIndex === lastIndex ? firstIndex : B.random(firstIndex, lastIndex + 1)   // DRAW on tie range
         list.splice(index, 0, ...actions) }
  // The scan assumes `list` is sorted. It is not re-sorted. Port the scan literally.
prioritizeAction(action, sourceEffect?):  // remove action from list if present; action.sourceEffect = sourceEffect; action.order = 3; unshift
changeAction(P, action):                  // cancelAction(P); action.pokemon ||= P; insertChoice(action)   (unused by gen-9 data)
cancelAction(P):                          // remove ALL actions with action.pokemon === P; returns whether any were removed
cancelMove(P):                            // remove the first 'move' action of P
willMove(P):                              // P.fainted ? null : first 'move' action of P
willSwitch(P):                            // first 'switch'/'instaswitch' action of P
willAct():                                // first move/switch/instaswitch/shift action
```

Data-side queue manipulations reachable in gen 9: After You (`data/moves.ts:206`,
prioritizeAction), Quash (`:14466`, `order = 201`), Instruct (`:9663`,
`prioritizeAction(resolveAction({choice:'move', pokemon, moveid, targetLoc})[0])`, which
draws like any resolve), the Pledge combos (`:5396`, `:7595`, `:20546`, prioritizeAction
with sourceEffect), Shell Trap (`:16304`), Round (`:15515`), Commander
(`data/abilities.ts:629` cancelAction, and `commanding` onBeforeTurn
`data/conditions.ts:840`), Color Change curse glitch (`data/abilities.ts:576`), and many
`willMove`/`willAct` reads.

---------------------------------------------------------------------------------------

## 8. `turnLoop` and `runAction`

### 8.1 `turnLoop()` (`sim/battle.ts:2936-2955`)

```ts
add('')                                      // "|"
add('t:', floor(Date.now()/1000))            // "|t:|..." on EVERY call, including resumptions after a switch request
if (requestState) requestState = ''
if (!midTurn) {
  queue.insertChoice({choice: 'beforeTurn'}) // order 4; lands at index 0 in practice (no draw)
  queue.addChoice({choice: 'residual'})      // order 300, pushed at the end
  midTurn = true
}
while ((action = queue.shift())) {
  runAction(action)
  if (requestState || ended) return          // a switch request or the battle ended: suspend
}
endTurn()
midTurn = false
queue.clear()
```

### 8.2 `runAction(action)`: per-type part (`sim/battle.ts:2669-2823`)

```ts
pokemonOriginalHP = action.pokemon?.hp
residualPokemon = []
switch (action.choice) {
case 'start': ...section 2.7...; break
case 'move':
  if (!action.pokemon.isActive) return false          // EARLY RETURN: no post-processing at all
  if (action.pokemon.fainted) return false            // EARLY RETURN
  actions.runMove(action.move, action.pokemon, action.targetLoc,
                  {sourceEffect: action.sourceEffect, zMove: undefined, maxMove: undefined, originalTarget: action.originalTarget})
  break
case 'terastallize': actions.terastallize(action.pokemon); break        // 8.5. No isActive/fainted check
case 'beforeTurnMove':
  if (!isActive || fainted) return false              // EARLY RETURN
  target = getTarget(pokemon, move, targetLoc)        // may draw
  if (!target) return false                           // EARLY RETURN
  move.beforeTurnCallback.call(B, pokemon, target)    // e.g. Counter: addVolatile('counter')
  break
case 'priorityChargeMove':
  if (!isActive || fainted) return false              // EARLY RETURN
  move.priorityChargeCallback.call(B, pokemon)        // e.g. Focus Punch '-singleturn', Chilly Reception volatile
  break
case 'event': runEvent(action.event, action.pokemon); break     // unused in gen 9
case 'team': ...; return                                        // unused
case 'pass': return                                             // never queued (resolveAction drops pass)
case 'instaswitch':
case 'switch':
  if (action.choice === 'switch' && action.pokemon.status)
    singleEvent('CheckShow', dex.abilities.getByID('naturalcure'), null, action.pokemon)
  if (actions.switchIn(action.target, action.pokemon.position, action.sourceEffect) === 'pursuitfaint') {
    hint("A Pokemon can't switch between when it runs out of HP and when it faints"); break   // gen 5+; unreachable in gen 9
  }
  break
case 'revivalblessing':
  action.pokemon.side.pokemonLeft++
  if (action.target.position < side.active.length)    // the revived Pokemon is the fainted occupant of a slot
    queue.addChoice({choice:'instaswitch', pokemon: action.target, target: action.target})   // PUSHED AT THE END (not inserted)
  T = action.target; T.fainted = false; T.faintQueued = false; T.subFainted = false; T.status = ''
  T.hp = 1; T.sethp(T.maxhp / 2)
  add('-heal', T, T.getHealth, '[from] move: Revival Blessing')
  action.pokemon.side.removeSlotCondition(action.pokemon, 'revivalblessing')
  break
case 'runSwitch': actions.runSwitch(action.pokemon); break     // 8.4
case 'shift': ...; break                                       // triples only
case 'beforeTurn': eachEvent('BeforeTurn'); break              // speedSort actives (may draw); e.g. commanding cancels its actions
case 'residual':
  add('')                                                      // "|"
  clearActiveMove(true)
  updateSpeed()
  residualPokemon = getAllActive().map(P => [P, P.hp])         // p1a,p1b,p2a,p2b (unfainted), NOT speed-sorted
  fieldEvent('Residual')                                       // 8.6
  if (!ended) add('upkeep')                                    // "|upkeep"
  break
}
```

### 8.3 `runAction` post-processing (`sim/battle.ts:2825-2933`), for every action that did not return early

```ts
// 1. phazing
for side of sides: for P of side.active:
  if (P.forceSwitchFlag) { if (P.hp) actions.dragIn(P.side, P.position); P.forceSwitchFlag = false }   // dragIn draws sample (8.4)
// 2.
clearActiveMove()
// 3. fainting
faintMessages()                       // section 10.2
if (ended) return true
// 4. switching
if (!queue.peek()) checkFainted()     // queue EMPTY: mark fainted actives (status 'fnt', switchFlag = true). End of turn only
else if (queue.peek().choice === 'instaswitch') return false   // more replacements pending: skip the rest
// (gen-7 mega branch skipped)
// 5.
if (action.choice !== 'start') {      // gen >= 5
  eachEvent('Update')                 // speedSort actives (may draw) + Update handlers
  for [P, originalHP] of residualPokemon: runEvent('EmergencyExit', P, undefined, undefined, originalHP)   // only non-empty after 'residual'
}
// 6.
if (action.choice === 'runSwitch') runEvent('EmergencyExit', action.pokemon, undefined, undefined, pokemonOriginalHP)
// 7. which sides must switch
switches = sides.map(side => side.active.some(P => P && !!P.switchFlag))
for i, side:
  reviveSwitch = false
  if (switches[i] && !canSwitch(side)) {                 // no unfainted bench Pokemon
    for P of side.active {
      if (side.slotConditions[P.position].revivalblessing) { reviveSwitch = true; continue }
      P.switchFlag = false
    }
    if (!reviveSwitch) switches[i] = false
  } else if (switches[i]) {
    for P of side.active:
      if (P.hp && P.switchFlag && P.switchFlag !== 'revivalblessing' && !P.skipBeforeSwitchOutEventFlag) {
        runEvent('BeforeSwitchOut', P)
        P.skipBeforeSwitchOutEventFlag = true
        faintMessages(); if (ended) return true
        if (P.fainted) switches[i] = side.active.some(sp => sp && !!sp.switchFlag)
      }
  }
// 8.
for s of switches: if (s) { makeRequest('switch'); return true }   // requests for every side with a switchFlag
// (gen < 5 Update skipped)
// 9. gen-8+ dynamic speed re-sort
if (queue.peek()?.choice === 'move' /* || 'runDynamax' */) {
  updateSpeed()                                         // all unfainted actives (events)
  for a of queue.list: if (a.pokemon) getActionSpeed(a) // in list order; MAY DRAW (7.4); recomputes priority for moves
  queue.sort()                                          // speedSort the whole list (may draw)
}
return false
```

Early-return summary. Post-processing is **skipped entirely** for:
* `move`, `beforeTurnMove` and `priorityChargeMove` whose user is inactive or fainted;
* `beforeTurnMove` without a target;
* `pass`/`team`.

Post-processing stops after the faint step (no Update, no switch check, no re-sort)
when the next action is an `instaswitch`.

Consequences:
* A Pokemon fainting **mid-turn** is not replaced until the queue is empty, which in
  practice means after `residual`. Its remaining actions stay in the queue and still take
  part in re-sorts (speed recomputation and target draws). They are skipped when popped.
* After any non-early-return action, if the next action is a `move`, the whole remaining
  queue is re-speeded and re-sorted. So speed changes mid-turn (Tailwind, Trick Room,
  paralysis, Icy Wind, ...) reorder later moves, and `prioritizeAction`-ed actions
  (order 3) stay first.

### 8.4 Switching internals (`sim/battle-actions.ts:62-189`)

```ts
switchIn(P, pos, sourceEffect = null, isDrag = false):
  if (!P || P.isActive) { hint("A switch failed because the Pokémon trying to switch in is already in."); return false }
  side = P.side; oldActive = side.active[pos]
  if (oldActive?.hp) {                                         // switching out a living Pokemon
    oldActive.beingCalledBack = true
    switchCopyFlag = (sourceEffect && typeof sourceEffect.selfSwitch === 'string') ? sourceEffect.selfSwitch : false   // 'copyvolatile' (Baton Pass), 'shedtail'
    if (!oldActive.skipBeforeSwitchOutEventFlag && !isDrag) { runEvent('BeforeSwitchOut', oldActive); eachEvent('Update') }
    oldActive.skipBeforeSwitchOutEventFlag = false
    if (!runEvent('SwitchOut', oldActive)) return false         // never false in gen 9 data
    if (!oldActive.hp) return 'pursuitfaint'
    singleEvent('End', oldActive.getAbility(), oldActive.abilityState, oldActive)
    singleEvent('End', oldActive.getItem(), oldActive.itemState, oldActive)
    queue.cancelAction(oldActive)                               // its pending move/tera etc. are removed
    if (switchCopyFlag) P.copyVolatileFrom(oldActive, switchCopyFlag)
    oldActive.clearVolatile()
  }
  if (oldActive) {
    oldActive.isActive = false; isStarted = false; usedItemThisTurn = false; statsRaisedThisTurn = false; statsLoweredThisTurn = false
    oldActive.position = P.position
    if (oldActive.fainted) oldActive.status = ''               // undoes checkFainted's 'fnt'
    P.position = pos; side.pokemon[P.position] = P; side.pokemon[oldActive.position] = oldActive
  }
  P.isActive = true; side.active[pos] = P; P.activeTurns = 0; P.activeMoveActions = 0
  for ms of P.moveSlots: ms.used = false
  P.abilityState = initEffectState({id: P.ability, target: P})  // effectOrder = B.effectOrder++
  P.itemState    = initEffectState({id: P.item, target: P})     // effectOrder = B.effectOrder++
  runEvent('BeforeSwitchIn', P)                                 // Illusion sets P.illusion here
  add(isDrag ? 'drag' : 'switch', P, P.getFullDetails, ...(sourceEffect ? [`[from] ${sourceEffect}`] : []))   // e.g. "[from] U-turn"
  P.previouslySwitchedIn++
  if (isDrag) runSwitch(P)                                      // gen >= 5: immediately
  else queue.insertChoice({choice: 'runSwitch', pokemon: P})    // order 101; may draw on speed ties (7.6)
  return true

dragIn(side, pos):
  P = getRandomSwitchable(side)        // sample(unfainted Pokemon at party index >= 2)  -> DRAW (even if only one)
  if (!P || P.isActive) return false
  old = side.active[pos]; if (!old) throw; if (!old.hp) return false
  if (!runEvent('DragOut', old)) return false                   // Suction Cups, Ingrain, commanding...
  return switchIn(P, pos, null, true)

runSwitch(P):
  switchersIn = [P]
  while (queue.peek()?.choice === 'runSwitch') switchersIn.push(queue.shift().pokemon)   // absorbs all adjacent runSwitch actions
  allActive = getAllActive(true); speedSort(allActive)          // comparePriority on Pokemon => by .speed desc; ties DRAW
  speedOrder = allActive.map(a => a.getFieldPositionValue())    // used by resolvePriority for *SwitchIn handlers
  fieldEvent('SwitchIn', switchersIn)                           // events doc; abilities/items' onStart run here
  for poke of switchersIn: if (poke.hp) { poke.isStarted = true; poke.draggedIn = null }
  return true
```

The ordering this produces:
* **Turn-start switches** (order 103). Each `switch` action calls switchIn, which inserts
  a runSwitch (101). Since 101 sorts before the remaining 103s, the switch-in effects of
  the first switcher happen **before** the second switch. Each switch is fully
  processed before the next.
* **Replacements** (`instaswitch`, order 3). Every instaswitch runs first; their
  runSwitches (101) queue up behind the remaining instaswitches and are then absorbed
  into **one** `runSwitch` call, so the switch-ins are simultaneous. The order of the
  instaswitches themselves comes from the speed of the Pokemon **being replaced**
  (the fainted Pokemon's `getActionSpeed()`).
* **Drags** (Roar, Whirlwind, Dragon Tail, Circle Throw, Red Card) happen in the
  post-processing phazing step, with runSwitch immediately inside switchIn.

### 8.5 `terastallize` action (`sim/battle-actions.ts:1918-1955`)

Ogerpon with a non-{Fire, Grass, Rock, Water} tera type gives a side-only hint and
return. Ogerpon/Terapagos illusion breaks. Otherwise:
`add('-terastallize', P, teraType)`, `P.terastallized = teraType`, and
**`canTerastallize = null` for every Pokemon on the side**. Then `addedType = ''`,
`knownType = true`, `apparentType = type`. Then the Ogerpon forme change
(`formeChange(..., null, true)`), Terapagos-Terastal → Stellar, the Morpeko
formeRegression, and finally `runEvent('AfterTerastallization', P)`. Order 106 means
every tera of the turn happens after all switches and before any move, sorted by
speed among themselves.

### 8.6 `fieldEvent('Residual')` (`sim/battle.ts:488-575`), summary

The handler list (`getKey = 'duration'`) is built in this order:
field handlers (`onFieldResidual`); per side (p1, p2): side handlers
(`onSideResidual`); then per active (non-null, including fainted): pokemon handlers
(`onResidual`, status/volatiles/ability/item/species/slot conditions), the side's
`onResidual` with that Pokemon as holder, the field's `onResidual` with that Pokemon
as holder, and battle/format handlers. Then `speedSort(handlers)` (may draw), and for
each handler in order:
* skip it if the holder is a fainted Pokemon (unless it is a slot condition);
* if it is Residual with `end` and `state.duration`, then `duration--`, and at 0 call
  `end(...)` and `continue` (return if ended);
* skip stale states, i.e. effects removed by an earlier handler;
* `singleEvent(handlerEventid, ...)`;
* then `faintMessages()`, and return if `ended`.

Faints during residuals are therefore announced right after the handler that caused them.

---------------------------------------------------------------------------------------

## 9. End of turn: `endTurn()` (`sim/battle.ts:1627-1805`)

### 9.1 Pseudo-code (gen-9 doubles path)

```ts
turn++
lastSuccessfulMoveThisTurn = null
// dynamax ending: none in gen 9
trappedBySide = []; stalenessBySide = []
for side of sides (p1, p2):
  sideTrapped = true; sideStaleness = undefined
  for P of side.active:                          // BOTH slots, fainted ones INCLUDED until noted
    if (!P) continue
    P.moveThisTurn = ''; P.newlySwitched = false
    P.moveLastTurnResult = P.moveThisTurnResult; P.moveThisTurnResult = undefined
    if (turn !== 1) { P.usedItemThisTurn = false; P.statsRaisedThisTurn = false; P.statsLoweredThisTurn = false; P.hurtThisTurn = null }
    P.maybeDisabled = false; P.maybeLocked = false
    for ms of P.moveSlots: { ms.disabled = false; ms.disabledSource = '' }
    runEvent('DisableMove', P)                                        // may draw
    for ms of P.moveSlots:
      am = dex.getActiveMove(ms.id)
      singleEvent('DisableMove', am, null, P)                         // move-level onDisableMove (e.g. Belch, Stuff Cheeks)
      if (am.flags.cantusetwice && P.lastMove?.id === ms.id) P.disableMove(P.lastMove.id)   // Gigaton Hammer, Blood Moon
    if (P.getLastAttackedBy()) P.knownType = true                     // gen >= 7
    for (i = attackedBy.length-1; i >= 0; i--): a = attackedBy[i]
      if (a.source.isActive) a.thisTurn = false else attackedBy.splice(attackedBy.indexOf(a), 1)
    if (!P.terastallized) {                                           // gen >= 7
      seen = P.illusion || P; real = seen.getTypes(true).join('/')
      if (real !== seen.apparentType) {
        add('-start', P, 'typechange', real, '[silent]'); seen.apparentType = real
        if (P.addedType) add('-start', P, 'typeadd', P.addedType, '[silent]')
      }
    }
    P.trapped = P.maybeTrapped = false
    runEvent('TrapPokemon', P)                                        // Shadow Tag, Arena Trap, Magnet Pull, trapping volatiles; may draw
    if (!P.knownType || dex.getImmunity('trapped', P)) runEvent('MaybeTrapPokemon', P)   // getImmunity false for Ghost types
    for source of P.foes():                                           // foe actives with hp > 0, slot order
      species = (source.illusion || source).species
      for slot in species.abilities (key order '0','1','H','S'):
        name = species.abilities[slot]
        if (name === source.ability) continue
        // hackmons check: ruleTable has 'obtainableabilities' and format.team -> not hackmons
        if (slot === 'H' && species.unreleasedHidden) continue
        ability = dex.abilities.get(name)
        if (ruleTable.has('-ability:' + ability.id)) continue        // never here
        if (P.knownType && !dex.getImmunity('trapped', P)) continue
        singleEvent('FoeMaybeTrapPokemon', ability, {}, P, source)    // may set P.maybeTrapped
    if (P.fainted) continue                                           // ---- below: unfainted only ----
    sideTrapped = sideTrapped && P.trapped
    staleness = P.volatileStaleness || P.staleness; if (staleness) sideStaleness = sideStaleness === 'external' ? sideStaleness : staleness
    P.activeTurns++
  trappedBySide.push(sideTrapped); stalenessBySide.push(sideStaleness)
  side.faintedLastTurn = side.faintedThisTurn; side.faintedThisTurn = null
if (maybeTriggerEndlessBattleClause(trappedBySide, stalenessBySide)) return   // 9.2
// (triples centering skipped)
add('turn', turn)                                                     // "|turn|N"
// (multi dynamax, gen 2/3 Quick Claw skipped)
makeRequest('move')
```

For fainted actives, `runEvent(..., P)` collects **no Pokemon or side handlers** because
`P.isActive` is false. Only field/battle handlers are collected (`sim/battle.ts:1057`).
The sleep-clause pseudo-weather has none of these events. Any draw still comes only
from `speedSort` of the collected list.

### 9.2 Turn limit (`maybeTriggerEndlessBattleClause`, `sim/battle.ts:1807-1907`)

```ts
if (turn <= 100) return
if (turn > 1000) { add('message', "It is turn 1000. You have hit the turn limit!"); tie(); return true }
if ((turn >= 500 && turn % 100 === 0) || (turn >= 900 && turn % 10 === 0) || turn >= 990) {
  turnsLeft = 1000 - turn
  add('bigerror', `You will auto-tie if the battle doesn't end in ${turnsLeft === 1 ? '1 turn' : `${turnsLeft} turns`} (on turn 1000).`)
}
if (!ruleTable.has('endlessbattleclause')) return     // returns here for this format
```

The tie happens inside the `endTurn` that would begin turn 1001 (`turn` is already 1001).
No `|turn|` line is emitted.

---------------------------------------------------------------------------------------

## 10. Fainting, win, tie, battle end

### 10.1 `Pokemon.faint(source, effect)` (`sim/pokemon.ts:1581-1593`)

```ts
if (fainted || faintQueued) return 0
d = hp; hp = 0; switchFlag = false; faintQueued = true
B.faintQueue.push({target: this, source, effect})
return d
```

It is called from `Battle.damage`/`directDamage` when HP reaches 0, and from
self-KO effects (moves doc). It also clears a pending `switchFlag`, so a U-turner
that dies to Rocky Helmet does not get a switch request.

### 10.2 `faintMessages(lastFirst = false, forceCheck = false, checkWin = true)` (`sim/battle.ts:2539-2608`)

```ts
if (ended) return
length = faintQueue.length
if (!length) { if (forceCheck && this.checkWin()) return true; return false }
if (lastFirst) faintQueue.unshift(faintQueue.pop())            // move the last entry to the front
while (faintQueue.length) {
  faintQueueLeft = faintQueue.length
  faintData = faintQueue.shift(); P = faintData.target
  if (!P.fainted && runEvent('BeforeFaint', P, faintData.source, faintData.effect)) {
    add('faint', P)                                             // "|faint|p2a: Name"
    if (P.side.pokemonLeft) P.side.pokemonLeft--
    if (P.side.totalFainted < 100) P.side.totalFainted++
    runEvent('Faint', P, faintData.source, faintData.effect)
    singleEvent('End', P.getAbility(), P.abilityState, P)
    singleEvent('End', P.getItem(), P.itemState, P)
    if (P.formeRegression && !P.transformed) { P.baseSpecies = dex.species.get(P.set.species || P.set.name); P.baseAbility = toID(P.set.ability) }
    P.clearVolatile(false)                                      // keeps switchFlag/forceSwitchFlag
    P.fainted = true; P.illusion = null; P.isActive = false; P.isStarted = false
    delete P.terastallized
    if (P.formeRegression) { P.details = P.getUpdatedDetails(); add('detailschange', P, P.details, '[silent]'); P.updateMaxHp(); P.formeRegression = false }
    P.side.faintedThisTurn = P
    if (faintQueue.length >= faintQueueLeft) checkWin = true
  }
}
// (gen <= 3 queue clearing skipped)
if (checkWin && this.checkWin(faintData)) return true          // faintData = LAST shifted entry (even if it was skipped)
if (faintData && length) runEvent('AfterFaint', faintData.target, faintData.source, faintData.effect, length)
return false
```

Callers: every `runAction` post-processing pass, after each `fieldEvent` handler,
`BeforeSwitchOut` handling (8.3 step 7), `spreadDamage` instafaint
(`faintMessages(true)`, `sim/battle.ts:2187`), the multi-hit loop
(`sim/battle-actions.ts:969`, `checkWin = !pokemon.hp`), Dancer (`:336`), and the end
of `useMove` (`:347`, followed by a bare `checkWin()` at `:348`).

### 10.3 `checkFainted()` (`sim/battle.ts:2528-2537`)

`for side, for P of side.active: if (P.fainted) { P.status = 'fnt'; P.switchFlag = true }`.
It runs only when the queue is empty at post-processing time (end of turn, or the end
of a replacement batch). The `switches` step then clears these flags on sides with no
unfainted bench Pokemon (8.3 step 7).

### 10.4 `checkWin(faintData?)` (`sim/battle.ts:2610-2621`), `win`, `tie`

```ts
checkWin(faintData):
  if (sides.every(s => !s.pokemonLeft)) { win(faintData ? faintData.target.side : null); return true }   // gen > 4
  for side of sides (p1 then p2): if (!side.foePokemonLeft()) { win(side); return true }     // foePokemonLeft = foe.pokemonLeft
  return undefined

win(side):            // sim/battle.ts:1524-1547
  if (ended) return false
  side = string ? getSide : (valid Side ? side : null)
  winner = side ? side.name : ''
  add('')                                   // "|"
  side ? add('win', side.name) : add('tie') // "|win|Name" or "|tie"
  ended = true; requestState = ''; for s: s.activeRequest = null
  return true
tie() = win()
```

* If every side is out, the side of the **last processed faint entry** wins
  (gen > 4). With no `faintData` (a bare `checkWin()` or `forceCheck`), the result is a tie.
* The queue is not cleared on a win. `turnLoop` and `runAction` return because
  `ended` is set.
* After the battle ends, `B.choose` gives "Can't do anything: The game is over".
* `sendUpdates()` then emits `end` with JSON
  `{winner, seed, turns, p1, p2, p1team, p2team, score:[p1.pokemonLeft, p2.pokemonLeft], inputLog}`
  (`sim/battle.ts:3273-3303`). This is harness-only.

---------------------------------------------------------------------------------------

## 11. Forced switches mid-turn, by source

Mechanism: something sets `P.switchFlag` (truthy) or `P.forceSwitchFlag` during an
action. At the end of that action, post-processing (8.3) turns it into a `switch`
request (step 8) or a drag (step 1).

| source | flag | where set | notes |
|---|---|---|---|
| U-turn, Volt Switch, Flip Turn, Parting Shot, Teleport, Chilly Reception, Baton Pass (`'copyvolatile'`), Shed Tail (`'shedtail'`), Revival Blessing (`selfSwitch: true`) | `source.switchFlag = move.id` | `sim/battle-actions.ts:1300-1302` (only if the move "did something", `source.hp` and not `commanded`) | resolveAction of the chosen instaswitch converts the string into `sourceEffect` (log `[from] U-turn`) and copies volatiles for Baton Pass / Shed Tail |
| Eject Button | `target.switchFlag = true` | `data/items.ts:1680-1703` (onAfterMoveSecondary) | aborts if any active already has `switchFlag === true`. On successful use it sets `source.switchFlag = false` (cancels the attacker's U-turn) |
| Eject Pack | `switchFlag = true` via `onUse` | `data/items.ts:1705-1760` | triggers on AnySwitchIn / AnyAfterMove / Residual (order 29) after a stat drop |
| Emergency Exit, Wimp Out | `target.switchFlag = true` | `data/abilities.ts:1250-1262`, `:5496-5507` | **clears `switchFlag` on every active of both sides first**. Runs from `runEvent('EmergencyExit')` in move damage (`sim/battle-actions.ts:529,539,1007,1122,1383`), after residuals (8.3 step 5) and after runSwitch (step 6) |
| Roar, Whirlwind, Dragon Tail, Circle Throw (`forceSwitch`) | `target.forceSwitchFlag = true` | `sim/battle-actions.ts:1342-1358` | dragged in post-processing: `dragIn` = `sample` from the bench (draw), then switchIn(isDrag) then runSwitch immediately |
| Red Card | `source.forceSwitchFlag = true` | `data/items.ts:5146-5165` | as above |
| fainted active | `switchFlag = true` | `checkFainted` when the queue is empty | replacement request at end of turn |

Full mid-turn flow (verified with U-turn):
1. The move action's runAction returns `true` after `makeRequest('switch')`. Sides
   with a flagged active get `{forceSwitch: [...], side, noCancel?}`, and the other
   side gets `{wait: true, side}`. `turnLoop` returns, leaving `midTurn = true` and the
   remaining queue intact.
2. The player sends `switch N` (or `pass` where `forcedPassesLeft` allows it). This
   produces an `instaswitch` action. `B.choose` calls `commitChoices`, which saves
   `oldQueue`, resolves and sorts the new instaswitches, then appends `oldQueue`.
3. `turnLoop` emits `|` and `|t:|` again and does not add beforeTurn/residual. The
   instaswitch runs `switchIn` with `sourceEffect`. `BeforeSwitchOut` was already run
   during post-processing, so `skipBeforeSwitchOutEventFlag` skips it now and resets
   the flag. The pending actions of the outgoing Pokemon are canceled. runSwitch
   runs next, then the turn continues (with a gen-8 re-sort if a move is next).
4. End of turn: after `residual` the queue is empty, so `checkFainted` runs. Sides
   with replacements get `forceSwitch`. After the replacements run and the queue is
   empty again, `checkFainted` runs again (no new flags if nothing else fainted), and
   then `endTurn`.

Interaction to port literally: if Emergency Exit triggers in the post-residual step,
`checkFainted` has already set fainted Pokemon's `switchFlag`, and Emergency Exit's
handler then clears **all** actives' flags. So the first request is only for the
Emergency Exit Pokemon. Fainted slots are re-flagged by the `checkFainted` that runs
once the queue is empty again, which produces a second request.

Revival Blessing: the `revivalblessing` slot condition (duration 1) plus
`switchFlag = 'revivalblessing'`. The request shows `reviving: true` on that active.
`chooseSwitch` requires a **fainted** party member and may target a fainted Pokemon
still sitting in an active slot. The `revivalblessing` action (order 6) revives at
half HP. If the revived Pokemon occupies a slot, the action **pushes** an instaswitch
to the end of the queue. Whether that runs before or after other actions depends on
whether a gen-8 re-sort happens first (instaswitch order 3).

Commander (Tatsugiri + Dondozo): the `commanding` slot is auto-passed in choices, its
actions are canceled in `BeforeTurn`, and it is shown with `commanding: true` in
request data.

---------------------------------------------------------------------------------------

## 12. PRNG inventory for the lifecycle (checklist)

Things in this document that can draw, in the phase where they happen. The leaves
listed under events/moves/items draw inside handlers documented elsewhere.

1. **setPlayer**: `sample(['M','F'])` per Pokemon without a valid gender on a dual-gender species.
2. **switchIn**: `BeforeSwitchIn` handler sort. **insertChoice(runSwitch)**: `random(first, last+1)` on ties.
3. **runSwitch**: `speedSort(getAllActive(true))`, the `fieldEvent('SwitchIn')` handler sort, and handler draws.
4. **Every non-early-return runAction**: `eachEvent('Update')` actives sort (except `start`).
   Phazing: `dragIn` sample. Gen-8 re-sort: `getActionSpeed` of every queued action
   with a pokemon (`getTarget` draws, speed events) plus `queue.sort`.
5. **endTurn**: `DisableMove`, `TrapPokemon`, `MaybeTrapPokemon` runEvent sorts per active (both sides, slot order, fainted included).
6. **makeRequest/getRequests**: no draws. `priorityEvent` uses `compareRedirectOrder` with `Array.sort`.
7. **Choice parsing**: no draws.
8. **commitChoices**: `updateSpeed` events. Per action resolve (7.2 order):
   beforeTurnMove target, tera speed, priorityCharge target, FractionalPriority
   (Quick Claw / Quick Draw rolls, handler ties), random target, getActionSpeed
   (getTarget, ModifyPriority, speed). Then `queue.sort` ties.
9. **turnLoop**: `beforeTurn` insertChoice (no draw in practice). The `beforeTurn` action does an `eachEvent('BeforeTurn')` actives sort.
10. **residual**: `updateSpeed` events, `fieldEvent('Residual')` handler sort, and per-handler draws.
11. **faintMessages**: `BeforeFaint`/`Faint`/`AfterFaint` runEvent sorts.

Verified example (p2 = two Lv5 Magikarp/Feebas with equal speed, p1 = Pikachu U-turn +
Snorlax Earthquake). Turn 1 draws, in order: EQ resolve random target, EQ getActionSpeed
target, `queue.sort` tie (Magikarp/Feebas), BeforeTurn eachEvent tie, Update eachEvent
tie, re-sort EQ target, re-sort tie, then the U-turn accuracy/crit/damage roll, and so on.
Fainted Magikarp's and Feebas's queued moves kept causing re-sort tie shuffles until
the end of the turn.

---------------------------------------------------------------------------------------

## 13. Gotchas and engine guards to mirror

* `singleEvent`/`runEvent` throw at `eventDepth >= 8` ("STACK LIMIT EXCEEDED").
  `singleEvent` also throws when `log.length - sentLogPos > 1000` ("LINE LIMIT
  EXCEEDED", `sim/battle.ts:580-595`). `sentLogPos` advances only in `sendUpdates()`.
  BattleStream calls that after every input line. The direct API calls it from
  `commitChoices` only when more than 500 lines are pending. The engine should treat
  both as harness-level crashes and surface them as errors, not mimic them.
* `makeRequest` throws if all sides are already done right after a request (an invariant).
* `autoChoose` throws `autoChoose crashed` / `infinite looping` (an invariant).
* `Battle.choose` with an unknown error emits "Unknown error for choice: ..." (should not happen).
* `|t:|` appears after `|` at the start of every `turnLoop` call, and once at construction.
* Messages emitted during `commitChoices` (resolve-time Quick Claw / Custap / Quick Draw)
  come **before** that `turnLoop`'s `|` line.

---------------------------------------------------------------------------------------

## 14. Ignored (explicitly out of scope for this format)

* **Team Preview**: `runPickTeam` preview branch, `teampreview` request/state,
  `team` action (`order 1`), `chooseTeam` (except its "not in a Team Preview phase"
  error), `pickedTeamSize`, `onTeamPreview`, `onChooseTeam`, `showOpenTeamSheets`.
* **Mega / Mega X / Mega Y / Ultra Burst / Z-Moves / Dynamax / Max & G-Max moves**:
  request fields `canMegaEvo*`, `canUltraBurst`, `canZMove`, `canDynamax`, `maxMoves`;
  actions `megaEvo/X/Y` (104) and `runDynamax` (105); `endTurn` dynamax expiry;
  `getActionSpeed` z/max substitution; `chooseMove` event branches (beyond the error they
  produce); gen-7 `deferPriority` and the mega re-insert in `runAction`;
  `Side.zMoveUsed` and `dynamaxUsed`; `canDynamaxNow`.
* **Older-gen branches**: gen-1 partial trapping cleanup, gen-1 `Side.commitChoices`
  lastSelectedMove logic, gen-1 Fight button; gen-2/3 `quickClawRoll`; gen ≤ 3 per-move
  `checkFainted` and queue clearing in `faintMessages`; gen ≤ 4 Pursuit
  switch continuation; gen < 5 `Update`; gen ≤ 4 negative-fractional reverse speed;
  gen-2 `draggedIn`; gen ≤ 4 `lastItem` transfer; gen-4 `newMove`; gen-5
  `adjacentAllyOrSelf` fainted ally; gen ≤ 2 DV evenizing; gen ≤ 1 stat modifiers.
* **Other game types**: triples/rotation (`shift` action/`chooseShift` except its error,
  triples centering in `endTurn`, `activePerHalf > 2` targeting), multi (`allySide`,
  `ally` request field, synced side conditions), free-for-all (`freeforall` branches in
  `validTargetLoc`, `lose`, `foes`, `foeSidesWithConditions`).
* **Server/room features**: timers/`tiebreak`, `forcewin`, `forcetie`, `forcelose`/`lose`,
  `chat`, `eval`, `editbattle`, `reseed`/`resetRNG`, `requestlog`/`requestexport`/`requestteam`,
  inactivity, `bestOfDefault`, rated/`|rated|`, `customRules`, `>version` input-log lines.
* **Endless Battle Clause** proper (the rule is absent). The turn-1000 tie and
  warnings are **not** ignored.
* **Debug/test hooks**: `debugMode`, `debug()` lines, `checkEVBalance`, `forceRandomChance`,
  `strictChoices`, `makeChoices` (helper), the `testfight` choice.
* **Serialization**: `toJSON`/`fromJSON`/`restart`/`deserialized`, `destroy`.
* **Mod hooks**: `champions*` currentMod branches, hackmons /
  `twisteddimensionmod` / `overflowstatmod` checks, Scripts/format object overrides.
* **Random team generation and PotD species forcing** (separate PRNG, separate concern).
* **`event` action type** (no gen-9 producer) and `changeAction` (no gen-9 caller).
* **Pursuit** (`isNonstandard: "Past"` in gen 9) and therefore `'pursuitfaint'`.
* Hidden Power / Return / Frustration name decorations in requests (not in the gen-9 movepool).

---------------------------------------------------------------------------------------

## 15. Open questions

1. **Timestamps**: the comparator must normalize `|t:|` lines. Confirm that the harness
   strips them rather than having the engine emit a fixed value.
2. **Revival Blessing instaswitch** is pushed with `addChoice` to the end of the queue
   (after `residual`). The exact interleaving with the re-sort and with residual
   needs a dedicated golden test if Pawmot (or another Revival Blessing user) can appear.
3. **Invalid-choice partial commits** (5.2): decide whether the fuzzer emits malformed
   choices. If it does, the engine must keep partial `choice.actions` and the
   `isChoiceDone`-ignores-`error` behavior.
4. **Locked-by-name choice** (`move outrage` needing a target) and other choice-error
   texts only affect side-channel `|error|` lines. Decide whether error parity is
   required, or only accept/reject parity.
5. **LINE LIMIT** (13) depends on how often the harness calls `sendUpdates`. Recommend
   that the oracle harness use BattleStream semantics (`sendUpdates` after each input).
