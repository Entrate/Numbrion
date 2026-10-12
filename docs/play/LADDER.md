# Run the Showdown ladder bot

The runner uses one account and plays one `gen9randomdoublesbattle` at a time. A run fixes the checkpoint,
team pools, native extension and search settings. Stop requests let the battle finish and capture its rating.
Disconnects leave battles pending; unseen endings are reported as unknown, never as losses.

Use the repo venv. From this worktree in PowerShell:

```powershell
$python = 'C:/Users/aminu/.t3/projects/numbrion/.venv/Scripts/python.exe'
```

1. Set the checkpoint and pool paths in `crates/pyengine/play/ladder.json`. The checkpoint path is a placeholder.
   The example uses CPU, four Torch threads, four search threads, a 3 s search allowance, a 6 s deadline and
   2 s minimum response time. Do not change these files during a run.
2. Put credentials in `%USERPROFILE%\.numbrion\showdown-credentials.json`:

   ```json
   {"username": "numbrion", "password": "YOUR PASSWORD"}
   ```

   Keep this file outside the repository. Alternatively set both `NUMBRION_PS_USERNAME` and
   `NUMBRION_PS_PASSWORD`. `--credentials PATH` overrides the default file; environment variables take precedence.
3. Start the first hour by hand:

   ```powershell
   & $python crates/pyengine/play/ladder.py --config crates/pyengine/play/ladder.json --minutes 60
   ```

   Preflight runs once before connection; failure exits 2. A missing preflight module produces a warning.
   `--no-preflight` explicitly skips it. The runner turns on every battle's timer before making a choice,
   rejects incoming challenges, and sends no PMs. Windows stays awake while the runner is active.

Test against your local Showdown server using a passwordless instant DefaultBot:

```powershell
& $python crates/pyengine/play/ladder.py --config crates/pyengine/play/ladder-local.json --local --games 1
```

`--minutes` measures time from the original run start, including downtime on resume. `--games` counts finished
battles. The first Ctrl-C stops matchmaking, cancels an outstanding search, and waits for the current battle
and rating capture. The second exits immediately; the battle keeps running on the server. Nothing forfeits.
Official ratings may lag; retries run at 5, 20, 60 and 180 seconds after the initial fetch. Matchmaking waits
for this capture; unconfirmed updates stay pending. Local mode makes no official rating requests.

Every run goes to `scratch/ladder/runs/<id>/`: `manifest.json`, flushed `events.jsonl`, and full battle
JSONL files under `battles/`, compressed after a finished room closes. The shared database is
`scratch/ladder/ladder.sqlite` in WAL mode. Passwords, login assertions and challenge strings are redacted.
`--runs-dir DIR` and `--db PATH` change these locations; relative storage and bot paths use the repo root.

Resume after a crash using the printed run id:

```powershell
& $python crates/pyengine/play/ladder.py --resume ID
```

Resume loads the original config and checks config, checkpoint, pool and extension hashes and git commit.
A difference exits 2. `--force` records an explicit identity change; prefer a new run for a changed bot.
Pending server games are rejoined before matchmaking continues.

Generate reports and inspect decisions (including compressed battle logs):

```powershell
& $python crates/pyengine/play/ladder_report.py --run ID
& $python crates/pyengine/play/ladder_report.py --all
& $python crates/pyengine/play/ladder_report.py --run ID --battle battle-gen9randomdoublesbattle-123
& $python crates/pyengine/play/ladder_report.py --run ID --decision battle-gen9randomdoublesbattle-123:7
```

`--run` also writes `report.md`. Optional `--plots` writes `elo.png` if Matplotlib is installed. Strength counts
all rated results, including technical failures. The report separates technical problems from clean battles,
shows pending/unknown records, and includes decision timings, calibration and suspicious decision ids.

For later supervised runs:

```powershell
& $python crates/pyengine/play/supervise.py --max-restarts 5 --window-min 30 -- --config crates/pyengine/play/ladder.json
```

The supervisor resumes the same run after crashes or a stuck worker (exit 4), with 10/30/60/120 s backoff.
It stops on success (0), config/preflight failure (2), health pause (3), or the restart limit. A health pause
can mean three authentication failures, recorder failure, an unhealthy room, or three consecutive battles
with driver errors or more than two fallbacks. A stuck worker exits after the battle or 120 s. Ctrl-C is
forwarded to the child and disables restarts; on Windows it uses a separate process group and Ctrl-Break.

Fast tests, using only localhost fake servers and no models:

```powershell
Set-Location crates/pyengine
& $python -m pytest play -q
```
