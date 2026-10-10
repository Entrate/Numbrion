# Before adding search: prerequisites

**Status: NOT DONE.** Do items 1 and 2 before search produces training targets, evaluation results or
ladder moves. `numbrion.GridExecutor` warns until `NUMBRION_SEARCH_PREREQS_DONE=1` is set; set it (and update
this status line) only when both are done.

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

## 2. MSVC + PGO build for training (do together with search)

**Why.** Ordinary self-play is network-bound (~200 battles/s), but search inside training (Gumbel AlphaZero,
2-16 simulations per decision) makes the engine the bottleneck. PGO measured +34% one-thread engine speed
over the same code without PGO (noisy; see [PGO.md](PGO.md)).

**Steps.**

1. Add a pyengine workload (a short `BatchEnv` and `GridExecutor` run) to the training step of
   `tools/pgo.ps1`; it currently profiles only the difftest bench and search primitives.
2. Build: `powershell -File tools/pgo.ps1 -Corpus <training battles, not fuzz-2000> -Maturin <maturin.exe>`.
3. Validate the wheel in a throwaway venv: `pytest crates/pyengine/pytests`, plus a short training smoke run.
4. Install into the training venv (`pip install --force-reinstall <wheel>`) only while no training runs.
5. Re-run the script after every engine change (a stale profile is still correct, just slower).

**Done when** the training venv runs the PGO wheel, pytest passes on it, and an idle-machine benchmark
confirms the gain.

## 3. Also check before relying on search

- Idle-machine 6/12-thread benchmarks of the engine and of `GridExecutor` (not yet re-measured at the
  current head).
- `GridExecutor(stop="turn")` answers mid-turn replacements with the first eligible party member; decide
  whether search needs its own replacement policy.
- The training-only oracle critic: confirm the policy network never receives oracle (opponent-row) inputs.
