# Transformer self-play (first TIPS-based run)

This pipeline implements the first round of [training tips](TIPS.md) on top of the
[training API](../design/TRAINING-API.md) and the [Windows GPU setup](WINDOWS-GPU.md).
The older `train.py` MLP baseline is unchanged.

| Piece | File |
|---|---|
| static game data (vocabularies, move/species properties, type chart, set beliefs) | `tools/training/export-dex.mjs` -> `data/training/dex.json`, `training/dex.py` |
| protocol tracker and token encoder | `training/encoder.py` |
| damage-calc features | `training/damage.py` |
| model | `training/model.py` |
| actor processes (rollouts with CPU inference) | `training/actors.py` |
| battle workers for evaluation | `training/vecenv.py` |
| PPO helpers | `training/ppo.py` |
| trainer | `train_selfplay.py` |
| duplicate evaluation | `evaluate.py` |
| tests | `training_tests/test_selfplay.py` |

Paths are relative to `crates/pyengine` except the tools and data.

## Run

From the repository root, with the `.venv` from the GPU setup:

```powershell
.venv\Scripts\python.exe crates\pyengine\train_selfplay.py --minutes 60 --output scratch\training\first-run
.venv\Scripts\python.exe crates\pyengine\evaluate.py --run scratch\training\first-run
.venv\Scripts\python.exe -m pytest crates\pyengine\training_tests -q
```

`--minutes` is the training wall-clock budget. The trainer writes `metrics.jsonl` after every update,
`snapshots/mNNN.pt` every five minutes and `latest.pt` (with optimizer state and opponent pool) every
ten updates; `--resume latest.pt` continues a run. Regenerate the dex data after changing the pinned
Showdown checkout with `node tools/training/export-dex.mjs`.

## What is implemented from TIPS

**Observation.** Only the side's own request and player-view log are read (rule 1); a test checks that a
row's policy inputs do not change with the opponent's view. There are 12 Pokemon tokens (own team in
request order, then foe active a, foe active b, other revealed foes) and a field token. Each Pokemon has
learned species, item, ability, type and move embeddings plus property features (base stats, weight,
116 move features), HP, status, stats (foes: estimated from base stats and level), boosts, Tera state,
PP, disabled moves, volatiles, and timing state: turns since switch-in, "first turn out" for Fake Out, and
Protect used last turn. Field: weather, terrain, Trick Room and side conditions with elapsed turns, Tera
used per side, request kind, and KO counts.

**Belief over hidden sets.** For each foe, the role sets of `doubles-sets.json` that are consistent with its
revealed moves, ability and Tera type are weighted by their team-generator share and by how likely they are
to contain the revealed moves. This gives inclusion probabilities for the unrevealed moves (top 10), and a
distribution over Tera types. Items and abilities use the team generator's marginals until revealed.
Ability and item immunities enter the damage features as expected type multipliers.

**Damage-calc features.** For every move code of both actives: expected damage to the foes and to the own
side (fraction of max HP, capped at current HP), KO chances, effectiveness, and whether the user moves
first. For every Pokemon: expected damage and KO chance from each opposing active, and whether it outspeeds
each opposing active. An opponent's possible moves are its revealed moves (probability 1) and its ten most
likely belief candidates with their inclusion probabilities, each with its real type, category, power,
accuracy, hit count and spread reduction. Assuming the moves are present independently and the best one is
used, threat is E[max damage] and KO chance is 1 - prod(1 - p KO). Generic 80-power STAB attacks stand in only
when a foe has neither revealed moves nor a belief. Checked against 4,677 landed engine moves of our own:
median actual/predicted 1.00, interquartile range 0.95-1.32, correlation 0.76. A test checks foe moves
against our Pokemon the same way. The features are computed on the CPU actors and stored with the rollout.

**Model.** A 3-layer, width-128 transformer over the 13 tokens without positional encoding, about 0.92M
parameters. Actions are scored from their own features, DouZero style: user token, move representation,
target token, Tera flag and the damage features of exactly that move and target. Slot b is conditioned on
slot a's chosen action vector. Illegal codes are masked before sampling.

**Oracle critic and auxiliary heads.** The value head also sees the opponent's full team, which in self-play is
the opponent row's own request. Auxiliary heads predict each revealed foe's hidden item, ability, Tera type
and moves. Both are training-only; the policy never reads them.

**Training.** PPO with GAE (gamma 0.995, lambda 0.95), clip 0.2, two epochs of 1,024-sample minibatches per
rollout of 128 battles x 32 decision boundaries. Rollouts and training overlap: six actor processes play
the battles with CPU copies of the policy and fill one of two shared-memory buffers while the GPU trains on
the other. Each rollout is therefore collected with the weights from one update earlier, and the PPO ratio
uses the stored behaviour log-probs. Rewards are +1/-1 at the end plus a KO-difference bonus of 0.05 that
decays linearly to zero at half the run. 20% of the battles, spread over the actors, put a random older
snapshot (added every three minutes, last 12 kept) on side 2. Learning rate (3e-4 to 9e-5) and entropy bonus
(0.01 to 0.003) anneal over the time budget. Adam and gradient clipping are written with elementwise ops
because DirectML runs `lerp` and the fused kernels on the CPU. The metrics log the trainer's memory.

**Evaluation.** Duplicate games on the held-out `eval-s43-200` teams: each matchup's second half replays the
first half's teams and battle seeds with the sides swapped. Baselines: a random legal player (switch 10%,
Tera 15%), the "always use the strongest attack" heuristic, and earlier snapshots. For the final snapshot it
also reports mistake counters (attacks into Protect, failed Fake Outs, moves with no target, own-side hits,
Tera and switch frequency), calibration of the critic, and Showdown HTML replays.

## Not implemented yet

- Left/right slot-swap augmentation, exploiter agents, R-NaD, and network surgery when the model grows.
- Search (phase 2), Elo over a full checkpoint league, SPRT acceptance, and per-archetype win rates.
- Ability-, item- and terrain-specific damage modifiers beyond immunities, Choice items and Life Orb.
- The mask still has the hidden-information caveat from the training API (trapping and Imprison).

## DirectML notes

Measured on the RX 5500 with torch-directml 0.2.5:

- `nn.Embedding` backward (scatter-add) and int64 advanced indexing are slow. The model builds embedding
  bags as a scattered multi-hot times the table, so the backward pass is a matmul (`training.model.bag`):
  forward and backward went from ~50 ms to ~2 ms per lookup.
- `gather` on expanded tensors fails, and `log_sigmoid` falls back to the CPU. One-hot products and an
  explicit softplus replace them.
- The damage features take ~57 ms per 256 rows on DirectML but ~22 ms on one CPU thread, so they run in the
  actors.
- Python encoding (~190 us per row) and CPU inference are the actors' costs; they run in parallel with
  training.
