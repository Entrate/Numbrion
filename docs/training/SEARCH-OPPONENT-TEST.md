# Search-opponent test (Windows, 1 hour from run 1's final model)

Question: does training with the in-training search opponent (`--search-fraction 0.2`, its decisions as distillation
labels; SELFPLAY.md, "Search opponent in training") improve the policy faster than run 1 did? Run 1 ended at m535
(update 7469); its last hour is the reference. Models improve more slowly as they get stronger, so the comparison
is at the same starting point, not against run 1's early hours.

## Setup

1. `origin/main` at 763c457 or later. From a worktree, pass `--pool` with the main checkout's
   `data\teams\train-s42-2000.txt` (team files are not in git).
2. Start: copy `scratch\distill-loop\train\latest.pt` (run 1 final, elapsed about 535.1 min, the m535 weights) to
   `scratch\search-ab\start.pt` **with the `distill_stream` key deleted**: its stream state refers to run 1's label
   shards and must not load into a fresh stream directory.
3. Nothing else uses the GPU or CPU during the run.

## Run

As a scheduled task (it survives SSH disconnects):

```
train_selfplay.py --resume scratch\search-ab\start.pt --width 192 --layers 4 --workers 6 --minutes inf
  --schedule-minutes 120 --lr-final 0.33 --entropy-target-final 1.2 --ko-bonus-end 0.15
  --teacher-coef 0.2 --teacher-end-minutes 36.5 --stop-minutes 595.1
  --search-fraction 0.2 --distill-stream scratch\search-ab\B-labels --distill-coef 0.5
  --distill-end-minutes inf --distill-max-batches 6 --distill-max-uses 1 --distill-max-age-minutes 10
  --output scratch\search-ab\B
```

Run 1's configuration plus the search opponent, for 60 training minutes. Every schedule is at its final value, so
the learning rate stays at 9.9e-5. `--distill-end-minutes inf` is required: with the default 60 the distillation
weight is already 0 at elapsed 535. `--search-labels` defaults to the stream directory, so the actors write labels
where the trainer reads them. `--workers 6`: with 4 workers the search makes updates 36% slower instead of 21%.

Check the first `metrics.jsonl` lines: `search_decisions` about 680 per update and `search_labels` equal to it,
`distill_batches` above 0 after a few updates, `distill_loss` finite, `cycle_s` about 4.8 s,
`search_unswapped` 0. If anything is off, stop and report.

## Evaluation

`crates\pyengine\compare.py` with defaults (4000 games, T=0.5), `--ref scratch\search-ab\start.pt`:

1. `--candidate scratch\search-ab\B\latest.pt --best scratch\search-ab\start.pt`: the hour's gain with the search
   opponent.
2. Reference (no training): `--candidate scratch\distill-loop\train\snapshots\m535.pt --best` the run 1 snapshot
   about 60 minutes earlier (m475 or the nearest): run 1's last-hour gain.

## Report

`scratch\reports\search-ab\REPORT.md`: updates and battles; mean `cycle_s`; mean `search_decisions` and
`search_labels` per update; `search_win_rate` (the learner's score against the search); `distill_loss`,
`distill_val_loss`, `distill_agree` and distill batches per update; both head-to-head scores with 95% intervals;
anything abnormal (crashes, illegal actions, NaNs, rejected shards).

Reading: each head-to-head is about ±1.5 points. If the search-opponent hour beats the reference hour by 2-3 points
or more, the search opponent works; within about 1.5 points, it did not show a difference.
