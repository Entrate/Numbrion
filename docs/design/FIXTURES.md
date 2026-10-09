# Differential-testing fixtures (the Showdown oracle)

Ground truth for numbrion comes from the real Pokemon Showdown simulator, pinned at the commit in
`tools/oracle/ORACLE_COMMIT` (`~/src/pokemon-showdown`, built to `dist/`, never modified). The tools in
`tools/oracle/` play complete `gen9randomdoublesbattle` battles on it with random legal choices and
record everything an engine needs to reproduce the battle bit for bit: the battle PRNG seed, both
packed teams, every request Showdown sent, every `choose` call (accepted or rejected), the omniscient
log, and the PRNG state at every decision point.

| file | purpose |
|---|---|
| `tools/oracle/gen-fixtures.mjs` | CLI: generate fixtures (`--count N --seed S --out f.jsonl[.gz] [--threads K] [--verify-replay F]`) |
| `tools/oracle/gen-teams.mjs` | writes N packed random-doubles teams, one per line, from deterministic seeds |
| `tools/oracle/check-choices.mjs` | brute-force validation of the choice enumerator against Showdown (dev tool) |
| `tools/oracle/lib/choices.mjs` | legal-choice enumerator + random picker (works from request JSON only) |
| `tools/oracle/lib/session.mjs` | `Session` (BattleStream-equivalent driver), `playBattle`, `replayFixture`, `replayViaBattleStream` |
| `tools/oracle/lib/common.mjs` | Showdown loading, seed derivation, `|t:|` normalization |

Sample data: `data/fixtures/sample-50.jsonl` (8.4 MB; `node tools/oracle/gen-fixtures.mjs ~/src/pokemon-showdown --count 50 --seed 1 --out data/fixtures/sample-50.jsonl`).
`gen-fixtures.mjs` writes and reads either plain `.jsonl` or gzipped `.jsonl.gz` (about 20x smaller; pick by file extension).
Replay-check an existing file with `node tools/oracle/gen-fixtures.mjs ~/src/pokemon-showdown --verify-file data/fixtures/sample-50.jsonl`.
Validate the choice enumerator with `node tools/oracle/check-choices.mjs ~/src/pokemon-showdown --count 25 --seed 1000`.

## 1. How seeding works (exact API)

Showdown has two PRNGs behind one class (`sim/prng.ts`): `PRNG` picks the implementation from the seed.

```js
new PRNG(null)             // no seed -> PRNG.generateSeed() -> 'sodium,<hex>': ChaCha20 based. NOT what we want.
new PRNG('1,2,3,4')        // first char is a digit -> new Gen5RNG([1,2,3,4])      <- used everywhere here
new PRNG('gen5,0001000200030004')  // also Gen5 (hex form); not used
```

* **Battle**: `new Battle({ formatid: 'gen9randomdoublesbattle', seed: 'a,b,c,d', send })`. The Battle
  constructor does `this.prng = options.prng || new PRNG(options.seed || undefined)`, so a string
  `"a,b,c,d"` of four decimal u16 integers gives the Gen5 LCG. (An array `[a,b,c,d]` also works:
  the PRNG constructor joins it.) `Session` asserts `battle.prng.rng instanceof Gen5RNG` and
  `battle.prng.getSeed() === seed` after construction, so a sodium PRNG cannot sneak in.
  `battle.prngSeed` (the "starting seed" that ends up in Showdown's own input log
  `>start {"formatid":...,"seed":"a,b,c,d"}`) is that same string.
* **Seed meaning**: `[a,b,c,d]` is the 64-bit LCG state, **most significant word first**:
  `state = a<<48 | b<<32 | c<<16 | d`. This is what `getSeed()` returns (decimal, comma separated),
  and matches `Prng::from_seed` in the Rust engine. One step (`Gen5RNG.next`) is
  `state = state * 0x5D588B656C078965 + 0x269EC3 (mod 2^64)`, and the returned u32 is the **new**
  state's top 32 bits (`state >> 32`). Checked against a BigInt implementation over 10^6 steps.
* **Teams**: `Teams.getGenerator('gen9randomdoublesbattle', 'a,b,c,d').getTeam()` (the generator owns
  its own `PRNG.get(seed)`, also Gen5), then `Teams.pack(team)`. Team generation never touches the
  battle PRNG. The packed string is passed to `battle.setPlayer('p1', { name: 'Alice', team: packed })`,
  which `Teams.unpack`s it (`Battle.getTeam`). Every generated set carries a gender (`M`/`F`/`N`), so the
  `Pokemon` constructor's `sample(['M','F'])` never draws from the battle PRNG.
* **Choices**: a separate `new PRNG('a,b,c,d')` (Gen5) picks the choices; it is never connected to the battle.

All four seeds of battle `i` of run `S` are a pure function of `(S, i)`:

```
h           = SHA-256( utf8("numbrion-oracle-v1/" + S + "/" + i) )         // 32 bytes
battleSeed  = 4 big-endian u16 words from h[0..8]
teamSeeds   = [ words(h[8..16]), words(h[16..24]) ]                          // p1, p2
choiceSeed  = words(h[24..32])
```

(`gen-teams.mjs` uses `SHA-256("numbrion-oracle-v1/teams/" + S + "/" + j)` bytes 0..8.) Fixtures record
all seeds, so a consumer never needs to re-derive them. Output is byte-identical for any `--threads`.

## 2. Flush semantics (BattleStream equivalence)

Showdown's `BattleStream` calls `battle.sendUpdates()` after **every** input line (`>start`, each
`>player`, each `>pN <choice>`). The `Session` class does exactly this: after the constructor, after
each `setPlayer`, and after each `Battle.choose`. Consequences that matter for an engine:

* Requests reach the players only through `sendUpdates()`, and only if the log grew since the last send.
  After a committing choice both players get a request (p1 first): either a real one or
  `{"wait":true,"side":{...}}`.
* The `LINE LIMIT EXCEEDED` guard in `singleEvent` (more than 1000 unsent lines) therefore counts
  lines since the last decision boundary. The largest step seen is reported by the generator summary.
  An engine should treat that as a harness-level crash, not mimic it.
* `replayViaBattleStream` (run for every `--verify-replay` fixture) feeds the recorded inputs into the
  real `BattleStream` class and checks that its `update` lines, per-side `sideupdate` messages
  (requests and `|error|`) and `end` message equal the fixture. So `Session` is checked to be equivalent to
  what a real server does.

## 3. Fixture format (version 1)

One JSON object per line. Key order is as listed. All numbers are integers.

```jsonc
{
  "v": 1,
  "format": "gen9randomdoublesbattle",
  "oracle": "7332b60e22b9e8194bb53549549eba241d73cc9a",   // Showdown commit
  "runSeed": 1,                                            // S
  "index": 17,                                             // battle number i within the run
  "battleSeed": [a, b, c, d],                              // initial Gen5 seed of Battle.prng
  "teamSeeds": [[..4..], [..4..]],                         // team generator seeds (p1, p2)
  "choiceSeed": [..4..],                                   // seed of the choice-picking PRNG
  "choicePolicy": { "switchProb": 0.1, "teraProb": 0.15, "implicitPassProb": 0.3, "revivePassProb": 0.05 },
  "players": [ {"id": "p1", "name": "Alice"}, {"id": "p2", "name": "Bob"} ],
  "teams": [ "<Teams.pack string p1>", "<Teams.pack string p2>" ],   // the EXACT team inputs
  "steps": [ Step, ... ],
  "end": End,
  "anomalies": [ "..." ]                                   // only present if the harness saw something unexpected; should never appear
}
```

`teams[k]` is passed verbatim as `team` to `setPlayer` (`p1` = `players[0]`). Player names are part of
the log (`|player|p1|Alice||`, `|win|Bob`, `|-sidestart|p2: Bob|...`). Avatar and rating are empty.

### Step: one decision boundary

A boundary is a moment where at least one side has a `move` or `forceSwitch` request. Step 0 is the
state right after the battle starts (turn 1, nothing chosen yet).

```jsonc
{
  "log":   [ "|t:|", "|gametype|doubles", ... ],   // battle.log entries appended since the previous boundary
  "turn":  3,                                       // battle.turn at the boundary
  "state": "move" | "switch",                       // battle.requestState
  "seed":  [a, b, c, d],                            // battle PRNG seed at the boundary, before any choice
  "requests": { "p1": Request, "p2": Request },     // exactly as sent (see below)
  "choices": [ Choice, ... ],                       // every Battle.choose call of this boundary, in call order
  "submitted": { "p1": "move 1 1, move 2", "p2": null }   // the accepted input per side; null for a waiting side
}
```

* **`log`**: entries of `battle.log` exactly as Showdown stores them, including the omniscient
  `|split|pN` marker followed by the secret line and the public line (the three entries are separate
  array items; an entry can be the empty string). For step 0 it starts at the first line of the
  battle (`|t:|`, `|gametype|doubles`, `|player|...`). For step `k+1` it is the output of committing
  step `k`'s choices (it normally starts with `|`, `|t:|`). It is the part of the log that was *produced
  to reach this boundary*. The stream seen by a client (the `update` messages) is identical; this
  is asserted for every battle (a mismatch lands in `anomalies`).
* **`|t:|` normalization**: Showdown emits `|t:|<unix seconds>` in the Battle constructor and at the
  start of every `turnLoop` call (so also when a mid-turn switch request is answered). Every entry
  matching `^\|t:\|\d+$` is rewritten to the bare `|t:|`. Nothing else is touched. Positions of these
  lines are part of the structure and are kept.
* **`seed`**: `battle.prng.getSeed()` parsed. `steps[k+1].seed` is the seed after committing step `k`;
  `end.seed` is the seed after the last commit. Choice parsing, rejections and request creation never
  draw from it.
* **`requests`**: the JSON text of what the server would send in `|request|` sideupdates, parsed. Key order
  is the order Showdown serializes (see `docs/showdown/02-turn-loop-and-choices.md` section 4) and is
  preserved on re-serialization, so a replayer may compare requests as JSON text. A side that cannot
  act has `{"wait":true,"side":{...}}` (this is what Showdown sends; it is not `null`). A request is `null` only
  when the battle is over, which cannot happen inside a step. Request kinds:
  `{"active":[..2..],"side":{..},"noCancel"?:true}` (move), `{"forceSwitch":[bool,bool],"side":{..},"noCancel"?:true}`
  (switch), `{"wait":true,"side":{..}}`. The harness checks at every boundary that the text sent by
  `sendUpdates()` equals `JSON.stringify(side.activeRequest)`.
* **`choices`**: the ordered list of `Battle.choose(side, input)` calls for this boundary, *including
  rejected ones*. Sides that are waiting are never called. Which acting side goes first is random (it
  comes from the choice PRNG; with both sides acting it is a fair coin). The call that completes the
  last acting side commits the turn (`commitChoices`), after which the next step begins.

```jsonc
{ "side": "p1", "input": "move 4 2, switch 3", "ok": true }
{ "side": "p2", "input": "move 1 1, switch 4", "ok": false,
  "error": "[Unavailable choice] Can't switch: The active Pokémon is trapped",     // text after "|error|"
  "request": { ...request re-sent by Showdown, with "update":true... } }            // only if Showdown re-sent it
```

  A rejection leaves the battle log and PRNG untouched. A side's earlier accepted choice stays stored
  while the other side is still choosing. After a rejection the picker works from the re-sent request when
  there is one, else from the old request. `ok:false` entries carry `error` and optionally `request`.

### End

```jsonc
{ "winner": "p1" | "p2" | null, "tie": false, "turns": 23, "seed": [..4..], "pokemonLeft": [0, 3],
  "log": [ "|", "|t:|", ..., "|", "|win|Bob" ] }
```

`log` is the final delta (since the last boundary). A tie ends with `|tie`. `turns` is `battle.turn`.

### Replaying a fixture in an engine

```
engine = new Engine(battleSeed, teams[0], teams[1], names)
assert engine.log == steps[0].log  (after |t:| normalization)
for step in steps:
    assert engine.seed == step.seed; engine.requests == step.requests; engine.state == step.state
    for c in step.choices:
        r = engine.choose(c.side, c.input)          // in this order
        assert r.ok == c.ok; (optionally) r.error == c.error; r.updatedRequest == c.request
    // the last accepted choice commits the turn
    next step.log == the log lines the engine produced during the commit
assert end.log, end.seed, end.winner/tie, end.turns, end.pokemonLeft
```

`tools/oracle/lib/session.mjs: replayFixture` is the reference implementation of this loop against
Showdown itself.

## 4. Choice policy and the legal-choice enumerator

`tools/oracle/lib/choices.mjs` builds choices **only from the request JSON** (plus the format rules).
Per active slot (`move N [target] [terastallize]`, `switch N`, `pass`), joined with `", "`:

* a slot whose Pokémon is fainted (`condition` ends in ` fnt`) or `commanding` cannot act and passes. The
  `pass` is written explicitly, or omitted with probability `implicitPassProb`: Showdown auto-passes
  those slots (`getChoiceIndex`), so `move 1 1` alone is legal when slot a is fainted. Slots that are
  not flagged in a `forceSwitch` request are treated the same way. An omission is only equivalent when no
  explicit `pass` follows: a `pass` token never auto-passes, it consumes the *next* slot in order
  (`Side.choosePass` calls `getChoiceIndex(true)`), so `"pass"` alone for a forced pass of slot b is read as
  the pass of slot a and fails with "Incomplete choice". See `canOmitPass`; `check-choices.mjs` tests both
  directions.
* every enabled move (`disabled` false) is usable. Target types `normal`, `any`, `adjacentAlly`,
  `adjacentAllyOrSelf`, `adjacentFoe` need a target location from `Battle.validTargetLoc` for doubles
  (`1`/`2` foe slot a/b, `-1`/`-2` own slot a/b; `any`/`normal`: 1, 2 and the ally slot; `adjacentFoe`:
  1, 2; `adjacentAlly`: ally slot; `adjacentAllyOrSelf`: ally slot and own slot). A target may be a
  fainted or empty slot (Showdown accepts it). All other target types (`self`, `all`, `allAdjacent*`,
  `allySide`, `foeSide`, `randomNormal`, ...) take no target.
* a **locked move** (Outrage, a charging or semi-invulnerable move, Recharge) shows a single entry without
  `target` and the slot has `trapped: true`; it is chosen as plain `move 1`. Struggle (`target:
  "randomNormal"`) is `move 1`. Choosing a locked move by *name* would need a target, so the picker
  always uses indexes.
* `canTerastallize` in the slot's request allows the `terastallize` suffix on a move, for at most one
  slot per side per turn, and not at all when any Pokémon of the side already has `terastallized` set.
* switch targets are bench Pokémon (`active:false`) that are not fainted, not already chosen by the other
  slot, and only if the slot is not `trapped:true`. `maybeTrapped` still allows switching (Showdown may
  reject it, see below).
* `forceSwitch` requests: with `out` flagged slots and `in` unfainted bench Pokémon, exactly `min(out,in)`
  slots switch and the other flagged slots must `pass` (so `"switch 3, pass"`, `"pass, switch 3"`).
  Revival Blessing (`reviving: true` on the active Pokémon) switches in any **fainted** party member,
  including a fainted Pokémon sitting in an active slot, and may `pass` only when no unfainted bench
  Pokémon exists.
* the picker is two-stage: with probability `switchProb` a slot with a legal switch switches; otherwise a
  move is picked uniformly, then one of its targets uniformly; `terastallize` is added with probability
  `teraProb` when offered. Slots are processed in random order (so the Tera and bench-target
  constraints are not biased towards slot a).

`check-choices.mjs` validates all of this against Showdown: at every boundary of N battles it throws
about 6000 brute-force choice strings per side (`pass`, `move 1..5` with every target token -3..3 with and
without `terastallize`, `switch 1..7`) at `Side.choose()` and compares the set Showdown accepts with
`enumerateAll(request)`; the two sets must be equal. It also checks that dropping the `pass` of auto-passing
slots is equivalent and that the picker only produces enumerated choices.

### Rejections

Showdown rejects a choice by returning `false` from `Battle.choose` and sending `|error|[Invalid choice] ...`
or, when the rejection leaks no more than the player is entitled to learn, `|error|[Unavailable choice] ...`
followed by a re-sent request (`"update":true` appended). The generator accepts only **hidden-information**
rejections: `[Unavailable choice]` with a request update where the changed slot carried `maybeTrapped`
(`Can't switch: The active Pokémon is trapped`) or `maybeDisabled`/`maybeLocked`
(`Can't move: <mon>'s <move> is disabled`). Everything else is counted as UNEXPLAINED and makes the run
fail (exit status 2): that would be an enumerator bug. The summary prints rejection counts by reason.

## 5. Things the engine author must know (observed on the oracle)

* **Setup draws.** Nothing draws from the battle PRNG in `setPlayer` (all generated sets carry a gender; the
  generator prints how many sets would draw, always 0). The first boundary's `seed` still differs from
  `battleSeed` in about 5% of battles (105 of 2000): speed ties among the four leads (`insertChoice` /
  `speedSort`), tied `SwitchIn` handlers and so on, as `docs/showdown/02-turn-loop-and-choices.md` section 2.7
  describes. Step 0's `seed` is the number to match.
* **Requests.** Every boundary sends a request to both sides (p1 first): a real one or `{"wait":true,"side":{..}}`.
  `noCancel:true` is present on the real request whenever exactly one side has one (mid-turn U-turn style
  switches, one-sided end-of-turn replacements), never on a wait request. A fainted active keeps an entry in
  `active[]` with its full move list. Locked entries are `{"move":"Outrage","id":"outrage"}` without
  `pp`/`target`/`disabled` and the slot has `"trapped":true` (also Recharge). Struggle is
  `{"move":"Struggle","id":"struggle","target":"randomNormal","disabled":false}`. Creating a request mutates
  state (`getMoveRequestData` sets `trapped`/`maybeX`), see spec section 4.4.
* **Continuations.** After a mid-turn switch request is answered, `turnLoop` starts again: the next step's `log`
  begins with `|`, `|t:|` again and `turn` is unchanged. A turn can therefore span several steps. Replacements
  after faints at the end of a turn are requested before `endTurn`: the step that leads to the request usually ends with
  the residual lines and `|upkeep`, and `|turn|N+1` arrives in the step that answers the replacement request.
* **No request after the end.** The final committing choice produces the `end` log (`|` `|win|Name`) and no
  further request. In 3000+ random battles no tie occurred: when both sides run out on the same action,
  `checkWin` awards the win to the side of the last fainted Pokémon; `|tie` only comes from the turn limit
  or `forcetie`.
* **Choice parsing quirks the picker works around.**
  * A `pass` token never auto-passes; it fills the *next* slot. Omitting the `pass` of a non-acting slot is only
    safe if no explicit `pass` follows (see section 4).
  * A locked move chosen by name (`move outrage`) is "needs a target"; `move 1` is right. Spread, self and
    field moves must be sent without a target; targeted moves must have one.
  * Target locations are validated without looking at who is there, so a fainted or empty slot can be targeted.
  * Not generated, but real: choosing a request-disabled move is `[Unavailable choice]`, Showdown adds
    `"disabledSource":""` to that move entry and `"update":true` to the request and re-sends it; `terastallize`
    on a locked move or Struggle is accepted and silently ignored; unknown choice words are reported but parsing
    continues (spec 5.2).
* **Rejections are free of side effects on the battle log and PRNG** (asserted by every replay), but a rejected
  choice may have mutated `maybeX` state while the request was rebuilt. Fixtures contain the rejected calls so
  an engine can reproduce the hidden-information handling: in 2000 default-policy battles all 35 rejections
  were `[Unavailable choice] Can't switch: The active Pokémon is trapped` (a foe's Shadow Tag / Arena Trap /
  Magnet Pull is `maybeTrapped`, 33 times) or `Can't move: <move> is disabled` (Imprison, `maybeDisabled`, 2 times).
* **Revival Blessing** (`reviving:true` on the user in a `forceSwitch` request) can bring back any fainted party
  member, including a fainted Pokémon still sitting in the other active slot. With no unfainted bench Pokémon the
  user may `pass` instead; the request is then re-issued after every remaining action of the turn (the slot
  condition and `switchFlag` stay set). The picker takes that path with probability `revivePassProb`.
* **Size of a step.** The largest single step seen has 72 log lines in 2000 battles (Showdown's LINE LIMIT is
  1000 unsent lines); battles have about 19 turns and 25 boundaries on average (max 73 turns / 78 boundaries).
