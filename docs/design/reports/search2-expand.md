# Fused two-ply expansion

Implementation on `feat/search2-expand`, based on `search2/base` and its final review decisions.

## API

```python
out = grid.expand(roots, actions, trackers, world=world, seeds=seeds, stop="boundary")
expansion = Expansion(**out)
```

The native signature omits the wrapper's `allow_live` debug option. Inputs and validation follow
`SearchGrid.run`: text-log roots, actions `[n, 2, 2]`, optional world indices `[n]`, seeds absent,
`[4]`, or `[n, 4]`, and a tracker pair shared by all roots or one pair per root. Both `boundary` and
`turn` stops work. Sides without a root action use `(-1, -1)`.

The result has exactly the `Expansion` fields. It retains a live `Battle` and both observation
trackers per running cell. Ended cells have `None` for both. Each root tracker is cloned and fed
its own side's leaf player-view lines once, including when that side is waiting. Retained battles
have empty logs, as `GridExecutor.run(keep_leaves=True)` does, and can be used as later search roots.
Simulation, tracker feeding, encoding, masks and battle retention run on the grid's worker pool
with the GIL released. Results follow input order regardless of the worker count.

`policy` contains `ids`, `floats`, `field`, `token_features`, `action_features` and `guard`, with
shape `[n, 2, ...]` and the same trailing dimensions and dtypes as `encode_states`. A side that
needs no action has a zero policy row. Joint masks are bool `[n, 2, 47, 47]`; policy action features
still have 40 move codes, as in the existing encoder.

`request_joint` is built in Rust from the returned request JSON. It ports `training.protocol.Options`:
per-slot move targets, PP and disabled flags, visible trapping, switching, forced moves, automatic
passes, and replacement/revival budgets. It reuses `SideActions` only for joint constraints, after
reconstructing all slot options and budgets from the request. Hidden engine restrictions never
enter this mask. `engine_joint` uses the world's actual `SideActions`.

The new `numbrion.FUSED_EXPAND` capability flag checks the native method. The Python wrapper only
exposes `expand` when that flag is true, so both flag checks and `hasattr(grid, "expand")` work
with older extensions. `timing` currently contains `total_ms` for the native call including Python
result construction.

There is no contract mismatch and no proposed change to `search2_types.py`.

## Reproduce

Python 3.14.7, matching the shared venv, with maturin 1.15.0, numpy 2.5.4, pytest 9.1.1, torch 2.14.1.
The interpreter, installation and extension are local to `.venv-expand` and this worktree. The
shared venv and the main checkout's extension were not modified. The ignored team corpora were
copied from the main checkout into this worktree for tests; source files were only read.

```sh
/opt/homebrew/bin/python3.14 -m venv .venv-expand
.venv-expand/bin/pip install maturin==1.15.0 numpy==2.5.4 pytest==9.1.1 torch==2.14.1
cd crates/pyengine
VIRTUAL_ENV="$PWD/../../.venv-expand" PATH="$PWD/../../.venv-expand/bin:$PATH" \
  CARGO_BUILD_JOBS=2 nice -n 10 ../../.venv-expand/bin/maturin develop --release --features python,search
cd ../..
nice -n 10 cargo test -p pyengine --release --locked -j 2
nice -n 10 cargo test -p pyengine --release --locked --features search -j 2
NUMBRION_SEARCH_PREREQS_DONE=1 PYTHONPATH=crates/pyengine:crates/pyengine/pytests \
  nice -n 10 .venv-expand/bin/python -m pytest --import-mode=importlib \
  crates/pyengine/training_tests/test_search*.py crates/pyengine/pytests/test_search_grid.py -q
cd crates/pyengine
nice -n 10 ../../.venv-expand/bin/python search2_expand_bench.py --repeats 30
```

Pytest needs importlib mode when collecting both directories together: their existing `test_search`
modules collide under pytest's default import mode. Adding `pytests` to `PYTHONPATH` keeps the existing
`test_search_grid.py` imports working. The original command without these options fails during collection.


## Validation

- `cargo test -p pyengine --release --locked -j 2`: 33 passed.
- The same command with `--features search`: 57 passed, 3 existing ignored tests.
- The requested Python search tests plus `pytests/test_search_grid.py`, using importlib mode: 50 passed in 9.24 s.
- `git diff --check`: passed.

The new test module has eight test cases. Six played eval-s43 games supply 71 distinct positions,
including early and late turns and all replacement boundaries. There are 568 cells per seed/stop
combination, tested with root seeds, one shared seed, and per-cell seeds, under both stop modes.
All 3,408 reference cells match at both one and four threads, for 6,816 fused cells. Every array
is compared by bytes, including floating-point bit patterns. Both returned trackers are re-encoded,
including the waiting side, and retained leaves are searched again to check their state and independence.

Directed cases cover a U-turn replacement, Revival Blessing, a two-Pokemon terminal win and the
turn-1000 tie. The hidden Shadow Tag fixture confirms `request_joint` is a strict superset of
`engine_joint`. There are also empty-batch, validation and post-error reuse checks. Four Rust unit
tests cover request mask construction, Tera and switch joint constraints, forced moves, automatic
passes, replacement scarcity and revival budgets.

## Measurements

M1 Pro shared Mac, release build without PGO, four grid threads, 30 repeats per operation after
warmup. Operations alternate in randomized order; setup, correctness checks, model inference and
output deletion are outside the expansion timer. Python GC is disabled during timed repetitions.
The layer-2 timer uses the existing fused value `SearchGrid.run`, not a policy/value network.
These are timings on the shared machine, not an idle-machine calibration.

| Operation | Mean ms | Std dev ms |
|---|---:|---:|
| Python reference expansion, 72 cells | 14.54 | 1.99 |
| Fused expansion, 72 cells | 3.52 | 0.36 |
| TrackerState.feed, 144 states | 0.88 | 0.28 |
| encode_states, 144 fed states | 1.39 | 0.42 |
| Feed + encode, 144 states | 2.33 | 0.49 |
| Layer 2, 72 leaves × 96 cells | 118.42 | 12.05 |
| Reference expansion + layer 2 | 132.59 | 5.76 |
| Fused expansion + layer 2 | 119.34 | 5.53 |

Fusing is worth using as the optional fast path: expansion is 4.13× faster and saves 11.02 ms per
72 cells. The measured expansion-plus-layer-2 path falls from 132.59 ms to 119.34 ms, about 10%.
Feeding plus encoding alone costs only 2.33 ms; most of the Python-path overhead comes from the
rest of the per-leaf work, including request masks, mask retrieval and result assembly. These timings
do not isolate each of those costs individually.

This saves roughly 0.4% of a 3-second decision budget per 72-cell expansion. It is a useful reduction
in glue cost, but it does not account for seconds of decision time or justify larger search settings
without measuring model inference and the rest of the pipeline.
