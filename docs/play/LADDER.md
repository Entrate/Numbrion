# Play the public ladder

`crates/pyengine/play/ladder.py` plays [Gen 9] Random Doubles Battle (`gen9randomdoublesbattle`) on the public
Showdown ladder with one registered account, one battle at a time, and records everything about each battle. A
run uses one frozen bot: the checkpoint, search settings, team pools, native extension and code are fixed and
hashed at the start, and a resumed run refuses to continue if any of them changed.

```
connect -> log in -> rejoin running battles -> search -> play -> record result and rating -> search again
```

## Once

1. **A search-enabled `numbrion`.** The venv's editable extension, built with search (it costs training no measurable
   speed). Rebuild after engine changes, while no training run has the extension loaded:

   ```sh
   cd crates/pyengine && maturin develop --release --features python,search
   ```

   `--features search` alone drops the `python` feature and fails. To pin the bot to a separate build instead, set
   `numbrion_path` in the config to an unpacked wheel directory; it goes first on `sys.path`.

2. **Team pools** for the world builder, from the pinned Showdown checkout (about 4 minutes each):

   ```sh
   for s in 1 2 3; do node tools/oracle/gen-teams.mjs scratch/pokemon-showdown --count 200000 --seed $s \
       --out data/teams/pool-s$s-200k.txt && gzip data/teams/pool-s$s-200k.txt; done
   ```

3. **Credentials**, outside the repository, in `%USERPROFILE%\.numbrion\showdown-credentials.json`:

   ```json
   {"username": "numbrion", "password": "..."}
   ```

   or the environment variables `NUMBRION_PS_USERNAME` and `NUMBRION_PS_PASSWORD`. The password and the login
   server's assertion never reach a log, a run file or the database.

4. **Is the live server still close to the engine's Showdown?** `tools/play/check-showdown-drift.sh` lists upstream
   changes since the pinned commit to the Gen 9 random-battle sets, the simulator and the Gen 9 data. If sets
   changed, regenerate the pools from an updated checkout; if mechanics changed, check the engine first.

## A run

The bot of a run is `crates/pyengine/play/ladder.json`: the 128x3 teacher m240, search with 12 of our pairs and 8
opponent pairs per world, up to 32 worlds, the three pools, CPU. Each decision gets a 3 s search allowance, is sent
no earlier than 2 s and no later than 6 s after its request arrived, and the battle timer is turned on as soon as
the room opens. Edit the file before a run, never during one.

```powershell
.venv\Scripts\python.exe crates\pyengine\play\supervise.py -- --config crates\pyengine\play\ladder.json --minutes 60
```

* Before connecting, a **preflight** plays one short in-process battle with the run's time rules and refuses to
  start unless the extension has search, at least 80% of decisions searched and none missed the deadline.
* Each finished battle prints a line such as
  `game 7 W vs Opp (1234) 18 turns | elo 1010 -> 1035 | 19 decisions: search 18, policy 1, fallback 0, default 0 |
  total p50 2.3 s p95 2.7 s max 2.9 s | refused 0`, and a warning when fewer than 80% of decisions searched.
* `--minutes` counts from the run's start, `--games` counts finished battles. When a limit is reached, or on the
  first Ctrl-C, the bot stops searching, finishes the current battle and waits for its rating. A second Ctrl-C exits
  at once; the battle keeps running on the server and is lost on time if nobody returns. Nothing forfeits.
* The supervisor restarts a crashed bot after 3 s (then 10, 30, 60, 120 s) and resumes the same run; the resumed bot
  rejoins its battle. Showdown forfeits a player who stays away about a minute with the timer on; a restart took
  17 s in testing. It stops on a clean exit, a config or preflight error (2), a health pause (3: three failed logins,
  a recorder failure, a stuck decision, or three battles in a row with errors or more than two fallbacks), or more
  than 5 restarts in 30 minutes. Without the supervisor, run `ladder.py` with the same arguments; resume a run with
  `ladder.py --resume RUN_ID`.
* Windows PCs and Macs are kept awake while the bot runs.

Challenges from other users are refused, and the bot sends no messages.

### Decisions against the clock

The room measures each decision from the receipt of its request (play/driver.py, "Deadlines"). The player gets the
search allowance as a budget, which buys as many worlds as fit (SEARCH.md, "Time budgets and deadlines"). Right
after the root policy it posts the policy's choice as a fallback; if the search is not done 6 s less 0.5 s after the
request, that choice is sent (or `default` if not even the policy finished), and the player is told so its worlds
replay what was really played. When Showdown's "Time left" line shows less than the deadline plus 10 s, the
deadline moves forward. A decision still running 30 s after its deadline marks the bot unhealthy.

### Ratings

Before leaving a finished rated battle the room waits up to 30 s for Showdown's rating lines (`NAME's rating: 1012
-> 1037 (+25 for winning)`). In the background the runner fetches the official snapshot
(`pokemonshowdown.com/users/numbrion.json`: Elo, GXE, Glicko rating and deviation, wins and losses) after each
battle until it counts the battle (5, 20, 60, 180 s), and at the start and end of the run. Updates that never show
stay pending; nothing is guessed. A battle the bot never saw end (for example after a crash it never recovered from)
is reported as `unknown`, not as a loss; a disconnect never counts as a result.

## What is recorded

`scratch/ladder/runs/<run_id>/`:

* `manifest.json`: run id, account, server, config and its hash, git commit and dirty diff hash, checkpoint, pool and
  extension hashes, seeds, timing, hardware, Python/torch/numpy versions, the preflight result, resumes.
* `events.jsonl`: connection, login, search, match, popups, resources (CPU, memory, worker queue every 15 s), rating
  snapshots, pauses and stops.
* `battles/<room>.jsonl.gz`: every websocket message of the room as received, every command sent, each request with
  the lines before it and the battle clock, each decision with its analysis (candidates with priors, values and
  weights, the opponent's likely replies, the policy's own choice, what was observed about each opponent Pokemon and
  what the sampled worlds inferred) and its timing (receipt, queue, compute, pacing, send), refusals, fallbacks,
  timer and rating lines, the end. Written and flushed event by event, compressed when the battle is over.

For diagnostic runs, add `"record": {"dir": "scratch/ladder/records"}` to the config. It is off by default.
This enables `analysis.search_record` on search decisions, in both the ordinary battle logs and
`<record.dir>/<run_id>/<room>.jsonl.gz`. Each event closes a complete gzip member, preserving earlier events on a crash. Files append across restarts
and flush every event. A bounded
background queue keeps disk I/O off the decision thread; capture/write errors and dropped events are logged.
Each room file contains timestamped raw messages, `request_received` events for every request (including waits),
dispatched requests, full decision analysis/timing, late decisions, and the result in the `end` event.

`search_record` version 1 preserves matrix order, unlike the sorted analysis summaries:

* `pairs`, `choices`, `prior`, `x`, `q`: our action codes, Showdown choices, solver prior, final strategy and values.
* `worlds`: equally weighted sampled worlds, each with its opponent `team` (species, moves, item, ability, tera),
  opponent `pairs`, `choices`, readable `text`, `prior` and final strategy `y`.
* Each world's `payoffs` is rows = our pairs, columns = opponent pairs, averaged over chance seeds, exactly as
  passed to the solver. `q` is that matrix times `y`. Values are from our perspective, rounded to six decimals.
* `sampled_valid` marks originally legal rows; `valid` is the solver mask after refused-choice retry substitution.
  `counts` gives simulated cells per entry. A zero count distinguishes substituted rows from measured outcomes.

No sampling, model calls or engine steps are added for recording. Policy/forced/fallback decisions retain their
full analysis but have no search matrix. A deadline fallback's eventual search is saved as `late_decision`.

For a detached runner, optionally set `"stop": {"file": "scratch/ladder/STOP"}` alongside the usual limits.
Creating that file requests the same graceful stop as one Ctrl-C: cancel matchmaking, finish the current battle
and collect its rating. Remove it before starting a new run. The supervisor exits when its child finishes.
`--minutes` still counts from the original run start across supervisor restarts. Idle-sleep prevention is scoped
to the processes; no persistent power settings are changed.

`scratch/ladder/ladder.sqlite` has the derived tables `runs`, `battles`, `decisions`, `ratings`, `events` and
`resources` for queries. The JSONL files are the complete record.

## Report

```powershell
.venv\Scripts\python.exe crates\pyengine\play\ladder_report.py --run RUN_ID          # also writes report.md
.venv\Scripts\python.exe crates\pyengine\play\ladder_report.py --all
.venv\Scripts\python.exe crates\pyengine\play\ladder_report.py --run RUN_ID --decision ROOM:SEQ
```

Strength: W/L/T with an interval, Elo per game and over time, current and peak, the official Elo, GXE and Glicko,
opponent ratings, win rate by opponent band, rolling win rate. Operations: decision time percentiles, decision
sources and why search did not run, fallbacks, deadline misses, refusals, late decisions, disconnects and rejoins,
timer losses, errors, resources, and clean battles against battles with a technical problem. Deeper: results by
opponent species, our Tera use, win-probability calibration, how often search disagreed with the policy, and a list
of suspicious decisions. `--decision ROOM:SEQ` prints one decision with the battle so far, the request, the
candidates, the opponent replies and the timing.

## Testing on the local server

```sh
tools/play/setup-showdown.sh                  # once (Node 22 or later)
tools/play/setup-showdown.sh --config --ladder   # ladder search on, local ratings
tools/play/start-showdown.sh
```

Then run two accounts against each other, for example the ladder bot and a policy-only opponent: copy `ladder.json`
twice with `"local": true`, a local `server`, different `account` names, and `"search": false` for the opponent,
and start both with `--local --games N --runs-dir ... --db ...` (separate from the real runs). `play/test_ladder.py`
and `play/test_deadline.py` test the same flows against fake servers in seconds.

## Measured (2026-10-12, Ryzen 5 3600, CPU only, a training run using the rest)

* Search versus the same checkpoint's plain policy, 40 mirrored pairs on held-out teams with request-only masks:
  **85.0% ± 4.1**; 96% of decisions searched, 1.5% had no world, no refused choices.
* Preflight: every decision searched, median 2.3-2.4 s with the run's settings.
* Local ladder, 5 games against a policy-only account: every decision sent between 2.0 and 3.8 s after its request,
  no fallbacks, refusals or deadline misses, ratings captured from the room; a bot killed mid-battle was back in the
  battle 17 s later and the battle was recorded once.

## Known limits

* After a rejoin (a crash or a lost connection) the bot plays its policy for the rest of that battle: a fresh player
  does not have our earlier choices, which the world builder needs.
* In long battles the world builder can lose every world after a run of unlikely events and not find them again
  (one 82-turn local game: 24 of its 86 decisions). The bot then plays its policy, which the report shows under
  "why search did not run".
* One battle at a time. The bot thinks on one worker thread, so a second simultaneous battle would queue behind it.
