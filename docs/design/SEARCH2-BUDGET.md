# Search2 budget handoff

`training/search_budget.py` implements the final six-method interface from
SEARCH2-PLAN.md. `search.py` and the shared contract are unchanged on this branch.
Existing ladder configurations are unchanged. `play/ladder-search2.json` opts in.

## Cost model and allocation

Each stage fits `ms = fixed + unit * n` with exponentially weighted nonnegative
ridge regression. Units are normalized by a reference batch size. Intercept
regularization is stronger because large batches alone cannot identify call
latency reliably. High outliers are clipped at 1.25 times prediction for fitting;
raw observations feed the tail estimates and decision-time feedback.

The chunk bound is `max(fitted_ms, prior_ms) * max(bound_factor,
1 + deviation_k * EMA(relative_absolute_error), 1.1 * recent_peak_ratio)`.
`bound_factor` defaults to 3. The prior floor prevents a noisy fit from admitting
a huge batch with a near-zero extrapolated cost. Architecture priors scale
network row costs by `(width / 128)^2 * layers / 3`. Calibration overrides live
in `budget.priors`, keyed by stage with `fixed_ms` and `unit_ms`.

The expected depth-2 cost includes nested samples, the extra best-q root, both
policies, worst-case replacement work, leaf grids/values, grouped solves and fixed
costs for the required chunks. The planner first grows worlds/chance when affordable.
To shrink it reduces worlds/chance and rounds, then replies, then leaf candidates
and seeds. It returns None when the smallest useful plan cannot fit. Presets cap
adaptive growth using the greater of architecture and measured network cost.

Recent total time uses an EMA with alpha 0.05. Allocation scale multiplies by
`clip(target / recent_mean, 0.8, 1.25)^0.08`, bounded to [0.5, 2]. This corrects
persistent under/over-allocation without changing fixed-mode sizes. Easy/decisive
positions receive less time; close positions and our team having at most three
survivors receive more. Difficulty has a nonzero floor.

## Core integration for Opus-1

1. In SearchBot construction, when budget is supplied, call
   `BudgetController.from_config(budget, net.width, net.layers, cell_ms)` and store
   it in `self.budget`. The serving worker has a getattr-based compatibility path
   for old constructors. Whole bot specs already reach constructors as kwargs.
2. Before layer 1, call `plan(cfg, cfg.depth2, budget_ms, deadline, time.monotonic())`.
   Use its worlds in a copied SearchConfig, keep the SAME Plan object, and use
   `plan.deadline`. A zero world count means policy fallback. Plan.deadline is
   the earlier of the driver's deadline and now + max_ms.
3. Post the layer-1 result as the driver's fallback, preserving its request token.
   Perform the known-threat check regardless of x confidence. Optionally attach
   `layer1.difficulty = {"known_threat": True, "remaining": public_remaining}`.
   Other accepted keys are factor, floor and forced. known_threat gives a floor
   of 1, even if factor is zero. The request-derived default counts only our team.
4. Call `plan_depth2(plan, layer1, time.monotonic())` after layer 1 completes.
   Its remaining target subtracts the actual elapsed layer-1 time.
5. Immediately before EVERY physical grid call and network batch, call
   `next_chunk_ok(stage, units, time.monotonic(), plan.deadline)`. Split calls
   until admitted by both the chunk cap and deadline, or retain the completed
   fallback. This includes layer 1. In particular, Net.values currently loops over
   2048-row batches internally; the check must be inside that loop or around
   externally split calls. A single monolithic grid call can also exceed chunk_ms.
   The 150 ms default margin covers final solve, analysis and send. Forced
   replacement expansion/policy calls also need admission checks.
6. Collect one Stage per actual uninterruptible call. Grid units are cells;
   policy/value units are actual rows; solve2 units are grouped games; worlds
   units are roots returned by world building. Measure CPU/GPU completion, not
   asynchronous dispatch. At decision completion, call `observe(stages, total_ms)`
   ONCE. Early policy decisions should also contribute to total-time feedback.

Fixed mode returns the exact configured worlds and the original Depth2Config.
It retains chunk admission and deadline checks. `search.depth2=false` or `"off"`
is normalized to disabled by both serving config parsers.

The shared Plan has no start clock and Layer1 has no difficulty metadata. The
controller retains start clocks by Plan identity, bounded to 128 active/recent
plans, and accepts an optional dynamic difficulty attribute. Suggested contract
follow-up: add `started: float` to Plan and `difficulty: dict` to Layer1, preferably
including public survivor counts for both teams. Do not copy/reconstruct Plan
between planning calls until the start clock becomes an explicit contract field.

## Validation and calibration

The final requested test selection passes 158 tests. It uses the supplied venv
and nice -n 10. Three tests initially failed because this worktree lacked the
ignored train-s42-2000.txt fixture; a symlink within this worktree to the shared
fixture resolves them. Existing search prerequisite warnings remain.

The deterministic bounded-tail simulation runs 10,000 decisions with 3,775
injected 3x spikes: mean after 500 warm-up decisions 3001.39 ms, max 4274.12 ms,
zero bound violations and zero deadline overruns. Deliberately adding 6000 ms
beyond an admitted bound causes one violation and one overrun in 1000 decisions,
max 6146.53 ms. Its learned tail then refuses expensive work. Arbitrarily slow
uninterruptible calls cannot have a mathematical hard deadline guarantee; the
driver fallback is still required.

`tools/search_cost.py` measured six warmed move-turn positions per checkpoint on
CPU with four threads, varying worlds 2/8/32. Per-decision mean stage costs:

- Spark 128x3: worlds 6.95 ms, root policy 3.59 ms, opponent policy 17.17 ms,
  grid 45.93 ms, value 170.43 ms. The two 32-world decisions took 640.24/543.25 ms.
- 192x4 m535: worlds 7.02 ms, root policy 3.96 ms, opponent policy 18.53 ms,
  grid 47.55 ms, value 269.79 ms. The two 32-world decisions took 861.03/775.15 ms.

Suggested fixed + per-unit value priors are 15.772 + 0.053623*n ms for Spark and
9.806 + 0.094289*n ms for 192x4. Grid slopes are 0.017147 and 0.017726 ms/cell.
Full samples and fits are in search2-cost-calibration.json. These are short
shared-machine measurements, not the final 300-position timing validation.
`--depth2` reports the merged core controller's stage costs; it is not exercised
here because this branch intentionally has no layer-2 core.
