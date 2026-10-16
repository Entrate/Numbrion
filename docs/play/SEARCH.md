# Search

`crates/pyengine/training/search.py` adds a one-turn search on top of a self-play checkpoint. The bots of the play
server and the extension's advisor use it. Without it a bot plays its policy (sampled at temperature 0.5, the
evaluation setting).

## How a decision is made

1. **Our candidates.** The policy reads our observation and lists our most likely pairs of actions (8 by
   default), with every legal slot-a code expanded.
2. **Worlds.** `numbrion.WorldBuilder` samples opponent teams that fit everything we have seen, and replays the
   battle to now in each (4 worlds by default). It reads only our player-view lines, our requests and our own
   choices, so it works the same on a Showdown server as in-process. Searching never touches a live battle.
3. **Opponent candidates.** In each world the policy, run from the opponent's side, lists the opponent's likely
   pairs (6 by default). The opponent's view is approximated by a tracker fed with our lines from their side,
   with our exact HP replaced by the percentages they see, plus the world's request for the opponent.
4. **Grid.** Every (world, our pair, their pair, chance seed) cell is simulated to the next decision: the next
   turn, a switch-in after a faint or a pivot, or the end. Cells of one world and seed share the random numbers.
   That is 4 x 8 x 6 x 2 = 384 cells per decision by default. `numbrion.SearchGrid` runs the engine and encodes
   each leaf on Rust threads.
5. **Leaf values.** The value head scores each leaf. It is an oracle critic: it also reads the other side's own
   team, which each world supplies. It only ever trained on positions where its side had to act, so a leaf where
   only the opponent must act is scored from the opponent's side and negated. Finished battles score +1, -1 or 0.
6. **Choice.** The payoffs form a matrix game per world. Our strategy is shared by all worlds, because we don't
   know which is true; the opponent's is per world. Smoothed fictitious play solves it, with the opponent anchored
   to its policy (`opponent_anchor` 0.1); our side is not nudged toward our policy (`anchor` 0). The bot plays the pair our strategy
   weights most.

A decision falls back to the policy when no world reproduces what was seen, when no candidate is legal in any
sampled world, or when there is only one legal choice. The analysis names the reason (`note`).

## Settings

`SearchConfig` fields, set per bot in `crates/pyengine/play/bots.json` (`"search": {...}`):

| Field | Default | Meaning |
|---|---:|---|
| `worlds` | 4 | sampled opponent teams per decision (`budget_ms` scales it, 2-32) |
| `ours`, `theirs` | 8, 6 | candidate pairs for us, and for the opponent per world |
| `seeds` | 2 | chance samples per cell |
| `opponent_temperature` | 1.0 | temperature of the opponent's policy prior |
| `anchor`, `opponent_anchor` | 0, 0.1 | KL weights to each side's prior; `opponent_anchor` 0 makes the opponent just play its prior. `anchor` was 0.05 until 2026-10-16: 0 beat it 61.5% ± 3.5 over 100 pairs (1-ply, ladder sizes, Spark) |
| `play` | `argmax` | `sample` draws from our strategy instead |
| `particles` | 8 | world-builder particles |
| `threads` | 4 | grid threads |
| `max_worlds` | 32 | most worlds a time budget can buy |

`"search": false` gives the plain policy. Other `SearchBot` arguments: `checkpoint`, `device` (`auto` uses the
Apple GPU for leaf values), `policy_temperature` (0.5, for policy decisions), `pools` (the team pool the world
builder samples sets from, default `data/teams/pool-s1-200k.txt.gz`).

The world builder needs that pool. Generate it once from the pinned Showdown checkout:

```sh
node tools/oracle/gen-teams.mjs <showdown> --count 200000 --seed 1 --out data/teams/pool-s1-200k.txt && gzip -k data/teams/pool-s1-200k.txt
```

## Time budgets and deadlines

A driver can hand the player two things before `decide` (play/driver.py does on the ladder, see
[LADDER.md](LADDER.md)):

* `budget_ms`, the search allowance. `SearchBot.config_for` turns it into a number of worlds (2 to `max_worlds`) from
  running estimates of the cost per cell and per decision, which every search decision updates.
* `deadline`, a `time.monotonic()` by which the choice must be sent, and `decision_token`. Right after the root
  policy the player posts the policy's most likely pair as `fallback = (token, Decision)`. If the decision is still
  running at the deadline the driver sends that choice and later tells the player with `record_choice`, so the
  world builder replays what was really played. After building the worlds the player keeps only as many as fit the
  time left, and plays the policy (note "out of time after building the worlds") when none fits.

A search decision's analysis also has `policy` (the policy's most likely pair, to find where search disagreed;
`agrees_with_policy`), `worlds_requested`, `budget_ms`, `observed` (what the protocol revealed about each opponent
Pokemon) and `inferred` (per seen species, the share of sampled worlds with each item, ability, Tera type and move,
plus the unseen species the worlds put on the bench).

`play/preflight.py` plays one short in-process battle with these time rules before a ladder run: on the Windows PC
(Ryzen 5 3600, CPU only, with a training run going) the 128x3 teacher m240 with `ours` 8 / `theirs` 6 and a 3 s
allowance searched all 15 decisions, 32 worlds each, median 1.1 s and at most 1.7 s per decision.

## Does it help?

`crates/pyengine/search_eval.py` plays mirrored pairs of games: the same teams and battle seed twice, with the
sides swapped, so team and seed luck cancel. The search bot sees only its player-view lines and requests and builds
its own legal masks from the requests. The plain policy plays as in `evaluate.py`/`compare.py`: temperature 0.5,
move guards, the engine's masks. Scores are the search bot's mean result; ± is one standard error of the pair
means. Checkpoint: the 1-hour 128x3 teacher run (`overnight/03-teacher/snapshots/m060.pt`) unless noted.

Tuning used the eval-s44-200 teams (150 pairs = 300 games per row). Most of these runs gave the search bot the
engine's masks, the same ones the plain policy gets.

| Variant (vs plain policy) | Score |
|---|---:|
| first version, best response to the opponent's policy (eval-s43, 100 pairs) | 82.0% ± 2.7 |
| best response, 4 worlds x 8 x 6 x 2 seeds | 79.7% ± 2.3 |
| ... sampling from our strategy instead of the best pair | 70.7% ± 2.5 |
| ... small: 2 worlds x 6 x 4 x 1 seed (48 cells) | 78.7% ± 2.2 |
| ... large: 8 worlds x 12 x 8 x 2 seeds (1,536 cells) | 81.7% ± 2.2 |
| ... opponent prior at temperature 1 | 83.7% ± 2.0 |
| ... opponent also optimises (anchor 0.1) | 82.0% ± 2.1 |
| ... both (the defaults) | 82.0% ± 2.2 |

Search against search (100 pairs each): default vs small 53.0% ± 3.2; equilibrium vs best response
49.0% ± 3.2; defaults vs best response 50.5% ± 3.2. More search per decision helps a little, and the solver
variants are equal against each other. The defaults let the opponent optimise too, which is the safer choice
against people who don't play like the policy.

Final check on a fresh seed of the held-out eval-s43-200 teams, with the defaults and request-only masks:

| Matchup | Pairs | Score |
|---|---:|---:|
| teacher + search vs teacher policy | 300 | **79.2% ± 1.6** |
| the same after the review fixes (refused choices, forbidden candidates), seed 23 | 300 | **81.8% ± 1.4** |
| teacher policy vs teacher policy (control: same harness, request-only masks) | 300 | 51.8% ± 1.8 |
| teacher + search vs smart | 200 | **78.5% ± 1.9** |
| teacher policy vs smart | 200 | 57.0% ± 2.2 |
| 192x4 m180 + search vs m180 policy | 200 | **72.8% ± 2.2** |
| teacher + search vs m180 policy | 200 | **77.5% ± 1.9** |
| teacher policy vs m180 policy | 200 | 47.0% ± 2.3 |

Search adds 21-30 points against every opponent tried, for the small and the large network. 3% of the search
bot's decisions fell back to the policy for lack of a world, and none of its choices were refused. Results and
logs are in `scratch/search-eval/` (git-ignored): `F1`-`F8` are the rows above, `v1-*`, `v2-*` and `h2h-*` the tuning.

## Speed

Per decision on the M1 Pro, defaults, from 106 decisions of 8 games (`scratch/bench_decisions.py`):

| Part | Before | Now |
|---|---:|---:|
| world building | 128-150 ms (16 particles) | 55 ms (8 particles) |
| our and the opponent's policy | 26 ms (Apple GPU) | 6 ms (CPU copy of the model) |
| grid: 384 cells simulated and encoded | 33 ms (`GridExecutor`, encoding in Python) | 5.5 ms (`SearchGrid`, encoding on Rust threads) |
| leaf values | 28-44 ms (CPU) | 30 ms (Apple GPU) |
| **whole decision** | **~250 ms** | **~110 ms** |

The fused grid costs 54 µs per cell on one thread against 86 µs for `GridExecutor` plus Python-side encoding, and
17 µs per cell on 6 threads; the old path stopped scaling at about 68 µs. `crates/pyengine/search_grid_bench.py`
measures it. Tried and dropped: a gather-based embedding path for inference (no faster than the one-hot matmul on
an idle machine), half precision on the GPU (5% faster), and a PGO wheel (`tools/pgo.sh --search`, no measurable
change in decision time).

The world builder is now the largest cost. Its time varies a lot: about 1 ms when the particles only need to
advance, up to a few hundred ms when they must be rebuilt from the start of the battle.

## Limits

- A world that has died can't always be rebuilt. After a run of unlikely events (crits, rolls, odd targets) the
  builder may find no world for the rest of a battle, and the bot plays its policy from then on. This happened in
  about 3% of decisions in evaluation.
- The opponent's view is approximate, see step 3.
- A bot that reconnects to a running battle starts a new player without its earlier choices, so it plays the
  policy for the rest of that battle.
- The world builder needs the viewer's real choices. The advisor gets them from the
  extension, so a battle the extension joined late gets policy advice.
- One turn of lookahead. Deeper search, and continuing through a switch-in after a faint with the policy instead
  of scoring the replacement position, are untested ideas.
