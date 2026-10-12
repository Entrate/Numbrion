# Training API (`numbrion` Python package)

`crates/pyengine` builds the Python package `numbrion` (PyO3 + maturin, one abi3 wheel for every CPython
>= 3.9). It exposes the engine to AlphaZero/PPO-style self-play through a **fixed discrete action space with
exact legality masks**, **hidden-information-safe observation streams**, and a **batched, auto-resetting
environment** that runs battles in parallel Rust threads with the GIL released. Everything here uses only the
public API of `crates/engine`; observation encoding and scripted play use `crates/encoder`.

| Piece | Where |
|---|---|
| `numbrion.Battle` | one battle: debugging, scripted play, search (`clone`, `restore_from`, `reset`, `reseed`) |
| `numbrion.BatchEnv` | N battles, `step(actions)` with auto-reset, masks, rewards |
| `numbrion.ObsEncoder`, `numbrion.SmartAgent` | in-place NumPy observation encoding and two-slot scripted play; `ENCODER=True` ([training integration](../training/SELFPLAY.md#rust-observation-encoding-and-scripted-play)) |
| `numbrion.GridExecutor` | one-decision search grids (restore root, both choices, next boundary) on Rust threads |
| `numbrion.SearchGrid`, `numbrion.TrackerState` | the same grids with each leaf encoded for the value network on the worker threads ("Fused leaf encoding") |
| action codes, masks | `src/action.rs`, `src/mask.rs` |
| search executor | `src/search.rs` (see "Search" below), fused leaf encoding `src/search_grid.rs`, `src/request_tree.rs` |
| tests | `tests/*.rs` (Rust, exhaustive mask checks) and `pytests/` (Python) |
| benchmarks | `crates/pyengine/bench.py`, `search_bench.py`, `search_grid_bench.py`, `examples/grid_bench.rs`, `examples/search_grid_bench.rs` |

## Build and install

Linux/macOS (the repo keeps its virtualenv in `.venv`, which is git-ignored):

```sh
uv venv .venv && uv pip install --python .venv/bin/python maturin numpy pytest   # or: python -m venv .venv
cd crates/pyengine
../../.venv/bin/maturin develop --release --uv      # editable install; use plain `maturin develop` with pip
../../.venv/bin/python -m pytest                    # 27 tests, ~15 s
../../.venv/bin/python bench.py --threads 1 6       # throughput
```

Windows (PowerShell; nothing in the crate or the Python side is Linux-specific: no fork, shared memory,
Unix sockets or absolute paths, only `std::thread`/rayon and `pathlib`):

For the AMD RX 5500, use the [verified Windows GPU setup](../training/WINDOWS-GPU.md)
and `tools/setup-training.ps1` instead of installing CUDA Torch. It builds this API,
runs its tests, verifies GPU gradients with DirectML, and provides a small PPO
self-play trainer with checkpoint resume. The GNU/MinGW build has now also been
built and tested natively on Windows, including the abi3 wheel and the 19 Python tests.

```powershell
# prerequisites: rustup with the x86_64-pc-windows-msvc toolchain (MSVC Build Tools, "Desktop development
# with C++"), Python >= 3.9
py -3 -m venv .venv
.venv\Scripts\activate
pip install maturin numpy pytest torch            # torch from the CUDA index you need
cd crates\pyengine
maturin develop --release                          # editable install into the venv
pytest
maturin build --release                            # distributable wheel: ..\..\target\wheels\numbrion-0.1.0-cp39-abi3-win_amd64.whl
```

The wheel is abi3 (`pyo3/abi3-py39`), so one build serves Python 3.9-3.14+ on the same OS and architecture,
and it needs no `python3.lib` at link time (PyO3 0.29 links the Python DLL through `raw-dylib`). Builds are
portable by default (`.cargo/config.toml` no longer sets `target-cpu=native`; the throughput below is the
portable build, the native one measured the same). For a build tuned to the local CPU use
`RUSTFLAGS="-C target-cpu=native"`. The Windows build was verified with
`cargo check -p pyengine --features python --target x86_64-pc-windows-msvc` from Linux.
Native Windows GNU/MinGW validation is documented in the GPU setup guide linked above.
Plain `cargo build/test -p pyengine` (no `python` feature) never needs
Python, and `cargo test --release -p pyengine` runs the Rust test-suite.

## Action space

A decision of one side is **two action codes**, one per active slot (slot 0 = `a`, slot 1 = `b`), each in
`0 .. N_ACTIONS = 47`:

| Code | Meaning |
|---|---|
| `0..40` | **move**: `code = slot*10 + target*2 + tera` with `slot` 0..3, `target` 0..4, `tera` 0/1 |
| `40..46` | **switch** to party index `p = code - 40` (0..5) |
| `46` | **pass** |

* `slot` is the index of the move in the Pokémon's request `moves` list. It is stable for the Pokémon's life,
  unlike a compacted list of the currently enabled moves.
* `target = signed Showdown target location + 2`: 0 = `-2` (own slot b), 1 = `-1` (own slot a), **2 = no
  target (automatic)**, 3 = `+1` (foe slot a), 4 = `+2` (foe slot b). Moves that do not take a target
  (spread, self, field, locked, Struggle) are only legal with target 2; moves that need one never with 2.
  Fainted/empty slots are valid targets, as in Showdown.
* `tera = 1` terastallizes with that move. At most one Tera per side per turn, and only while the engine
  offers it (`canTerastallize`).
* Switch indices are the **current** order of the request's `side.pokemon` array (`switch N` in Showdown is
  `p = N-1`); the two active Pokémon are 0 and 1, so only 2..5 are legal bench switches. 0 and 1 appear only as
  Revival Blessing revive targets (any fainted party member, including a fainted active one).
* **Forced moves** (Outrage-style locks, charging/semi-invulnerable moves, Recharge, Struggle, including the
  "every move is hidden-disabled" Struggle) have no choice to make. Their single canonical code is
  `move slot 0, target 2, tera 0` (= code 4, `numbrion.FORCED_MOVE`). Aliases the engine would also accept
  (a Tera suffix or a target on a locked move, another slot number under forced Struggle) are masked out, so
  every behaviour has exactly one code.
* A slot that cannot act (fainted active in a move request, an unflagged slot in a replacement request,
  `commanding`) must take **pass**. A flagged slot in a replacement request may pass only while forced
  passes remain (no bench Pokémon left).

`numbrion.describe_action(code)`, `decode_action(code)`, `move_action(slot, target, tera)` (target signed
-2..2, 0 = none) and `switch_action(party)` convert between codes and structure. `Battle.action_text(side, a0,
a1)` renders a legal pair as the Showdown choice string (`"move 1 +2 terastallize, switch 4"`), which
`Battle.choose` accepts and which has the same effect as the typed path (tested).

### Masks (exact, joint)

For a side that has to act, the legal pairs `(a0, a1)` are *not* a product set. The joint constraints, taken
from `Battle::legal_actions` (`sim/choices/legal.rs`), are:

* **move request**: the two incoming Pokémon differ; at most one Tera.
* **replacement request** (after faints or U-turn-style moves): every flagged slot must be consumed
  exactly: `forced_switches = min(flagged, living bench)` slots switch (distinct Pokémon), the others pass;
  Revival Blessing slots revive instead of consuming a bench Pokémon.

Mask API (all arrays are numpy `bool`, `True` = legal):

* `mask0[a0]`: codes slot 0 may take **such that some `a1` completes a legal choice**.
* `mask1(a0)[a1]`: codes slot 1 may take after slot 0 chose `a0` (empty for `a0` outside `mask0`).
* `mask1_any[a1]` (BatchEnv): union over all legal `a0`, for non-autoregressive heads.
* `joint[a0, a1]`: the full matrix, `joint[a0] == mask1(a0)`.

Sampling `a0 ~ mask0`, then `a1 ~ mask1(a0)` therefore never dead-ends. **Guarantee, tested exhaustively:** every
pair the mask allows is accepted by the engine (`choose_typed`, and its text rendering through `choose`, with
identical effect), and every other pair is rejected. The tests (`tests/masks.rs`) cover, per decision
boundary and acting side, all 47 x 47 code pairs through the typed path and the whole Showdown text grammar
(`pass`, `move 1..5 [-3..3] [terastallize]`, `switch 1..7`, ~6000 pairs) through `Battle::choose`, each on a
fresh copy of the battle, and require: allowed ⇒ accepted with the stored choice equal to the requested
codes; accepted ⇒ representable (modulo the forced-move aliases above) and allowed; unrepresentable text ⇒
rejected. Boundaries include move requests, replacement requests, forced passes, forced moves, auto-passing
slots, Tera offers and (steered play) Revival Blessing replacement requests. Results of the large runs are in
"Validation" below.

**Hidden information in the mask.** `legal_actions` is the engine's *authoritative* legality. Showdown's
request only says `maybeTrapped` / `maybeDisabled`; the player learns the truth by trying and getting an
`[Unavailable choice]` error plus an updated request. The mask already contains that truth, so it reveals to the
policy that a foe's Shadow Tag / Arena Trap / Magnet Pull / Imprison is active (the switch or the move is
masked) one decision earlier than a real client would learn it. The same information would be learned by
the failed attempt, so the effect is small, but it is a difference from a ladder client. A request-derived
mask that retries after `[Unavailable choice]` would need a re-ask protocol in `step`; it is not built.

## `numbrion.Battle`

```python
b = numbrion.Battle(seed=(a, b, c, d), p1=packed_team, p2=packed_team, names=("Alice", "Bob"), log=False)
b.start()                         # run to the first decision (turn 1)
```

| Member | Meaning |
|---|---|
| `request_json(side)` | the side's current request as Showdown JSON text (`{"wait":true,...}` while waiting; `None` after the end) |
| `choose(side, text)` | Showdown choice string; raises `numbrion.ChoiceError` (a `ValueError`) with Showdown's error text (`args[0]`, e.g. `[Invalid choice] Can't move: ...`; `args[1]` = re-sent request JSON when the rejection revealed information) |
| `choose_action(side, a0, a1)` | typed choice from two codes; raises `ChoiceError` if the pair is not in the mask (the message is the engine's own wording where it has one) |
| `legal_mask(side)` | `(mask0 [47], joint [47, 47])` |
| `legal_mask_slot1(side, a0)` | `mask1(a0)` |
| `legal_actions(side)` | readable dict: request kind, per-slot legal codes with names, forced switch/pass budgets |
| `action_text(side, a0, a1)` | Showdown text of a legal pair, else `None` |
| `needs_action(side)`, `request_kind(side)` | the side must choose now; `"move"`, `"switch"` or `None` |
| `turn`, `ended`, `winner` (0/1/`None`), `tie`, `pokemon_left` | state |
| `drain_log()` | **omniscient** raw lines (engine `battle.log`, `\|split\|pN` triples intact) since the last call; needs `log=True` |
| `drain_player_log(side)` | **player view**, see below; needs `log=True` |
| `clone()`, `copy_from(other)`, `restore_from(other)`, `reset(seed, p1=None, p2=None)`, `reseed(seed)`, `prng_seed`, `started` | search support, see below |
| `hidden_team_indices(viewer)`, `replace_hidden_set(viewer, index, packed_set)` | determinization, see below |

A committing `choose`/`choose_action` runs the battle synchronously to the next decision or the end. A side
may re-submit (Showdown's cancel support) until the other side has answered. Time-line `\|t:\|` entries have
an empty timestamp, as in the fixtures.

### Observation streams and hidden information

* `request_json(side)` is the side's *own* request: exact HP, stats, moves, PP, item, ability, Tera type of
  its own team; nothing about the opponent.
* `drain_player_log(side)` is what the player sees. It resolves Showdown's `\|split\|pN` triples exactly like
  `extractChannelMessages` (`sim/battle.ts:35`): player N receives the *secret* line, everybody else the
  *public* line, and empty lines are dropped. For example, a switch-in is `\|switch\|p1a: X\|X, L80\|259/259` in
  player 1's stream and `...\|100/100` in player 2's. Lines are collected after every engine call, matching a
  Showdown `BattleStream` that calls `sendUpdates` after every input, so the two player streams plus the
  requests are the complete, leak-free information a ladder client gets. `drain_log()` is the omniscient
  stream (all secret lines) and must never be fed to a policy.
* Build observations only from `request_json` and `drain_player_log` (docs/training/TIPS.md rule 1).

### `clone`, `restore_from`, `copy_from`, `reset`, `reseed` (search)

`clone()` returns an independent battle at the same decision boundary. **It copies the whole true state:
both teams in full, all hidden information, and the PRNG state.** A search that must not see the real future
randomness or the opponent's hidden sets must call `reseed(new_seed)` on the copy (it replaces the PRNG state;
the next draw is the first of the new stream) and must determinize the opponent: never-revealed opponent sets
can be swapped in place (next section); everything else in the copy is the real state. Unseeded clones replay
*exactly* what the original will do, which is useful for debugging and for reproducing a position.

| Method | Cost (noisy, this PC) | Contract |
|---|---:|---|
| `clone()` | ~8 us | new battle sharing the immutable team definitions (`Arc`); no team parsing, no constructor draws; allocates its own engine scratch; log views start empty |
| `restore_from(other)` | ~1 us | turns an existing battle into an exact copy of `other` (state, PRNG, teams, names), reusing its allocations; `other` may have **different** teams/names/seed (another world); only the log mode must match; unread log lines are dropped |
| `copy_from(other)` | ~1 us | the older, stricter form: state only, `other` must come from the same constructor arguments |
| `reset(seed, p1=None, p2=None)` | ~7 us | back to a freshly constructed `Battle(seed, p1, p2, names, log)` before `start()`, keeping names, log mode and allocations; teams default to the current ones (unspecified genders are drawn again from the new seed, exactly like the constructor); a bad team raises `ValueError` and changes nothing |

All of them require (and Python battles always are at) a flushed decision boundary. A restored or cloned battle
plays out bit-identically to the original under identical decisions, including the omniscient and both player
logs (tested at random boundaries of many battles, including mid-turn replacement and Revival Blessing
requests). In hot loops keep one worker battle and `restore_from(root)` it per sample, or run a whole grid with
`GridExecutor` (below), which does exactly that on Rust threads.

### Determinization: `hidden_team_indices`, `replace_hidden_set`

```python
world = root.clone()                                  # one sampled world per determinization
for i in world.hidden_team_indices(viewer):           # opponent Pokemon the viewer has never seen
    world.replace_hidden_set(viewer, i, sample_set())  # one packed set, e.g. a line's record from a team pool
world.reseed(world_seed)                              # the PRNG is hidden information too
worker = world.clone()                                # then worker.copy_from(world) per grid cell
```

* `hidden_team_indices(viewer)`: indices into the opponent's *packed team* (construction order, stable for the
  whole battle; not the request's party order) whose set can be swapped now: never switched in (the leads
  are), never chosen or inspected by an Illusion user as its disguise, and unchanged since construction.
  This is privileged bookkeeping, not a public observation: eligibility depends on hidden facts (an unseen
  Zacian/Zamazenta-style species is never eligible), so the returned indices and their count can differ from
  what the viewer can infer, and must not be fed to a policy. The API is a *partial* determinization:
  ineligible hidden Pokemon keep their true sets in every sampled world, and revealed Pokemon keep their
  hidden details. A fair-information search must also sample those; that is not supported yet.
* `replace_hidden_set(viewer, index, packed_set)` gives that Pokemon a new set, raising `ValueError` (battle
  unchanged) for any other Pokemon, after the end, for Zacian/Zamazenta-style species (their `BattleStart`
  forme change already ran at start), and for a random-gender species with an empty gender field when the
  replaced input had an explicit or fixed gender: write `M`/`F` into the packed gender field. When both sets
  leave a random gender unspecified, the replaced Pokemon's drawn gender is reused; give it explicitly to avoid
  that small leak. Everything derived from the set changes (species, stats, HP, types, weight, moves and PP,
  ability, item, Tera type, level, gender, name, details); no log lines, events or PRNG draws happen, and
  `teams` reports the new packed text, so `copy_from` between the world and battles cloned before the swap is
  refused (clone the world instead).
* **Contract (tested):** the result is exactly the battle constructed with the sampled set from the start
  (all genders resolved, PRNG at its post-construction state) after replaying the same choices: that replay
  reproduces the original log, PRNG and the viewer's requests at every boundary, ends in the identical state,
  and both then play out bit-identically. When the swap does not change the constructor's gender-draw count it
  is also the battle built from the original seed and the swapped packed teams. Checked by
  `crates/engine/tests/determinize.rs` (random battles, explicit and drawn genders) and
  `difftest determinize_check` on all 13,034 recorded gate battles (25,582 swaps at 33,505 boundaries,
  including mid-turn replacement requests); through this API by `crates/pyengine/tests/determinize.rs` and
  `pytests/test_determinize.py`. Engine API: `Battle::replace_hidden_set`, `hidden_mons`, `revealed_mons`
  (`crates/engine/src/battle/determinize.rs`).
* Not determinized: what is hidden about *revealed* Pokemon (unrevealed moves, item, ability, EVs, Tera type)
  stays true in the copy, and so does a choice the opponent already submitted at this boundary (a search
  re-submits both sides anyway).
* Cost (Ryzen 5 3600, shared and noisy): one swap ~1.7 us, a whole world (~4 hidden sets) ~6 us on top of a
  ~1.1 us restore, versus ~9 us for a scratch-reusing `reset_from_team_defs` and ~17 us for `from_team_defs`
  (~49 us from packed text) followed by a full history replay. `crates/difftest/src/bin/search_bench.rs`
  measures these (`--op replace|replace-all|reset-defs|construct-defs`).

## `numbrion.BatchEnv`

```python
env = numbrion.BatchEnv(n_envs, team_pool_paths, seed=0, threads=0, log=False, joint_mask=False,
                        on_illegal="raise", max_teams_per_file=None, teams=None)
r = env.reset()
r = env.step(actions)             # int32-compatible [n_envs, 2 sides, 2 slots]
```

* **Team pools:** `team_pool_paths` are files with one packed team per line (`.txt` or `.txt.gz`; all files are
  merged into one pool, `max_teams_per_file` keeps a prefix; `teams=[...]` adds packed teams directly).
  `tools/oracle/gen-teams.mjs` makes more; `data/teams/pool-s{1,2,3}-200k.txt.gz` are 200k teams each. Each
  team is parsed on first use and kept for the pool's lifetime, so memory grows with the number of distinct
  teams drawn (estimate: on the order of 1 KB each, i.e. several hundred MB over a long run on the full
  600k pool); `max_teams_per_file` bounds it. At
  every (auto-)reset both teams are drawn uniformly and independently from the pool, and the battle seed from
  the environment's own RNG.
* **Randomness / determinism:** each environment owns two SplitMix64 streams derived from `(seed, env index)`;
  results depend on `seed` and the actions only, **not on `threads` or scheduling** (tested). `threads=0` uses
  all hardware threads. Work runs on a private rayon pool with the GIL released; `step` is a barrier over all
  environments, so use enough environments per thread (>= 16) to hide the spread in battle lengths.
* **Logging:** battles run with `NoLog` (no text is ever formatted) unless `log=True`, which switches to
  `TextLog` (~15% slower) so that `observe` can return player-view lines.

### `step` result

`reset()`, `step()` and `current()` return a `dict` of fresh numpy arrays (`n` = `n_envs`, `A` = 47):

| Key | dtype / shape | Meaning |
|---|---|---|
| `needs_action` | bool `[n, 2]` | the side must submit now (a waiting side's actions are ignored) |
| `request_kind` | uint8 `[n, 2]` | 0 none/wait, 1 move request, 2 replacement (switch) request |
| `mask0` | bool `[n, 2, A]` | legal slot-0 codes (all False where `needs_action` is False) |
| `mask1_any` | bool `[n, 2, A]` | union of legal slot-1 codes over legal slot-0 codes |
| `joint` | bool `[n, 2, A, A]` | only with `joint_mask=True`: the full pair matrix |
| `reward` | float32 `[n, 2]` | +1 winner / -1 loser / 0 otherwise and on ties, only on the step a battle ended |
| `done` | bool `[n]` | a battle ended during this step; everything above describes the **new** battle that auto-reset started |
| `winner` | int8 `[n]` | winner of the finished battle (0/1), -1 for ties and where `done` is False |
| `final_turns` | int32 `[n]` | length of the finished battle (0 where not done) |
| `turn` | int32 `[n]` | turn counter of the battle now running |
| `battle_id` | int64 `[n]` | episode counter of the running battle, +1 at every auto-reset |
| `illegal` | bool `[n, 2]` | the side's submitted action was illegal and replaced by the engine default (`on_illegal="default"`) |
| `decisions` | int | side choices applied by this step |

`step(actions)`: for every environment, each side with `needs_action` must give `(a0, a1)` that is in the
mask; entries of sides that need no action are ignored (use -1). Each side answers once per boundary, and
**the policy acts for both sides** (self-play): the two sides of an environment are two rows of your batch. One
`step` applies those choices, advances each battle to its next decision boundary (a turn can contain several
boundaries: mid-turn U-turn switches, end-of-turn replacements), and auto-resets finished battles. A battle is
never returned in a finished state.

* `on_illegal="raise"` (default): `step` validates *every* action against the masks first; a bad one raises
  `ValueError` naming the environment and side, and **nothing is applied**. `"default"`: the offending side gets
  the engine's `default` choice and `illegal` is set. An engine rejection of a mask-legal action (a bug) raises
  `RuntimeError`.
* `mask_slot1(a0)`: `a0` is int `[n, 2]` (slot-0 codes you just sampled); returns bool `[n, 2, A]` with
  `mask1(a0)` per side, so an autoregressive policy needs no `[A, A]` tensors.
* `random_actions(switch_prob=0.1, tera_prob=0.15)`: random legal actions drawn in Rust (the Showdown fixture
  generator's picker: switch with probability `switch_prob`, else a uniform move, uniform target, Tera with
  `tera_prob`). `numbrion.sample_uniform(result, env, rng)` is the numpy equivalent that samples every legal
  code uniformly (much more Tera- and switch-happy; for tests).
* `observe(env, side)` -> `{"request": str, "log": [str], "prev_log": [str], "turn", "battle_id", "log_enabled"}`:
  the side's own request JSON, the new **player-view** lines since the last call for that side (only with
  `log=True`; otherwise `[]`), and `prev_log`, the unread lines of the battle that ended at the last
  auto-reset (`\|win\|...` etc.). Same hidden-information rules as `Battle`.
* `clone_battle(env)` -> `numbrion.Battle` copy of an environment's current position (for search).
* `run_random(battles_per_env)` -> counters and wall seconds; plays random legal self-play entirely in Rust
  (benchmark/smoke driver; same code path as `step`). `stats()` gives cumulative counters.

Typical loop:

```python
env = numbrion.BatchEnv(256, ["data/teams/pool-s1-200k.txt.gz"], seed=0, threads=6, log=True)
r = env.reset()
while training:
    obs = [[env.observe(e, s) for s in (0, 1)] for e in range(env.n_envs)]   # request + new player-view lines
    a0 = policy_slot0(obs, r["mask0"])                    # int32 [n, 2], sampled within the mask
    a1 = policy_slot1(obs, a0, env.mask_slot1(a0))        # conditioned on slot 0
    r = env.step(np.stack([a0, a1], axis=-1))
    # r["reward"], r["done"], r["battle_id"] feed the learner
```

NoLog mode (default) offers the request only; the opponent side of the board is not derivable from it. A
tracker of *revealed* information (so a NoLog environment could provide a public board state) does not exist
in the engine yet.

## Fair-information worlds (`--features python,search`)

Opt-in: build the wheel with `maturin develop --release --features python,search` (`numbrion.SEARCH` is then
True). Default wheels do not contain these classes and their engine is unchanged. Background and limits:
[SEARCH-PREREQUISITES.md](SEARCH-PREREQUISITES.md) item 1.

```python
pool = numbrion.SetPool(["data/teams/pool-s1-200k.txt.gz"])        # generated teams (deduplicated sets)
wb = numbrion.WorldBuilder(viewer, own_packed_team, pool, names=("Alice", "Bob"), seed=0)
wb.push_lines(battle.drain_player_log(viewer))                       # every player-view line, in order
# at each boundary where the viewer acts:
wb.push_request(battle.request_json(viewer))                         # optional, strengthens the check
worlds = wb.worlds(4, seed=rng_seed, log=True)                       # started Battles at this boundary
wb.push_action(a0, a1)                                               # or push_choice("move 1 +2, switch 3")
# ... then push the lines that choice produced
```

* `SetPool(team_pool_paths=(), max_teams_per_file=None, teams=None)`: packed-team files and/or teams; `len()`
  sets, `distinct`. Sampling follows the generator's empirical distribution.
* `WorldBuilder(viewer, team, pool, names=None, seed=0, particles=8, segment_budget=1000, rebuild_budget=16,
  fresh_per_update=1)`: `team` is the viewer's own packed team with genders resolved (the pools' teams are).
  Inputs are only the viewer's lines, requests and choices; the real battle is never consulted, so the same
  object works on a ladder stream. Results depend only on the inputs, `seed` and the call sequence. Push a
  request after the lines that led to it (a ladder client receives it before them).
* `worlds(k, seed, log=False)` (GIL released): `k` worlds drawn by weight, each reseeded from
  `seed`; raises `RuntimeError` (with the last failure reason) when no world reproduces the observations.
  `update()` does the work without returning worlds. `alive`, `weights` (`(weights, ess)`), `stats()`,
  `last_failure` and `public_state()` (the tracker's view) are diagnostics.
* `RevealTracker()`: `feed(lines)`, `state()` -> dict of the public information per Pokemon of both sides
  (moves used, item / ability / Tera shown, HP, status). This is the "public tracker" of request 2 below, for
  log-enabled streams.
* Worlds are ordinary `Battle`s with `is_world == True` (copies keep it); `GridExecutor.run` warns on roots
  that are not worlds unless `allow_live=True`. Build grid candidates from the world's own mask.

## Search (`numbrion.GridExecutor`)

> **Roots must be fair-information worlds** (`WorldBuilder` above, search wheels only), not live battles or
> their clones. Read [SEARCH-PREREQUISITES.md](SEARCH-PREREQUISITES.md) first. `numbrion.GridExecutor` warns
> until `NUMBRION_SEARCH_PREREQS_DONE=1` is set, and on every `run` whose roots are not worlds unless
> `allow_live=True`.

One-turn search evaluates a grid of joint actions: per decision ~10 candidate pairs per side, optionally
several determinized worlds, each cell = restore the root, submit both sides' choices, advance to the next
decision boundary, score the leaf with a value net. `GridExecutor` runs the engine part of that in Rust:

```python
ex = numbrion.GridExecutor(threads=3)                  # persistent thread pool, one reusable battle per thread
root = env.clone_battle(e)                             # any started Battle at a fresh decision boundary
actions = numbrion.grid_actions(p1_pairs, p2_pairs)    # [k1, 2] x [k2, 2] -> int32 [k1*k2, 2, 2], cell i*k2 + j
seeds = rng.integers(0, 1 << 16, size=(len(actions), 4))
out = ex.run(root, actions, seeds=seeds)               # GIL released; dict, one entry per cell in input order
```

`run(roots, actions, world=None, seeds=None, stop="boundary", requests=True, masks=False, logs=None,
keep_leaves=False)`:

* **Cell.** The worker of the thread that picks the cell restores the cell's root (`restore_from`, ~1 us, no
  allocation), reseeds it, submits side 0's then side 1's codes (only for sides that need an action at the root;
  the other side's entries are ignored, use -1) and runs to the stop point. Choices commit only when both sides
  have chosen, so the submission order is invisible.
* **`actions`** are the typed codes of "Action space", `[n, 2 sides, 2 slots]`, the `BatchEnv.step` layout.
  Every pair is checked against its root's mask before anything runs.
* **Worlds.** `roots` is one `Battle` or a list; `world[i]` picks the root of cell `i` (default 0). Worlds may
  differ in teams, names and seeds (only `log` must agree), so determinized worlds are just roots built by the
  caller (clone the root, `replace_hidden_set` for the opponent's never-revealed Pokemon, see
  "Determinization"). Passing all worlds in one call keeps the threads busy without a per-world barrier. The
  roots are snapshotted at the call and never modified.
* **Seeds.** `None`: every cell keeps its root's PRNG state, i.e. replays the real future randomness (debugging;
  a leak in real play). `[4]`: every cell is reseeded with the same seed (common random numbers: cells of one
  world differ only by their actions, which lowers the variance of action comparisons). `[n, 4]`: one seed per
  cell.
* **Stop.** `"boundary"` (default) stops at the **first** decision boundary after the root choices, exactly
  one `BatchEnv.step`: the next turn's move request, or a mid-turn replacement request (U-turn, Volt Switch,
  Eject Button, ...: the switching side gets `request_kind` 2, the other side waits with 0), an end-of-turn
  faint replacement, a Revival Blessing request, or the end of the battle. The value net then scores a
  replacement boundary like any other training boundary. `"turn"` additionally answers every replacement
  request with Showdown's `default` choice (`Side.autoChoose`: the first eligible party members) until a move
  request or the end; `steps` counts the choice rounds (1 + replacement rounds). That is a convenience with a
  fixed replacement policy, not a search over replacements; for that use `keep_leaves` and search the leaf.
* **Roots** must be started, not ended, and at a fresh boundary where neither side has chosen yet. A root on
  which one side already chose (possible with Showdown's cancel support, e.g. a per-side loop that re-reads
  `needs_action` after the other side's choice already ran the turn) is rejected, because the other side's
  choice would run the turn with that stored choice.
* **Determinism.** A cell's result depends only on its root, codes, seed and the options: never on `threads`,
  the scheduling, or what a worker ran before (`restore_from` resets every transient engine field). Tested
  against a naive per-cell copy (Rust: `tests/search.rs`, 1 and 3 threads, dirty workers, several worlds per
  call, all seeding modes, both stop modes; Python: `pytests/test_search.py`).
* **Errors.** Bad input raises `ValueError` before anything runs (shapes, an illegal pair naming the cell and
  side, world index, seed range, mixed log modes, `logs=True` on `log=False` roots, unstarted, finished or
  half-chosen roots). `RuntimeError` means the engine rejected a mask-legal action (a bug).

Result keys (`n` cells, `A` = 47):

| Key | dtype / shape | Meaning |
|---|---|---|
| `ended` | bool `[n]` | the battle ended inside the cell |
| `winner` | int8 `[n]` | 0/1 when ended with a winner; -1 for a tie or a running battle |
| `turn` | int32 `[n]` | turn counter at the leaf |
| `steps` | int32 `[n]` | choice rounds applied (always 1 with `stop="boundary"`) |
| `needs_action`, `request_kind` | bool / uint8 `[n, 2]` | as in `BatchEnv.step`, at the leaf |
| `requests` | list of `(str or None, str or None)` | with `requests=True` (default): each side's request JSON at the leaf |
| `logs` | list of `([str], [str])` | with `logs` (default: when the roots have `log=True`): each side's **player-view** lines from the root to the leaf |
| `mask0`, `mask1_any` | bool `[n, 2, A]` | with `masks=True`: the leaf masks, as in `BatchEnv.step` |
| `leaves` | list of `Battle` | with `keep_leaves=True`: independent leaf battles (one `clone()`, ~8 us each) for deeper search |

**Evaluating a leaf.** Observations are built only from a side's own request and its player-view lines
(TIPS rule 1), so a leaf observation for side `s` is: the root's observation tracker state for `s`, fed with
`out["logs"][i][s]`, encoded with `out["requests"][i][s]`. That is exactly what `BatchEnv.observe` would
have delivered at that boundary (same split-line resolution), so the training encoder needs no search-specific
path. It needs `log=True` roots; `log=False` roots return requests only (cheaper: no log text is formatted).
The executor does not return tensors: feature encoding stays in Python, batched over all cells.

**Cost** (`search_bench.py`, 32 mid-battle roots at turns 3-10, 10 x 10 cells each, every cell reseeded and
returning both requests; noisy PC shared with other jobs, 4 alternating rounds of 1.5 s per mode):

| Mode | NoLog us/cell | NoLog cells/s | log=True us/cell | log=True cells/s |
|---|---:|---:|---:|---:|
| `GridExecutor(1)` | 65.6 | 15,200 | 86.6 | 11,500 |
| `GridExecutor(3)` | 27.3 | 36,700 | 41.8 | 23,900 |
| Python loop, `worker.restore_from(root)` per cell | 67.5 | 14,800 | 97.3 | 10,300 |
| Python loop, `root.clone()` per cell | 89.2 | 11,200 | 112.2 | 8,900 |

A cell is dominated by the simulated turn (~60 us here); restore is ~1 us of it, so the executor's single-thread
cost equals a hand-written restore loop and its value is running the cells in parallel with the GIL released
(2.4x on 3 threads on the shared machine; per-cell work is independent, so it should scale with idle cores).
`cargo run --release -p pyengine --example grid_bench` measures the same without Python (NoLog: executor/1
70 us, executor/3 35 us, sequential restore 70 us, clone per cell 75 us).

### Fused leaf encoding (`numbrion.SearchGrid`)

`GridExecutor` returns each leaf as strings (player-view lines, request JSON) that Python must feed to the
observation encoder. `SearchGrid` runs the same cells and encodes each leaf for the value network on the worker
thread that ran it, straight into fresh NumPy arrays. It needs no `search` feature (it uses only the encoder and
the executor; the engine is unchanged), but its roots should still be `WorldBuilder` worlds: like
`GridExecutor` it reminds of SEARCH-PREREQUISITES.md and, in search wheels, warns on roots that are not worlds
(`allow_live=True` silences that).

```python
enc = numbrion.ObsEncoder(dex_path, 2)                 # the bot's trackers: row s is side s, fed as usual
pair = (enc.snapshot(0), enc.snapshot(1))              # TrackerState copies at this decision
worlds = wb.worlds(4, seed=s, log=True)                # roots need log=True
grid = numbrion.SearchGrid(dex_path, threads=4)        # persistent pool: worker battle, tracker, cache per thread
opp = grid.encode_states([pair[1 - me]] * 4, [w.request_json(1 - me) for w in worlds])   # the opponent's view
out = grid.run(worlds, actions, pair, me, world=world, seeds=seeds)                      # one call per decision
state = model.trunk(out["ids"], out["floats"], out["field"], action_features, out["token_features"])
value = model.value(state, out["partner_ids"], out["partner_floats"])   # from out["eval_side"]'s point of view
```

* **`TrackerState`**: an immutable copy of one observation tracker, i.e. everything `encode_row` reads for a row
  besides the new lines and the request. `ObsEncoder.snapshot(row)` takes one (the encoder is unchanged),
  `ObsEncoder.restore(row, state)` puts one back (same side only: row `% 2`), `TrackerState(side)` is an empty
  tracker and `state.feed(lines)` a fed copy; `side`, `turn`, `battle_id` describe it.
* **`run(roots, actions, trackers, eval_side, world=None, seeds=None, stop="boundary", masks=False,
  action_features=False, guard=False, allow_live=False)`**: the cells, inputs and validation of
  `GridExecutor.run`, plus `trackers`, the side-0 and side-1 `TrackerState`s at the root (one pair for every
  root, or a list with one pair per root), and `eval_side`, the searching player's side. Per cell, on the worker:
  * the **eval side** is `eval_side` if that side must act at the leaf, else the other side if it must, else -1
    (the battle ended; all of the cell's encoded rows are zero). The value head was only trained on rows whose
    side must act, so the caller negates values evaluated from the other side and scores ended cells from
    `winner`;
  * the eval side's **observation**: its root tracker, copied and fed its own player-view lines from the root to
    the leaf, encoded with its own leaf request (ids, floats, field, token features; action features and guard
    when asked; the value head does not read them);
  * the **partner** (what `Model.value` reads from the other side's row): the other side's tracker, lines and
    leaf request the same way, own tokens 0-5 only (ids and floats, no damage features).
* **`encode_states(states, requests, lines=None, action_features=True, guard=True)`**: hypothetical
  observations without touching an encoder, e.g. the opponent's at each world's root (its tracker state, the
  world's `request_json(opp)`, no lines). Row `i` is a copy of `states[i]` fed `lines[i]` (None: none), encoded
  with `requests[i]` (None or "" for none), plus damage features. Keys `ids`, `floats`, `field`,
  `token_features`, `action_features`, `guard`. Runs on the grid's threads with the GIL released.
* **Exactness.** Every array equals, bit for bit, what `ObsEncoder.encode_batch` writes for the same row when a
  tracker with the same history is fed the same leaf lines and request (both tested on random cells, including
  ended leaves, one-sided replacement leaves and `stop="turn"`). Leaf requests are not formatted as JSON and parsed
  back: `request_tree.rs` builds the parsed tree from the engine's request data, and a test compares it with
  `Json::parse(request_json)` at every boundary of many battles (51,770 requests with `NUMBRION_TEST_BATTLES=400`,
  including requests updated by rejected choices). A change to the engine's request JSON must be repeated there.
* **Determinism and threads.** A cell depends only on its root, codes, seed, trackers, `eval_side` and the
  options, never on `threads` or the scheduling (each pool thread has its own worker battle, scratch tracker and
  static-feature cache; a cached feature equals a computed one). Calls on one grid serialize; `TrackerState`s
  are immutable and may be shared between grids and threads.
* **The opponent's row is an approximation.** The bot's encoder feeds the opponent row the bot's own lines (the
  real opponent view is not observable), so that tracker has the bot's own Pokemon at exact HP instead of the
  percentages the opponent sees, and nothing the opponent alone was shown. The opponent's own tokens come from
  its request (in a world: the sampled team), and its leaf lines are its real player view of the world.

Result keys of `run` (`n` cells; `ended`, `winner`, `turn`, `steps`, `needs_action`, `request_kind` and with
`masks=True` `mask0` / `mask1_any` exactly as `GridExecutor.run`):

| Key | dtype / shape | Meaning |
|---|---|---|
| `eval_side` | int8 `[n]` | the side whose observation was encoded, -1 when the battle ended |
| `ids` | int64 `[n, 12, N_IDS]` | eval side's observation, the `ObsEncoder` layout |
| `floats` | float32 `[n, 12, N_POKEMON_FLOATS]` | |
| `field` | float32 `[n, N_FIELD_FLOATS]` | |
| `token_features` | float32 `[n, 12, N_TOKEN_FEATURES]` | damage-calc token features (what the trunk reads) |
| `partner_ids`, `partner_floats` | int64 / float32 `[n, 6, ...]` | the other side's own tokens 0-5 at the leaf |
| `action_features` | float32 `[n, 2, 40, N_ACTION_FEATURES]` | with `action_features=True` |
| `guard` | bool `[n, 2, 4, 2]` | with `guard=True` |

**Cost** (`search_grid_bench.py`: 8 decisions at turns 3-10, each 4 worlds x 8 x 8 pairs = 256 cells with one
seed per cell; Apple M1 Pro shared with other jobs, load 7-8, 4 alternating rounds of 1 s). `slow` is
`GridExecutor` plus `ObsEncoder.encode_batch` of the leaves from Python, with every row restored from the root
snapshots (two rows per cell); `grid` is `GridExecutor` alone with logs and requests:

| Threads | grid us/cell | slow us/cell | fused us/cell | fused cells/s |
|---:|---:|---:|---:|---:|
| 1 | 34.7 | 85.6 | 53.7 | 18,600 |
| 2 | 21.6 | 73.4 | 30.4 | 32,900 |
| 4 | 16.5 | 70.1 | 20.9 | 47,900 |
| 6 | 14.5 | 68.1 | 17.0 | 58,800 |

The slow path stops scaling because its encoding runs on one thread; the fused path adds ~19 us of encoding to a
cell on one thread and scales with the engine. Per leaf side (one thread, `examples/search_grid_bench.rs`):
tracker copy 0.7 us, feeding ~17 lines 2.6 us, request tree 3.3 us (JSON writing plus parsing was 8.9 us),
row 3.4 us, token features 2.9 us (4.4 with action features), own tokens only 2.6 us.

## Performance

Ryzen 5 3600 (6 cores / 12 threads), engine at `df62d35` (perf round 1 merged), portable release build, team pool = first 20,000 teams of `pool-s1-200k`, random legal self-play
(`crates/pyengine/bench.py --threads 1 6 12 --seconds 6 --log`, 16 environments per thread). A random-policy
battle lasts ~19 turns and ~44 side decisions. "rust" is `BatchEnv.run_random` (no Python in the loop);
"step" is a Python loop of `env.step(env.random_actions())`; "numpy" samples the actions in numpy from the
masks; "step+log" is "step" with `TextLog` battles.

| Level | Threads | Battles/s | Decisions/s | Battles/s/thread |
|---|---:|---:|---:|---:|
| rust | 1 | 1,022 | 45,600 | 1,022 |
| step | 1 | 909 | 40,900 | 909 |
| numpy | 1 | 904 | 38,200 | 904 |
| step+log | 1 | 775 | 34,900 | 775 |
| rust | 6 | 5,703 | 256,000 | 951 |
| step | 6 | 4,032 | 181,000 | 672 |
| numpy | 6 | 3,765 | 160,000 | 628 |
| step+log | 6 | 3,145 | 141,000 | 524 |
| rust | 12 | 6,795 | 304,000 | 566 |
| step | 12 | 5,067 | 228,000 | 422 |
| numpy | 12 | 4,742 | 202,000 | 395 |
| step+log | 12 | 4,165 | 188,000 | 347 |
| step, 64 envs/thread | 6 | 4,657 | 208,000 | 776 |

So the engine itself sets the ceiling (about 1,000 battles/s per core); the interface costs ~10% on one thread
and ~27% on six threads with 16 environments per thread (the per-step barrier waits for the slowest
environment; 64 environments per thread recover it to ~14%), and building masks is ~1.5 us per side-boundary. The network, not the simulator, will be the bottleneck in
training (docs/training/TIPS.md). At `df62d35` construction of a battle cost ~70 us (`Scratch::default` was
~30 us of it), about 7% of a battle. At `a5fee3d` (2026-10-10, noisy shared-CPU runs) construction from packed
text is ~46-49 us and from pre-parsed teams ~17 us; auto-reset reuses the battle (`reset`, ~7 us) and
`Battle.clone()` skips parsing (~8 us).

## Validation

* `cargo test --release -p pyengine` (about 20 s): codec, split-line resolution; typed and text exhaustive
  mask checks; Revival Blessing boundaries; clone/reseed determinism; BatchEnv invariants, thread-count
  independence, illegal-action handling, observation hooks; 400 complete battles through the Rust driver
  with no rejection or panic.
* `pytest` in `crates/pyengine` (about 15 s): the same properties through the Python API, including 300
  battles driven by `BatchEnv.step` with numpy-sampled actions, `restore_from`/`reset` equivalence, and
  `GridExecutor` against a per-cell Python loop on 1 and 3 threads.
* `tests/search.rs`: `GridExecutor` cells against naive copies (requests, logs, masks, outcome, PRNG of the
  kept leaves, and the leaves played on); with `NUMBRION_TEST_BATTLES=400`: 4,418 boundary-mode cells (1,014
  replacement leaves: 61 pivots, 941 faints, 12 Revival Blessing) and 4,271 turn-mode cells, all identical on
  1 and 3 threads.
* `tests/search_grid.rs` and `pytests/test_search_grid.py`: `SearchGrid` leaves against `GridExecutor` logs and
  requests encoded by an `ObsEncoder` (restored from the snapshots, and in Python also a fresh encoder replaying
  the whole history), 1 and 3 threads, eval-side rules, `encode_states`, input errors, `WorldBuilder` worlds as
  roots with the bot's tracker pair; the request tree against the parsed request JSON. With
  `NUMBRION_TEST_BATTLES=400`: 4,770 cells (752 ended, 553 with one side acting) and 51,770 requests, all identical.
* `NUMBRION_TEST_BATTLES=N` scales the number of battles of every test; the figures below were produced with it.

Large runs on this PC (engine perf round 1), all green:

| Check | Scale |
|---|---|
| typed path vs mask (`NUMBRION_TEST_BATTLES=400`) | 400 battles, 10,140 boundaries, 17,775 acting sides, **39.3M code pairs** thrown at `choose_typed`: 2,128,713 mask-legal pairs all accepted with the stored choice equal to the codes; 249,218 pairs legal per slot but excluded by a joint constraint all rejected; 806 forced-move aliases accepted by the engine and correctly canonicalized |
| text grammar vs mask (40 battles) | 966 boundaries, **10.2M choice strings**, 205,685 accepted, each accepted one representable and mask-legal |
| mask vs the engine's own `is_legal_joint_choice` (4,000 battles) | 102,188 boundaries, 179,863 acting sides, all 2,209 pairs each |
| Revival Blessing replacement requests (steered play, 400 battles) | 364 boundaries, 0.8M typed pairs and 0.7M strings |
| BatchEnv from Python on the real 600k-team pool (`pool-s{1,2,3}-200k`), 6 threads | **33,721 complete battles**, 1.5M side decisions, 0 illegal actions, 0 rejected choices, 0 panics |

The battles of the mask tests come from `crates/pyengine/testdata/pool-400.txt.gz` (400 teams, 11 with
Revival Blessing); the Python run used the full pools.

## Requests for the engine (not blocking)

1. Done: `impl Clone for Battle`, `Battle::restore_from` and `Battle::reset` (used by `clone`, `restore_from`,
   `copy_from`, `reset`, `BatchEnv` auto-reset and `GridExecutor`).
2. A public tracker of revealed information, so NoLog environments can offer a public board state.
   (`Battle::revealed_mons` covers only which Pokemon were ever switched in or used as a disguise.) Partly
   done: `RevealTracker` (search wheels) builds it from player-view lines, so it needs `log=True` streams.
