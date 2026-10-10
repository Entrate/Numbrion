# Owner T (textlog): report

Branch `worktree-agent-a0ca720a924313ab2`, base `e784f91`. Paths owned and edited:
`crates/engine/src/log/{text.rs,format.rs,battle.rs}` plus the private test tree
`crates/engine/src/log/text/tests/` (declared from `text.rs`). Probes: `tools/probes/textlog/`.
`log/mod.rs` (frozen interface) and everything else are untouched.

## Implemented

| Function | Showdown | Notes |
|---|---|---|
| `TextLog::emit` | battle.ts:3081-3120 | plain entry = one `String`; split entry = `\|split\|pN`, secret line, shared line or `""` (secret-only) |
| `TextLog::edit_move` | battle.ts:3121-3144 | `attrLastMove` / `retargetLastMove` on one absolute `last_move_line`; `[still]` blanks field 4, deletes a `\|-anim\|` line and clears the pointer, short lines are padded like JS arrays |
| `TextLog::drain_into` | battle.ts:3265-3278 | cursor only; history is kept for later edits |
| `Battle::add` | battle.ts:3091-3113 | unchanged (skeleton) |
| `Battle::add_move`, `attr_last_move` | :3115-3144 | counts one logical line / zero lines |
| `Battle::hint` | :3069-3079 | plain hint, `once` (bit `state.illusion_hint`), per-side secret-only split |
| `Battle::emit_opening_log` | :191-316,3253 | resets the count to 0, then adds the four buffered lines: `\|t:\|`, `\|gametype\|doubles`, `\|player\|p1\|<name>\|\|`, `\|player\|p2\|<name>\|\|` |
| `Battle::send_updates` | :3265 | `unsent_lines = 0` for every sink (LINE LIMIT input) |
| `format::{write_ident, write_health, write_details, write_effect, write_arg}` | pokemon.ts:531,544,2060; dex-data.ts:129 | signatures as in the skeleton |
| extra: `format::{write_fullname, write_stored_details}` | pokemon.ts:335,377,1155 | request JSON `ident` / `details` helpers for owner C (always the real mon, `pN: Name`, base-species details, no tera / disguise); verified like the rest |

## Tricky behaviours (all checked against the real Showdown, see below)

* Ident is `getSlot() + fullname.slice(2)` while `ACTIVE`, else `fullname`; the name is `SetDef.name`
  (never the forme); an Illusion disguise substitutes the **disguise's name** but keeps the real mon's
  slot letter and side.
* HP: `0 fnt` (no status) in both views; secret `hp/max`; shared `ceil(100*hp/max)` clamped to 99 while
  `hp < max`; status id appended to both; always evaluated when the line is added; Illusion keeps the
  **real** mon's HP.
* Details: `Species[, L<lvl>][, M|F][, shiny][, tera:<Type>]`; `Greninja-Bond` / `Rockruff-Dusk` print their
  base species; with a disguise the **disguise's** details (its own level, gender, shiny) are printed, the
  tera suffix is the real mon's. The stored `details` is derived from `base_species`, the live
  `getUpdatedDetails()` (used by `replace`) from `species` (Illusion on a transformed active ally prints
  the ally's *transformed* species; seen in directed-smoke).
* `Battle::add_move` / `TextLog` pointer: the sink repoints `last_move_line` for the commands `move` and
  `-anim`, the only `addMove` callers in scope. A nested move therefore steals the pointer (Dancer /
  Magic Bounce: the outer move's later `[spread]` lands on the inner line, real behaviour).
* `[spread] ` with an empty list keeps its trailing space; `LogTag::Value(name, Empty)` prints `[name] `.
* `-0` prints `0`; `LogArg::Decimal` uses ECMAScript `Number::toString`.
* `LogArg::Mon(MonId::NONE)` prints the empty string (null Pokemon), as does `EffectRef::None` /
  `EffectId::NONE`.

## Conventions the other owners must follow (mod.rs carries no docs for these)

| Intent | Encoding |
|---|---|
| `battle.add('x', ...)` | `LogEntry::new("x", args, tags)`; line is `\|x\|arg\|...\|tag\|...` |
| blank `add('')` | `LogEntry::new("", &[], &[])` -> `\|` |
| `add('t:', Date.now())` | `LogEntry::new("t:", &[LogArg::Empty], &[])` -> `\|t:\|` (no-arg form also prints `\|t:\|`) |
| `[from] ${effect.fullname}` | `LogTag::From(effect)` (`move: X`, `ability: X`, `item: X`, `pokemon: X`, bare condition name) |
| `[from] ${effect}` / `.name` (e.g. switch `[from] U-turn`) | `LogTag::Value("from", LogArg::Effect(e))` |
| `[from] <literal>` (`recoil`, `psn` for tox, `confusion`, `drain`, `stealeat`) | `LogTag::Value("from", LogArg::Text(..))` (the **caller** maps tox -> psn, already done in gpt-mutators) |
| `[of] ${source}` | `LogTag::Of(mon)`; a literal `null` source is `Value("of", Text("null"))` |
| `[silent]`, `[eat]`, `[upkeep]` ... | `LogTag::Bare("silent")` |
| `[wisher] Name`, `[move] Bug Bite`, `[anim] Tera Blast Ground` | `LogTag::Value("wisher", Text(..))`; the anim text is `Parts(&[Text("Tera Blast "), Type(t)])` |
| HP-bearing line (`-damage`, `-heal`, `-sethp`) | `LogEntry::split(cmd, &[Mon(m), Health(m)], tags, m.side(), false)` |
| `switch` / `drag` | `LogEntry::split(cmd, &[Mon(m), FullDetails(m)], tags, side, false)` (`FullDetails` = `details\|health`, one part) |
| `detailschange` | `[Mon(m), Details(m)]` -> `(illusion \|\| this).details` + tera suffix; `-formechange` third field `Empty` |
| `replace` | `[Mon(m), Details(m)]`; the sink prints the plain `getUpdatedDetails()` (no disguise, no tera) for the command `replace` only |
| `Side.toString` (`-sidestart`) | `LogArg::Side(side)` -> `p2: Bob`; side id `SideId`; bare name `PlayerName`; status ids `Status(s)`; type names `Type(t)`; species names `Species(id)` |
| `\|move\|`, `\|-anim\|` | `Battle::add_move(LogEntry::new("move", &[Mon(user), Text(name), Mon(target)], &[From(source_effect)]))`. A bare `\|move\|p\|Name` has two args |
| `attrLastMove('[still]')` etc. | `attr_last_move(MoveLineEdit::{Still, Miss, NoTarget})`; `[spread] a,b` -> `MoveLineEdit::Tag(LogTag::Value("spread", LogArg::Spread{mons,len}))` (targets still hit, in `getMoveTargets` order, `len` may be 0); `[anim] ..` / anything else -> `Tag(..)`; `retargetLastMove` -> `Retarget(mon)` |
| `hint(text, once, side)` | `Battle::hint(LogArg::Text(..), once, side)`. `once` is only supported without a side (Illusion Level Mod); `hint` owns `state.illusion_hint`: effect code must **not** set or test it itself |
| player/rule/format header | `emit_opening_log()` writes the first four lines. `start()` (owner L) must add the rest itself: `\|gen\|9`, `\|tier\|[Gen 9] Random Doubles Battle`, the `\|rule\|` lines from the rule Begin hooks, then `\|`/`\|t:\|`, `\|teamsize\|p1\|N`, `\|teamsize\|p2\|N`, `\|start`, the four switch triples, ..., `\|turn\|1` |

`tests/shapes.rs` is a cookbook with one such call per catalog shape.

### Interface notes and requested changes (log/mod.rs is frozen, so nothing was edited)

1. **`MoveLineEdit::Spread(u8)` cannot express the line.** `[spread]` lists `hitSlot` in
   `getMoveTargets` order (`[spread] p2b,p1a,p1b`, battle-actions.ts:614), which a mask cannot carry.
   Implemented as a best-effort packed encoding (bits 0-1 count, then up to three 2-bit field positions
   `side + 2*slot`), tested, but callers should use
   `MoveLineEdit::Tag(LogTag::Value("spread", LogArg::Spread { mons, len }))`. Proposal: delete
   `MoveLineEdit::Spread` (or change it to `Spread { mons: [MonId; 4], len: u8 }`). Existing callers:
   none yet; the future caller is `trySpreadMoveHit` (owner M).
2. Variant docs for `LogArg::{Details, FullDetails, Parts, Decimal, Spread}`, `LogTag::{Value, Text}`
   and `MoveLineEdit` are missing; the table above can be pasted into `mod.rs`.
3. No signature change is required for correctness.

## Verification

All expected values come from the real pinned Showdown (7332b60) or from its recorded logs:

* `tests/state.rs` + `state-vectors.tsv` (1092 rows from `tools/probes/textlog/state-vectors.mjs`, real
  `Pokemon.toString`, `getUpdatedDetails`, `(illusion||this).details+tera`, `getHealth` secret/shared,
  `getFullDetails`, `fullname`, stored `details`): full HP grid on three max-HP values (clamp, ceil, tiny
  HP, faint), every status, tera incl. Stellar, Illusion on every disguise (active/bench, hp, status,
  tera), permanent forme changes, Greninja-Bond.
* `tests/edits.rs` + `edit-vectors.tsv` (600 random op sequences run through the real
  `Battle.addMove/attrLastMove/retargetLastMove`, 32 of them hit the `-anim` + `[still]` deletion).
* `tests/replay.rs`: real fixture logs translated back to `LogEntry` values (idents resolved against a
  state tracked from the log) and replayed through `add`/`add_move`/`attr_last_move`, whole log compared
  byte for byte, plus unsent count and chunked drain. Committed set: 60 battles, 29k entries, chosen by
  greedy set cover to contain **all 168 catalog shapes** (`node tools/probes/textlog/shape-coverage.mjs`
  runs `log-shapes.mjs` over it and reports 0 missing). Ignored test `replay_corpus_from_env`
  (`TEXTLOG_REPLAY=<tsv from tools/probes/textlog/extract-replay.mjs>`) was run over sample-50 +
  directed-smoke + fuzz-2000 (2434 battles, 1,197,410 entries, 181,283 split triples, 142,837 move lines,
  26,956 attr edits, 117 Illusion switch-ins) and fuzz-switchy (1000 battles, 676,886 entries): all
  byte-exact.
* `tests/shapes.rs`: one explicit case per shape; `tests/args.rs`: JS number printing (checked against
  Node), effect name vs fullname for every dex kind and the condition views, ActiveMove effects, tag
  order and forms; `tests/moves.rs`: edit semantics; `tests/counting.rs`: opening log, NoLog/TextLog
  count parity, `send_updates`, drain cursor, hints.

`cargo build -j 2` and `cargo test -j 2` pass (the one ignored test is the env-driven corpus replay).

## Known gaps

* `-anim` + `[still]` deletes a log entry; `scratch.unsent_lines` is not decremented for it (the sink
  cannot reach scratch and NoLog does not know the line kind). Never observed in 3434 battles; the only
  effect would be a one-line overcount of the LINE LIMIT guard.
* Pre-start count is 4 (as specified); real Showdown counts 1 at that point because the harness flushes
  after `>start` and `>player p1`. The 3-line difference only matters within 3 lines of 1000.
* The `Format` / `Confused` / `StruggleRecoil` / `Recharge` / `Fainted` / `MindBlownRecoil` synthetic
  effect names (never printed in the corpus) use the TS ids or the format name; `Synchronize` prints
  `synchronize`.
* `once` hints with a side panic (not reachable in scope).

## Integration notes for the lead

* Owner L: call `emit_opening_log()` first in `start()`, then add `gen`/`tier` etc. as above; call
  `send_updates()` where Showdown's `sendUpdates` runs (harness: after the constructor, each
  `setPlayer`, every input; `commitChoices` when `unsent_lines > 500`). `drain_log` is independent.
* Owner D / effect files: keep tox -> psn in the damage mutator; pass `FullDetails` only in split
  entries (`add` panics on an HP-dependent argument without a split side, which would otherwise print a
  wrong line silently).
* Owner C: `format::write_fullname` and `format::write_stored_details` print the request JSON `ident`
  and `details` exactly like `getSwitchRequestData` (JSON escaping is C's).
* Regenerating vectors: `node tools/probes/textlog/state-vectors.mjs > .../tests/state-vectors.tsv`,
  `node tools/probes/textlog/edit-vectors.mjs > .../tests/edit-vectors.tsv`,
  `node --max-old-space-size=6000 tools/probes/textlog/extract-replay.mjs --out .../tests/replay-vectors.tsv --select 120 data/fixtures/sample-50.jsonl.gz data/fixtures/directed-smoke.jsonl.gz fuzz-out/fuzz-2000.jsonl fuzz-out/fuzz-switchy.jsonl`.
