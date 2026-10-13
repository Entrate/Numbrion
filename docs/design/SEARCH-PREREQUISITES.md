# Before adding search: prerequisites

**Status: item 1 implemented (opt-in), item 2 scripted but not yet built on the Windows training machine.**
Do items 1 and 2 before search produces training targets, evaluation results or ladder moves.
`numbrion.GridExecutor` warns until `NUMBRION_SEARCH_PREREQS_DONE=1` is set, and separately whenever a root
is not a `WorldBuilder` world; set the variable (and update this status line) only when both items are done.

Search support is an **opt-in cargo feature** (`search`, engine and pyengine). It is off by default because
its PRNG hook adds a counter and a compare to every draw and 152 bytes to `BattleState`; a default build is
unchanged (grid cells at ~27 us/cell for the base commit, the default build and the search build alike on an
M1 Pro, within noise). Build a search wheel with `maturin develop --release --features python,search` (maturin replaces
the `pyproject.toml` feature list, so `python` must be named too); `numbrion.SEARCH` tells which wheel is
installed.

## 1. Fair-information determinization (required)

**Problem.** The engine holds the true battle state, including the opponent's hidden information. A search
that simulates futures from a copy of the live battle knows the opponent's real sets. The current API
(`Battle.replace_hidden_set`, see [TRAINING-API.md](TRAINING-API.md)) is only a *partial* determinization:

- it can swap only opponent Pokemon that were **never revealed**;
- revealed Pokemon keep their **true** unseen moves, item, ability and Tera type in every sampled world;
- unseen Zacian/Zamazenta-style species (forme change at battle start) cannot be swapped at all;
- `hidden_team_indices` is privileged bookkeeping: its result depends on hidden species, so it must never
  reach a policy or value input.

**Why it matters.** Search targets built with hidden information teach the policy to expect knowledge it
will not have on the ladder, and self-play evaluation overstates strength. On the ladder the server never
sends the opponent's team, so search there must build worlds from samples anyway.

**What to build.**

1. A revealed-information tracker per Pokemon: moves used, item / ability / Tera type shown (from the
   player-view protocol, as `training/encoder.py` already does for observations, or in the engine; see
   "Requests for the engine" #3 in TRAINING-API.md).
2. A set sampler consistent with what was revealed: the random-battle set lists
   (`data/random-battles/gen9/doubles-sets.json`) or the learned belief (`training.dex.Dex.belief`).
3. A world builder: construct the battle from the sampled teams and replay the choice history; reject the
   sample if the replayed player-view log differs from what was observed. Build each world once per
   decision and reuse it for the whole grid. A fast path (`replace_hidden_set`) is fine only where the
   swapped details provably did not affect history.
4. The same code path must work from protocol-only input (ladder), never from a copy of the live battle.

**Done when.**

- *Leak test:* two roots with identical public information but different opponent hidden information give
  the same search output (same seeds). Run it on many positions, including revealed Pokemon with unseen
  moves/items and unseen BattleStart-forme species.
- No search path clones or restores the live battle as a world; worlds come only from the builder.

Until then: use `GridExecutor` / `replace_hidden_set` for experiments and benchmarks only, not for training
targets or strength claims.

### Implementation (2026-10-11, `crates/engine/src/search`, feature `search`)

| Step | Where |
|---|---|
| 1. Revealed-information tracker | `search::reveal::RevealTracker` (Python `numbrion.RevealTracker`): public facts per Pokemon of both sides from player-view lines: moves chosen, item / ability (starting and current, frozen after Trick/Skill Swap-style changes), Tera, HP, status, Illusion attribution |
| 2. Set sampler | `search::sets::SetPool` (Python `numbrion.SetPool`): generated packed teams, deduplicated with counts, sampled by count among the sets satisfying the most revealed facts; unrevealed slots from the pool's distribution under the species clause |
| 3. World builder | `search::world::WorldBuilder` (Python `numbrion.WorldBuilder`): constructs battles from the viewer's team and a sampled opponent team and replays the history; rejects worlds whose replayed player view (and, when pushed, the viewer's requests) differs from the observed one |
| 4. Protocol-only input | the builder's only inputs are the viewer's team, its choices, its requests and its player-view lines; it never sees a `Battle` |

How the replay finds the hidden randomness: on a mismatching line the draws behind it are resampled
(`Prng::set_resample` replaces the PRNG stream from a draw index on, keeping every earlier draw), backing off
to earlier lines when one keeps failing; a line that resampling cannot fix marks a wrong hypothesis and the
world is dropped. The opponent's choices are read off the protocol (move, shown target, Tera, switch target,
Instruct and Encore-style mismatches covered by a small exploration probability). Worlds are kept
incrementally between decisions and weighted by 1/attempts per segment, a search-effort heuristic (not a
likelihood estimate) that makes hypotheses needing far more luck count less; `worlds(k, seed)` draws by
weight and reseeds every world. Pushed viewer requests are checked at every boundary a world reaches,
including the current decision.
A newly seen opponent Pokemon is swapped into a never-revealed slot with `replace_hidden_set`; a viewer
Pokemon transformed into an opponent reveals that opponent's move order through the viewer's request.

**Done-when status.**

- *Leak test:* `crates/pyengine/tests/world.rs` (`identical_public_information_gives_identical_worlds`)
  builds pairs of real battles whose opponent teams differ only in what the viewer never saw (never-revealed
  bench sets, including BattleStart species when drawn; unused moves, items, abilities and Tera types of
  revealed Pokemon), checks that the viewer's observations are identical, and requires identical builder
  statistics, sampled teams, world states and simulated futures. 300 of 300 battles gave such a pair, all
  identical (`NUMBRION_TEST_BATTLES=300`).
- *No live worlds:* worlds come only from the builder; `Battle.is_world` marks them (and copies of them), and
  `GridExecutor.run` warns on any other root unless `allow_live=True`.

**Quality and cost** (Apple M1 Pro, one thread, pool-400 test pool, 8 worlds, random play): worlds reproduce
the viewer's request exactly at every checked boundary; about 95-96% of decisions get worlds (287 of 299
random positions from scratch with the default budgets, 292 of 299 with budgets twice as large; about 95% when
maintained incrementally through whole battles, failures clustering in a few battles). Incremental cost per decision: median about 9 ms, mean about 60 ms, p90 about 180 ms (tail:
fresh worlds replay the whole history). When no world is found the builder raises a clear error with the
reason (`last_failure`); fall back to the policy for that decision.

**Known limits.** The world distribution is not a calibrated posterior: weights are a search-effort
heuristic, and the opponent's policy is modelled as uniform over choices consistent with the protocol.
Worlds also depend on when `update`/`worlds` are called, not only on the inputs and seed. Generator team rules other than
the species clause are not modelled. Illusion entry owners are pinned by stint; team construction solves
the evolving party order (switches, faints, Ally Switch and Revival Blessing) before replay. Ordering includes
the Tera exceptions for Ogerpon/Terapagos; replay checks ambiguous copied forms under Transform. Fresh worlds
include a small pool-frequency prior for a hidden Illusion user (its own RNG stream, so the plain path is
unchanged). Rebuilds try hypotheses over up to six ambiguous entries (rotating, evidence first) before patching
off-pool sets only on strong evidence (an off-pool fact, a known Illusion stint, a fainted Pokemon back); on
weak evidence (worlds died while a possible disguise was active) they get every other build, at most a quarter
of the budget, so battles without Illusion rebuild as before. A patched set costs a world a factor of about
1/1000 in weight, and while every survivor is patched one fresh build per update looks for an exact
explanation. Each hypothesis folds those stints onto the user without
changing the tracker, including a silent disguised faint followed by the living disguise. Known broken
stints have no six-entry limit. Hypotheses share the existing replay/build budgets, so ambiguous histories
can still exhaust them. Viewer request content beyond what the engine itself writes (Showdown's `rqid`) is ignored;
ladder-specific protocol lines are excluded from comparison by `search::protocol::is_compared`. The pool
must contain the opponent's species (`UnknownSpecies` otherwise); use the 600k-team pools for real runs.

## 2. MSVC + PGO build for training (do together with search)

**Why.** Ordinary self-play is network-bound (~200 battles/s), but search inside training (Gumbel AlphaZero,
2-16 simulations per decision) makes the engine the bottleneck. PGO measured +34% one-thread engine speed
over the same code without PGO (noisy; see [PGO.md](PGO.md)).

**Steps.**

1. Profile the pyengine paths. The workload (`crates/pyengine/src/pgo.rs`: `BatchEnv` self-play with and
   without TextLog, `GridExecutor`, `SearchGrid`, clone/restore/`replace_hidden_set`) runs inside an instrumented wheel
   in the training step of `tools/pgo.ps1` and `tools/pgo.sh`. Running it from a separate cargo binary
   would not help, because that profile does not match the wheel's symbols (see [PGO.md](PGO.md)). Not yet
   run on Windows.
2. Build: `powershell -File tools/pgo.ps1 -Corpus <training battles, not fuzz-2000> -Maturin <maturin.exe>
   [-Pool data/teams/pool-s1-200k.txt.gz]`, or on macOS/Linux `tools/pgo.sh --corpus <training battles>
   [--pool ...]`.
3. Validate the wheel in a throwaway venv: `pytest crates/pyengine/pytests`, plus a short training smoke run.
4. Install into the training venv (`pip install --force-reinstall <wheel>`) only while no training runs.
5. Re-run the script after every engine change (a stale profile is still correct, just slower).

**Done when** the training venv runs the PGO wheel, pytest passes on it, and an idle-machine benchmark
confirms the gain.

## 3. Also check before relying on search

- Idle-machine 6/12-thread benchmarks of the engine and of `GridExecutor` (not yet re-measured at the
  current head).
- `GridExecutor(stop="turn")` answers mid-turn replacements with the first eligible party member; decide
  whether search needs its own replacement policy. (Not changed: `stop="boundary"` plus `keep_leaves` already
  lets the search answer replacements itself.)
- Candidate actions for a search grid must come from the world's (or the request's) legal mask, not from the
  live battle's: the live mask contains hidden trapping / Imprison facts (see "Hidden information in the
  mask" in TRAINING-API.md).
- The training-only oracle critic: confirm the policy network never receives oracle (opponent-row) inputs.
