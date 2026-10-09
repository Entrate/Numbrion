# Directed oracle corpora and handler coverage

These tools use the pinned Showdown checkout from `tools/oracle/ORACLE_COMMIT`
(currently `7332b60e22b9e8194bb53549549eba241d73cc9a`). They require Node.js and
the checkout's built `dist/` tree; no npm dependencies are added to this project.
The default checkout is `~/src/pokemon-showdown`; override it with `--ps PATH`.

## Generate and verify battles

```sh
node tools/oracle/gen-directed.mjs --list
node tools/oracle/gen-directed.mjs --selftest
node tools/oracle/gen-directed.mjs --profile slice0 --count 100 --threads 2 \
  --out /tmp/slice0.jsonl.gz --verify --summary-json /tmp/slice0-summary.json
node tools/oracle/gen-directed.mjs --profile batch:weather_terrain --count 100 \
  --threads 2 --out /tmp/weather.jsonl.gz --verify
node tools/oracle/gen-directed.mjs --profile effect:abilities:intimidate --count 20 \
  --out /tmp/intimidate.jsonl.gz --verify
node tools/oracle/gen-directed.mjs --verify-file /tmp/weather.jsonl.gz --threads 2
```

| Profile | Content |
| --- | --- |
| `slice0` | Plain damaging moves, no items, abilities restricted to inert families. |
| `slice1` | Adds declarative statuses, flinch, boosts, secondaries, recoil and drain. |
| `slice1x` | Adds healing, self/forced switches and self-destruct. |
| `slice2` | Adds Protect, confusion, common items and Intimidate/Pressure/Levitate/Regenerator/Adaptability. |
| `batch:NAME` | Slice 1 plus effects assigned to a group in `EFFECT-BATCHES.json`. |
| `effect:KIND:ID` | Slice 1 plus a specified effect and available creators. |

Append `+tera` to enable Terastallization. The formes/Tera batch enables it itself.
Rules and core battle mechanics apply to every profile. Expanded batch/effect
profiles explicitly admit Levitate: it has no callbacks, but Ground immunity and
terrain grounding remain part of the core engine. Slice profiles suppress moves
that would activate their otherwise inert ability families.

These are constructed teams for testing mechanics. Species, levels and abilities
come from the random-doubles census, but moves and items may be combined with
species outside their actual random-team sets. Summaries count these loosened
move assignments. Condition creators that fall outside the profile are listed as
unreachable; focus placement can also be skipped when no suitable species exists.
Placement does not guarantee that a callback runs. Measure execution with coverage.

Every battle is determined by profile, run seed and index. Default seeds are listed
by `--list`; override them with `--seed`, and partition a run with `--start-index`.
Worker count does not change fixture ordering. Outputs use the fixture schema in
`FIXTURES.md`, adding a `profile` string. `--verify` rebuilds the teams, checks the
profile's content constraints, and replays logs, requests, results and PRNG states
both directly and through Showdown's BattleStream. The self-test compares eight
ordinary random battles with the shared driver byte for byte. Exit code 2 reports
a crash, unexplained rejection, anomaly or failed verification.

## Measure reached callbacks

```sh
node tools/oracle/coverage.mjs data/fixtures/sample-50.jsonl.gz /tmp/weather.jsonl.gz \
  --threads 2 --json /tmp/coverage.json
node tools/oracle/coverage.mjs /tmp/weather.jsonl.gz --threads 2 \
  --by-fixture=/tmp/by-fixture.jsonl.gz --find abilities:drizzle --top 3
```

Function callbacks are wrapped with counters. Constant callbacks are counted when
the dispatcher collects them. Declarative moves are counted from protocol move
lines. The report includes invoked/missing handlers and per-batch totals; JSON
retains counts for each handler and effect. `--find` lists the smallest observed
battles invoking an effect or handler. `--limit N` restricts each input file.

Coverage also checks replay identity, so instrumentation must preserve the recorded
battle. For directed fixtures, coverage replays the recorded teams; use the
generator's verification mode to independently check team reconstruction and
profile constraints. Reached callbacks show oracle coverage, not Rust parity or
proof that every branch inside a callback was tested.

The original 2,000-battle coverage run is preserved in
`reports/oracle-coverage-2000.json` and `reports/oracle-coverage-2000.txt`.
Its full fixtures and per-battle index are retained in the local recovery archive.

`data/fixtures/directed-smoke.jsonl.gz` contains the 384-battle recovery check:
16 battles for each of four slices, all 18 batches and two single-effect examples.
Revalidate it with `gen-directed.mjs --verify-file`; per-profile verification and
unreachable/placement notes are in `reports/recovery-validation.json`.
