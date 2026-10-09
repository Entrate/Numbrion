# difftest: the differential-test harness

`crates/difftest` replays the Showdown oracle fixtures (`docs/design/FIXTURES.md`) through an engine and
reports, per battle, the **first** place where the engine differs from Showdown, grouped by a normalized
*signature* so the most frequent bug class is always on top. It is a separate cargo workspace (own
`[workspace]`, own `Cargo.lock`, own `target/`) because it has registry dependencies (`serde`,
`serde_json`, `flate2`); the root workspace excludes it.

```
cd crates/difftest
cargo build --release -j 4
./target/release/difftest replay ../../data/fixtures/sample-50.jsonl            # the real engine
./target/release/difftest replay ../../data/fixtures/sample-50.jsonl --sim mock # perfect engine: must pass
cargo test                                                                       # unit tests + fault-injection battery
```

Features: `engine` (default) links `../engine` for `--sim engine`; `cargo build --no-default-features` builds
the harness without the engine (useful while the engine is mid-edit).

## 1. What the engine must provide

The harness talks to an engine only through the `Sim` trait in `crates/difftest/src/sim.rs`. The adapter
`crates/difftest/src/engine_sim.rs` (`impl Sim for EngineSim`) is currently a **stub**; filling it in is the
engine author's job. Everything below is what the engine API has to expose so that the adapter is a few
lines per method.

```rust
pub trait Sim: Sized {
    fn start(seed: [u16; 4], p1: (&str, &str), p2: (&str, &str)) -> Result<Self, String>;
    fn start_fixture(fx: &Arc<Fixture>) -> Result<Self, String> { /* default: calls start() */ }   // keep default
    fn seed(&self) -> [u16; 4];
    fn turn(&self) -> Option<u32> { None }                         // optional extra check
    fn drain_log(&mut self, out: &mut Vec<String>);
    fn request_json(&self, side: usize) -> Option<String>;
    fn choose(&mut self, side: usize, input: &str) -> Result<(), Rejection>;
    fn ended(&self) -> Option<Outcome>;
}
pub struct Rejection { pub error: String, pub request: Option<String> }
pub struct Outcome  { pub winner: Option<usize>, pub tie: bool, pub turns: u32, pub pokemon_left: [u32; 2] }
```

| method | what Showdown does / what the engine must return |
|---|---|
| `start(seed, (name1, team1), (name2, team2))` | `new Battle({formatid:'gen9randomdoublesbattle', seed})`, `setPlayer('p1', {name, team})`, `setPlayer('p2', ...)`. The battle starts on the second `setPlayer`; afterwards the first decision boundary (turn 1, nothing chosen) is pending. `seed` is the Gen5 seed words, most significant first (`Prng::from_seed`). `team` is the exact packed string from the fixture. **Player names are part of the log** (`\|player\|p1\|Alice\|\|`, `\|-sidestart\|p2: Bob\|...`, `\|win\|Bob`) and of the requests (`side.name`), so the engine's start must accept names. `Err` for anything unexpected (unknown species in a team, ...). |
| `seed()` | `battle.prng.getSeed()` as four u16 words (`Prng::seed`). Compared at every boundary and at the end. |
| `turn()` | `battle.turn`. Optional; return `Some` to enable a per-boundary turn check. |
| `drain_log(out)` | Append every `battle.log` entry emitted since the previous call, in Showdown's exact entry form, in order, then clear. One entry per `Vec` item, **not** one per text line of a multi-line blob. `\|split\|p1` + secret line + public line are three entries. Entries can be `""` or `"\|"`. `\|t:\|<unix>` may carry any timestamp or none: the harness applies the oracle's normalization (`^\|t:\|\d+$` to `\|t:\|`) to the engine output. The first call (right after `start`) must return the whole opening log from `\|t:\|` through `\|turn\|1`; later calls return what the committing choice produced (it normally starts with `\|`, `\|t:\|`), including the final `\|`, `\|win\|Name`. |
| `request_json(side)` | The request pending for the side, serialized exactly as `JSON.stringify(side.activeRequest)`: key order and value types exactly as Showdown writes them (`active`/`forceSwitch`/`wait`, `side`, `noCancel`, `update`, ...). A side that cannot act has `{"wait":true,"side":{...}}`, never `None`. Both sides have a request at every boundary. Whitespace does not matter (values are compared as JSON); key order matters (reported as a *warning* class, see below). `None` only once the battle is over. |
| `choose(side, input)` | `Battle.choose(side, input)`. `Ok(())` if accepted. When the call completes the last acting side, the turn is committed and run synchronously until the next boundary or the end. `Err(Rejection)` if rejected: `error` is the text after `\|error\|` **including the tag** (`[Invalid choice] ...` / `[Unavailable choice] ...`), and `request` is `Some(json)` iff Showdown re-sent the request to that side (hidden-information rejections: `[Unavailable choice]` with `"update":true`), else `None`. A rejection must not emit log entries or draw from the PRNG (the harness checks it), but it may mutate `maybeTrapped`/`maybeDisabled` request state exactly like Showdown. Sides that are waiting are never called. |
| `ended()` | `Some(Outcome)` once the battle has ended: `winner` (`Some(0|1)`, `None` for a tie), `tie`, `turns` = `battle.turn`, `pokemon_left` = `side.pokemonLeft` per side. After the end `request_json` must return `None` for both sides. |

Suggested adapter shape (pseudo-code: `set_names`, `start`, `request_json`, `outcome` etc. are what the engine API
should offer; each method is a one-liner once the engine has a text log sink):

```rust
pub struct EngineSim { battle: engine::Battle<TextLog>, cursor: usize }
fn start(seed, p1, p2) -> Result<Self, String> {
    let mut battle = engine::Battle::with_log(seed, p1.1, p2.1, TextLog::default()).map_err(|e| e.to_string())?;
    battle.set_names(p1.0, p2.0);       // names must be set before start (they appear in the opening log)
    battle.start();                      // emits the opening log, creates the first requests
    Ok(EngineSim { battle, cursor: 0 })
}
fn drain_log(&mut self, out: &mut Vec<String>) { out.extend(self.battle.log.entries[self.cursor..].iter().cloned()); self.cursor = self.battle.log.entries.len(); }
fn seed(&self) -> [u16; 4] { self.battle.state.prng.seed() }
fn request_json(&self, side: usize) -> Option<String> { self.battle.request_json(side) }
fn choose(&mut self, side: usize, input: &str) -> Result<(), Rejection> { self.battle.choose(side, input).map_err(|e| Rejection { error: e.text, request: e.resent_request_json }) }
fn ended(&self) -> Option<Outcome> { self.battle.outcome() }
```

(`TextLog` = a `LogSink` that formats each `LogEntry` into the Showdown line, `split_side` producing the
three entries, and pushes it to a `Vec<String>`; the `drain_log` cursor makes `drain_log` O(new lines).)

Engine behaviours the harness relies on (all documented in FIXTURES.md): panic loudly on anything
unimplemented (`todo!()`, `unreachable!()`, capacity overflow). A `Sim` panic is **caught per battle** and
reported as a `panic` divergence whose signature is the panic location and message
(`panic @crates/engine/src/effects/moves/foo.rs:42: not yet implemented`); that is the intended way to
discover missing hooks. Infinite loops cannot be caught: a watchdog aborts the run (exit status 3) naming the
battle if one battle exceeds `--battle-timeout` seconds (default 120).

## 2. What a replay compares

`replay_fixture` follows the loop in FIXTURES.md section 3. At boundary `k` (step `k` of the fixture) the
checks run in this order and the first failing one is the battle's divergence:

1. **log**: `drain_log` output (normalized) vs `steps[k].log`, exact string equality, line by line.
2. **not ended**: `ended()` must be `None`.
3. **turn**: `turn()` (if provided) vs `steps[k].turn`.
4. **seed**: `seed()` vs `steps[k].seed`.
5. **requests**: `request_json(0|1)` vs `steps[k].requests.p1|p2`. Fast path: identical text. Otherwise both are
   parsed and compared as JSON *values* (key order ignored); if the values are equal but the key order differs
   it is a **warning** (`request key_order <path>`), not a failure (use `--strict-order` to fail on it).
6. **choices**, in fixture order, every `choose` call including rejected ones: accept/reject, the error text
   (exact), whether a request was re-sent and its content (as 5). A call that does not commit the turn (any
   rejected call, or an accepted call that is not the last of the step) must not emit log entries
   (`choice log_before_commit`) or move the PRNG (`choice seed ...`).

After the last step: **end log** (vs `end.log`), **end seed**, **outcome** (`winner`, `tie`, `turns`,
`pokemonLeft`; `ended()` must be `Some`), and no request pending for either side.

Statistics per battle: steps fully matched (`steps_ok`), log lines matched. A passing battle matched every
boundary and the end record.

## 3. Failure signatures

Every failure carries `category` and `signature`. The report ranks signatures by how many battles have it as
their first divergence; examples are listed smallest-repro first (`battle@step`, lowest step number first).

| category | signature examples | meaning |
|---|---|---|
| `fixture` | `fixture: unusable line` | unparsable / unsupported fixture line |
| `start` | `start: <Sim::start error text>` | `Sim::start` returned `Err` (digits in the text are normalized to `N`) |
| `panic` | `panic @crates/engine/src/x.rs:42: <message>` | the Sim panicked (location and first message line, digits normalized) |
| `log` | `log \|-damage\| arg2`, `log \|move\| -tag[miss]`, `log \|-heal\| +tag[from]`, `log \|-damage\| tag[from]`, `log expected \|-damage\| got \|-heal\|`, `log missing \|-miss\|`, `log extra \|-crit\|` | first mismatching line. Computed from a small LCS diff of the following 40 lines: a pure deletion is `missing <kind>` (engine skipped an expected line), a pure insertion is `extra <kind>`, otherwise the two lines are compared token by token: other kind, `argN` (N-th argument after the kind), `+tag[x]`/`-tag[x]` (tag only in actual / only in expected), `tag[x]` (same tag, different text). Kind = first token, e.g. `\|-damage\|`. |
| `seed` | `seed +1 draws`, `seed -3 draws`, `seed +10 or more draws`, `seed unrelated` | the log matched but the PRNG differs. `+n`: the engine drew `n` more times than Showdown (extra `random` call), `-n`: fewer. `unrelated`: not reachable by LCG steps either way. The last matching log lines show which action is nearby. |
| `turn` | `turn` | `battle.turn` differs |
| `request` | `request.active[].moves[].disabled`, `request.side.pokemon[].condition`, `request.kind expected=wait actual=move`, `request.active[].trapped (missing)`, `request.side.pokemon (length)`, `request missing`, `request invalid_json`, `request key_order <path>` (strict only) | first differing field of the request, in expected-key order. `(missing)` = key absent in actual, `(unexpected)` = key only in actual, `(length)` = array length. Array indices are replaced by `[]` (use `--keep-indices` for exact `moves[2]`); the exact path is always in the message. |
| `choice` | `choice unexpected_reject [Invalid choice]`, `choice unexpected_accept [Unavailable choice] Can't switch`, `choice error_text`, `choice.request not_resent`, `choice.request unexpected_resend`, `choice.request.<field path>`, `choice log_before_commit`, `choice seed +1 draws` | see section 2, item 6 |
| `end` | `end premature`, `end not_ended`, `end.winner`, `end.tie`, `end.turns`, `end.pokemon_left`, `end.seed +1 draws`, `end.request_after_end` | outcome checks (log differences in the end record are `log ...` signatures with `step == step_count`) |

Warnings never fail a battle: `request key_order <path>` and `fixture has anomalies`.

## 4. CLI

```
difftest replay <fixtures.jsonl|.jsonl.gz|-> [options]
  --sim engine|mock      engine (default when built in) or the fixture-replaying mock
  --limit N  --skip N    first N battles / skip the first N (positions, 0-based, blank lines ignored)
  --battle SPEC          only these positions: 3  or  3,7,10-20   (a fixture's position equals its `index` for a fresh run)
  --stop-on-first        stop at the first failing battle (with --jobs > 1 a few more may already be in flight)
  --jobs K               worker threads (default: all cores)
  --json                 one JSON document on stdout (see below)
  --show N               failing battles printed in detail (default 5)      --sig TEXT  only those whose signature contains TEXT
  --top N                rows of the signature table (default 40)           --context N matching log lines of context (default 15)
  --strict-order         request key-order differences fail the battle
  --keep-indices         signatures keep array indices
  --full                 whole expected/actual request texts in request divergences
  --no-catch-panics      do not convert Sim panics into `panic` divergences
  --failed-out FILE      write failing positions, comma separated:  difftest replay F --battle "$(cat FILE)"
  --battle-timeout SECS  watchdog for a hung Sim (default 120, 0 = off; exit status 3)
  --corrupt SPEC         (mock) inject a fault into every battle
  --quiet                no progress line
difftest stats    <input> [--limit N --skip N --battle SPEC --jobs K --top N --json]
difftest selftest [input.jsonl] [--limit N]
```

Input is streamed line by line through a bounded channel (peak memory about 10-20 MB for the 335 MB
`fuzz-2000.jsonl`); gzip is detected from the magic bytes, so `.jsonl.gz` and `zcat f | difftest replay -`
both work. Exit status: 0 all passed, 1 some failed, 2 usage/I/O error, 3 hung battle.

Typical bug-fix loop:

```
difftest replay fuzz-out/fuzz-2000.jsonl --show 1          # top of the table = most frequent first divergence
difftest replay fuzz-out/fuzz-2000.jsonl --sig '|-damage| arg2' --show 3 --jobs 4
difftest replay fuzz-out/fuzz-2000.jsonl --battle 17 --context 40 --full --jobs 1   # one battle, more context
difftest replay fuzz-out/fuzz-2000.jsonl --failed-out /tmp/failed.txt --quiet; difftest replay fuzz-out/fuzz-2000.jsonl --battle "$(cat /tmp/failed.txt)"
```

Human report per failing battle: signature, where (phase, step, turn, log line), progress (steps matched
before it), the choices submitted at the previous boundary ("led by"), the three seeds (previous boundary,
expected, actual) with the draw offset, a message with the exact mismatching field, the last 15 matching log
lines, and a `- expected / + actual` diff of the next lines. Then the summary: pass/fail counts, steps matched,
throughput, first-divergence categories, and the signature table.

### JSON (`--json`)

```jsonc
{ "version": 1,
  "summary":    { "input", "sim", "jobs", "total", "passed", "failed", "battles_with_warnings",
                  "steps_total", "steps_matched", "log_lines_matched", "wall_secs", "cpu_secs", "battles_per_sec" },
  "categories": [ { "category": "log", "count": 12 }, ... ],                 // sorted by count
  "signatures": [ { "signature", "category", "count", "examples": [ {"pos", "step", "turn"} ] }, ... ],  // sorted by count
  "warnings":   [ { "signature", "battles", "examples": [pos] } ],
  "failures":   [ { "pos", "index", "run_seed", "battle_seed", "steps", "steps_ok", "log_lines_ok", "micros",
                    "warning_count", "warnings": [..],
                    "divergence": { "category", "signature", "phase", "step", "step_count", "turn", "choice", "side", "input",
                                    "line", "message", "expected", "actual", "diff": [..], "context": [..], "prev_choices": [..],
                                    "seed_prev", "seed_expected", "seed_actual", "seed_note", "expected_full", "actual_full" } } ] }
```

`failures` lists every failing battle (sorted by `pos`); passing battles are only counted.

## 5. The mock and the self-test

`MockSim` (`src/mock.rs`) replays the fixture itself: the perfect engine. `difftest replay F --sim mock` must
pass 100 % (it does for `sample-50.jsonl` and for all 2000 battles of `fuzz-2000.jsonl`). It also carries a
fault injector, `--corrupt SPEC` (mock only), which makes the mock deviate in one controlled way and so proves
the harness notices it at the right place:

```
start-error | panic:STEP | hang:STEP | log-edit:STEP:LINE | log-drop:STEP:LINE | log-insert:STEP:LINE   (STEP may be `end`)
seed:STEP[:DRAWS] | turn:STEP | request:STEP:SIDE[:PATH] | request-order:STEP:SIDE | request-kind:STEP:SIDE
request-none:STEP:SIDE | choice-flip:STEP:N | choice-error:STEP:N | choice-no-resend:STEP:N
choice-resend-edit:STEP:N[:PATH] | choice-early-log:STEP:N | choice-early-seed:STEP:N
end-winner | end-turns | end-left | end-seed | end-premature:STEP | end-never | end-request
```

`difftest selftest [fixtures]` (and `cargo test`) run a battery of about 30 of these on a hand-built synthetic
fixture and on real fixtures, asserting category, step, signature prefix, choice index and log line of the
reported divergence (and that the unmodified mock passes with zero warnings). A corruption that does not apply
to a fixture (step out of range) surfaces as a `start:` failure rather than a silent pass.

## 6. Layout

```
crates/difftest/
  Cargo.toml        own [workspace]; features: engine (default)
  src/sim.rs        Sim trait, Outcome, Rejection
  src/fixture.rs    typed fixture (requests kept as RawValue text), |t:| normalization
  src/replay.rs     the per-battle comparison loop, Divergence/BattleResult, panic capture
  src/diff.rs       log LCS diff, token diff, JSON diff, signatures
  src/seed.rs       Gen5 LCG arithmetic (draw-count explanation of seed divergences)
  src/runner.rs     streaming reader (plain/gzip/stdin) + thread pool
  src/driver.rs     run_replay: runner + aggregation + watchdog
  src/report.rs     signature aggregation, human and JSON rendering
  src/stats.rs      `difftest stats`
  src/mock.rs       MockSim + Corruption
  src/selftest.rs   fault-injection battery + synthetic fixture
  src/engine_sim.rs STUB: impl Sim for EngineSim (engine author fills in)
  src/main.rs       CLI
```

## 7. TODO

* `difftest minimize`: shrink a failing battle to a minimal repro (drop steps/turns from the front by
  re-simulating from a state snapshot, or shorten teams) and emit a standalone fixture. Not needed yet; needs
  engine state snapshots (`BattleState` is `Copy`, so cloning at a boundary should make "resume from step k" cheap).
* An engine-side oracle for choices: `Sim::choose` rejections are only compared on the recorded calls, there is
  no brute-force legal-choice enumeration against the engine (the oracle has one: `tools/oracle/check-choices.mjs`).
* A per-battle `--sim-timeout` that kills a single hung battle without aborting the whole run (would need the
  Sim on its own thread).
