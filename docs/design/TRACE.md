# Event / RNG traces (finding the root cause of a divergence)

`difftest` (`docs/design/DIFFTEST.md`) reports the first place where the engine differs from a Showdown
fixture: a log line, a request field or a PRNG seed. That is a symptom. The *root cause* is usually earlier and
invisible in those three channels: a handler that ran in the wrong order, an extra `random()` whose result
happened not to matter yet, a relay value that was 57 instead of 58. A **trace** records the inner life of one
battle: every PRNG draw, every log line as it is appended, optionally every event call, the sorted handler list
of every event and every handler invocation. The oracle writes traces from the real Showdown
(`tools/oracle/trace.mjs`), the engine writes the same format behind a debug feature, and
`tools/oracle/trace-diff.mjs` aligns the two and prints the first differing record with context and a
classification.

| file | purpose |
|---|---|
| `tools/oracle/trace.mjs` | CLI: replay a fixture battle on the pinned Showdown with the tracer attached, write the trace |
| `tools/oracle/trace-diff.mjs` | CLI: first divergence of two traces |
| `tools/oracle/lib/trace-core.mjs` | the `Tracer`: runtime wrappers around `Battle.prototype`, `PRNG.prototype`, `Gen5RNG.prototype` |
| `tools/oracle/lib/trace-replay.mjs` | `traceFixture`: `Session.replayFixture` + tracer + `boundary` markers |
| `tools/oracle/lib/trace-schema.mjs` | version, kind to level table, canonical form used for comparison |
| `tools/oracle/lib/trace-io.mjs` | (gzipped) JSONL helpers |
| `tools/oracle/lib/trace-selftest.mjs` | end-to-end test of the above (traces, self diff, perturbed diffs) |

## 1. Usage

```
# trace battle 17 (0-based position in the file, what `difftest --battle 17` means), everything:
node tools/oracle/trace.mjs ~/src/pokemon-showdown data/fixtures/sample-50.jsonl.gz --battle 17 --out /tmp/b17.jsonl

# only turns 5..6, only RNG + log (level 1), or events (2) / handler lists (3, default)
node tools/oracle/trace.mjs ~/src/pokemon-showdown data/fixtures/sample-50.jsonl.gz --battle 17 --from-turn 5 --to-turn 6 --level 1 --out /tmp/b17-t5.jsonl

# the engine's trace (a Rust debug feature, same format), then:
node tools/oracle/trace-diff.mjs /tmp/b17.jsonl /tmp/b17-engine.jsonl --context 8
```

`trace.mjs <path-to-pokemon-showdown> <fixtures.jsonl[.gz]> --battle SPEC [--out FILE|DIR]`

| option | meaning |
|---|---|
| `--battle SPEC` | `17`, `3,7,10-20` or `all`; 0-based position among the non-blank fixture lines. `--index` selects by the fixture's `index` field instead |
| `--out FILE` | the trace (`.gz` is gzip-compressed, `-` is stdout). With several battles `--out DIR` writes `DIR/battle-<position>.jsonl` (`--gz` for `.jsonl.gz`); without `--out` only the checks and the statistics run |
| `--level 1\|2\|3` | what to record, section 2 (default 3) |
| `--from-turn T`, `--to-turn U` | only records emitted while `battle.turn` is in `[T, U]`; the replay itself always runs from the start |
| `--stack` / `--no-stack` | oracle-only call-site annotations on `rng` records (default: on at level 3) |
| `--skip-empty` | drop `ev`/`evx` pairs that contain nothing and leave the relay value unchanged (more than 90 % of all event calls) |
| `--time-plain` | also time the untraced replay (after a warm-up replay) |

Every traced replay is the normal `replayFixture` of `tools/oracle/lib/session.mjs` (same `Session`, same
`sendUpdates()` after every input, same recorded choose order): the replayed log, PRNG seeds at every boundary,
requests, accept/reject results and end record are compared with the fixture. A clean run (exit status 0)
therefore also proves that tracing did not change the battle; status 2 means the traced replay differed
(the trace is still written).

`trace-diff.mjs expected.jsonl actual.jsonl` (expected = oracle, actual = engine; `.gz` ok) exits 0 when the
traces are identical under the chosen projection, 1 on a divergence, 2 on bad input. Options in section 8.

## 2. Format overview

* **JSON Lines**, UTF-8, one record per line, `{"k": "<kind>", ...}`. The first line is a `hdr`.
* Flat records, a small fixed set of kinds (12), short keys. Key order is not significant to the tools (the
  oracle writes `k` first); numbers are plain JSON numbers (integers, or floats where Showdown has floats).
* **Records are written at the moment the thing happens**, so a trace reads like an execution log: an `ev` precedes
  everything it causes, its `evx` follows, a `log` line sits exactly where Showdown appended it, the `rng` record of
  a shuffle precedes the `sort` it belongs to.
* **Top-level keys starting with `_` are non-normative** (oracle-only annotations, never compared; `_` keys inside values are data). The depth field `d` is
  redundant (derivable from `ev`/`evx` nesting) and also never compared; an engine should still emit it because it
  makes traces greppable.
* **Ids** are Showdown `toID` form (lower case alphanumerics: `ironleaves`, `stompingtantrum`). Event names are
  Showdown's CamelCase tokens as passed to `runEvent`/`singleEvent` (`ModifyDamage`, `SwitchIn`, `AllyDamagingHit` is
  a *callback* name, see `cn`).
* The trace is the sequence of what happens inside `Battle`; the replay driver adds markers around it
  (`hdr`, `boundary`; `choose`/`chosen` come from `Battle.choose`).

### Levels

| level | adds | kinds |
|---|---|---|
| 1 | markers, log, PRNG draws | `hdr` `boundary` `choose` `chosen` `log` `logedit` `rng` |
| 2 | event calls | `ev` `evx` |
| 3 | sorted handler lists, handler invocations | `sort` `hc` `hcx` |

`hdr.level` says what a file contains. `trace-diff` compares at the lower of the two levels, so a level-3 oracle
trace can be diffed against a level-1 engine trace.

### Turn window

`--from-turn T --to-turn U`: a record is written iff `from <= battle.turn <= to` *at the moment it is emitted*
(`battle.turn` is 0 before the battle starts and is incremented at the start of `endTurn`, so the `ev`s that
`endTurn` runs after the increment, then `|turn|T`, belong to turn `T`). An `ev` and its `evx` are written together
or not at all (the decision is taken at the `ev`), likewise `choose`/`chosen` and `hc`/`hcx`. `d` is absolute, so
the first record of a window may have `d > 0`. `hdr` is always written.

## 3. Conventions shared by all records

### 3.1 PRNG seeds

`[a, b, c, d]`: four u16 words, most significant first, exactly the fixture format (`battle.prng.getSeed()` of
the Gen5 LCG, `docs/design/FIXTURES.md` section 1).

### 3.2 References (`t`, `s`, `h`, `x`, `ef`, `to`, `ix`...)

Typed reference fields hold a string (or `null`/`false`):

| form | meaning |
|---|---|
| `p1a` `p1b` `p2a` `p2b` | a Pokemon that is active (`pokemon.isActive`: set when `switchIn` puts it in `side.active`, cleared when it switches out or its faint is processed; `side.id` + position letter, `Pokemon#getSlot()`) |
| `p1:3` | a Pokemon that is not active: side and 0-based index into the side's **original team order** (the packed team string of the fixture). It does not change when Pokemon switch; fainted and benched Pokemon are always in this form |
| `side:p1` | a side (side conditions, side-targeted events) |
| `field` | the field (weather, terrain, pseudo-weather holder) |
| `battle` | the battle itself (format/rule handlers; the target of an event run without a target) |
| `ability:intimidate`, `move:tackle`, `item:leftovers`, `condition:confusion`, `status:par`, `weather:raindance`, `terrain:grassyterrain`, `rule:sleepclausemod`, `pokemon:wugtrio` | an effect: lower-cased `effectType`, `:`, `effect.id` |
| `str:Fire`, `str:strugglerecoil` | a plain string used where Showdown accepts an effect or source (`Effectiveness` events pass the attacking *type name* as source; `faint(..., {id: 'strugglerecoil'})` passes a bare `{id}` object) |
| `anon` | an inline handler container with neither id nor type (a move's `secondary` object passed to `singleEvent('Hit', ...)`) |
| `null` | no value (also: Showdown's "no effect" sentinel `{id: ''}` that `battle.effect` holds outside events) |
| `false` | `source === false` |
| `[ref, ...]` | an array target (`runEvent` with a `Pokemon[]`, `fieldEvent` targets) |

### 3.3 Values (`v`, `r`, `a`, `cv`)

Relay variables, handler return values and PRNG arguments are JSON values with these conventions, chosen so
that `undefined`, `null`, `false`, `0` and `true` are always distinguishable:

| Showdown value | JSON |
|---|---|
| `undefined` | `"@undefined"` (a key is never omitted for it) |
| `null`, booleans, finite numbers | as is (`-0` is written `0`) |
| `NaN`, `Infinity`, `-Infinity` | `"@NaN"`, `"@Infinity"`, `"@-Infinity"` |
| string | the string; if it starts with `@`, one more `@` is prepended |
| `Pokemon`, `Side`, `Field`, `Battle` | `"@"` + reference of 3.2: `"@p1a"`, `"@p1:3"`, `"@side:p1"`, `"@field"`, `"@battle"` |
| Effect object (`ActiveMove`, `Move`, `Ability`, `Item`, `Condition`, `Species`...) | `"@"` + effect reference: `"@move:tackle"`, `"@pokemon:wugtrio"`, `"@condition:choicelock"` |
| array | array of values (more than 64 elements: first 64 and `"@..."`) |
| plain object (boost tables, `{target: ...}`) | object of values (more than 48 keys: first 48 and `"@...": true`); key order is not significant |
| nested deeper than 4 levels | `"@..."` |
| function | `"@fn"` |
| `Set`, `Map`, `symbol`, `bigint` | `"@set"`, `"@map"`, `"@symbol"`, `"@bigint:123"` |

## 4. Record reference

### 4.1 Driver markers (level 1)

```
{"k":"hdr","v":1,"level":3,"format":"gen9randomdoublesbattle","oracle":"7332b60e...","fixture":{"runSeed":1,"index":0,"position":0},"battleSeed":[34875,39376,17730,7959],"from":null,"to":null,"stack":true,"skipEmpty":false}
{"k":"boundary","step":3,"turn":2,"state":"move","seed":[48746,3697,49364,39030]}
{"k":"choose","side":"p1","input":"move 1 2, move 2 -1 terastallize"}
{"k":"chosen","side":"p1","ok":true}
```

* `hdr` (first line, once). Normative: `v` (format version, 1) and `level`. An engine writes `{"k":"hdr","v":1,"level":L}`; the
  other keys are oracle metadata.
* `boundary`: a decision boundary, written by the replay driver right after it takes the fixture step's snapshot
  (`Session.snapshot()`; in Rust: the difftest harness at the point where it compares step `k`). `step` is the
  0-based fixture step, `turn` = `battle.turn`, `state` = `battle.requestState` (`move` | `switch`), `seed` = the PRNG seed
  at the boundary (= `steps[k].seed`). The final snapshot (the battle has ended) is `state:"end"`, `step` = number of
  steps. Everything between two `boundary` records is the work done by the `choose` calls of that step.
* `choose` / `chosen`: one `Battle.choose(side, input)` call, including rejected ones (`ok:false`). `choose` is written
  on entry, `chosen` on exit, so the whole commit of a turn (events, draws, log lines) sits between the `choose` and `chosen`
  of the call that completes the last acting side. `input` is the exact string. A rejected call (and an accepted call that does not complete the
  turn) still contains event records, because choice parsing itself runs events (`priority LockMove` / `SemiLockMove` via `Pokemon#getLockedMove`, `MaybeTrapPokemon`...),
  but never `log` or `rng` records.

### 4.2 `log`, `logedit` (level 1)

```
{"k":"log","i":31,"line":"|move|p2a: Klefki|Thunder Wave|p1a: Wugtrio"}
{"k":"logedit","i":31,"line":"|move|p2a: Klefki|Thunder Wave|p1a: Wugtrio|[miss]"}
```

`log`: an entry was appended to `battle.log` at index `i` (entries as in the fixture: a `|split|p1` marker and its two
lines are three entries; entries may be `""`). `line` has the `|t:|<unix>` normalization applied (`^\|t:\|\d+$` becomes `|t:|`).
`logedit`: an existing entry changed (`Battle#attrLastMove` appends `[miss]`, `[still]`, `[spread] p1a,p1b`,
`[notarget]`, `[anim] ...`; `retargetLastMove`). `line:null` means the entry was removed (`splice`; only the
`|-anim|` + `[miss]` case; later indexes shift). About one `logedit` per 20 log lines. The fixture contains the final
text; the trace shows both moments. An engine that only knows the final text when it appends a line can be compared
with `trace-diff --merge-logedits`.

The `Battle` constructor itself runs before any trace consumer exists; the oracle still sees its log lines
(`|t:|`, `|gametype|doubles`) and its events (section 7) because the tracer attaches to `battle.log` at the moment
the constructor assigns it.

### 4.3 `rng` (level 1)

```
{"k":"rng","m":"randomChance","a":[1,3],"r":false,"n":1,"s0":[56786,2313,44436,61038],"s1":[33850,7621,7299,36393],
 "_cs":["data/conditions:onStallMove@448[stall]","data/moves:onPrepareHit@14027[protect]","BattleActions.trySpreadMoveHit@490"],"_fx":"condition:stall","_ev":"StallMove"}
{"k":"rng","m":"sample","a":[2],"r":0,"n":1,"s0":[14794,48661,64129,18774],"s1":[16482,50978,20161,37809],"_it":"@p2a","_cs":["Side.randomFoe@191","Battle.getRandomTarget@2067","BattleQueue.resolveAction@168"]}
{"k":"rng","m":"shuffle","a":[2,4],"r":[3],"n":1,"s0":[52890,40013,58443,65273],"s1":[54043,29385,49764,30720]}
```

One record per **outermost PRNG API call on `battle.prng`** (so `randomChance` is one record, not a `random` inside it):

| `m` | `a` | `r` | `n` (next() calls) |
|---|---|---|---|
| `random` | the arguments as passed, trailing `undefined`s dropped: `[]`, `[n]` or `[m, n]` | the returned number (`random()` is `u32 / 2^32`, a float) | 1 |
| `randomChance` | `[numerator, denominator]` | boolean | 1 |
| `sample` | `[list length]` | the chosen **index** | 1 |
| `shuffle` | `[start, end]` (defaults resolved: `end` = list length) | array of the picked indexes, one per Fisher-Yates step | `end - start - 1` (when positive) |

`s0` / `s1` are the seed before / after the call. A call that draws nothing (`n` = 0, e.g. `sample` of an empty
list throws) writes no record. Draws on any other PRNG instance (the team generator, the choice picker) are not
traced. Oracle-only: `_it` (the sampled item), `_cs` (nearest 3 frames of `sim/` and `data/`, as `Type.fn@line` or
`data/<file>:fn@line[entryId]`, only with `--stack`), `_fx` (`battle.effect`) and `_ev` (`battle.event.id`) at the
time of the draw. `Battle#random`, `#randomChance`, `#sample` are plain forwarders to the same `battle.prng` calls.

### 4.4 `ev`, `evx` (level 2): event calls

```
{"k":"ev","d":0,"f":"run","e":"ModifyDamage","t":"p1b","s":"p2a","x":"move:avalanche","v":23,"_ts":"dondozo","_ss":"klefki"}
{"k":"evx","d":0,"e":"ModifyDamage","v":24}
```

`ev` opens an event call, `evx` closes it (properly nested; `d` = number of enclosing open `ev`s, 0 at top level).

| `f` | Showdown call | fields of `ev` |
|---|---|---|
| `run` | `runEvent(e, target, source, sourceEffect, relayVar, onEffect, fastExit)` | `e`, `t` (`battle` when the target argument is falsy), `s`, `x` = sourceEffect, `v` = relayVar as passed; `oe:true` if `onEffect`, `fe:true` if `fastExit` (set directly, i.e. not through `priorityEvent`) |
| `priority` | `priorityEvent(...)` (= `runEvent` with `fastExit`) | as `run` |
| `single` | `singleEvent(e, effect, state, target, source, sourceEffect, relayVar, customCallback)` | `e`, `ef` = `effect`, `t`, `s`, `x` = sourceEffect, `v` |
| `each` | `eachEvent(e, effect, relayVar)` | `e`, `x` = `effect \|\| battle.effect`, `v` |
| `field` | `fieldEvent(e, targets)` (only `Residual` and `SwitchIn`) | `e`, `t` = array of targets or `null` |

`evx`: `e` (repeated for readability), `v` = the value the call returned. For `run`/`priority` that is the final
`relayVar` (after the integer `modify` that `runEvent` applies on exit; for an array target the array of per-target
relay values); a missing relay variable is `true` there. For `single`: `returnVal === undefined ? relayVar : returnVal`
(`true` if no relay variable was passed). `each` and `field` return `"@undefined"`. If the call threw, `evx` has
`thr: "<message>"` instead of `v`.

When an `ev` is written:

* `run`, `priority`, `each`, `field`: on **every** call, also when there is no handler at all (about 6000 calls per
  battle; more than 90 % are trivial: no handler, value unchanged, nothing happens inside). `trace.mjs --skip-empty` and
  `trace-diff.mjs --squash-empty` drop *trivial pairs*: an `ev` immediately followed by its `evx` and `evx.v` equal to `ev.v`
  (or `ev.v` absent (`"@undefined"`/`null`) and `evx.v` = `true`).
* `single`: only when a callback exists: `customCallback || effect["on" + e]` is not `undefined`. This is
  decided before any of `singleEvent`'s early returns, so a call that exists but is suppressed (status mismatch,
  Mold Breaker, Embargo/Klutz/Magic Room, Gastro Acid, Air Lock) is still written, with `evx.v` = the unchanged relay value.
  Calls with no callback are not events as far as the trace is concerned.

Oracle-only: `_ts`, `_ss` (species id of the target / source Pokemon).

### 4.5 `sort`, `hc`, `hcx` (level 3): handler lists and invocations

```
{"k":"sort","d":1,"w":"event","e":"ModifyDamage","sm":"speed","sh":1,"it":[
   {"x":"condition:lightscreen","h":"side:p2","cn":"onAnyModifyDamage","o":0,"p":0,"so":4,"sp":0,"eo":0},
   {"x":"condition:reflect","h":"side:p2","cn":"onAnyModifyDamage","o":0,"p":0,"so":4,"sp":0,"eo":0}]}
{"k":"hc","d":1,"cn":"onAnyModifyDamage","x":"condition:lightscreen","h":"side:p2","v":60}
{"k":"hcx","d":1,"r":"@undefined"}
```

**`sort`**: a list was sorted. Fields: `d` (number of enclosing open `ev`s), `w` (what was sorted), `e` (event name,
for `event`/`field`/`each`), `sm` (sort mode), `sh` (number of tie groups that were shuffled with the PRNG during this
sort: each has its own `rng` `shuffle` record immediately before the `sort`), `it` (the list **after** sorting).

| `w` | list | emitted | item fields |
|---|---|---|---|
| `event` | the handler list of the enclosing `run`/`priority` event | every such event with at least 1 handler | handler item |
| `field` | the handler list of the enclosing `fieldEvent` | at least 1 handler | handler item |
| `each` | the active Pokemon of the enclosing `eachEvent` (comparator `b.speed - a.speed`) | at least 2 | `{h, sp}` |
| `queue` | the action queue (`BattleQueue#sort`: in `commitChoices`, and in gen 8+ again at the end of every `runAction` while the next queued action is a move) | at least 2 | action item |
| `switchin` | all active Pokemon in `BattleActions#runSwitch` (this also sets `battle.speedOrder`) | at least 2 | `{h, sp}` |
| `mons` | any other `speedSort` of Pokemon (Magician, Neutralizing Gas end, Dynamax end) | at least 2 | `{h, sp}` |

`sm`: `speed` = `Battle#speedSort` (selection sort by `comparePriority`: `order` ascending with 0 = last, `priority`
descending, `speed` descending, `subOrder` ascending, `effectOrder` ascending; ties are broken by a PRNG shuffle of each
tie group in list order); `ltr` = `Array#sort(Battle.compareLeftToRightOrder)` (order, priority desc, `ix` asc; no PRNG), used by
`runEvent` for `Invulnerability`, `TryHit`, `DamagingHit`, `EntryHazard`; `redirect` = `Array#sort(Battle.compareRedirectOrder)` (priority desc, speed desc, holder's
`abilityState.effectOrder` asc; no PRNG), used by `priorityEvent`. The list *before* the sort is the order in which
`findEventHandlers` assembled it (own handlers, ally/any/foe handlers, source handlers, side, field, battle), which matters for tie shuffles.

*Handler item* `{x, h, cn, o, p, so, sp, eo, [ix], [cv]}`: `x` effect reference; `h` the holder (`effectHolder`, or the
`customHolder` fieldEvent substitutes: the active Pokemon); `cn` the callback name it was resolved for
(`onModifyDamage`, `onAllyModifyDamage`, `onFoeTryMove`, `onAnySwitchIn`, `onSourceDamagingHit`, `onSideResidual`...);
the sort keys exactly as the comparators read them, normalized: `o` = `order || 0` (0 means "default, last"), `p` = `priority || 0`,
`so` = `subOrder || 0`, `sp` = `speed || 0` (holder's `speed`; fractional for `SwitchIn`: `speed - speedOrder.indexOf(fieldPosition) / (activePerHalf * 2)`;
0 for non-Pokemon holders), `eo` = `effectOrder || 0` (set only for `*SwitchIn` and `*RedirectTarget` callbacks; for `sm` = `redirect` the holder's `abilityState.effectOrder`);
`ix` = the position in the target array for array-target events; `cv` = the value of a non-function callback (a constant
such as `false` for `onTakeItem: false`) or `"@undefined"` for handlers that only exist for their duration countdown (`fieldEvent('Residual')`).

*Action item* `{c, h, [m], [to], o, p, sp, so, eo}`: `c` = `choice` (`move`, `switch`, `instaswitch`, `residual`, `beforeTurn`, `runSwitch`,
`terastallize`, `shift`, `revivalblessing`...), `h` the acting Pokemon (`null` for `residual`/`beforeTurn`/`start`), `m` the move id of
move actions, `to` the target Pokemon of switch actions, `o` = order, `p` = priority (already including fractional priority), `sp` = speed.

**`hc` / `hcx`**: a handler function from a handler list was invoked (`handler.callback.apply(...)`) and returned.
`hc` is written on entry, `hcx` on exit.
`hc`: `cn`, `x` (effect), `h` (holder), `v` = the relay value the handler was called with (present iff the event has a relay variable). `hcx`: `r` = the raw return
value (`"@undefined"` = "no change"); `thr` instead of `r` if it threw. Rules:

* only function callbacks (a constant `cv` handler returns its constant without a call);
* handlers of a sorted list that are skipped by `runEvent` (suppressed by Mold Breaker, Gastro Acid, Embargo/Klutz, Air Lock, status mismatch, array
  index with a falsy relay value, or because an earlier handler made the relay variable falsy and ended the loop) have no `hc`: compare the `sort` list with the `hc`s that follow;
* a bare `singleEvent` (no `customCallback`) has no `hc`: the `ev single`/`evx` pair *is* the invocation. `fieldEvent` passes each handler's callback to
  `singleEvent` as `customCallback`, so its handlers appear as `ev field > ev single > hc`;
* nesting: `hc` has `d` = number of enclosing open `ev`s *including the event that owns the handler*, so the events the handler body runs have the same `d`
  as the `hc` (`ev d=0, sort d=1, hc d=1, ev d=1 ... evx d=1, hcx d=1, evx d=0`).

## 5. Ordering guarantees

Within one trace the records are in execution order. In particular:

* `choose` < everything the commit does < `chosen`; `boundary` records sit between `choose`s of different steps.
* `ev` < (for `run`/`priority`/`field`: the `rng` shuffles of its sort < its `sort`) < `hc` < the handler's own records < `hcx` < ... < `evx`.
  For `run`/`priority` the `sort` comes before any `hc` and the *whole* list is known up front (handlers are resolved and sorted once; a handler
  added or removed during the loop does not change it).
* a `log` record is written when `battle.log.push` runs (`Battle#add`, `addSplit`, `addMove`, `hint`...), a `logedit` when `attrLastMove`/`retargetLastMove` edit it.
* an `rng` record is written when the call returns.

## 6. Which Showdown code each record comes from (for the engine's emitter)

| record | Showdown hook (`sim/battle.ts` unless noted) |
|---|---|
| `rng` | `PRNG.prototype.random/randomChance/sample/shuffle` (`sim/prng.ts`) when `this === battle.prng`; `Gen5RNG.prototype.next` is counted for `n` |
| `log`, `logedit` | Proxy around `battle.log` (set trap: append vs. overwrite); `splice` in `attrLastMove` |
| `choose`, `chosen` | `Battle#choose` |
| `ev` `evx` | `singleEvent`, `runEvent`, `priorityEvent` (flags the `runEvent` it calls), `eachEvent`, `fieldEvent` |
| `sort` (`event`) | `runEvent`'s `speedSort(handlers)` / `handlers.sort(compareLeftToRightOrder \| compareRedirectOrder)`; the array is captured from `findEventHandlers` |
| `sort` (others) | `speedSort` (`BattleQueue#sort`, `eachEvent`, `fieldEvent`, `runSwitch`, ...) |
| `hc` `hcx` | `resolvePriority` wraps each function `handler.callback` it resolves (the same object `runEvent` and `fieldEvent` call) |
| `boundary` | `Session.snapshot()` (`tools/oracle/lib/session.mjs`) |

For the Rust engine: put the emission at the same *semantic* place (where the engine appends to the log, draws from
the PRNG, enters/leaves `run_event`/`single_event`, finishes ordering a handler list, calls a handler). The `Prng` API
should keep the four entry points (`random`, `random_chance`, `sample`, `shuffle`) because the trace names them; an engine that implements `sample` through
`random` can still be compared with `trace-diff --rng-raw`. Everything is plain data: a `TraceSink` trait with a `JsonlSink` writing the lines above, and a
no-op sink compiled in when the feature is off, is enough.

## 7. What is deliberately NOT in the trace

* Oracle-only annotations (`_`-prefixed keys: call sites, species ids, sampled items, `battle.effect`/`event` at a draw) and the redundant depth `d`: never compared.
* `|t:|` timestamps (normalized exactly like the fixtures).
* Draws on PRNGs other than `battle.prng`; the team generator and choice picker never touch it.
* `singleEvent` calls with no callback; handler `state`/`effectState` objects, duration countdowns, `battle.eventDepth`, `this.event`, `activeMove`; `debug()` output; request construction other than the events it runs.
* Which handlers were suppressed or skipped (inferable: listed in `sort`, absent from `hc`).
* Team generation (`Teams.getGenerator(...).getTeam()`): outside the battle; the fixtures carry the packed teams.

What the oracle *does* see that is easy to forget: the `Battle` constructor runs `addPseudoWeather` for rules that
have event handlers (`sleepclausemod` in this format), which produces a `run PseudoWeatherChange` event, and `Side`/`Pokemon`
construction runs `ModifySpecies` for each team member, p1's six before the `|player|p1` line and p2's six before `|player|p2`.

## 8. trace-diff

```
trace-diff.mjs expected.jsonl actual.jsonl [--context N] [--level 1|2|3] [--kinds k1,k2]
    [--skip-kinds k1,k2] [--skip-events E1,E2] [--squash-empty] [--merge-logedits] [--rng-raw] [--ignore-seeds]
    [--window W] [--full] [--json]
```

Comparison projection (what "same" means): both traces are filtered to the common level (or `--level`), `hdr` records are dropped, every record is
turned into its canonical form (keys sorted, top-level `_` keys and `d` removed) and compared as text.

| option | effect |
|---|---|
| `--level L`, `--kinds a,b` | compare only these levels / kinds (e.g. `--kinds log,rng` when the engine has no events yet) |
| `--skip-kinds a,b` | compare everything except these kinds (e.g. `boundary,choose,chosen` if the harness does not write the markers) |
| `--skip-events E1,E2` | drop these event names entirely: their `ev`/`evx` and the `sort`/`hc`/`hcx` they own (records they cause are kept). Typical: `Update,ModifyBoost,ModifySpe,Type` |
| `--squash-empty` | drop trivial `ev`/`evx` pairs (section 4.4) from both sides |
| `--merge-logedits` | fold `logedit` into the `log` record it edits |
| `--rng-raw` | replace every `rng` record by one `{s0, s1}` record per underlying `next()`: only the seed chain is compared, not the API call or arguments |
| `--ignore-seeds` | do not compare `s0`/`s1`/`boundary.seed` |
| `--window W` | how far ahead (records) to look for the point where the traces agree again (default 1000) |
| `--context N` | N identical records before and N records after the divergence (default 6) |
| `--json` | one JSON object instead of the text report |

The first differing record is classified; the cheapest explanation (fewest records skipped, insertion and deletion preferred over substitution) after which the next 4
records agree again decides between *extra*, *missing* and *different*:

| class | meaning |
|---|---|
| `extra RNG draw` / `missing RNG draw` | the actual has `rng` records the expected does not / lacks some (the next record says which call: `randomChance(1,24)`...) |
| `extra` / `missing` `log line`, `log edit`, `event call`, `handler list / sort`, `handler invocation` | same for the other kinds |
| `different RNG call` | same position, other `m`/`a` (the engine drew with different arguments) |
| `PRNG state differs before the draw` | same call, but the seed already differs: earlier draws differed in count; the report says "actual is N draws ahead / behind" |
| `different log line`, `different log edit` | text differs |
| `different event call` | `f`, `e`, `ef`, `t`, `s` or `x` differ |
| `different relay value in` / `out` | same call, `ev.v` / `evx.v` differ |
| `different handler order` | same handlers, other order (the report prints both orders and the shuffle counts) |
| `different handler ordering keys` | same handlers, different `o/p/so/sp/eo` for one of them (the field and both values are printed) |
| `different handler list` | a handler is missing or extra |
| `different handler invoked` / `different handler return value` | `hc` / `hcx` differ |
| `different decision boundary` | seed/turn/state at a boundary differ |
| `actual ends early` / `extra records at the end of actual` | one trace is a prefix of the other |

The report also prints where the divergence is: the last `boundary` (fixture step, turn) with its `choose` inputs, the last `log` line before it, the stack of open events/handlers
(`run:ModifyDamage(p1b) > handler:ability:prankster[onModifyPriority]`), the oracle call site if the expected trace carries one,
the common context and the next records of both traces.

## 9. Things learned about Showdown while instrumenting it

* **Shuffles come from four places, and the most common are not the ones one expects.** In 100 fuzz battles (18 217 draws, 1400 of them `shuffle` calls) the
  tie groups that were shuffled came from: `fieldEvent('Residual')` handler lists 684 (mostly `condition:stall` and `condition:protect` on the same Pokemon,
  equal `subOrder` 2 and equal speed, at the end of the turn after a Protect-like move; also `reflect`/`tailwind`...), `eachEvent` sorts of the active Pokemon 593, the action queue 64,
  the switch-in order (`runSwitch`) 37 and `runEvent` handler lists 22 (`Reflect` + `Light Screen` of one side tie in `ModifyDamage`: both `subOrder` 4, speed 0).
* **A speed tie among the active Pokemon consumes PRNG draws at every `eachEvent`.** `eachEvent` (`Update` after every move and switch, `Weather`, `BeforeTurn`,
  `TerrainChange`...) sorts the active Pokemon with `speedSort` before running the event for each, and `speedSort` shuffles tie groups. These draws are invisible in the log;
  the first symptom is a seed mismatch at the next boundary. (`w:"each"`, `sh>0` in the trace.)
* **The action queue is re-sorted after every action** (gen 8+, end of `runAction` while the next action is a move: speeds are re-read with `getActionSpeed`, which re-runs `ModifyPriority`/`ModifySpe`),
  so speed ties among the remaining actions are re-shuffled after each move, not just once per turn.
* **Mid-turn insertion into the queue breaks ties with a plain draw**, not a sort: `BattleQueue#insertChoice` calls `battle.random(first, last + 1)` (`rng` `random` with two arguments, call site `BattleQueue.insertChoice`).
* **The handler list is fixed before the first handler runs**: `resolvePriority` reads `speed`, `priority`, `order`... once; handlers that change speed, remove a condition or add one mid-loop do not re-sort or extend it. Only a few checks skip a listed handler
  (status changed, ability/item suppressed, weather suppressed, a falsy relay value ends the loop). The trace's `sort` is the list, the `hc` records are what was called.
* **Most event calls are noise**: about 6000 `ev` pairs per battle; `--skip-empty` keeps 5-8 % of them (87 % are directly adjacent `ev`/`evx` pairs, the rest are empty parents of empty children). By count (100 battles): `Update` 17 %, `ModifyBoost` 13 %, `ModifySpe` 11 %, `Type` 9 %, `LockMove` 4 %,
  `ModifyPriority` 3.5 %, `SemiLockMove` 3.4 %, `EmergencyExit` 3 %. Use `--squash-empty` or `--skip-events`.
* **`priorityEvent` is `runEvent(..., fastExit = true)`** and its handler list is sorted with `compareRedirectOrder` (no PRNG), not `speedSort`; `RedirectTarget`, `LockMove`, `SemiLockMove`, `WeatherModifyDamage` use it.
  `TryHit`, `DamagingHit`, `Invulnerability`, `EntryHazard` sort with `compareLeftToRightOrder` (no PRNG). Only the plain `speedSort` path shuffles.
* **`runEvent` post-processes numeric relay values**: on exit, any non-negative integer relay value goes through `modify(relayVar, this.event.modifier)`, and `event.modifier` is what `chainModify` accumulates, so `evx.v` of an
  integer-relay event already contains every handler's modifier (e.g. `ModifySpe` 172 in, 344 out with Tailwind).
* **`SwitchIn` handler speeds are fractional** (`171.75`): `resolvePriority` subtracts `speedOrder.indexOf(position) / (activePerHalf * 2)` so the field-position order established by `runSwitch` survives re-sorting.
* **`fieldEvent` runs handlers through `singleEvent` with a `customCallback`**, so `ev field` contains `ev single` records for each handler, and those handlers' callbacks are the only functions that show up under both an `ev single` and an `hc`.
* **The log is edited in place** (`|move|` lines get `[miss]`, `[still]`, `[spread] ...`, `[notarget]` appended after the fact; one in 20 lines): code that builds the line once at append time will not reproduce the order of `log` records unless it also knows the final text early.
* **`ModifySpecies`, `PseudoWeatherChange` run during construction**, before the first `|player|` line (section 7).
* **Some event "sources" are not Pokemon**: `Effectiveness`/`Immunity`/`NegateImmunity` pass the move type as a string (`str:Fire`); a few `x` are `{id}` pseudo-effects (`strugglerecoil`) or inline `secondary` objects (`anon`).
* Event nesting reaches depth 6 in practice (Showdown aborts at 8).

## 10. Cost and verification

Measured on `data/fixtures/sample-50.jsonl.gz` (battles 0-9, warm process, median of 3 runs; one battle is about 19 turns / 26 steps / 30 log lines per step):

| config | ms / battle | records | KiB | gzip KiB |
|---|---|---|---|---|
| untraced replay | 75 | - | - | - |
| level 1 | 96 | 966 | 67 | 11 |
| level 2 | 113 | 15 505 | 1283 | 68 |
| level 2 `--skip-empty` | 102 | 1704 | 123 | 15 |
| level 3 (with `_cs` call sites) | 138 | 17 675 | 1544 | 84 |
| level 3 `--no-stack` | 128 | 17 675 | 1519 | 79 |
| level 3 `--skip-empty` | 130 | 5209 | 480 | 27 |

A fresh `trace.mjs` process spends about 0.4 s loading the dex before the first battle. The largest of the first 300 fuzz battles is 2.9 MiB at level 3 (250 ms), 1.1 MiB with `--skip-empty`.

Verification (all with exit status 0): every battle of `sample-50.jsonl` at levels 1, 2 and 3, and the first 500 battles of `fuzz-out/fuzz-2000.jsonl` at levels 1, 2 and 3
(500 battles at level 3 = 7.3 million records, 621 MiB, 57 s), replay identically to their fixtures with the tracer attached (`trace.mjs --battle all`, `--battle 0-499`).
`node tools/oracle/lib/trace-selftest.mjs ~/src/pokemon-showdown data/fixtures/sample-50.jsonl.gz --battle 0` additionally checks that traces are internally consistent (the `rng` seed chain
is gapless and ends at the fixture's end seed, `ev`/`evx` and `hc`/`hcx` balance), that self diffs and projected diffs report no divergence, and that 15 controlled perturbations of a
level-3 trace (missing/extra/different RNG draw, seed drift, relay value in/out, handler order/key/list, handler return value, event target, extra event, log line, log edit, truncation) are
found at exactly the perturbed record with the right class.
