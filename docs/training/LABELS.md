# Learning from search labels (2026-10-15, Windows)

Why PPO and the label cycles stalled at m535, and what to do instead. Raw data and scripts are in the Windows
machine's `scratch/search-checks/` and `scratch/label-curve/` (not in git).

## Search is worth learning from

- **The gain is general.** m535 + search (default SearchConfig) beat the m535 policy by about 24 points against m535,
  the 256x6 average and m240 alike (~200 mirrored pairs each), and by 16.5 against smart (70.8% -> 87.2%, about the
  same step in log-odds). The search is not just exploiting a known opponent.
- **The labels are mostly signal.** Searching the same position three times with independent seeds: the same top
  pair 85% of the time, 77% of the label's shift away from the prior repeats, centred q correlation 0.82. Where
  the label overrules the policy (27% of positions) the top pair repeats 66% of the time.

## A fresh policy learns search from labels alone

`crates/pyengine/train_labels.py`: a fresh model, no self-play, cross-entropy against the search's strategy over
the top-4 candidates (the `--distill` target), lr 1e-3 decayed to 0 by a cosine over 4 passes. Test: the held-out
full-search shards (48k positions). Strength: compare.py vs m535, policy only, T=0.5, 4,000 games unless noted.

| Labels (128x3) | Picks search's top pair | vs m535 [95%] |
|---|---:|---:|
| 33k full | 46.6% | 25.6% (2,000 games) |
| 131k full | 54.9% | 40.9% (2,000 games) |
| 262k full | 57.7% | 46.8% [45.4, 48.3] |
| 524k full | 59.4% | 50.0% [48.6, 51.5] |
| 956k full | 60.4% | 50.6% [49.2, 52.1] |
| **1.80M full + shallow** | 60.2% | **52.7% [51.3, 54.2]** |
| m535's own policy | 57.8% | - |

- **More labels keep helping** past 1M (+2.1 points for about twice the labels). Returns per doubling shrink.
- **Model size is not the limit yet.** 192x4 learned exactly as much as 128x3 on all 956k full labels (49.3% vs
  49.5% vs m535, constant-lr runs) and at 262k (44.8% vs 46.4%).
- **Teacher strength barely matters.** 262k labels from teachers m120-m300: 45.3%; from c01-c19 and m480+: 46.3%;
  a random mix: 46.8%. 441k strong-teacher labels (49.7%) did as well as 524k mixed ones (50.0%).
- **Shallow labels are as good as full ones.** 524k labels from the in-training search opponent (one world, 4x3
  pairs, one seed): 50.3% vs 50.0% for 524k full-search labels. They are about 10x cheaper to make.
- The label cycles (`docs/training/DISTILL-LOOP.md`, ~22k labels per cycle on top of m535) could not show this:
  one cycle is a small fraction of what moves a policy at m535's level.
- Label-trained models have untrained value and hidden-information heads: they can play policy-only but cannot
  drive search until those heads are trained (`train_selfplay.py --critic-warmup-updates`).
- Evaluation note: with a constant lr, validation loss was too noisy for early stopping (it stopped the large runs
  ~1-2 points early); the cosine schedule with final weights fixed that.

## Next: a label-only generator

The shallow search only runs inside PPO training today (`training/search_opponent.py` in the actors, ~125 labels/s
on Windows with 20% search games, the actors mostly waiting for the learner). The plan is a driver without PPO:

- `shallow_labels.py`: `training.actors.Actors` with every env a `SEARCH` env and no learning step. Both weight slots
  hold the teacher; side 1 plays the teacher policy (sampled, which also varies the positions), side 2 the shallow
  search, whose decisions the actors already write as label shards (`search_labels=`). Options: workers, envs,
  minutes or a label target, low priority, `--follow DIR` to reload the newest teacher between rollouts.
- CPU only, so the GPU can train a student on the growing set at the same time (`train_labels.py` streams).
- Disk: about 0.86 KB per label (10M labels ~ 9 GB).
- Pilot first: labels per second with 1..N workers.
- **Fair information:** the shallow search knows the unrevealed moves, item, ability, Tera type and stats of the
  opponent Pokemon it has seen (the unseen bench is resampled). A `--fair` mode should build those sets from what
  was seen, as the full search does (~25-35 ms instead of ~4-5 ms per decision); pilot both. See
  [SEARCH-PREREQUISITES.md](../design/SEARCH-PREREQUISITES.md).

Then: millions of labels -> a fresh student over several passes -> compare vs m535 and the 256x6 average; if it wins,
it becomes the next teacher.
