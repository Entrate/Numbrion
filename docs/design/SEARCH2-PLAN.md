# Two-ply search for the ladder: plan (draft for review)

Owner: Claude Opus (orchestrator). Implementation: 2 Opus subagents and 2 GPT-6.1-Sol subagents, each in its own git
worktree and branch. I review and merge everything into main myself.

## Goal and constraints

- **Ladder bot gets stronger.** Two problems in particular:
  (a) blindness to sleep threats (Spore, Sleep Powder, Yawn: Yawn's sleep lands a turn later);
  (b) endgame stalling (Roost, Wide Guard, Protect spam when attacking or trading is right, or when a one-hit KO
  threat makes stalling pointless).
- **Time.** Ladder decisions average about 3 s and never exceed 6 s (driver `deadline_s` 6.0, `reserve_s` 0.5).
  Today's 1-ply search uses ~520 ms on the Mac (it hits the cap of 32 worlds, 12x8 pairs, 2 seeds; ~5k cells), so
  ~2.5 s per decision is unused.
- **Model size.** It must scale to bigger models later: every size knob must be derived from measured costs, so a
  slower network automatically gets a smaller second layer (or none).
- **Leave alone:** value discounting (gamma 0.995 in the value target), the networks, and the training code.
- **Hidden information stays fair:** search reads only our observations, as the 1-ply search does today.

## Current 1-ply search (training/search.py)

- `SearchPlayer.decide` steps:
  1. root policy, giving our candidate pairs (`ours` = 12) and their prior;
  2. `WorldBuilder` samples up to 32 worlds;
  3. `run_search`: opponent candidates per world (`theirs` = 8, from the policy run on the opponent-view tracker
     + the world's request);
  4. `SearchGrid.run` simulates every (world, our pair, their pair, seed) cell to the next decision boundary and
     encodes the leaf for the value head;
  5. payoff matrices per world;
  6. `solve` (smoothed fictitious play, KL anchors 0.05 / 0.1);
  7. play argmax x.
- Budgeting: `SearchBot.config_for(budget_ms)` scales the world count with a running `cell_ms`; the deadline trims
  worlds.

## Second-layer design

After layer 1 we have x (our strategy), ys[k] (the opponent strategy per world, from the solved game, NOT the
raw prior) and q.

1. **Select.**
   - Our top `K1` pairs by x. K1 = 3 by default, 2-4 in ablations.
   - Always add the best pair by layer-1 value q if it isn't already in.
   - Optional: add one "safety" candidate: the best switch pair, if any is in the shortlist.

   We only re-rank among these, so every compared candidate is deepened the same way (same worlds, same
   opponent replies, same chance samples). That keeps 1-ply and 2-ply values from being mixed.
2. **Worlds.** `W2` of the layer-1 worlds (default 4), drawn uniformly (the worlds are equally weighted
   samples). The same subset is used for every candidate.
3. **Opponent replies.** Per selected world, the opponent's top `K2` pairs by y_k (default 3), plus the
   leftover weight. The leftover can either be renormalised or kept as a "rest" term valued at layer 1's q.
   I'll decide on the evidence.
4. **Chance.** `C2` seeds per (world, our pair, their pair) (default 2), common random numbers across our
   candidates.
5. **Expand.** Simulate those K1 x W2 x K2 x C2 cells (~72) to the next boundary, keeping the leaf battles, each
   side's player-view lines and requests (`GridExecutor.run(..., keep_leaves=True, logs=True, requests=True)`
   already does this). Leaves that ended get +1/-1/0.
6. **Layer 2 at each leaf.**
   - Our candidates: the policy on our leaf tracker (root `TrackerState` fed with the leaf lines) + leaf
     request + leaf masks (`ours2` = 8).
   - Opponent candidates: same from the opponent-view tracker (`theirs2` = 6).
   - `SearchGrid.run` with the leaves as roots (`seeds2` = 2) and the value head at the next boundary.
   - Solve each leaf's one-world game with `solve_batch` (same anchors). V2(leaf) = x2·A·y2.

   Leaves where only one side acts (forced replacement) work like the 1-ply case: the other side passes.
7. **Back up.** Two options; ablation decides:
   - (a) fixed: Q2[i] = mean over worlds of sum_j y_k[j] · mean over seeds of V2, with y_k restricted to the K2
     replies;
   - (b) re-solve: build payoffs2[k][i,j] from V2 and re-solve the reduced Bayesian game (K1 x K2 per world) with
     `solve`, so the opponent's reply weights also see the deeper values.
8. **Decide.** Play argmax of the 2-ply result among the K1 candidates. Keep the layer-1 decision when layer 2
   didn't complete in time, or any of its steps failed (logged in the analysis note).
9. **Analysis.** Add a `depth2` block: candidates with 1-ply and 2-ply values, the replies and weights used,
   cells, timing, and whether the decision changed. The extension, ladder DB and recorder show it.

Default size: ~72 leaves x (8x6x2 = 96 cells) ≈ 7k layer-2 cells, ~1.3x layer 1. Expected ~0.7-1.2 s on the
Mac. Bigger budgets grow W2 and C2 first, then K1 and K2.

### Hidden-information notes
- Leaf worlds are the layer-1 sampled worlds stepped forward; they keep their sampled opponent sets.
- Our policy at a leaf sees our own team truly (it's ours). The opponent policy at a leaf uses the opponent-view
  tracker, as layer 1 does.
- Masks at leaves come from the world. Our side may see a trap that the real request would only show as
  maybeTrapped; that's acceptable for evaluation.

### Time budget and model scaling (new `training/search_budget.py`)
- An online cost model (EMA, per bot, measured, not guessed) for:
  world building, root policy, opponent policy rows, layer-1 cells, expansion cells, leaf encoding + policy rows,
  layer-2 cells, value rows.
  A bigger network raises the per-row costs, so planned sizes shrink automatically.
- A per-decision plan, given the remaining time to the deadline and a target average:
  - Layer 1 first, as now: worlds up to the cap.
  - Then layer 2 sized to fit min(budget_left, deadline − margin).
  - Size ladder: W2 → C2 → K1/K2 → ours2/theirs2, chosen to fit the predicted cost.
  - A running controller keeps the mean decision time near the target (3 s) by scaling the next budgets.
  - Difficulty weighting: when layer 1 is decisive (top x ≥ 0.9 and a large q gap) or the decision is forced,
    spend little; when it's close, or few Pokémon are left, spend more.
  - Hard rule: never pass the deadline. Layer 2 checks the clock between stages and returns the best finished
    result.
- Config presets keyed by the measured cost, so bigger models (192x4, 256x6) get smaller default second
  layers. The config can force sizes, or `depth2: off`.

## Work split (4 agents, parallel, fixed interfaces)

Each agent works in its own worktree off main a6a6139, on its own branch, and doesn't touch the main checkout
(the live public bot runs from it). Python-only agents run with the shared venv
`/Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/.venv/bin/python`, with
`PYTHONPATH=<worktree>/crates/pyengine`. Its compiled `numbrion` loads from the main checkout via a .pth file.
The Rust agent builds its own venv in its worktree and must NOT run maturin into the shared venv.

1. **Opus-1, core (`feat/search2-core`).**
   - `training/search2.py`: `Depth2Config`, `deepen(...)`, expansion (Python path via GridExecutor), layer-2
     solve, backup options, analysis.
   - A small hook in `SearchPlayer.decide` / `run_search` (return ys and worlds).
   - Unit tests:
     - deterministic seeds give the same result;
     - with K1 = all candidates and a trivial layer 2, the decision is consistent;
     - terminal leaves are handled;
     - forced-replacement leaves work;
     - layer 2 is skipped cleanly on timeout.
2. **Sol-1, fused expansion in Rust (`feat/search2-expand`).**
   - `SearchGrid.expand(roots, actions, trackers, world, seeds)` returning, in one threaded call:
     - leaf `Battle`s;
     - leaf `TrackerState`s for both sides (the root tracker fed with the leaf lines);
     - leaf requests;
     - both sides' policy inputs at the leaf (ids, floats, field, token_features, action_features, guard; masks).
   - A test that it matches the Python path bit for bit. Benchmarks.
   - Plugs in behind the core's `expand()` interface when present.
3. **Sol-2, budget controller and plumbing (`feat/search2-budget`).**
   - `training/search_budget.py` (cost model, planner, running-average controller, presets).
   - Integration in `SearchBot.config_for` / decide.
   - Ladder and advisor config fields (`depth2`, `target_ms`).
   - Tests with a fake clock: mean near target; never over the deadline.
4. **Opus-2, evaluation (`feat/search2-eval`).**
   - `search_eval.py`: separate search configs for the two sides (including depth2), fixed-size mode (no ms
     budgets, so concurrency doesn't change what's searched), per-decision time stats and percentiles.
   - Scenario suite (`tools/scenarios/` + test): directed positions built from teams + scripted choices:
     - Spore / Sleep Powder / Yawn threats, with and without counterplay available (Grass type, Insomnia /
       Vital Spirit, Electric or Misty Terrain, Safeguard, a switch to a sleep absorber, attacking first);
     - endgames: 1v1 and 2v1 where attacking is right and Roost/Protect only delays a loss; a one-hit KO threat.

     Reports each config's choice, values and time. Expected answers are given only where unambiguous.
   - A timing harness replaying stored positions (from the ladder recorder).

### Interfaces (fixed now so the agents can build in parallel)
```python
# training/search2.py (Opus-1 owns)
@dataclass
class Depth2Config:
    enabled: bool = False
    ours_top: int = 3          # K1
    theirs_top: int = 3        # K2 per world
    worlds: int = 4            # W2
    chance: int = 2            # C2
    ours: int = 8              # layer-2 our candidates
    theirs: int = 6            # layer-2 opponent candidates
    seeds: int = 2             # layer-2 chance seeds per leaf
    backup: str = "resolve"    # "resolve" | "fixed"
    rest: str = "renormalize"  # leftover opponent weight: "renormalize" | "layer1"
    safety_switch: bool = False

def deepen(bot, cfg, cfg2: Depth2Config, battle, side, worlds, ours, our_full, layer1: dict,
           deadline: float | None) -> dict | None:
    """-> dict(x2, q2 [K1], chosen index into ours, cells, timing, analysis) or None (keep layer 1)."""

def expand(bot, worlds, actions, world_idx, seeds, states, side) -> Expansion:
    """Python path; uses grid.expand (Sol-1) when the extension has it."""

# training/search_budget.py (Sol-2 owns)
class BudgetController:
    def plan(self, bot, cfg, cfg2, deadline, budget_ms, difficulty) -> tuple[SearchConfig, Depth2Config | None]: ...
    def observe(self, timing: dict, sizes: dict) -> None: ...
```
`run_search` gains `ys` and `worlds` in its result (backwards compatible).

## Tests I run (the Mac now; Windows after its 6 h ladder recording run ends)

Rule for strength tests: compare at matched compute using fixed sizes, not ms budgets, so concurrency on a busy
machine doesn't distort them. Timing is checked separately on an idle machine.

- **E0 correctness:** unit tests; 20-game smoke; no crashes; analysis complete.
- **E1 timing:**
  - per-decision time distribution over ~300 positions at the ladder budget;
  - mean ≈ 3 s, p99 < 5 s, max < 6 s;
  - the same with a 192x4 network, to check scaling.
- **E2 strength (main):** 2-ply (default sizes) vs today's 1-ply ladder config, both searching, 200-300 mirrored
  pairs (search_eval, eval-s43 teams).
- **E3 depth vs width at equal compute:** 2-ply vs a 1-ply widened to the same cell count (more worlds, ours 16,
  theirs 12, seeds 4). It answers whether depth is what helps, not just more compute.
- **E4 scenarios:** the scenario suite for both configs; sleep and endgame positions must improve.
- **E5 ablations (100-150 pairs, or scenarios):** K1 2/3/4; K2 2/3; backup resolve/fixed; rest; W2 4/8; C2 1/2/4;
  safety_switch.
- **E6 (later, user's call):** a ladder session with 2-ply.

Estimated cost: an E2 game at default sizes is ~2 s per side per turn of compute. 300 pairs ≈ 600 games ≈ 3-4 h
on the Mac at concurrency 3. Ablations use fewer pairs or the scenario suite.

## Risks
- **Opponent model at depth.** Layer 2's opponent replies come from the policy prior plus the solved game; a
  human may play differently. The KL anchors (0.05 / 0.1) stay as they are.
- **Hidden info.** W2 = 4 worlds can be noisy on sets that matter (whether the foe has Spore). W2 is the first
  thing to grow with budget.
- **Value-head errors.** Depth can't fix a head that misjudges positions at the deeper leaves. Discounting stays
  as it is for now (user decision).
- **Cost of Python glue** (TrackerState.feed per leaf, many small arrays). Sol-1's fused expand addresses it.

---

# Decisions after review (final, supersedes the draft above where they differ)

The Sol review (GPT-6.1-Sol, 2026-10-16) found real problems. Final calls:

1. **Hidden information (strategy fusion).** Our layer-2 strategy is SHARED across the leaves that look the
   same to us: same root pair i, same observed opponent reply, same chance index. That gives one Bayesian game per
   group across the worlds, exactly as layer 1 does. Independent per-leaf solving is only an ablation
   (`info="independent"`). Our candidate masks at leaves are request-derived (`request_mask` of the leaf
   request), never the engine mask. Refusals are modelled as at layer 1 (a request-allowed pair the world's
   engine refuses is worth our best allowed pair there).
2. **Opponent model.** The opponent is a 1-ply player with FIXED layer-1 weights y_k. Default backup is the
   correction estimator
   `Q2[i] = Q1[i] + mean_worlds sum_{j deepened} y_k[j] (V2[k,i,j] - A1[k,i,j])`,
   so undeepened replies and worlds keep their layer-1 values; there is no renormalisation. Re-solving is an
   ablation (`backup="resolve"`).
3. **Candidates: sequential halving.**
   - The shortlist is the top 4 by x, plus the best by q. Round 1 deepens all of them with 4 worlds x 1 chance
     draw; round 2 keeps the best 2 and grows to 8 worlds x 2 chance draws. Samples are nested and reused.
   - Rounds give natural bounded work chunks for the budget controller.
   - Replies per (world, candidate): the top 2 by y_k, plus 1 "danger" reply, the lowest A1 among replies with
     y ≥ 0.02.
   - Making sure sleep threats reach the layer-1 opponent shortlist at all is a separate fix, after the ladder
     recordings are diagnosed.
4. **Depth = move turns.** After the root turn, forced replacements (faints, pivots) are answered by the policy
   argmax and stepped through, at most `replacement_steps` times, so layer 2 starts at a move turn. Terminal
   leaves score ±1/0 and aren't passed to SearchGrid.run. A side with no action uses (-1, -1).
5. **Deadline safety.** Before deepening, the player re-posts its driver fallback as the LAYER-1 decision, so
   an overrun sends the 1-ply choice, not the plain policy. Layer 2 runs in bounded chunks (rounds, grid calls,
   network batches) and doesn't start a chunk whose predicted cost (a conservative bound) passes the
   deadline. Layer-1 and layer-2 costs are measured separately. Layer 2 is sized after layer 1 has finished, from
   its measured cost.
6. **Evaluation.**
   - Deterministic RNG per game and player, so fixed-size results don't depend on concurrency.
   - Report capped games (they hide stalling).
   - Calibrate fixed configs to equal measured time on an idle machine, then compare them at fixed sizes.
   - A pilot first to measure variance. Ablations are screening only; the main comparisons get 300+ pairs.
7. **Fused Rust expansion** is built in parallel against the `Expansion` contract, with a bit-for-bit test against
   the Python reference. It's adopted only if profiling shows the Python glue matters.

## Ownership (one owner per file)

| File(s) | Owner |
|---|---|
| `training/search2_types.py` (shared contract, branch `search2/base`) | orchestrator only |
| `training/search.py` (all edits), `training/search2.py`, analysis schema (`depth2` block), fallback re-post, per-player RNG (`SearchBot.new_player(room, seed=None)`), Python `expand` | Opus-1 core |
| `src/search_grid.rs`, `src/py_search_grid.rs`, `python/numbrion/__init__.py` exposure (`SearchGrid.expand`) | Sol-1 Rust |
| `training/search_budget.py`; config parsing in `play/*.py` and the json configs (`search.depth2`, top-level `budget`) | Sol-2 budget |
| `search_eval.py`, `tools/scenarios/`, timing harness | Opus-2 eval |

Sol-2 does not edit search.py. Opus-1 calls the controller through the interface below, and builds and tests with
a trivial stand-in until Sol-2's branch lands.

## Budget controller interface (Sol-2 implements, Opus-1 calls)
```python
class BudgetController:
    @classmethod
    def from_config(cls, budget: dict | None, width: int, layers: int, cell_ms: float) -> "BudgetController": ...
    def plan(self, cfg, cfg2, budget_ms, deadline, now) -> Plan: ...            # before layer 1: worlds
    def plan_depth2(self, plan, layer1, now) -> Depth2Config | None: ...       # after layer 1: sized rounds
    def next_chunk_ok(self, stage: str, units: int, now: float, deadline: float | None) -> bool: ...
    def observe(self, stages: list[Stage], total_ms: float) -> None: ...
    def stats(self) -> dict: ...
```
Bot config:
- `search: {..., depth2: {...Depth2Config...}}`
- top-level `budget: {target_ms: 3000, max_ms: 5500, mode: "adaptive" | "fixed", presets: ...}`

`SearchBot.__init__` gets `budget` and builds `self.budget = BudgetController.from_config(...)` when it is given
(Opus-1 adds that line).
