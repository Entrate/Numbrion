# Continuous search distillation

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

On Windows, start from an existing checkpoint. Later starts prefer `train\latest.pt` automatically.

```powershell
.\tools\distill\windows-train.ps1 -Action start -Resume C:\path\to\m115.pt
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
`update`, `minutes`, `distill_fresh`, `distill_used`, `distill_teacher_lag`, and active/used/heldout shard counts.
Stdout appends to `train\trainer.log`; stderr appends to `train\trainer.log.err`.

On the Mac:

```bash
tools/distill/mac-labels.sh start
tools/distill/mac-labels.sh status
tools/distill/mac-labels.sh stop
```

Start refuses an empty teacher directory. It launches the label maker and sync with `nohup`, with pids and logs in
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
`--minutes inf` runs until `STOP`; `--checkpoint FILE --minutes 60` still makes a finite one-shot run.

The config block at the top of `mac_sync.py` lists every host/path override, including `DISTILL_LAPTOP`,
`DISTILL_LAPTOP_PORT`, `DISTILL_HOP_KEY`, `DISTILL_RELAY`, `DISTILL_WINDOWS`, `DISTILL_WINDOWS_LABELS`,
`DISTILL_WINDOWS_SNAPSHOTS`, `DISTILL_LABELS`, `DISTILL_TEACHER`, and `DISTILL_SYNC_STATE`. `DISTILL_BASE` changes
Mac wrapper state and log paths. Windows accepts `DISTILL_WINDOWS_REPO`, `DISTILL_WINDOWS_TRAIN`, and
`DISTILL_WINDOWS_LABELS`; start passes these resolved paths to the scheduled supervisor. Trainer flags are in one
block at the top of `windows-train.ps1`.

## Shards, use-once semantics and crashes

A process writes `labels-<UTC start>-<six-digit sequence>.npz`. It saves into `.npz.tmp`, flushes and fsyncs the file,
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

Windows only opens finished `labels-*.npz` files. Training rows are sampled uniformly without replacement from all
fresh rows. Each update gets at most four batches of exactly 128 rows, further limited by fresh rows and PPO
minibatches. Remainders wait. No labels means an ordinary PPO update. Holdouts go to `labels\heldout\`; fully used
training shards move to `labels\used\` only after `latest.pt` saves their consumption state. The checkpoint tracks
partly consumed row masks and consumed shards awaiting moves. The existing static `--distill` loader still accepts
round-1 `labels-NNNN.npz` files without metadata.

After a trainer crash, resume never reuses a row consumed by an update recorded in `latest.pt`. Rows consumed after
the last checkpoint can be used again because those model updates were lost too. If sync crashes after Windows
renamed a shard but before the Mac saved its ledger, a retry skips any matching active, used, or heldout Windows
file rather than publishing it again. Keep those directories with the training checkpoint. A label-maker crash
can lose rows still in memory or inside unfinished games; `.tmp` shards remain invisible. Finished shards survive.
Run names use seconds; a second process started in the same second in the same output directory refuses to overwrite
an existing run. A corrupt finished teacher or shard needs manual repair; partial transfer names are safe to retry.

## Rates and local checks

The expected Mac rate is about 25 labels/s at concurrency 5 with MPS. Windows takes about 4.1 seconds per PPO update
without distillation. That produces about 103 labels per update before holdouts, roughly one 128-row distill batch
per update on average. Actual extra GPU time may change that balance; watch fresh-row backlog and teacher lag.
CPU label making is slower. The validated concurrency-2 CPU run made about 13.5 labels/s.

The local mode maps the laptop and Windows to directories and never invokes SSH/scp. Windows simulation uses
`labels/` and `train/snapshots/` below its local root. Creating `FAIL_TRANSFER` in the Windows local root injects a
failure after copying `.part` files and before publication; removing it allows retry.

```bash
PY=/Users/aminaliu/.t3/worktrees/numbrion-mac/t3code-c1f8987c/.venv/bin/python
"$PY" -m pytest crates/pyengine/training_tests/test_search_labels.py tools/distill/tests -q
"$PY" tools/distill/mac_sync.py --local --once \
  --labels /tmp/loop/mac/labels --teacher /tmp/loop/mac/teacher --state /tmp/loop/mac/sync \
  --local-laptop /tmp/loop/laptop --local-windows /tmp/loop/windows
```

Remote transport and PowerShell execution must be checked on their target machines. The local tests cover
publication boundaries, ledger persistence, retries, already-consumed shards, numeric teacher selection and pruning.
