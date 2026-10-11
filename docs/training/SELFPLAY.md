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
| scripted "smart" baseline | `training/smart.py` |
| tests | `training_tests/test_selfplay.py`, `training_tests/test_smart.py` |

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
  non-self opponent on side 2. A third of them (10% of the envs) play a scripted player: by default the smart
  baseline below with temperature 0.05 (`--scripted heuristic` selects the strongest-attack heuristic, now shared
  with `evaluate.py`). Both attack predictably, so Protect, switching and support moves pay off; smart also
  protects, uses Fake Out and focus fire, and beat the first run's final model. The rest
  play a snapshot, chosen per rollout by prioritized fictitious self-play:
  - Weight: `(1 - s)^2`, where `s` is an EMA (rate 0.02 per game) of the learner's score against that snapshot.
  - 10% of the weight is spread uniformly, so beaten snapshots stay in play. Unplayed snapshots start at 0.5.
  - Only games that began during the rollout are credited, so a snapshot change in mid-battle does not blur the
    estimates. The actors report per-env results, and the trainer attributes them to the opponent it assigned.
- **KO shaping** (`training/ppo.py`, `ko_shaping`). The bonus is now potential-based:
  `reward += Phi(after) - Phi(before)` with `Phi = bonus * (foe KOs - own KOs)`, and `Phi = 0` once the battle
  ends. Each row keeps its last potential across rollouts and pays back exactly that, so the bonus sums to zero over
  a battle even while it decays (and still settles once it reaches zero). It still gives credit for KOs early, but no
  longer pays for the KO margin.
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

## Smart scripted baseline

The strongest-attack heuristic never uses Protect, Fake Out, switching or focus fire, so it cannot show whether a bot
handles them. `training/smart.py` (`SmartHeuristicAgent`, name `smart`) is a doubles-aware scripted player with the
same interface as the other agents. It reads only the row's own observation arrays and the masks. It scores every
pair of slot a and slot b codes with the damage-calc module:

- Combined damage on a foe is capped at its HP. This gives focus fire without overkill.
- A KO or Fake Out flinch before a foe acts removes that foe's damage for the turn.
- Foes are assumed to aim where they do more damage. A foe that revealed Protect, or likely has it, may protect;
  its Protect blocks both of our attacks.
- Protect is credited with a random 55-85% of the damage it avoids. It is never used twice in a row and is left
  out of 15% of decisions.
- Tera is used only when it changes a KO chance (a foe's or the user's) by 25+ points, and it still pays a cost.
- Voluntary switches happen only out of a 50%+ KO threat into a much safer Pokemon. Replacements pick the best
  matchup.
- Damage to the partner counts 1.5x.
- Fake Out is used on a Pokemon's first turn out unless another attack likely KOs.
- Common status moves get small fixed values.

Slot a is planned with an estimate of slot b's options, then slot b is re-picked over its exact mask. It costs about
1 ms per row on one CPU thread. `temperature` adds Gumbel noise to the pair values, for use as a training opponent.

`evaluate.py` adds:

- `smart vs heuristic` and `smart vs random` sanity rows.
- Every evaluated snapshot against smart. These matchups use their own seeds, so the older matchups keep theirs.
- Logged final-vs-smart games (`final_mistakes_vs_smart`, `smart_replays`), and `smart_mistakes` from smart's games
  against the heuristic.

New mistake counters:

- Attacks at a foe that could Protect (it protected in an earlier turn, but not the turn before), and how many were
  blocked.
- Own Protect uses, blocks and consecutive-turn repeats.
- Fake Out uses per battle.
- Switches, split into switch actions, pivots (U-turn, Eject Button) and replacements after faints.
- Focus fire and overkill, from the action codes that `play` records per turn for the evaluated player. Overkill
  means an attack chosen at a foe that had already fainted that turn, which then retargets or fails with
  `[notarget]`.

Light check (duplicate games on `eval-s43-200`, 2 workers, 160 games per matchup; full-size runs pending):

| Matchup | Score |
|---|---:|
| smart vs random | 99.4% |
| smart vs strongest-attack heuristic | 81.3% (lost both games of 2 of 80 pairs) |
| first run's final model (60 min) vs smart | 38.1% (about -84 Elo) |

Against smart, the 60-minute model's counters (187 logged battles) show:

- 8.8% of its attacks hit a Protect, and 38% of its attacks at foes that could Protect were blocked.
- 93 of its 190 Protects came right after a Protect by the same Pokemon. Such a repeat fails two times out of
  three.
- It used Fake Out once and terastallized in every battle.
- 5.4% of its chosen single-target attacks went at a foe that had already fainted that turn (smart: 1.6% against
  the heuristic).

Smart itself protected 0.9 times per battle with no repeats, used Fake Out 0.2 times per battle with no failures, and
terastallized 0.75 times per battle.

## Engine from `main` (perf rounds 2-4)

The training work is merged with `main` at `a5fee3d`. That brings the faster engine (typed ids, per-id dex
tables, cheap `clone`/`restore_from`/`reset`, battle reuse on auto-reset) and the new search API
(`GridExecutor`, hidden-set determinization). The textual merge was clean; the training code is Python
only. Rebuild the extension after pulling:
`maturin develop --release` in `crates/pyengine`.

What the training code relies on was rechecked on the new engine:

- `observe`: the finished battle's unread lines arrive in `prev_log`, and the new battle's log starts with its
  switch-ins and `|turn|1`.
- Team draws and battle seeds of the k-th battle per env do not depend on the actions, which duplicate
  evaluation needs.
- `battle_id` starts at 1 after `reset()`.
- Step results, conditional masks and per-process `BatchEnv` thread pools are unchanged.

These were checked with the engine's 27 Python API tests, the 67 training tests, direct reproductions and a
GPT-6.1-Sol review.

Speed (`bench.py`, Rust-driven random play; noisy, a review job ran at the same time):

| | old engine | new engine |
|---|---:|---:|
| 1 thread | 949 battles/s | 1,303 battles/s (+37%) |
| 6 threads | 4,623 battles/s | 5,608 battles/s (+21%) |

Training barely changes, because the actors spend ~1.2 ms of a ~36 ms step in the engine and the GPU update is
the bottleneck. A 3-minute smoke run gave 1,700 trained samples/s and 59 battles/s with no errors (shared CPU).

Two caveats:

- The engine's team pool keeps every parsed team for the pool's lifetime (`crates/pyengine/src/pool.rs`).
  That is bounded for the 2,000-team training pool, but would grow in every actor with the 200k-team pools.
- The review also found an older tracker bug that is now fixed. When Zoroark's Illusion broke (`|replace|`),
  the disguise's record kept what was observed under the disguise, as a phantom benched Pokemon. The engine's
  `replace` line carries no HP, so the revealed Zoroark also showed full health. Now the stint's HP, status,
  boosts, timing, every move used, and any item, ability or Tera seen move to the real Pokemon, whose ability
  becomes Illusion. The disguise's record is dropped if that was its only appearance, and otherwise returns to
  what was known before. Checked on 40 Illusion breaks in engine battles: every revealed HP matched the
  owner's request.
- Still not handled (Illusion is limited to the two Zoroark formes):
  - an earlier disguised stint that ended unrevealed;
  - the real Pokemon and its disguise active at the same time;
  - a disguised Pokemon fainting unrevealed;
  - our own disguised Pokemon's boosts before the reveal.

## Second run (2026-10-11)

60 minutes with the learning setup above and all defaults (`--minutes 60`, four actors, 128 battles): 1,157 PPO
updates and 287,769 battles (81 battles/s, 1,924 trained samples/s; the first run had 53 and 1,248). The plan was
to stop at 30 minutes if it had plateaued; it had not (the training-time score against smart was still rising
about 3 points per 5 minutes), so it ran the full hour. The trainer stayed at 520-670 MB for the whole run and
the largest actor below 470 MB, so the leak fix holds.

Training-time signals (5-minute windows; smart is the training opponent at temperature 0.05):

| minutes | score vs smart | score vs PFSP snapshots | entropy (target) | self-play turns |
|---:|---:|---:|---:|---:|
| 0-5 | 9% | 99% | 2.8 (2.5) | 12.7 |
| 10-15 | 20% | 55% | 2.3 (2.3) | 10.5 |
| 25-30 | 26% | 60% | 2.1 (2.1) | 10.4 |
| 40-45 | 32% | 59% | 1.8 (1.8) | 10.4 |
| 55-60 | 36% | 60% | 1.6 (1.6) | 10.7 |

The entropy controller tracked its target all run (the coefficient touched its 0.05 cap briefly around minutes
7-10 and ended at 0.018). At the end its PFSP score against the pooled snapshots was 0.50-0.68 (the newest lowest).

Evaluation (`evaluate.py`, temperature 1, duplicate games on `eval-s43-200`, 400 games per row, 1,000 for 60 min):

| Snapshot | vs random | vs strongest-attack heuristic | vs smart | final (60 min) vs this snapshot |
|---|---:|---:|---:|---:|
| 0 min | 46.8% | 0% | 0% | 99.8% |
| 10 min | 98.5% | 46.0% | 17.8% | 74.8% |
| 20 min | 99.2% | 55.2% | 23.5% | 66.2% |
| 30 min | 99.2% | 59.8% | 31.0% | 60.5% |
| 40 min | 99.5% | 66.0% | 32.0% | 54.2% |
| 50 min | 99.5% | 62.3% | 32.8% | 54.5% |
| 60 min | 99.5% | 68.6% | 34.3% | |

Smart itself scores 81.8% against the heuristic. Unlike the first run, the final model still beats its 40- and
50-minute snapshots (about +30 Elo, error +/-35), so it was still learning slowly at the end.

Against the first run (1,000 duplicate games each, same seeds for both runs):

| Matchup | temperature 1 | temperature 0 |
|---|---:|---:|
| second run 60 min vs first run 60 min | 47.9% | 56.1% |
| first run 60 min vs smart | 35.6% | 39.1% |
| second run 60 min vs smart | 37.0% | 39.7% |

So after an hour the two runs are about equally strong. The second run is slightly ahead when both play their
most likely action, and slightly behind when both sample, which fits its higher entropy (1.6 against 0.7):
more of its sampled moves are exploration. Neither beats smart.

Behaviour of the final model (from the evaluation's logged games):

- It now protects (0.76 times per battle against the heuristic, 1.13 against smart) and makes about one
  voluntary switch per battle. In training, `use_protect` stayed at ~5% and `use_status` at ~10% of slot decisions.
- It still terastallizes in every battle and almost never uses Fake Out (0.03 per battle, and two thirds of
  those fail).
- Against smart, 9.3% of its attacks hit a Protect, and 46% of its attacks at foes that could Protect were
  blocked (first run: 8.8% and 38%).
- 5.2% of its chosen single-target attacks go at a foe already fainted that turn (first run 5.4%).
- The critic still ranks positions well but is underconfident against the heuristic (predicts 45% where 66% won).

Outputs (git-ignored): `scratch/training/second-run/` with `metrics.jsonl`, `snapshots/`, `evaluation.json`,
`midcheck-m030-m020.json` and replays. The cross-run checks are `scratch/training/h2h.py` and
`scratch/training/vs_smart.py`.

### Second hour

The run continued from the 60-minute `latest.pt` for another hour (`--resume ... --minutes 120`), with more scripted
games and schedules that continue from where the first hour ended:

| Setting | First hour | Second hour |
|---|---|---|
| `--pool-fraction` / `--scripted-share` | 0.3 / 0.33 (10% smart, 20% snapshots) | 0.45 / 0.5 (22% smart, 23% snapshots) |
| entropy target | 2.5 -> 1.5 | 1.5 -> 1.2 (`--entropy-target 1.8 --entropy-target-final 1.2` over 120 minutes) |
| learning rate | 3e-4 -> 9e-5 | 1.05e-4 -> 6e-5 (`--lr 1.5e-4 --lr-final 0.4`) |

`--minutes 120` with these start and end values makes the schedules continue at the 60-minute values instead of jumping
back. 296,626 more battles (584,395 in total), 78-87 battles/s, trainer memory 580-710 MB. The training-time score
against smart rose from 38% (minutes 60-65) to 42% (85-90) and 46% (115-120).

Evaluation (`evaluate.py` on `scratch/training/second-run-2h`, which holds the 60-minute snapshot and the new ones):

| Snapshot | vs heuristic | vs smart | 120 min vs this snapshot |
|---|---:|---:|---:|
| 60 min | 67.5% | 37.0% | 58.0% |
| 70 min | 74.8% | 40.2% | 54.0% |
| 80 min | 67.5% | 38.8% | 54.2% |
| 90 min | 73.0% | 41.0% | 53.8% |
| 100 min | 72.5% | 42.0% | 50.5% |
| 110 min | 70.2% | 40.2% | 49.8% |
| 120 min | 72.5% (1,000 games) | 42.7% (1,000 games) | |

So the second hour added about +55 Elo, most of it by minute 90; the last 20 minutes were flat. Against the first
run's final model (1,000 duplicate games): 59.3% at temperature 1, 60.8% at temperature 0. Against smart, four
separate 1,000-game checks gave 39.9-45.3% (mean about 42%); the spread is larger than the duplicate games suggest, so
single 1,000-game checks against smart are good to about +/-3 points.

Its habits did not change: it terastallizes in every battle, uses Fake Out 0.02 times per battle (72-77% of them
fail), and against smart 46% of its attacks at foes that could Protect are blocked. It switches a little more (1.1-1.3
voluntary switches per battle). These are the actions the features did not describe; see the next section.

## Action effects: Fake Out, Protect and Tera

After two hours of training the policy still terastallized in every battle, almost never used Fake Out (0.02 per
battle, and three quarters of those failed), and 46% of its attacks at foes that could Protect were blocked. The
action features did not describe these actions:

- Every code's damage features were the same with and without Tera, so the model never saw what Tera did.
- Fake Out looked like a working 40-power priority attack on every turn. Only the user's `fresh` flag said it would
  fail, and nothing said that a flinched foe loses its move.
- Nothing tied a repeated Protect to its 1/3 success chance, or an attack to the chance that its target protects.

`training/damage.py` now appends effect columns (`ACTION_COLUMNS`, `TOKEN_COLUMNS`) after the existing ones:
23 action features (was 9) and 10 token features (was 6). Per move code:

| Column | Meaning |
|---|---|
| `fail` | Fails outright: Fake Out or First Impression after the user's first move action since switching in; a priority move at a foe that Psychic Terrain (grounded) or Armor Tail, Dazzling or Queenly Majesty protects; a Prankster status move at a Dark-type foe |
| `flinch_a`, `flinch_b` | Chance to flinch foe a/b before it acts: flinch chance (Serene Grace doubles it, Sheer Force removes it) x accuracy x acting first x the foe can flinch (no Inner Focus or Shield Dust unless the user has Mold Breaker, no Covert Cloak or Substitute; known or by belief) x not immune |
| `stop_damage`, `stop_ko` | Damage and KO chance those flinches prevent this turn (each foe's best threat against our actives) |
| `protect` | Success chance of a Protect-like move: 1, or 1/3 right after a successful one |
| `protect_damage`, `protect_ko` | `protect` times the damage and KO chance it blocks this turn (what the user faces minus what gets through: moves without the `protect` flag and Unseen Fist contact moves) |
| `foe_protect_a`, `foe_protect_b` | Chance that the targeted foe can Protect: it has a Protect move (revealed, or by the set belief) at full or 1/3 success; 0 for moves without the `protect` flag (Feint) and Unseen Fist contact moves |
| `tera_damage`, `tera_ko` | Tera codes only: change of damage and KO chance on the foes from terastallizing (Tera STAB; Stellar 2x on the original types and 1.2x otherwise; Tera Blast takes the Tera type and the higher attacking stat) |
| `tera_threat`, `tera_threat_ko` | Tera codes only: change of the damage and KO chance the user faces this turn (its Tera type) |

"Acting first" uses the priority the move will have (Prankster status moves, Gale Wings at full HP, Grassy Glide in
Grassy Terrain), then speed (Trick Room flips it), and a fresh foe that may have Fake Out goes first against anything
below +3 and against a slower +3 move. The legacy `first` columns are unchanged and still treat any positive priority
as first.

Per Pokemon: `fake_out_ready` (active, fresh and has Fake Out, known or by belief), `flinchable`, `fake_out_threat`
(an opposing active can Fake Out it this turn, on both sides) and `protect_ready`.

Known gaps: after two successful Protects in a row the success chance is 1/9, but the columns still say 1/3 (the
tracker only keeps "protected last turn"). Triple Arrows' flinch chance (30%) is hard-coded because `dex.json`'s
`secondaryChance` is the largest of a move's secondaries (its Defense drop, 50%). King's Rock is ignored. The values
are chances that a foe *can* do something (Protect, Fake Out), not that it will.

The Tera rows ride along in the existing damage calls as two extra attackers and defenders, and the shared per-Pokemon
state is computed once. The features take 12.9 ms per 32 rows on one thread (7.1 ms before), about 6 ms more per
actor step. In 3-minute smoke runs four actors still kept ahead of the learner (rollout 2.96 s, training 3.56 s,
1,888 trained samples/s); five actors gave 1,844 samples/s. Late in the second hour, with shorter battles and 22%
smart games, rollouts (3.07 s) had caught up with training (3.0 s), so a long run with those settings may be
actor-bound by about 10-15%.

The tracker also had two errors that these columns depend on:

- `|cant|X|flinch` (also par, slp, frz, recharge, Truant, Taunt) now counts as a move action. The engine counts
  `activeMoveActions` before `BeforeMove`, so a Pokemon flinched by a Fake Out on its first turn could no longer
  Fake Out, but the tracker still showed it as fresh. `cant` lines with `[of]` name the Pokemon whose ability blocked
  the move, and are skipped.
- A failed Protect (`|move|X|Protect||[still]`, then `|-fail|X`) resets the counter, so it no longer counts as
  "protected last turn". Switching out also resets it. The smart baseline reads these flags too, and may now protect
  again after a failed Protect, as it should.

The first 9 action and 6 token columns are unchanged: on 3,840 rows of random play they are bit-identical to the
old code. Only `fresh` (48 entries) and `protect_last` (244 entries) changed, from the tracker fixes. A model reads
only the columns it was built with, and `Model.from_state` (used by `evaluate.py`) infers them from a checkpoint, so
the first and second runs' models still play against new ones. They cannot be resumed with the new features.

New training diagnostics: `fake_out_ready` (share of slot decisions with a legal Fake Out that would flinch a foe),
`use_fake_out_when_ready`, `fake_out_stale` (share of Fake Outs by a Pokemon that already used a move action since
switching in) and `protect_repeat` (share of Protects right after a successful one). Tests:
`training_tests/test_effects.py`. A GPT-6.1-Sol review found nine issues; all but the 1/9 Protect chance are fixed
and tested (Unseen Fist through Protect, Tera Blast and Stellar Tera, Prankster/Gale Wings/Grassy Glide priority,
Fake Out against a faster Fake Out, Sheer Force/Serene Grace/Mold Breaker, Triple Arrows, and the diagnostics).

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
  tensors (the update is ~3% slower).
- DirectML also compiles kernels for every new input shape and never frees them, ~4.4 MB per minibatch size.
  The trained-row count changes every rollout, so the last minibatch had a new length almost every update:
  the trainer grew ~3.8 MB per update (~4 GB per hour). `training.ppo.minibatches` cuts the last minibatch to
  a multiple of 128 rows (and still drops it below 256), so a run sees at most seven sizes. Measured with
  `ppo_update` alone on one saved rollout: 3.3 MB per update with a varying row count, 0.06-0.1 MB with a
  fixed one, and with four sizes in turn growth only on each size's first use. In an 8-minute run with the
  fix, the trainer grew 0.4 MB per update while the opponent pool filled (each snapshot ~3.7 MB) and
  0.1 +- 0.15 MB per update over the 39 updates after it was full. The next-run benches skipped most last
  minibatches (below 256 rows), which is why they grew less than the second-run smoke run. Their remaining
  ~1.2 MB per update with a full pool does not show on `second-run`; its cause was not pinned down (next-run's
  `ppo_update` with a fixed row count is flat on its own).
- The encoder's static-feature caches are capped at 50,000 entries each: foe reveal states never stopped
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
actors need 2.3-2.6 s per rollout against 3.1-3.3 s of training. The per-minibatch `.cpu()` syncs for
statistics and gradient clipping cost ~10% of the update (3.46 s vs 3.09 s); they are removed (statistics and
the clipping scale stay on the device and are read once per update). 2,048-sample minibatches would save
another ~8% but are not applied. With a faster learner, five actors would keep the rollouts shorter than
the update.

Integrated smoke run (`second-run` with the league, smart opponents, entropy controller and the sync removal;
four actors, 128 battles, 4 minutes): 70 updates, 14,800 battles, 1,821 trained samples/s and 64 battles/s
from update 3 on, a 3.2 s cycle with rollouts of 2.7 s. The trainer's memory grew by about 3.8 MB per
update there; that was the varying last-minibatch size (see the DirectML notes above), now fixed.
