# Continuous search distillation

## Label-only students with a critic

`crates/pyengine/distill_train.py` trains a fresh policy, value head and hidden-information heads from search labels.
The defaults follow the label learning-curve experiment: 128x3, four epochs, batches of 256, four candidate pairs,
Adam at 1e-3 with cosine decay to zero, and the final weights. Policy loss is `training.distill.distill_loss`.
Each training row appears once per epoch. Compact arrays stay resident up to `--resident-rows`, default 60000;
larger datasets shuffle shards through blocks controlled by `--shuffle-buffer`, default 32768 rows.

New `search_labels.py` shards add these optional row arrays. Existing `value` and `q` keep their search meanings.

| Field | Shape per row, dtype | Meaning |
|---|---|---|
| `partner_ids` | 12 x N_IDS, int64 | Opponent row's ids from its own request and player-view log |
| `partner_floats` | 6 x N_POKEMON_FLOATS, float32 | Opponent row's own-team floats, tokens 0:6 |
| `foe_match` | 6, int64 | Labelled side's revealed foe tokens mapped into the partner's own-team order, -1 for unmatched |
| `outcome` | scalar, float32 | Labelled side's terminal reward: win +1, loss -1, tie 0; NaN if unfinished |
| `remaining_steps` | scalar, int32 | Decision boundaries after this row until the terminal transition; -1 if unfinished |

Both own views are encoded before either side chooses. This matches `Actors.observe` and `partner_view`, where
row `2e+s` pairs with row `2e+(s^1)`. Search players keep their public opponent views. The extra encoder skips damage
features. The last decision has zero remaining steps. Forced switches and boundaries where the labelled side waits
still count, as they do in `BatchEnv.step` and GAE. This is a terminal Monte Carlo return, without PPO's optional KO
shaping or lambda bootstrap.

The critic target is `L * outcome * gamma**remaining_steps + (1-L) * value`, with `--value-target-mix L`, default
0.5, `--gamma 0.995`, and `--value-coef 0.5`. Equal mixing is a starting compromise between noisy game outcomes and
the teacher critic's biased root estimates; it has not been tuned for strength. Set L to 1 for terminal returns or
0 for search-root targets. Legacy rows and unfinished games have no critic loss, even at L=0. Rows with partner
inputs and `foe_match` train the same auxiliary targets and hidden-information masks as PPO, weighted by
`--aux-coef 0.03`. Auxiliary training can use unfinished games. Old rows have neither critic nor auxiliary loss.
PPO's `--distill` and `--distill-stream` accept either format and continue to add policy loss only.

Run from `crates/pyengine`, with paths adjusted to your label and checkpoint directories:

```bash
../../.venv/bin/python search_labels.py \
  --checkpoint ../../scratch/value-labels/m535.pt --minutes 60 \
  --device auto --concurrency 5 --shard 2000 --out ../../scratch/value-labels/labels
../../.venv/bin/python distill_train.py train \
  --labels ../../scratch/value-labels/labels --output ../../scratch/value-labels/student \
  --device auto --width 128 --layers 3 --epochs 4 --lr 1e-3 \
  --value-target-mix 0.5 --value-coef 0.5
../../.venv/bin/python distill_train.py score ../../scratch/value-labels/student/latest.pt
```

Holdouts are whole shards with `holdout=true`, files in `heldout/`, or files explicitly passed with `--heldout`.
For legacy shards without metadata, every twentieth shard is held out. Root, `used/` and `heldout/` directories are
discovered together. Training refuses a split without both training and held-out rows. For a tiny run with fewer
than twenty shards, pass `--heldout FILE`. Shards from one game can span both splits, so these metrics do not replace
an independent battle evaluation. Score reports agreement among stored candidates, conditional CE, policy loss,
value MSE and explained variance against the mixed target. Undefined value metrics are JSON null.

`latest.pt` saves after every complete epoch. Continue label training with `train --resume FILE --output DIR
--epochs N`, where N is the total epoch budget. Resume keeps the saved split and requires the same loss, batch,
candidate and learning-rate settings. `--checkpoint FILE` instead initializes weights with a fresh optimizer.
Checkpoints load with `Model.from_state`, search, `compare.py`, `search_eval.py`, and PPO `--resume`, including
`--critic-warmup-updates`. For PPO, pass the checkpoint's width, layers and joint-feature flag. Its PPO update and
elapsed-time schedules start at zero; label-training progress is stored separately. A resumed label run can replay
work since the last completed epoch. Match `score`'s mix and gamma flags to any nondefault training target.

The Mac makes search labels from its newest teacher checkpoint. A Mac sync process sends finished shards through
the Linux laptop to Windows and brings back trainer snapshots. Windows runs PPO self-play with a stream of search
labels and restarts training after a crash. All commands below run from the appropriate repository root.

| Machine | Process | Files |
|---|---|---|
| Mac | `crates/pyengine/search_labels.py --follow` | `scratch/distill-loop/teacher/`, `scratch/distill-loop/labels/` |
| Mac | `tools/distill/mac_sync.py` | `scratch/distill-loop/sync/sent.json`, `status.json` |
| Laptop | SSH/scp relay | `/home/amin/distill-relay/` temporary batches |
| Windows | `tools/distill/windows-train.ps1` supervisor and trainer | `scratch/distill-loop/train/`, `scratch/distill-loop/labels/` |

Mac paths default to `/Users/aminaliu/.t3/projects/numbrion-mac/scratch/distill-loop`.
Windows paths default to `C:\Users\aminu\.t3\projects\numbrion\scratch\distill-loop`.
Deploy with the matching trainer support for `--distill-stream`, `--schedule-minutes`, and `STOP`. The static
`--distill` loader must also ignore scalar shard metadata when concatenating row arrays.
The Mac connects to `amin@100.105.66.101` on port 91. That laptop connects to `amin@100.96.185.127` using
`/home/amin/.ssh/id_ed25519`. The Mac's key is not accepted by Windows. Sync uses a persistent Mac-to-laptop SSH
connection and encoded PowerShell commands for the Windows hop. It batches up to 64 shards per transfer.

## Start, stop and status

Copy a trusted trainer snapshot into the Mac teacher directory before starting. Use a `.part` destination while
copying, then rename it to `m115.pt` or the snapshot's actual name. Never overwrite a finished teacher in place.
The label maker chooses `m<number>.pt` by integer, so `m1000.pt` is newer than `m999.pt`. If there are no numbered
snapshots, it chooses the newest finished `*.pt` by modification time. `.part` and `.tmp` files are ignored.

On Windows, start from a full trainer checkpoint such as `latest.pt`, including optimizer, pool, elapsed time and
RNG state. Start checks it with the repository's venv Python before registering the task. Teacher snapshots are
only for label making. Later starts prefer `train\latest.pt` automatically; any supplied `-Resume` is also checked.

```powershell
.\tools\distill\windows-train.ps1 -Action start -Resume C:\path\to\latest.pt
.\tools\distill\windows-train.ps1 -Action status
.\tools\distill\windows-train.ps1 -Action stop
```

Start registers and runs a scheduled task for the current Windows user with an interactive, limited token. Task
Scheduler owns the supervisor, so ending the OpenSSH session does not end training. It needs no administrator
rights or stored password on a normal Windows setup, but that user must already be logged in and must stay logged
in. A locked desktop is fine. A machine whose policy forbids user-created tasks will need its task permissions
adjusted. The task has no execution time limit and may run on battery. It does not start automatically after reboot;
run `-Action start` again after logging in. A file lock and task state prevent duplicate supervisors.

The supervisor runs `.venv\Scripts\python.exe`, resumes `latest.pt` when available, and restarts after 30 seconds
unless `train\STOP` exists. Stop creates that file. The trainer saves `latest.pt` at the next update boundary, exits,
and the supervisor ends. Status prints supervisor/trainer pids, scheduled task state, the latest metrics line with
`update`, `minutes` and distillation counters, and active/used/heldout shard counts.
Stdout appends to `train\trainer.log`; stderr appends to `train\trainer.log.err`.

On the Mac:

```bash
tools/distill/mac-labels.sh start
tools/distill/mac-labels.sh status
tools/distill/mac-labels.sh stop
```

Start refuses an empty teacher directory and holds an exclusive `start.lock` directory through startup. It launches
the label maker and sync with `nohup`, with pids and logs in
`scratch/distill-loop/`. The label maker uses the 200k set pool at
`/Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/data/teams/pool-s1-200k.txt.gz`, the training team pool
`data/teams/train-s42-2000.txt`, exploration 0.25, 256-row shards, and concurrency 5. Stop creates `labels/STOP`,
waits for current games to finish and the partial shard to flush, then signals sync to attempt a final send/pull
round before exiting. If sync is already down, stop runs one sync round directly. A network failure can leave a
backlog; status reports the last error. Restart sync or the full loop to retry it. Stop Windows after stopping the
Mac if you want to deliver all final labels first. Windows may still have queued labels when stopped.

`PY` overrides the Mac Python interpreter. `DISTILL_CONCURRENCY` changes concurrency. The teacher check defaults
to 20 minutes. To check every five minutes, stop and restart with:

```bash
DISTILL_RELOAD_MINUTES=5 tools/distill/mac-labels.sh start
```

For a direct label-maker run, use `--reload-minutes 5`. Existing games finish with their original bots; new games
rebuild thread-local bots for the current teacher. Logs show teacher switches and `labels/s` every 30 seconds.
If a candidate disappears during selection or cannot be read, the label maker keeps its current teacher and retries
at the next check. Each label maker holds an exclusive `labels/.labels.lock` for its lifetime, including draining
games and flushing rows. Another process using the same output directory refuses to start. The lock file stays on
disk; the operating system releases the lock when the process exits.
`--minutes inf` runs until `STOP`; `--checkpoint FILE --minutes 60` still makes a finite one-shot run.

The config block at the top of `mac_sync.py` lists every host/path override, including `DISTILL_LAPTOP`,
`DISTILL_LAPTOP_PORT`, `DISTILL_HOP_KEY`, `DISTILL_RELAY`, `DISTILL_WINDOWS`, `DISTILL_WINDOWS_LABELS`,
`DISTILL_WINDOWS_SNAPSHOTS`, `DISTILL_LABELS`, `DISTILL_TEACHER`, and `DISTILL_SYNC_STATE`. `DISTILL_BASE` changes
Mac wrapper state and log paths. Windows accepts `DISTILL_WINDOWS_REPO`, `DISTILL_WINDOWS_TRAIN`, and
`DISTILL_WINDOWS_LABELS`; start passes these resolved paths to the scheduled supervisor. Trainer flags are in one
block at the top of `windows-train.ps1`.

## Shards, reuse and crashes

A process writes `labels-<RUN>-<six-digit sequence>.npz`. RUN begins with the UTC start time as
`YYYYMMDDTHHMMSSZ`, followed by six-digit microseconds and a random UUID suffix. Names sort by start time and remain
unique for starts within one second. It saves into `.npz.tmp`, flushes and fsyncs the file,
then renames it atomically. Every shard includes scalar `teacher`, `teacher_minutes`, `created`, and `holdout`
metadata alongside the existing observation/search arrays. Missing checkpoint minutes become NaN. Every twentieth
shard, sequence 19, 39, and so on, is a validation holdout. A teacher switch flushes any pending rows. Late results
from older games also stay in separate shards, so each row's teacher is the shard's recorded teacher.

Sync checks for labels about every 30 seconds, stages several `.part` files on the laptop, copies them to Windows,
and renames them there in one PowerShell call. It records their names in an atomic Mac ledger only after that call
succeeds. Failed operations retry with exponential backoff from 5 seconds up to 5 minutes. A separate teacher check
runs about every five minutes, fetches only a newer numbered snapshot into `.pt.part`, fsyncs it and renames it, then
keeps the newest three teachers. A sync lock prevents two copies from sharing the ledger. Mac labels remain on disk
after delivery; disk usage grows until you archive or remove sent shards.

Windows only opens finished `labels-*.npz` files. Each training row can appear in at most `--distill-max-uses` batches
over its life, default 4. A row used at update u waits until update u + `--distill-reuse-gap`, default 50, before
becoming eligible again. A row never appears twice in one update, even with a gap of 0. Selection takes eligible
0-use rows before any 1-use row, then 1-use rows before 2-use rows, and so on, uniformly at random within each tier.
`--distill-max-uses 1` restores use-once selection and consumption.

An update gets k = min(floor(eligible rows / `--distill-batch`), `--distill-max-batches`, PPO minibatches) batches,
each containing exactly `--distill-batch` distinct rows. Sampling uses no replacement within the update, and the
batches attach to PPO minibatches spread evenly across it. Remainders wait. No full eligible batch means an ordinary
PPO update. The trainer admits newest shards first into free slots, up to `--distill-max-resident-rows`, default
60000 unfinished rows, including rows waiting for their reuse gap. Other rows stay compressed on disk. MPS keeps
resident tensors on device; other devices, including Windows DirectML, cache them on CPU and move only sampled batches.

A row retires when current trainer elapsed minutes minus its shard's `teacher_minutes` exceeds
`--distill-max-age-minutes`, default 60. For NaN teacher minutes, age starts at the trainer minute when the shard was
first admitted, saved in the checkpoint. Retirement completes rows even if they have uses left. Once admitted, this
age rule covers every unfinished row in the shard, including its rows still on disk. Shards waiting entirely on disk
face the rule when admitted, so stale backlog after a long outage retires immediately and frees room for later shards.

These flags affect only `--distill-stream`; static `--distill` sampling is unchanged:

| Flag | Trainer default | Windows wrapper |
|---|---|---|
| `--distill-max-uses` | 4 | 4 |
| `--distill-reuse-gap` | 50 updates | 50 updates |
| `--distill-max-age-minutes` | 60 minutes | 60 minutes |
| `--distill-max-batches` | 4 | 6 |
| `--distill-batch` | 128 rows | 128 rows |
| `--distill-max-resident-rows` | 60000 rows | 60000 rows |

Max uses must be positive. Reuse gap and max age may be 0; `--distill-max-age-minutes inf` disables retirement.
Holdouts remain validation only and go to `labels\heldout\`. A training shard completes when every row is spent at
its use limit or retired, and moves to `labels\used\` only after `latest.pt` saves that state. The checkpoint tracks
per-row use counts, last-used updates, completion masks, each admitted shard's admission minute, cumulative counters,
and completed shard names. Completed names survive archival, so a resent shard never trains again. The existing
static `--distill` loader still accepts round-1 `labels-NNNN.npz` files without metadata.

After a trainer crash, resume preserves saved row-use counts, last-used updates and retirement counts. Uses after the last
checkpoint can be replayed because those model updates were lost too. Legacy use-once checkpoints still load; their
already-consumed rows stay complete. If sync crashes after Windows renamed a shard but before the Mac saved its ledger,
a retry skips any matching active, used, or heldout Windows
file rather than publishing it again. Keep those directories with the training checkpoint. A label-maker crash
can lose rows still in memory or inside unfinished games; `.tmp` shards remain invisible. Finished shards survive.
A corrupt finished shard moves to `labels\rejected\` with a logged reason; partial transfer names are safe to retry.
An unreadable teacher is skipped until a later check succeeds or a newer teacher arrives.

The trainer reports these stream metrics each update:

| Metric | Meaning |
|---|---|
| `distill_eligible` | Resident rows eligible before this update's sampling, after retirement and admission |
| `distill_used` | Row-uses in this update |
| `distill_used_fresh` | Rows receiving their first use in this update |
| `distill_used_total` | Cumulative row-uses |
| `distill_rows_spent_total` | Cumulative rows completed at their use limit |
| `distill_rows_retired_total` | Cumulative rows completed by age retirement |
| `distill_retired_unused_total` | Cumulative rows retired with zero uses, wasted labels |
| `distill_mean_uses_completed` | Mean uses of all spent and retired rows, null until a row completes |
| `distill_batches` | Full distillation batches in this update |
| `distill_backlog_rows` | Unfinished training rows waiting on disk outside the resident cap |
| `distill_shards_pending` | Shards with unfinished resident rows after sampling |
| `distill_teacher_lag_min` | Trainer minutes minus mean sampled teacher minutes, null with no samples or any NaN teacher |

`distill_eligible` replaces the use-once `distill_fresh` counter. Validation still reports `distill_val_loss` and
`distill_val_agree`; training batches still report `distill_loss` and `distill_agree`.

## Rates and local checks

The expected Mac rate is about 25 labels/s at concurrency 5 with MPS. Windows takes about 4.1 seconds per PPO update
without distillation. Reusing labels four times gives roughly three 128-row batches per Windows update on average,
once the reuse gaps have filled:

| Quantity | Approximate rate |
|---|---|
| Fresh Mac labels per Windows update | 100, before holdouts |
| Training row-uses per update at 4 uses | 400, before holdouts and retirement |
| 128-row distillation batches per update | 3 |

Actual extra GPU time may change that balance; watch eligible rows, backlog, wasted labels and teacher lag.
CPU label making is slower. The validated concurrency-2 CPU run made about 13.5 labels/s.

The local mode maps the laptop and Windows to directories and never invokes SSH/scp. Windows simulation uses
`labels/` and `train/snapshots/` below its local root. Creating `FAIL_TRANSFER` in the Windows local root injects a
failure after copying `.part` files and before publication; removing it allows retry.

```bash
PY=/Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/.venv/bin/python
"$PY" -m pytest crates/pyengine/training_tests tools/distill -q
"$PY" tools/distill/mac_sync.py --local --once \
  --labels /tmp/loop/mac/labels --teacher /tmp/loop/mac/teacher --state /tmp/loop/mac/sync \
  --local-laptop /tmp/loop/laptop --local-windows /tmp/loop/windows
```

Remote transport and PowerShell execution must be checked on their target machines. The local tests cover reuse
limits and gaps across resume, least-used selection, age retirement, resident caps, use-once compatibility, publication
boundaries, ledger persistence, retries, completed-shard resends, numeric teacher selection and pruning.
