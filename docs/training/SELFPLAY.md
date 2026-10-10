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
median actual/predicted 1.00, interquartile range 0.95-1.32, correlation 0.76. Foe moves against our
Pokemon, revealed or belief candidates, match as well: 1,766 attacks over ten seeds, median 1.00,
interquartile range 0.93-1.31, correlation 0.81 (a test checks one seed). The features are computed on the
CPU actors and stored with the rollout.

**Model.** A 3-layer, width-128 transformer over the 13 tokens without positional encoding, about 0.92M
parameters. Actions are scored from their own features, DouZero style: user token, move representation,
target token, Tera flag and the damage features of exactly that move and target. Slot b is conditioned on
slot a's chosen action vector. Illegal codes are masked before sampling.

**Oracle critic and auxiliary heads.** The value head also sees the opponent's full team, which in self-play is
the opponent row's own request. Auxiliary heads predict each revealed foe's hidden item, ability, Tera type
and moves. Both are training-only; the policy never reads them.

**Training.** PPO with GAE (gamma 0.995, lambda 0.95), clip 0.2, two epochs of 1,024-sample minibatches per
rollout of 128 battles x 32 decision boundaries. Rollouts and training overlap: four actor processes play
the battles with CPU copies of the policy and fill one of two shared-memory buffers while the GPU trains on
the other. Each rollout is therefore collected with the weights from one update earlier, and the PPO ratio
uses the stored behaviour log-probs. The PPO update is the bottleneck, so more actors or battles do not
help (see the throughput notes). Rewards are +1/-1 at the end plus a KO-difference bonus of 0.05 that
decays linearly to zero at half the run. 20% of the battles, spread over the actors, put a random older
snapshot (added every three minutes, last 12 kept) on side 2. Learning rate (3e-4 to 9e-5) and entropy bonus
(0.01 to 0.003) anneal over the time budget. Adam and gradient clipping are written with elementwise ops
because DirectML runs `lerp` and the fused kernels on the CPU. The metrics log the memory of the trainer
and of the largest actor.

**Evaluation.** Duplicate games on the held-out `eval-s43-200` teams: each matchup's second half replays the
first half's teams and battle seeds with the sides swapped. Baselines: a random legal player (switch 10%,
Tera 15%), the "always use the strongest attack" heuristic, and earlier snapshots. For the final snapshot it
also reports mistake counters (attacks into Protect, failed Fake Outs, moves with no target, own-side hits,
Tera and switch frequency), calibration of the critic, and Showdown HTML replays.

## First run (2026-10-10)

60 minutes of training on the Ryzen 5 3600 + RX 5500 with the defaults above: 712 PPO updates and
189,395 self-play battles (about 53 battles/s, 4.5M trained decisions). Evaluation took 4 minutes,
using duplicate games on the held-out teams (400 games per matchup, 1,000 for the final baselines):

| Snapshot | vs random | vs strongest-attack heuristic | final (60 min) vs this snapshot |
|---|---:|---:|---:|
| 0 min (untrained) | 42.2% | 0.3% | 99.8% |
| 5 min | 100% | 53.0% | 65.5% |
| 10 min | 99.3% | 60.5% | 66.5% |
| 20 min | 99.5% | 63.0% | 59.0% |
| 30 min | 99.5% | 62.3% | 57.0% |
| 35 min | 99.5% | 67.5% | 49.5% |
| 45 min | 100% | 65.7% | 60.3% |
| 55 min | 99.3% | 69.0% | 50.5% |
| 60 min | 100% (1,000 games) | 67.8% (1,000 games, about +130 Elo) | |

The heuristic itself beats the random player in all 400 games. Both TIPS tripwires for the first hours
(95%+ vs random, then beating "always use the strongest attack") were met within five minutes. Most of the
gain came in the first 5-10 minutes, slower gains followed until about 35 minutes, and the last 25 minutes
were roughly flat (final vs 35/55 min: 49.5% and 50.5%, error about +/-5%).

The final policy plays an all-out attack style: self-play battles average 9.5 turns, games against the
heuristic 5-6, it terastallizes in nearly every battle (usually on turn 1), and it almost never uses Fake
Out (3 times in 1,260 games) or other status moves. Policy entropy fell from 4.0 to about 0.7 during the run.
Its mistake rates are low (0.35% of moves without a target, 0.16% hitting its own side), but the
heuristic never uses Protect, so these games do not test reading Protect. The critic ranks positions
well: higher values always meant more wins. Against the weaker heuristic it is underconfident, for example
predicting 55% where the actual rate was 67%, because it was trained on self-play, where both sides win
equally often.

Outputs (git-ignored): `scratch/training/first-run/` with `metrics.jsonl`, `snapshots/`,
`evaluation.json` and four HTML replays in `replays/`.

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
- DirectML keeps host memory for every call with an `alpha=`/`value=` argument or a new Python scalar per
  step. The first run's Adam (`addcdiv_(..., value=-lr / c1)`, `v / c2`) lost ~3.6 MB per update this way:
  the trainer grew from 850 MB to 1.37 GB in 18 minutes. Adam now takes these scalars as 0-dim device
  tensors (the update is ~3% slower). The PPO update alone then grows 0.1 MB per update; the whole trainer
  still grows ~1.2 MB per update with the opponent pool full (~1.3 GB per hour, cause not found). The
  encoder's static-feature caches are capped at 50,000 entries each: foe reveal states never stopped
  adding entries (~0.5 GB per actor per hour). `Dex.belief` is an LRU of 200,000 entries.

## Throughput

Measured on the Ryzen 5 3600 (6 cores, 12 threads) next to the RX 5500. One actor step on one thread takes
36 ms for 16 battles (32 rows) with 20% pool battles: Python encoding 6.5 ms (~200 us per row), damage
features 7.9 ms, CPU inference 20 ms (trunk 10.7, critic 2.3, policy heads 3.2, pool opponent 4.3 for its
3 rows) and the engine step 1.2 ms. With 21 battles it takes 45 ms, so a 32-step rollout takes 1.2-1.4 s
on an idle machine.

The learner is the bottleneck. A PPO update takes ~0.5 ms per trained sample (two epochs, ~3.5 s for 7,000
samples) on its own and 3.6-4.0 s while the actors run. The actors therefore wait for the learner at every
worker count (wait 0 s in every run). Samples/s below is trained decisions per second of update cycle;
battles/s depends on battle length, which changes as the policy learns. Three-minute runs, 128 battles,
first-run Adam:

| actors | samples/s | battles/s | cycle s | train s | rollout s | actor inference s |
|---|---:|---:|---:|---:|---:|---:|
| serial (first run, same 3 min) | 1,370 | 43.5 | 5.0 | 3.45 | 1.59 | in the update |
| 4 | 1,927 | 51.0 | 3.62 | 3.62 | 2.23 | 1.22 |
| 6 | 1,874 | 50.9 | 3.73 | 3.73 | 1.82 | 0.96 |
| 8 | 1,770 | 47.6 | 3.92 | 3.92 | 1.77 | 0.92 |

More actors only slow the learner through CPU contention, and more battles only make each update bigger:
192 battles with four actors took 6.2 s per update for 10,600 samples, with 12% fewer samples/s than 128
battles in the same session. Lower actor priority did not speed up the learner. With pool battles, four
actors need 2.3-2.6 s per rollout against 3.1-3.3 s of training. Two learner speedups are not applied yet:
the per-minibatch `.cpu()` syncs for statistics and gradient clipping cost ~10% of the update (3.46 s vs
3.09 s), and 2,048-sample minibatches would save another ~8%. With a faster learner, five actors would
keep the rollouts shorter than the update.
