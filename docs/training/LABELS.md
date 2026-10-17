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

## The label-only generator: shallow_labels.py (2026-10-15, Mac)

`crates/pyengine/shallow_labels.py` runs `training.actors.Actors` with every env a `SEARCH` env and no learning
step. Both sides of every battle search (`Actors(search_both=True)`) with the teacher's candidates and priors, so one
battle gives two sides' labels. Each side's search hides the other side's unseen bench with pool sets. With
probability `--explore` (default 0.25, as in search_labels.py) a side plays a pair sampled from the teacher policy
at temperature 1 instead of the search's pick. That keeps the positions varied, and the label is still the search's
strategy. `--value-checkpoint` gives the leaf values from a separate fixed net of any size. Each actor loads it
once, and it never needs a weight sync. The actors run on the CPU, one torch thread each, at `--nice 10`.
They use `Actors(label_only=True)` to skip unused PPO rollout buffers, rollout value-head calls, and bootstraps.

    .venv/bin/python crates/pyengine/shallow_labels.py --checkpoint scratch/label-student/c128-all.pt \
        --value-checkpoint scratch/play-runtime/models/distill-run1/m535.pt \
        --workers 7 --envs 168 --minutes 600 --out scratch/gen1/shallow-labels-c128-m535v

- Rows have search_labels.py's format, so distill_train.py and `training.distill` read them as they are. The
  partner arrays come from the env's other encoder row, the opponent's own view (`train_selfplay.partner_view`).
  A row waits until its battle ends. Then `outcome` is +1/-1/0 from the labelled side, and `remaining_steps`
  counts every boundary of the env up to the terminal transition, forced switches and waits included, as GAE
  counts them. Rows of battles still running at shutdown, or of a battle an engine panic aborted, are written
  capped (NaN / -1). They keep their policy target and get no value target.
- The standalone generator excludes labels when exact legality exposes trapping or Imprison facts absent from
  the player's request, or when any unseen opponent set cannot be swapped. This covers failed donor retries,
  battle-start forms, and Illusion-protected bench sets. `rejected_labels` counts the exclusions. Retained masks
  and priors use only request-visible legality. Play continues through excluded decisions.
- Stop with `--minutes`, `--labels` (finished rows), or `touch <out>/STOP`. Each stops after the current
  `--steps` rollout. `--follow DIR` reloads the newest teacher of DIR between rollouts, and the value net stays
  fixed. A status line every `--status-seconds` reports labels/s, pending rows, disk, actor cores and RSS. It also
  goes to `<out>/status.jsonl`. SIGINT and SIGTERM sent to the parent also drain the current rollout and save
  capped rows. Engine panic reports are saved in `<out>/crashes.jsonl`.
- train_selfplay's `--search-fraction` opponent uses the same code with side 2 only and no exploration. Its labels
  now carry the value fields too, and its rollouts are bit-identical to before (same buffer hash).
- Pilot on the M1 Pro (8 cores), c128-all + m535 values, both sides, 7 workers x 24 envs: ~1,180 labels/s, about
  6.2-6.7 cores busy, ~720 MB RSS per actor, 885 bytes per label. 6 workers (side-2 mode) gave ~1,080 labels/s.
  The search's top pair differed from the policy's top pair in 34% of rows. Value vs final outcome correlated at
  0.61 (0.95 within 3 steps of the end, 0.35 at 10-30 steps).
- 10 hours is about 42M labels and 37 GB. `scratch/gen1/shallow-labels.sh` caps the run at 45M labels.
- **Fair information:** the shallow search knows the unrevealed moves, item, ability, Tera type and stats of the
  opponent Pokemon it has seen. Only the unseen bench is resampled. A `--fair` mode would build those sets from
  what was seen, as the full search does, at ~25-35 ms instead of ~4-5 ms per decision. See
  [SEARCH-PREREQUISITES.md](../design/SEARCH-PREREQUISITES.md).

### On the Windows PC, sent to the Mac (2026-10-17)

shallow_labels.py runs on Windows too (a positive `--nice` means below-normal priority there). `send_labels.py`
runs beside it on the same directory and copies finished shards to the Mac over SSH (key login as `aminaliu` over
Tailscale). It scp's them into `<remote>/.incoming/`, then renames each into the label directory unless that name
is already there or in `used/` / `heldout/`. So `distill_train --follow` never reads a partial file, and a retry
never adds a shard twice. Sent shards move into `<dir>/sent/` locally. The sender exits
after the maker finishes. Use the Mac's code version and settings, with another `--seed`:

    .venv/Scripts/python.exe crates/pyengine/shallow_labels.py --checkpoint scratch/gen3/teacher-win/numbrion-gen2.pt         --workers 6 --envs 144 --explore 0.25 --ours 4 --theirs 3 --seeds 1 --seed 9001 --minutes 600         --out scratch/win-labels/NAME
    .venv/Scripts/python.exe crates/pyengine/send_labels.py scratch/win-labels/NAME         --to aminaliu@100.98.169.85:/Users/aminaliu/.t3/projects/numbrion-mac/scratch/gen3/labels-win

- Mac checkpoints pickle `PosixPath`s, which Windows cannot load. Re-save them with the paths as strings, under the
  same file name, so the labels name the same teacher (`scratch/gen3/teacher-win/`).
- Ryzen 5 3600 (6 cores / 12 threads), 1.6 min tests: 6 workers x 24 envs gave ~545 labels/s steady (gen2) and
  ~575/s (Spark), on 5.8 cores, ~450 MB RSS per actor, 0 crashes. 11 workers gave ~595/s, so the hyperthreads
  add little. That is about a third of the M1 Pro's rate (~1,600/s on gen3), ~2M labels/hour, ~1.8 GB/hour.
- The shards have the Mac's format: same fields, dtypes and shapes, and teacher `numbrion-gen2.pt`. SSH sends at
  2-3 MB/s, and labels come at ~0.5 MB/s.
- Taildrop (`tailscale file cp`) did not work unattended: the Mac holds incoming files until they are accepted.

Then: millions of labels -> a fresh student over several passes -> compare vs m535 and the 256x6 average; if it wins,
it becomes the next teacher.
