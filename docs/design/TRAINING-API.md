# Training API (`numbrion` Python package)

`crates/pyengine` builds the Python package `numbrion` (PyO3 + maturin, one abi3 wheel for every CPython
>= 3.9). It exposes the engine to AlphaZero/PPO-style self-play through a **fixed discrete action space with
exact legality masks**, **hidden-information-safe observation streams**, and a **batched, auto-resetting
environment** that runs battles in parallel Rust threads with the GIL released. Everything here uses only the
public API of `crates/engine`; feature engineering (turning requests and logs into tensors) is out of scope.

| Piece | Where |
|---|---|
| `numbrion.Battle` | one battle: debugging, scripted play, search (`clone`, `reseed`) |
| `numbrion.BatchEnv` | N battles, `step(actions)` with auto-reset, masks, rewards |
| action codes, masks | `src/action.rs`, `src/mask.rs` |
| tests | `tests/*.rs` (Rust, exhaustive mask checks) and `pytests/` (Python) |
| benchmark | `crates/pyengine/bench.py` |

## Build and install

Linux/macOS (the repo keeps its virtualenv in `.venv`, which is git-ignored):

```sh
uv venv .venv && uv pip install --python .venv/bin/python maturin numpy pytest   # or: python -m venv .venv
cd crates/pyengine
../../.venv/bin/maturin develop --release --uv      # editable install; use plain `maturin develop` with pip
../../.venv/bin/python -m pytest                    # 19 tests, ~15 s
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
| `clone()`, `copy_from(other)`, `reseed(seed)`, `prng_seed` | search support, see below |
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

### `clone`, `copy_from`, `reseed` (search)

`clone()` returns an independent battle at the same decision boundary. **It copies the whole true state:
both teams in full, all hidden information, and the PRNG state.** A search that must not see the real future
randomness or the opponent's hidden sets must call `reseed(new_seed)` on the copy (it replaces the PRNG state;
the next draw is the first of the new stream) and must determinize the opponent: never-revealed opponent sets
can be swapped in place (next section); everything else in the copy is the real state. Unseeded clones replay
*exactly* what the original will do, which is useful for debugging and for reproducing a position.

Implementation: `Battle` is not `Clone`, so `clone()` rebuilds the battle from the packed teams and overwrites
its `BattleState` (a `Copy` snapshot, ~38 KB) with the original's; the copy's log buffers start empty.
Measured on this PC: `clone()` ~45 us, `copy_from()` ~0.7 us, `legal_mask` ~1.5 us. In hot loops keep one
scratch clone per search worker and call `scratch.copy_from(root)` (same seed, teams and names required) to
reset it. A cloned battle plays out bit-identically to the original under identical decisions, including the
omniscient and both player logs (tested at random boundaries of many battles).

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
  Only the count is visible to a player, so which index holds which unseen Pokemon carries no information.
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
  `tools/oracle/gen-teams.mjs` makes more; `data/teams/pool-s{1,2,3}-200k.txt.gz` are 200k teams each. At
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
training (docs/training/TIPS.md). Construction of a battle costs ~70 us (`Scratch::default` is ~30 us of it),
about 7% of a battle; `Battle.clone()` pays the same.

## Validation

* `cargo test --release -p pyengine` (about 20 s): codec, split-line resolution; typed and text exhaustive
  mask checks; Revival Blessing boundaries; clone/reseed determinism; BatchEnv invariants, thread-count
  independence, illegal-action handling, observation hooks; 400 complete battles through the Rust driver
  with no rejection or panic.
* `pytest` in `crates/pyengine` (about 15 s): the same properties through the Python API, including 300
  battles driven by `BatchEnv.step` with numpy-sampled actions.
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

1. `impl Clone for Battle` (or `Battle::restore_from(&Battle)`): `Battle` has no `Clone`, so `clone()`
   rebuilds team definitions and a fresh `Scratch` (~70 us instead of ~1 us).
2. `Battle::reset(seed, p1, p2)` reusing an existing `Scratch`, to make auto-reset cheaper.
3. A public tracker of revealed information, so NoLog environments can offer a public board state.
   (`Battle::revealed_mons` covers only which Pokemon were ever switched in or used as a disguise.)
