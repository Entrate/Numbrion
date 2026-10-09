**Keep the faithful event semantics and exact RNG. Build a compact native engine around them, then optimize verified paths within that engine.** I would reject a separate, approximate direct-mechanics engine for this project: self-play will find discrepancies that ordinary competitive play rarely encounters.

Your priorities should be:

1. Freeze the format and build the differential oracle.
2. Establish ordering, state identity, arithmetic, and choice semantics.
3. Implement generic move processing and effects.
4. Optimize measured hot paths while retaining exact differential agreement.

I reviewed checkout `7332b60e22b9e8194bb53549549eba241d73cc9a`. No files changed. Performance estimates below are planning estimates, not measurements.

**1. Architecture: your direction is right, with two changes**

First, distinguish **Showdown semantics** from **Showdown’s implementation architecture**. You need its event collection, ordering, suppression, relay values, and mutation behavior. You do not need dynamic objects, string lookup, generic JavaScript values, or a general-purpose plugin system.

Second, use generated data for ordinary moves. Do not create hundreds of handwritten files for moves adequately described by base power, target, accuracy, flags, boosts, status, and secondaries. Reserve effect source files for custom behavior.

I would build:

```text
immutable generated Dex + team definitions
                    |
compact mutable BattleState
                    |
typed event collector + ordered executor
                    |
generic move pipeline + custom handlers
                    |
typed public events / observations / choices
```

The executor should initially mirror these entry points:

- `singleEvent`
- `runEvent`
- `priorityEvent`
- `eachEvent`
- `fieldEvent`, which handles residuals and switch-ins in this checkout

Use a generated `HandlerId` and a dispatcher with direct calls. **A table of function pointers removes string lookup, but still introduces indirect calls and limits inlining.** Benchmark that against generated per-event `match` dispatch. Avoid one enormous dispatcher with every event/effect combination: instruction-cache footprint can become the next bottleneck.

Give hook relationships explicit IDs: ordinary, source, ally, foe, any, side, field. An event’s name alone does not describe which holders participate.

Make return values explicit. Showdown distinguishes `undefined`, `null`, `false`, numeric zero, and special failure results. Flattening these into `Option<bool>` will break move success, logging, and subsequent processing. See [runEvent](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:803) and the [spread move pipeline](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:584).

Zero runtime dependencies in the core is reasonable. Zero dependencies in exporters, fuzzing tools, Python bindings, and development utilities would be needless friction.

Your measured profile supports a rewrite, but does not establish an enormous speedup by itself. Eliminating a 60% cost completely yields only:

```text
1 / (1 - 0.60) = 2.5×
```

Larger gains must also come from native arithmetic, fewer allocations, compact state, eliminating generic move cloning, and avoiding text generation.

For budgeting, I would use these deliberately broad ranges, with the same battle mix, excluding team generation and without protocol formatting:

| Implementation | Plausible single-core planning range | Relative to 18 battles/s |
|---|---:|---:|
| Compact native engine with faithful event execution | 180–720 battles/s | 10–40× |
| Aggressively specialized direct processing | 360–1,800 battles/s | 20–100× |

These ranges can miss by severalfold. They are **targets to test**, not evidence that abandoning events buys 2×. A good typed executor and a good direct engine may end up much closer.

The comparison to poke-engine needs qualification: its own README describes a singles-only engine that is intentionally incomplete. Its search throughput is not a correctness-matched doubles benchmark. [poke-engine README](https://github.com/pmariglia/poke-engine)

The hybrid I recommend is:

- One state representation.
- One set of effect implementations.
- An exact generic executor.
- Specialized paths for common event configurations.
- Differential checks between generic and specialized execution, including RNG state.

**Keep the Gen5 LCG in training initially.** Its arithmetic is cheap. Removing formatted logs should be an output-sink change, not a mechanics mode.

**2. RNG compatibility is mostly an ordering problem**

The RNG itself is straightforward:

```rust
fn next_u32(&mut self) -> u32 {
    self.state = self.state
        .wrapping_mul(0x5D588B656C078965)
        .wrapping_add(0x00269EC3);
    (self.state >> 32) as u32
}

fn below(&mut self, n: u32) -> u32 {
    ((self.next_u32() as u64 * n as u64) >> 32) as u32
}
```

Pack the four numeric seed components in big-endian significance. Use multiply-high range mapping, **not modulo or rejection sampling**. Those change outputs or draw counts. See [PRNG.random](/home/aminaliu/src/pokemon-showdown/sim/prng.ts:91) and [Gen5RNG](/home/aminaliu/src/pokemon-showdown/sim/prng.ts:235).

This checkout defaults to Sodium RNG when no seed is supplied. Explicit numeric seeds select Gen5. Thus your compatibility claim should be “exact against Showdown with explicitly supplied Gen5 seeds.” [Seed selection](/home/aminaliu/src/pokemon-showdown/sim/prng.ts:35)

The consequential consumption sites are:

| Site | Required behavior | Source |
|---|---|---|
| Handler and action speed ties | Exact selection-sort swaps followed by forward Fisher–Yates | [speedSort](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:433), [shuffle](/home/aminaliu/src/pokemon-showdown/sim/prng.ts:150) |
| Per-Pokémon events | `eachEvent` speed-sorts actives before running their events | [eachEvent](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:469) |
| Queue insertion | Random insertion position among tied actions; does not sort the existing queue | [insertChoice](/home/aminaliu/src/pokemon-showdown/sim/battle-queue.ts:369) |
| Dynamic queue updates | Gen 9 updates remaining action speeds and sorts again between moves | [runAction](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:2917) |
| Switch-ins | Pre-sort all actives, including fainted entries, to create `speedOrder` | [runSwitch](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:181) |
| Random targets | Samples eligible allies or foes; sampling one candidate still draws | [getRandomTarget](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:2494), [randomFoe](/home/aminaliu/src/pokemon-showdown/sim/side.ts:369) |
| Retargeting | A selected foe fainting can cause another random target choice | [getMoveTargets](/home/aminaliu/src/pokemon-showdown/sim/pokemon.ts:821) |
| Accuracy | Numeric accuracy, including 100, draws; boolean `true` bypasses the draw | [hitStepAccuracy](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:726) |
| Critical hits | Roll depends on `willCrit` and crit ratio; prevention can happen afterward | [getDamage](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:1610) |
| Damage variation | `100 - random(16)`, with the exact surrounding truncations | [randomizer](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:2395) |
| Multihit | Weighted sample, possible additional Loaded Dice draw, repeated accuracy checks | [hit loop](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:850) |
| Secondaries | Draw happens before checking whether chance is absent or guaranteed | [secondaries](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:1325) |
| Self boosts/drops | Certain self effects draw even without a stated chance | [selfDrops](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:1306) |
| Status/volatile behavior | Sleep duration, paralysis, thawing, confusion duration/self-hit, trapping duration, rampage duration, Protect checks | [conditions](/home/aminaliu/src/pokemon-showdown/data/conditions.ts:39) |
| Effect-specific RNG | Trace, Harvest, Effect Spore, contact status abilities, Cursed Body, Healer, Shed Skin, others | [Trace](/home/aminaliu/src/pokemon-showdown/data/abilities.ts:5120), [Harvest](/home/aminaliu/src/pokemon-showdown/data/abilities.ts:1801) |
| Initialization | Missing gender can consume battle RNG | [Pokémon constructor](/home/aminaliu/src/pokemon-showdown/sim/pokemon.ts:340) |

Two examples explain why ordinary optimization instincts are dangerous:

- A 100% secondary still consumes a draw.
- Outside sun, Harvest rolls **before** checking whether there is an eligible berry to restore. Moving the eligibility check earlier desynchronizes RNG even when no item restoration occurs.

Also, `speedSort` is not interchangeable with “stable sort, then shuffle equal groups.” Selection-sort swaps can rearrange later tied groups. For input:

```text
A(speed 1), B(speed 1), C(speed 2), D(speed 1)
```

the first swap leaves the lower-speed group as `B, A, D`. A stable sort would leave `A, B, D`. Identical shuffle draws then select different identities.

The ordering rules to reproduce are:

- **Normal comparator:** order ascending, priority descending, speed descending, subOrder ascending, effectOrder ascending. Zero order is treated as the default last order. [Comparator](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:408)
- **Special comparators:** `Invulnerability`, `TryHit`, `DamagingHit`, and `EntryHazard` use stable left-to-right sorting. `priorityEvent` uses redirect ordering. These paths do not use normal random tie-shuffling. [Selection](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:793)
- **Holder collection:** status → volatiles → ability → item → base-species handlers → slot conditions. [Pokémon handlers](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:1101)
- **Relationship collection:** target handlers, allies/self prefixes, foe prefixes, source prefixes, side handlers, field handlers, battle/rule handlers. Multi-target events collect separately for each target; do not casually deduplicate repeated handlers. [findEventHandlers](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:1039)
- **Active ordering:** side order, then active-slot order: ordinarily `p1a, p1b, p2a, p2b`. Some helpers filter by HP; others filter by `fainted`. Those are not always equivalent during resolution. [getAllActive](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:1369), [Side.allies](/home/aminaliu/src/pokemon-showdown/sim/side.ts:390)
- **Insertion ordering:** volatile, side-condition, slot-condition, and pseudo-weather maps preserve string-key insertion order. Updating an existing key preserves position; deleting and reinserting moves it to the end. Audit any integer-like keys separately.
- **Effect order:** it is a separate global creation counter, not volatile insertion position. Its assignment depends on holder activity. [initEffectState](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:3320)
- **Switch-ins:** implicit subOrder depends on effect class and holder kind; switch-in speeds receive fractional offsets from the precomputed `speedOrder`. Ability/item `onStart` may be invoked as `onSwitchIn`, using switch-in metadata. [resolvePriority/getCallback](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:954)
- **Residuals:** effects with durations participate even without callbacks. Duration decrement, expiration, state-identity checks, and faint processing happen in a specific sequence. [fieldEvent](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:488)
- **Mid-turn choices:** new choices are sorted, then the saved queue is appended. Do not globally sort both queues together. [commitChoices](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:2996)
- **Party ordering changes:** switching swaps entries in `side.pokemon`; preserving original roster order alone is insufficient. [switchIn](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:119)

**Do not skip an `eachEvent` call just because every callback would do nothing. Its active-speed sort may still draw.**

**3. State layout: fixed and Copy, but budget 8–16 KiB first**

A few KiB is a useful optimization goal. It should not be a correctness constraint imposed before auditing effect state.

Separate three things:

```rust
struct Engine {
    state: BattleState, // Copy; sufficient at every decision boundary
    scratch: Scratch,   // handler lists, event frames, current move processing
    // immutable Dex and team definitions referenced by IDs
}
```

Clone only at documented decision boundaries. Include the unfinished action queue because U-turn and Revival Blessing can pause a turn. Execution scratch can remain outside snapshots if callbacks have unwound before returning a request.

Use stable Pokémon identities `0..11`, with a separate party-order array for each side:

```rust
#[derive(Clone, Copy)]
struct SideState {
    party: [MonId; 6],       // current Showdown party order
    active: [MonId; 2],
    tera_used: bool,
    pokemon_left: u8,
    // choice phase, faint history, condition-list heads
}
```

For each Pokémon retain:

- Original set ID; current and base species; current and base ability.
- HP/max HP, stored/base stats, boosts, types, Tera type/state, Stellar-used-type mask.
- Current move slots and original move PP state.
- Item, last item, relevant usage flags.
- Status/ability/item/species state references.
- Volatile list.
- Last move, last target location, move-result enums, relevant turn counters.
- Switching/fainting flags and illusion identity.
- Compact attack-history summaries.

Original IVs/EVs and names can live in immutable set definitions. Form changes still need access to them for stat recalculation.

Do not assume `lastMove` is always just an immutable move ID. Audit consumers of mutable `ActiveMove` data and retain the fields they actually read.

For effect state, I favor **a fixed arena with typed payload schemas**, plus insertion-ordered lists per holder:

```rust
#[repr(C)]
#[derive(Clone, Copy)]
struct EffectCell {
    effect_order: u32,
    generation: u32,

    effect: u16,
    source_effect: u16,
    next: u16,
    prev: u16,

    duration: i16,
    owner: u8,
    source: u8,
    source_slot: u8,
    flags: u8,
    aux: u16,

    payload: [u32; 6],
} // 48 bytes
```

This is an illustrative envelope, not a claim that every payload fits 24 bytes.

Each effect declares its payload schema. Generated accessors interpret fields as, for example:

```text
Substitute: HP
Encore: move ID + slot + duration
Protosynthesis: best stat + from-Booster flag
Partial trapping: source identity + source slot + counters
```

Use dedicated fixed storage for unusually large payloads rather than inflating every cell. Preserve absent versus present fields when that distinction affects behavior.

Important invariants:

- List position preserves insertion order.
- Removal must not use `swap_remove`.
- Reinsertion gets a new generation.
- Captured handlers refer to a state instance, not merely an effect ID.
- Source Pokémon identity and source slot are separate.
- Pool reuse must not make an old captured handler accidentally reference a new effect.

Residual processing explicitly compares captured state-object identity. [Identity checks](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:529) Conversely, do not add blanket invalidation to every event: `runEvent` and `fieldEvent` do not perform identical stale-handler checks.

An initial size budget:

| Component | Approximate bytes |
|---|---:|
| 12 Pokémon, 192–256 bytes each | 2,304–3,072 |
| 128 effect cells × 48 bytes | 6,144 |
| Sides, field, queue, pending choices, counters | 1,000–2,000 |
| Exceptional fixed payload storage | 500–1,500 |
| Total | Roughly 10–13 KiB |

The 128-cell capacity is provisional. **Prove or conservatively bound capacities before release.** Fuzzing without overflow is not a proof.

A useful format-specific reduction: Showdown’s `attackedBy` array can accumulate across turns while attackers remain active. The relevant consumers here appear reducible to per-source latest-attack records, ordering stamps, and a “damaged by this source this turn” mask. Avalanche uses the latter; Anger Shell/Berserk use recent damage. See [attack history](/home/aminaliu/src/pokemon-showdown/sim/pokemon.ts:917), [pruning](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:1705), and [Avalanche](/home/aminaliu/src/pokemon-showdown/data/moves.ts:908).

That reduction is much more valuable than copying a growing array faithfully.

Keep ordinary alignment. Avoid `repr(packed)`. Hash named semantic fields, not raw Rust struct bytes: padding and unused arena contents are unsuitable for stable state hashes.

**4. Reachability: build an audited dependency graph, not a sampled inventory**

The local doubles JSON contains:

- **501 species keys**
- **788 set variants**
- **357 distinct movepool moves**
- **201 listed abilities**
- All 19 Tera types, including Stellar

These are initial candidate inventories, not the complete closure or proof that every candidate survives generator constraints.

The format itself has **Sleep Clause Mod and Illusion Level Mod, and no Team Preview**. Do not accidentally inherit Doubles OU rules. [Format definition](/home/aminaliu/src/pokemon-showdown/config/formats.ts:165)

I would derive scope in five steps:

1. **Freeze inputs.** Record source commit, built-data fingerprint, resolved rule table, generator configuration, and `Config.potd`. Freeze these before assigning effect work.
2. **Extract initial candidates.** Read doubles sets, `getForme`, move-generation constraints, ability selection, priority items, and doubles items. Items are algorithmic, not fully listed in the JSON. [randomSet](/home/aminaliu/src/pokemon-showdown/data/random-battles/gen9/teams.ts:1498), [items](/home/aminaliu/src/pokemon-showdown/data/random-battles/gen9/teams.ts:1157)
3. **Resolve Gen 9 data through Dex.** Export effective definitions after mod inheritance, aliases, and normalization. Include embedded move/ability/item conditions.
4. **Expand dependencies to a fixed point.** Declarative statuses and conditions; handler-added effects; called/reflected/repeated moves; core-created conditions; form transitions; ability replacements; item transfers; species hooks; rule hooks; request-only hooks.
5. **Audit dynamic edges.** Every computed `formeChange`, `setAbility`, condition ID, or called move needs an explicit dependency rule and source reference.

Sampling teams proves that something occurs. It cannot prove that something never occurs. Arbitrary TypeScript callbacks also prevent a simple extractor from proving exact semantic reachability. Use a conservative dependency manifest, then audit exclusions. Track relationships such as `(species, ability, item, transformed)` when you want to eliminate impossible combinations.

Specific findings:

| Mechanic | Scope in this checkout |
|---|---|
| Metronome, Assist, Copycat, Nature Power, Sleep Talk, Mimic, Sketch | Absent from initial doubles movepools. Smeargle does not imply arbitrary Sketch: its generated set has fixed moves. |
| Transform / Imposter | Required. Copies moves, stats, boosts, and ability with specific exclusions and PP rules. Does not copy the target’s item. [Transform](/home/aminaliu/src/pokemon-showdown/sim/pokemon.ts:1275) |
| Illusion | Required, including disguised level, party ordering, disclosure, and Tera interactions. [Illusion Level Mod](/home/aminaliu/src/pokemon-showdown/data/rulesets.ts:2923) |
| Commander | Absent from initial ability pools. Tatsugiri is generated with Storm Drain. Exclude after auditing form/ability creation paths; “Tatsugiri exists” is insufficient reason to implement Commander. |
| Doodle / Skill Swap / Role Play / Entrainment | Absent from initial movepools. |
| Trace | Required. Copies eligible current foe abilities and consumes RNG. |
| Instruct | Required. It inserts a prioritized repeat action in the queue in this checkout. [Instruct](/home/aminaliu/src/pokemon-showdown/data/moves.ts:9641) |
| Dancer / Magic Bounce | Required paths for repeating or reflecting reachable moves, without introducing arbitrary new moves. [Dancer processing](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:320) |
| Revival Blessing | Required. A slot condition initiates a request selecting a fainted party member; revival can concern an active-slot occupant. [Move](/home/aminaliu/src/pokemon-showdown/data/moves.ts:15110), [choice handling](/home/aminaliu/src/pokemon-showdown/sim/side.ts:965), [execution](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:2785) |
| Ogerpon / Terapagos | Required. Include Tera forms and resulting abilities, not merely initial abilities. [Terastallization](/home/aminaliu/src/pokemon-showdown/sim/battle-actions.ts:1918) |
| Booster Energy / Protosynthesis / Quark Drive | Required: separate ability, volatile, selected-stat state, environmental activation, and item activation. [Protosynthesis](/home/aminaliu/src/pokemon-showdown/data/abilities.ts:3514) |
| Item transfer/restoration | Trick, Magician, Pickpocket, Recycle, Harvest, and consumption interactions matter. A holder can receive an item it would never generate with. |
| Struggle / recharge / generic conditions | Core-created behavior must be included even when absent from the set JSON. |
| Other form changes | Palafin, Minior, Eiscue, Mimikyu, Morpeko, Cramorant, Meloetta, and freeze-induced Shaymin changes require auditing. |

There is a particularly subtle additional root: **request generation checks hypothetical abilities from species data**, not just the current/generated ability. This supplies trapping uncertainty. You may need particular `FoeMaybeTrapPokemon` hooks from otherwise unused abilities without implementing their entire battle behavior. [Hypothetical ability checks](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:1737)

Pre-generated teams are a good first step. Store **complete teams**, preserving lead positions, move order, gender, shiny, level, IVs/EVs, and Tera type. Resample teams from a refreshed pool; do not assemble teams from independently sampled Pokémon.

Team generation uses a separate generator seed in this checkout. Explicit teams bypass it, so you need not reproduce generator RNG consumption in battle compatibility tests. [getTeam](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:3163)

**5. Differential harness: compare every decision boundary, then trace the first mismatch**

Build this before substantial effect implementation.

Each fixture should contain:

```text
oracle commit + data/schema fingerprints
explicit complete teams
explicit battle seed
independent choice-generator seed
ordered submitted choices
first mismatch boundary and mismatch fingerprint
```

Compare initialization and **every request boundary**, including mid-turn replacement and revival requests:

1. Incremental protocol log.
2. Both side requests.
3. Full RNG state and draw count.
4. Canonical semantic state.
5. Outcome and phase.

Normalize timestamps explicitly. Showdown already provides a normalization precedent. [State.normalizeLog](/home/aminaliu/src/pokemon-showdown/sim/state.ts:157) Its existing dual runner compares normalized logs and serialized states. [Dual runner](/home/aminaliu/src/pokemon-showdown/sim/tools/runner.ts:193)

For logs, retain blank lines, attributes, ordering, and `|split|` private/public variants. Do not sort lines or broadly strip messages to obtain agreement.

For requests, normalize JSON object-key order but preserve array order and absent/null distinctions. Compare moves, PP, targets, disabled flags, trapping uncertainty, Tera availability, forced switches, and wait states.

For state, compare a purpose-built projection:

- Stable Pokémon identity plus current party order.
- HP, stats, boosts, types, moves/PP, items, abilities, forms.
- Ordered conditions, sources, durations, payloads, effectOrder.
- Pending actions, phase, relevant history/counters, RNG.

Do not require raw Showdown JSON equality. Your compact engine deliberately omits or summarizes implementation-only state.

**Drive choices from the oracle.** Enumerate complete side choices, including targets, Tera, switches, passes, and revival targets, then submit identical choices to Rust. Never sample from the intersection of both engines’ legal actions; that conceals missing legality.

Use a separate choice RNG. Sample both sides’ simultaneous choices before committing either side. Keep live transitions out of speculative choice enumeration.

On mismatch, replay with detailed tracing:

```text
boundary/action ordinal
event and hook relationship
collected handlers in original order
resolved sort keys and sorted order
handler state identity and return value
RNG call site, arguments, output, pre/post seed
semantic mutations
```

The first divergent RNG call or handler list is usually more useful than a turn-end HP difference.

Minimize in stages:

- Keep only the prefix through the first divergence.
- Simplify choices while preserving legality and the same failure fingerprint.
- Simplify teams, effects, and relevant numeric state.
- Use oracle snapshots for rapid local shrinking.
- Retain a reproduction from initialization when feasible.

Deleting an arbitrary choice from the middle changes decision alignment. A shrinker must replay and reject invalid candidates.

For coverage gating, gate on the **closure of the team pair**, including generic conditions, copied abilities, forms, rule hooks, and request hooks. Unsupported behavior should fail loudly.

Random legal play is only one campaign. Add directed cases for equal speeds, simultaneous switch-ins, hazard insertion order, ally attacks, protected spread targets, status timing, item consumption, Tera forms, Instruct, and revival. Reuse the relevant Showdown tests as specifications.

Shard by deterministic test ID. Keep a shared immutable regression corpus and deduplicate failures by the first divergent event/callsite/state field. The Showdown oracle will dominate campaign cost; spend expensive traces on failures.

**6. RL throughput: batch the boundary and budget CPU inference**

Start with **PyO3 around a Rust batch object**, releasing the GIL during stepping. Current PyO3 exposes this through `Python::detach`. [PyO3 parallelism](https://pyo3.rs/main/parallelism.html)

The useful API shape is:

```text
reset_into(env_ids, team_ids, seeds, output_buffers)
step_into(env_ids, packed_joint_actions, output_buffers)
observe_into(env_ids, player_ids, observation_buffers)
```

Use preallocated contiguous buffers for observations, action descriptions/masks, rewards, terminal flags, and request kinds. Keep Python objects out of the per-Pokémon/per-action loop.

| Boundary | Recommendation |
|---|---|
| PyO3 | First implementation; batching makes crossing overhead manageable |
| C ABI | Useful if you need other languages or strict ABI control; unlikely to materially improve a properly batched Python interface |
| Shared memory | Add for an established process pipeline or remote inference, after measuring copies/IPC |

Generate observations and masks inside Rust. Use integers for identities and categorical features; avoid giant one-hot arrays across the boundary.

A doubles action mask must express joint constraints: two slots cannot switch to the same reserve, and only one can Tera. Either enumerate legal joint side actions or use an autoregressive policy with the second slot’s mask conditioned on the first choice.

Advance environments to their next decision boundary. Compact ready environments into inference batches. Start around 128–512 environments and measure; thousands of environments are not automatically better.

On the Ryzen 3600:

- Start with six workers on physical cores for engine-only measurements.
- Measure SMT again after the rewrite; the Node result does not prove the Rust result.
- Give each worker a persistent environment shard and scratch storage.
- Discover CCX and SMT placement from cache-sharing and sibling information; do not assume logical CPU numbering.
- Keep worker queues, counters, and mutable shards local; avoid false sharing and unnecessary cross-CCX work stealing.
- Treat the two L3 domains as cache topology, not separate NUMA memory.
- During training, budget actor and learner threads alongside environment workers. Benchmark allocations such as four environment cores plus two model cores.
- Restrict PyTorch/BLAS threads explicitly to avoid nested oversubscription.

The GPU limitation makes CPU policy inference and learning central. A small batched policy may become the bottleneck before the battle engine does. Report environment-only speed and end-to-end decisions/s separately.

Use AoS for individual battle simulation and contiguous batch tensors for observations. SIMD is more promising for observation packing, inference, and homogeneous numeric kernels than for the entire divergent battle pipeline.

For release builds, start with:

```toml
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
```

Use `-C target-cpu=znver2`, or `native` when building on the deployment CPU, and verify supported target names. Benchmark fat LTO afterward. [Rust codegen options](https://doc.rust-lang.org/rustc/codegen-options/index.html)

Apply PGO once the implementation is stable. Train profiles on varied teams, policies, and search workloads, rather than only random players. [Rust PGO workflow](https://doc.rust-lang.org/rustc/profile-guided-optimization.html)

Measure turns/s, decisions/s, clone cost, observation cost, and p95 latency as well as battles/s. Improving policies can lengthen battles and make battles/s misleading.

**7. Parallel-agent work: establish contracts before distributing effects**

The foundation needs coordinated ownership:

1. Frozen source/data exporter and dependency manifest.
2. IDs, numeric semantics, RNG, comparators.
3. State identities, ordered condition storage, typed payload access.
4. Event collection/execution and suppression.
5. Queue, switching, fainting, requests, generic move pipeline.
6. Differential oracle and canonical projection.

Deliver a small complete vertical slice early: ordinary damage, Protect, status, switching, residuals, and a paused turn. Do not wait for a perfect framework before exercising the full lifecycle.

Then distribute work by mechanic families:

| Batch | Natural ownership |
|---|---|
| Ordinary moves | Generated move metadata |
| Status and duration mechanics | Sleep, confusion, rampage, trapping |
| Damage/stat modifiers | Related abilities and items with shared rounding tests |
| Protection/redirection | Protect variants, guards, Follow Me, Rage Powder |
| Weather/terrain | Setters, modifiers, residual behavior, associated abilities |
| Item lifecycle | Consumption, restoration, theft, transfer |
| Forms/Tera | Transform, Illusion, Ogerpon, Terapagos, other form changes |
| Queue/request mechanics | Instruct, Quash, pivots, Revival Blessing |

Per-effect files work well for custom effects. Each should declare hooks, priorities, payload schema, dependencies, source references, and tests.

**Do not make `build.rs` infer semantic metadata by scanning Rust function names or parsing arbitrary source text.** Use explicit machine-readable descriptors or constrained declarations. Directory discovery can generate module/registry membership; metadata should be intentional.

Keep generated registries in `OUT_DIR`, with deterministic generation. Agents should not edit a central registry. Version team binaries and fixtures against ID/schema fingerprints.

Each agent should provide:

- Mechanics changes.
- Oracle-backed focused cases.
- Coverage/dependency updates.
- A short explanation of special ordering and RNG behavior.

Prevent test “fixes” through ownership:

- Effect agents cannot modify the oracle adapter, normalizers, comparator, or existing goldens without separate review.
- Expected results come from the pinned oracle.
- New failures enter the shared regression corpus before fixes.
- Skips require a specific unsupported dependency.
- Broad exception lists are rejected.
- Framework fixes and effect fixes are reviewed separately.

A low-conflict file structure does not solve shared semantic mistakes. Central ownership of ordering and the oracle matters more than worktree count.

**8. Biggest risks and the highest-value less-obvious wins**

**Hidden-information leakage is the largest transfer risk.** Random Doubles has no Team Preview. A policy trained on the full simulator state sees the opponent’s unrevealed team, moves, ability, item, and Tera type. It will exploit that.

Maintain a player-visible state derived from public events plus that player’s private request. Preserve uncertainty in trapping and move availability. An action mask must not reveal information unavailable to the real client. Showdown explicitly hides some disabled states in [getMoves](/home/aminaliu/src/pokemon-showdown/sim/pokemon.ts:1025).

An asymmetric privileged critic can be intentional; a privileged actor is a different task.

**Do not let search know the real RNG state or hidden opponent state.** Exact RNG is a debugging facility. Planning against a known future seed lets search predict crits, misses, and tie outcomes. Inference-time search should sample plausible hidden states and independent future randomness. Simultaneous opposing choices also require a simultaneous-game treatment; ordinary alternating AlphaZero search assumes an information structure you do not have.

**Integer arithmetic and rounding deserve dedicated tests.** `chainModify` and `modify` deliberately use different rounding offsets. Damage modifier order matters. Port the intermediate truncations, not a mathematically simplified formula. [Modifier arithmetic](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:2310)

**Coverage-gated success can create false confidence.** A million battles among easy effects says little about Revival Blessing plus Transform or hazards plus simultaneous switch-ins. Track interaction coverage and retain directed tests.

**A training turn cap changes the task.** Showdown has a turn limit even without Endless Battle Clause. A shorter rollout limit should be treated as truncation, rather than silently changing terminal outcomes. [Turn limit](/home/aminaliu/src/pokemon-showdown/sim/battle.ts:1840)

The efficiency opportunities I would prioritize are:

- **Compile a small mechanics language for ordinary moves.** Declarative moves plus shared primitives reduce code volume, review effort, and inconsistent implementations.
- **Generate event-specific relevant-holder masks.** Skip holders with no relevant hooks while preserving collection order, timer participants, and RNG-producing sorts. Cache membership only after mutation invalidation is proven.
- **Separate typed public events from text formatting.** Training still needs disclosure and observation updates. Showdown also edits previously emitted move lines, so typed events must support those edits or defer final emission.
- **Store search nodes as parent/action/value records with periodic checkpoints.** Copying 10 KiB for traversal can be cheap; retaining it in every node can dominate memory. At 100,000 nodes, states alone approach 1 GiB.
- **Preserve team-distribution freshness.** Rotate generator-produced training pools and hold out fresh teams. A fixed pool can turn hidden-team inference into memorization.

The architecture decision I would make now is: **one compact Rust engine, exact ordering and RNG, generated ordinary moves, typed custom hooks, and an oracle-owned regression corpus.** The first speed milestone should be a correctness-matched vertical slice—not a fast engine whose missing semantics are still unknown.