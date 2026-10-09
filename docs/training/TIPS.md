# Training tips

A recap of the approaches judged most likely to help train a `gen9randomdoublesbattle`
bot from pure self-play on this PC. These are working notes, not decisions. The
engine contract in `docs/design/ARCHITECTURE.md` outranks anything here.

## Starting point

- Hardware: Ryzen 5 3600 (6 cores, 12 threads), 16 GB RAM, Radeon RX 5500
  (Navi 14). ROCm does not officially support this GPU, so plan on CPU-only PyTorch.
- Showdown baseline: 18.3 battles/core/s, about 3 ms per doubles turn. Engine target
  is at least 915 battles/core/s.
- On this CPU, the network costs more than the simulator. A rough estimate for a
  ~0.6M-parameter transformer over 13 tokens: about 16 MFLOP per forward pass, and
  about 13x that per training sample (rollout plus 4 PPO epochs of forward and
  backward). At 50 decisions per battle that is about 10 GFLOP, or roughly 0.3 core-s
  per battle. Showdown takes about 0.055 core-s per battle. CPU-only training should
  reach only about 10 to 30 battles/s in total, and the Rust engine speeds it up by
  only about 1.2x. The engine pays off with a GPU, where the network is cheap and
  the simulator becomes the bottleneck, and for search. Estimate, not measured.
- Pool size: 501 species, 788 sets, 357 moves and 201 abilities in
  `data/random-battles/gen9/doubles-sets.json`.

## Rules that prevent silent failure

1. **Build observations only from per-side protocol.** The bot reads the same
   Showdown protocol stream in training as on the ladder, through one parser. Hidden
   information then can't leak into training, and the bot behaves the same online.
2. **Don't wait for the engine.** Start the training pipeline on the Showdown oracle
   now. Swap in the engine behind the same interface once difftest parity holds.
3. **Train only on engine paths with proven parity.** A mechanics bug becomes a
   strategy the bot learns to exploit.
4. **Test pieces in isolation.** RL bugs don't crash. They make learning slightly
   worse. Before scaling, check encode/decode round-trips, action masks and value
   targets on hand-made positions.

## Observation (encode)

- Represent each of the 12 Pokémon as one token, plus a field token for weather,
  terrain, Trick Room, Tailwind, screens and turn number.
- Use learned embeddings for species, move, item, ability and type IDs, **and add
  property features**: base power, accuracy, priority, category, type, spread and
  contact flags, base stats. Rare moves and species then work like similar common
  ones instead of staying as untrained random vectors.
- **Add damage-calc features.** For every move against every target, including the
  ally, give expected damage %, KO chance and who moves first. The network shouldn't
  have to learn the damage formula from wins and losses.
- **Mark unrevealed opponent information explicitly.** Track a belief over the
  species' possible random-battle sets that narrows as moves, items and abilities
  are revealed.
- Include the timing state the rules depend on, such as turns since switch-in
  (Fake Out), Protect used last turn and its counter, remaining Tera, and turns left
  on field effects. Any rule the bot can't see is a rule it can't follow.
- Use symmetry. Give bench tokens no position encoding, and augment training data by
  swapping the left and right active slots.

## Model

- Transformer encoder over the 13 tokens. On CPU, about 0.5M to 1M parameters
  (for example width 128, 3 layers). With a GPU, 2M to 10M parameters and 4 to 6 layers.
- **Choose slot B conditioned on slot A's action.** Independent heads can't
  coordinate. A typical symptom is both Pokémon attacking a foe the first attack
  already KOs.
- Score each legal action from its own features, as DouZero does, and mask illegal
  actions before sampling.
- Heads:
  - policy A and policy B
  - value, the chance to win
  - **auxiliary heads** predicting the opponent's hidden moves, item, ability and
    Tera type, plus HP after the turn. Self-play always knows the true answers. These
    heads speed up learning and give a learned belief for search later.
- **Oracle critic.** The training-only value head also sees both sides' full hidden
  information. The policy sees only the player's view. This reduces noise in
  learning from crits, misses and unknown sets.

## Training

- PPO with GAE value bootstrapping. Adapt CleanRL's PPO rather than writing one from
  scratch.
- Reward: +1 for a win, -1 for a loss. Small HP and KO shaping is fine early, but
  **phase it out to zero**. Watch game length and stalling. If the shaped reward rises
  while win rate stays flat, the bot is gaming the shaping.
- **Opponent pool.** Play about 80% of games against the latest model and 20 to 30%
  against random older checkpoints. Later, add exploiter agents trained only to beat
  the frozen main bot. Try R-NaD regularization (DeepNash) if strategies cycle.
- Resume from checkpoints after changes. When the architecture grows, carry the old
  weights over by enlarging the network in place (OpenAI Five's "surgery"), rather
  than retraining from zero.
- Watch policy entropy. Near zero means exploration has stopped. Use an entropy bonus
  or a lower learning rate.

## Search, phase 2

- Simultaneous moves: sample about 10 candidate joint actions per side from the
  policy, score the 10×10 grid with the value net and the engine, solve it with
  regret matching (as the Diplomacy bots did), and play the result.
- Sample opponent hidden sets from the belief before each search (determinization,
  as Foul Play does).
- Distill: train the policy toward the search result. Gumbel AlphaZero works with
  only 2 to 16 simulations.
- `BattleState` is `Copy`, about 37.7 KB, so snapshotting at decision boundaries is
  cheap enough for search.

## Evaluation

- **Duplicate evaluation.** Play each team pair twice with the sides swapped and the
  same seed, so luck mostly cancels.
- **Fishtest-style acceptance.** Accept a change only if new vs old passes a
  sequential statistical test (SPRT). Most changes should fail.
- Baselines:
  - random
  - "always use the strongest attack"
  - poke-env heuristic players
  - a pool of old checkpoints. Check for non-transitive cycles, where A beats B,
    B beats C and C beats A.
- **Mistake counters.** Turn each mistake pattern found in replays into a metric:
  overkill, Fake Out after the first turn, attacking into Protect, ally damage from
  spread moves.
- Value calibration: check that predicted 70% positions actually win about 70%.
- Win rate broken down by species, weather, Trick Room and team archetype.
- **Luck vs mistake.** Replay lost games from snapshots with many different seeds.
  A loss in most replays points to the turn where the bot's chances collapsed.
  Otherwise it was luck.
- Once the bot is stronger than its developers:
  - **search audit**: flag positions where deep search strongly disagrees with the
    policy
  - exploiter agents
  - games against strong humans
- Save every 1000th self-play game as a Showdown log for replay review.

## Tripwires

| By | Expect | Otherwise suspect |
|---|---|---|
| First hours | 95%+ vs random | Training or reward bug |
| ~1M games | Beats "always use the strongest attack" | Bug or missing inputs |
| A few million games | Beats poke-env heuristics | Encoding or reward |
| Ongoing | Elo vs checkpoints keeps rising, no cycles | Opponent pool |
| First ladder games | Ladder results roughly match offline evaluation | Information leak or protocol mismatch |

Check Showdown's rules on bots before playing on the ladder.

## Renting compute (budget about €100)

- Rent only after all of these hold:
  - the pipeline works locally
  - a local run shows rising win rate vs heuristics
  - throughput has been measured
  - checkpoint and resume work
- Rent a GPU instance (RTX 4090 at about $0.15 to 0.35/h on Vast.ai or RunPod
  Community, 2026 prices) with at least 16 vCPUs for engine workers. A CPU-only box
  makes sense only while training on Showdown.
- Suggested split: about €10 on scaling tests, €70 on one long run with interruptible
  instances and checkpoints every 15 minutes, €20 in reserve.
- Join the instance to the tailnet with an ephemeral key, and view dashboards over
  Tailscale with no public ports. Destroy the instance afterwards. Stopped instances
  still bill for storage.

## Main risks

- Self-play has no convergence guarantee in imperfect-information games. Cycling or
  self-specific play is the main reason for the opponent pool.
- Multi-turn plans such as Trick Room setup under Follow Me are hard to discover by
  random exploration. Search and the opponent pool help.
- Luck caps how dominant the bot can be. A 60 to 65% win rate against strong humans
  would be a good result.
- Nobody knows how much data doubles needs. Judge from the slope of the early
  learning curve.
- The biggest risk is engineering stalling before the first training run.
