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

## Learning setup after the first run

The first run committed early: entropy fell to 1.5 within six minutes while the KO bonus paid for aggression,
and pure self-play never punished the one all-out style. The changes below replace the entropy, opponent and
reward defaults described under "Training" above. `train_selfplay.py --help` lists the options.

- **Exploration floor** (`training/ppo.py`, `EntropyController`). After every update the entropy coefficient
  adapts so that the logged entropy follows a target that falls linearly from 2.5 to 1.5 over the run. The logged
  entropy is slot a plus slot b, averaged over trained rows, with forced decisions counting as zero. The update is
  `log(coef) += 0.2 * (target - entropy)`, with the error clipped to +/-1 nat. The coefficient starts at 0.01
  and is clamped to [0.001, 0.05]. For comparison, the first run reached 1.5 at minutes 3-6 and ended at 0.72.
  `--entropy-target 0` restores the fixed 0.01 -> 0.003 schedule. The controller state is saved in `latest.pt`.
- **Opponents** (`training/league.py`, `training/scripted.py`). 30% of the envs (previously 20%) have a fixed
  non-self opponent on side 2. A third of them (10% of the envs) play the strongest-attack heuristic, now shared
  with `evaluate.py`. It attacks predictably, so Protect, switching and support moves pay off against it. The rest
  play a snapshot, chosen per rollout by prioritized fictitious self-play:
  - Weight: `(1 - s)^2`, where `s` is an EMA (rate 0.02 per game) of the learner's score against that snapshot.
  - 10% of the weight is spread uniformly, so beaten snapshots stay in play. Unplayed snapshots start at 0.5.
  - Only games that began during the rollout are credited, so a snapshot change in mid-battle does not blur the
    estimates. The actors report per-env results, and the trainer attributes them to the opponent it assigned.
- **KO shaping** (`training/ppo.py`, `ko_shaping`). The bonus is now potential-based:
  `reward += Phi(after) - Phi(before)` with `Phi = bonus * (foe KOs - own KOs)`, and `Phi = 0` once the battle
  ends. It sums to zero over a battle. It still gives credit for KOs early, but no longer pays for the KO margin.
  Under the old bonus, the margin was worth up to +/-0.25 on top of the +/-1 result. The default is 0.02
  (previously 0.05). It decays to zero at 25% of the run (previously 50%).
- **Diagnostics** (`training/diagnostics.py`). Every metrics line now includes:
  - games, score and mean battle turns per opponent kind (`self_*`, `pool_*` for snapshots only, `scripted_*`).
    `self_turns` is comparable with the first run's 9.5.
  - `opponent_minutes` and the PFSP table `pfsp` ([minutes, learner score EMA, games] per snapshot).
  - `entropy_target` and `entropy_coef`.
  - Action usage over the learner's slot decisions on move turns (forced switches and PASS excluded):
    `use_tera`, `use_tera_when_legal`, `use_switch` (voluntary), `use_protect` (Protect, Spiky Shield, Baneful
    Bunker, Burning Bulwark), `use_status` (other status moves) and `use_fake_out`.
  - `choice_slots`: slots per row with two or more legal codes, which shows how much forced decisions dilute
    the entropy.
- **Evaluation temperature.** `evaluate.py --temperature T` samples the snapshots at temperature T (default 1,
  unchanged; 0 = the most likely legal code). With a higher entropy floor, comparing T = 1 with T = 0.5 separates
  exploration noise from strength.

Tests: `training_tests/test_league.py`. In the next run, watch for:

- `entropy_coef` stuck at 0.05: the target is out of reach, so raise the cap or lower the target.
- Whether `use_protect`, `use_switch` and `use_status` rise above the first run's near-zero levels.
- `scripted_win_rate` rising together with `pool_win_rate` rather than at its expense.
- Snapshots whose `pfsp` score stays below 0.5: strategies the learner has not learned to beat.

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
