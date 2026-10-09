# Numbrion architecture v1

Status: Stage 1 implemented. This is the contract for subsequent stages, not a claim
that battle execution already exists. The only format is `gen9randomdoublesbattle`.
Oracle: `/home/aminaliu/src/pokemon-showdown`, commit
`7332b60e22b9e8194bb53549549eba241d73cc9a`, built `dist/`; never write there.
The TypeScript outranks `docs/showdown/01..04` and `06` when they disagree.

## Invariants and performance target

Given explicit packed teams, a Gen5 seed, and choices, preserve the oracle's draws,
mutation order, results, requests and protocol. No alternate training mechanics.
`BattleState` is `Copy`, with no allocation, strings, pointers or references. Dex and
team definitions are immutable. Snapshot only at decision boundaries, with event
and move scratch idle; queues and requests survive such boundaries. All capacities
and unknown/unimplemented mechanics fail explicitly in every build.

The measured baseline is 18.3 battles/core/s and approximately 3 ms/doubles turn.
The eventual target is at least 915 battles/core/s (50x), measured with identical
teams and choices, excluding generation and text output. Stage 1 makes no
throughput claim. Keep the verified PRNG and fixed-point math modules unchanged.
Build at most six jobs, `target-cpu=native`, on this Zen 2 PC. Optimize measured
paths only after differential parity; every required no-op tie shuffle stays.

## Identities

`EffectId(u16)` has sentinel 0 and contiguous, lexically sorted generated ranges:

| Kind | Half-open range | Records |
|---|---|---:|
| Species | 1..573 | 572 |
| Move | 573..932 | 359 |
| Ability | 932..1139 | 207 |
| Item | 1139..1200 | 61 |
| Condition | 1200..1275 | 75 |
| Rule | 1275..1287 | 12 |

Conditions embedded in moves/abilities are distinct from their parent ids. Preserve
bare, nonexistent condition records such as `bloodmoon`; absence of callbacks is
not absence of state. `EffectType` comes from Dex, not from the numeric kind.
Sleep Clause appears in both the scope condition and rule catalogs; generated
`CONDITION_RULE_ALIASES` canonicalizes its live state to the rule id.

Species inherit base-species condition callbacks (`dex-species.ts:529-536`). This
does not make their conditions interchangeable: `conditions.getByID('zacian')`
has BattleStart, but `conditions.getByID('zaciancrowned')` has none. Generated
`SPECIES_CONDITION_VIEWS` preserves the actual Arceus/Zacian/Zamazenta condition
views without inflating the 75-record scope. `EffectRef` tags these views and bare
move conditions, prefixed item/ability views, and synthetic Format/Confused/
StruggleRecoil/Recharge/Fainted effects. Synthetic effects have no fake dex rows.

`MonId(u8)` is original identity, side*6+original index, 0..11; 255 means none.
`Side.party[6]` and `Pokemon.position` encode Showdown's changing party permutation.
`SlotId(u8)` is side*6+party position, 0..11, with 255 absent: getSlot can produce
letters a..f, including a benched source captured in attackedBy. Active positions
are 0/1. Field position is side+2*position, hence p1a,p2a,p1b,p2b; traversal order is
p1a,p1b,p2a,p2b. Neither position encoding is a stable Pokemon identity.
`Holder(u8)`: Pokemon 0..11, Side 12/13, Field 14, Battle 15, absent 255.
`TypeId(u8)`: 0 absent, 1..19 normal through Stellar, 20 `???`.

## Generated dex

`tools/codegen/gen-dex.mjs` reads the pinned `Dex.forFormat`, verifies `Dex.mod('gen9')`,
and reads `data/scope.json`. It writes four compact Rust files with commit and
SHA256-of-body headers. No timestamps, packages or checkout writes. Generation
order is explicit ASCII id order; nested object property order is retained.

Typed static arrays provide species stats/types/gender ratios/all ability slots,
weight, tags, forme and required-item/ability/move/Tera links; move accuracy (`true`
is distinct from numeric 100), category/target/PP/priority/crit ratio/flags/traits,
ordered boosts, multihit, recoil/drain/heal ratios, damage, switching, self,
selfBoost and secondaries; ability flags/weather suppression; item flags, Fling,
Natural Gift, plate types, users and forced formes. Names are separate immutable
log data. Type chart rows are defending types, columns attacking types, with
Showdown codes 0 neutral, 1 weak, 2 resist, 3 immune. Generate the non-type immunity
keys too (burn, freeze, poison, paralysis, powder, trapping, Prankster, weather).
Generate all 25 natures, including exact plus/minus stat identities.

Integer-keyed `DataValue`/`Property` trees retain every other declarative Dex field,
including arrays, nulls, and ordered sparse objects. They exclude functions and
parent `condition` objects because conditions have their own rows. No JSON parser
runs in the engine. Use typed data on the generic hot path; effect-specific
constants can be read by generated `FieldId`, without string-key lookup. Add a
new typed view to codegen when profiling identifies another common field.

64 species-slot ability names are outside the executable ability closure. They
remain named `AbilitySlot`s with no scoped id. They are hypothetical request
candidates, not real abilities granted by team input. Codegen verifies none has
`onFoeMaybeTrapPokemon`; the actual trapping hypotheses have scoped ids.

127 `EventId`s cover scoped handler keys, direct callbacks, literal events fired
by sim and scoped callbacks, dynamic ModifyAtk/Def/SpA/SpD/Spe, and Side/Field
Residual/SwitchIn. This is deliberately a superset: validator/ignored-mechanic
keys and the never-fired EntryHazard sort key remain for auditability.

`HookId(u16)` indexes 879 manifest entries: 852 function sites, four constants,
23 metadata-only absent callbacks. Each records effect, raw key, nested site,
EventId, HookRel (On/Source/Ally/Foe/Any/Direct), order/priority/subOrder and value.
Keep `secondary` and `secondaries.0` as distinct sites; Dex exposes both, while the
generic pipeline selects `secondaries`. Direct BasePowerCallback, DamageCallback,
DurationCallback and FlingEffect have explicit ids; they do not collect listeners.
`onPlate` is data. `onModifyPriority` is an event; `onModifyPriorityPriority` is
metadata. A duration-only `onSideResidualOrder` is retained even without a callback.
Per-effect manifests add duration, durationCallback presence, EffectType and two
u64 event-mask words. Duration collection must bypass event-mask skipping when a
live state has a truthy duration. `CALLBACKS_SHA256` fingerprints callback bodies
as well as the per-file content hashes and scope fingerprint.

## Exact snapshot layout

`#[repr(C)]` components have compile-time sizes on the supported x86-64 target.
`state/mod.rs`, `effects.rs` and `choices.rs` are authoritative field declarations.

| Component | Count/capacity | Bytes |
|---|---:|---:|
| Pokemon | 12, 540 each | 6,480 |
| Side | 2, 64 each | 128 |
| Field | 1 | 34 |
| EffectArena | 640 cells, 44 each, free head/length | 28,164 |
| ActionQueue | 64 actions, 40 each, length/padding | 2,568 |
| SideRequest | 2, 96 each | 192 |
| Remaining battle fields and alignment | PRNG, faint queue, counters, flags | 98 |
| **BattleState** | **Copy** | **37,664** |

Each Pokemon retains persistent and active data together: base/current species,
base/current ability, item/last item; HP; base/current stored stats; base and virtual
move slots; types/apparent types, boosts, weight/speed; status and four effect-state
cell ids; ordered volatiles; attackedBy summaries; last move/target/switch state;
trapping/request/log flags and battle-long ability/Tera flags. Keep u32 action,
turn and attack counters without saturation. `turn` is u16 and `effect_order` and
attack sequence are checked u32 counters. No wrapping counters.

Ordinary moveSlots read/write the base slots directly: Showdown's shallow copies
alias the same MoveSlot objects (`pokemon.ts:1520`). Transform alone activates four
virtual slots. Clearing Transform restores the base slots, including their PP.
Results use Undefined/Null/False/True; trapping uses No/Yes/Hidden; Tera type 0 is
absent. Move-slot flags retain false/true/hidden disabling, used and presence bits.
Side faint flags retain presence only; Retaliate is outside this scope.

Census maxima in 04 §7 were five volatiles, four side conditions, one slot condition,
two pseudo-weathers, queue seven, attack history 25, HP 645, stat 391 and speed 1790.
These are observations, never hard bounds. Allocate all 45 scoped volatile keys
per Pokemon, all nine side keys, two slot keys per slot, and 14 pseudo-weather keys
(12 rules plus two pseudo-weather conditions). This also permits temporary Baton
Pass/Shed Tail copies on inactive mons. 48 Pokemon singleton states + two field
singletons + format state + 12*45 + 2*9 + 4*2 + 14 = 631 live cells; eight temporary
event states fit below 640. Collected listeners can additionally retire cells, so
640 is a guarded arena budget, not a proof of every transient peak. Record its
high-water mark during Stage 2 fuzzing; overflow must report the fixture.

Queue 64 is a guarded working limit above the observed seven. It includes move,
BeforeTurnMove, PriorityChargeMove, switch/instaswitch, revival, runSwitch, Tera,
start, beforeTurn, residual, pass and event records. No spill allocation. Preserve
both priority and fractionalPriority as f64, order/speed, stable actor/target/
originalTarget ids, targetLoc, sourceEffect, Tera and queued move overlay. Requests
can interrupt a turn, so neither queues nor queued move modifications are scratch.
Faint queue capacity is 12, since faintQueued prevents duplicate entries.

### Effect cells and object lifetime

A 44-byte cell contains generation, effectOrder, a presence word, four payload
words, effect/sourceEffect/linkedStatus ids, a linked-Pokemon bitmask, immutable
physical owner, mutable tagged target, source, sourceSlot, optional duration (-1
absent, zero present), and a free-link/listener-pin-count word. Common presence
bits are reserved; bits 8..30 are per-effect and bit 31 means retired. Payloads
have effect-owned, typed accessors; no general map and no unchecked transmutation.
Four words accommodate ordered seven-stat boosts or a precise f64 multiplier plus
flags/counters. Linked mons are a 12-bit set. The scoped trapped/trapper conditions
have no End callbacks, so their removal order cannot change draws or effectOrder;
the generic helper still calls singleEvent End (`pokemon.ts:2035-2058`). Revisit this
encoding if the linked-condition scope expands.

Singleton states have direct cell ids. Each volatile/side/slot/pseudo-weather
holder has an insertion-ordered list of cell indices. Removal shifts; reinsertion
appends. Restart keeps identity and position. Overwriting a state allocates a new
object identity. `initEffectState` post-increments effectOrder only for a nonempty
id and a Side or active Pokemon target, unless explicitly overridden. Field
states created targetless start at zero; switch-in ability then item allocation
advances the global counter even when no relevant callback exists.

`runEvent` writes state.target to its effect holder before a function callback.
Do not derive it from owner: this changes field/slot ordering and location checks.
`clearEffectState` clears id/order/payload/optional keys and preserves target and
object identity. Species states are retained rather than prematurely dropped.

Collection pins captured `(cell,generation)` identities. Removal detaches the
holder entry and retires a pinned cell; its payload stays mutable until all pins
are released. **runEvent still invokes removed captured effects**, subject only to
its specified suppression/status checks. **fieldEvent checks current location
and identity**, and skips removed/replaced effects. Do not use `is_live` to skip
runEvent callbacks. Only unpinned retired cells re-enter the free list.

### attackedBy

Allocate 12 source-indexed records (the self entry is unused), 24 bytes each.
For each source retain last sequence/damage/slot/move and last numeric
sequence/damage/slot, plus presence and positive-damage-this-turn bits. A numeric
zero is a qualifying damageValue. The latest overall sequence implements
getLastAttackedBy; the latest numeric sequence among eligible sources implements
getLastDamagedBy; Avalanche reads the positive-damage bit for its target source.
`Some(false)` and `Some(true)` both exclude allies: the TS filter tests undefined.

At endTurn, preserve the pre-prune knownType check; then remove every record whose
source is inactive, clear positive bits, and advance the common sequence boundary.
`thisTurn` compares the relevant sequence against that boundary. clearVolatile
clears the records. Do not compress to four/six sources: mid-turn replacements
can introduce all eleven other identities. A 5,000-step full-list comparison
covers mixed numeric/non-numeric attacks, allies, slot changes and pruning.

## Scratch, relay and callback interface

Scratch is owned by a worker, allocated once at construction, never in snapshots.
Stage 2 adds eight EventFrames, eight move frames, eight listener buffers of 2,304
entries, tie-index buffers of the same capacity, eight ordered boost/type/
secondary frames, and sixteen CallArgs frames (event plus direct-callback nesting).
A conservative Pokemon-array collection bound is
4*(10*51 + 4*9 + 16 + 1)+4 = 2,256 entries, including repeated relationship
collections and onEffect. Guard all lengths. Showdown rejects singleEvent/runEvent
entry at eventDepth >= 8; eachEvent/fieldEvent themselves do not increment it.

`Relay` is a 16-byte Copy tag: Undefined, Null, Bool, Number(f64), NotFail (empty
string), Pokemon, Effect, Type, Move, Species, synthetic Recharge and immunity
names, and scratch indices for type lists, ordered boosts, ActiveMove and
secondaries. FAIL=false, SILENT_FAIL=null and HIT_SUBSTITUTE=numeric zero; never
collapse these. Numeric negatives, fractions, NaN and -0 stay representable.
`truthy` and result-combination follow JS, not Option<bool>. Scratch-object values
are truthy even if their collection is empty. Handler Undefined preserves relay.

EventArg covers undefined/null/bools/numbers, holders, effects, type/immunity names,
move-frame indices, slots and static text. Source is not always a Pokemon:
NegateImmunity and Effectiveness pass a type string (`pokemon.ts:2215,2249`).
Each EventFrame stores exact callback args and arity, event target/source/effect,
relay, hasRelay, fastExit and modifier plus modifier presence. runEvent starts
modifier at 4096; singleEvent has no modifier property. `HookCtx` is Copy, with
event/call-argument/move indices, captured state, holder, target/source/sourceEffect, relay and
array-target index (255 absent). Nested calls restore current event/effect/state.

The uniform effect entry signature is:

```rust
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay;
```

A handler accesses live state through b and cx.state, ordered argument objects
through b.scratch and the call-argument index, and current modifier through chain_modify.
No handler borrows a frame across a nested event. Stage 2 supplies helpers for
state access, argument assertions, changes, chaining and direct callback invocation.
Direct callbacks and rule Begin do not create a singleEvent/runEvent context or
increment eventDepth. Their argument frame is separate, and the current
event/effect/effectState remain the caller's; event-frame index 255 means the
initial empty context. Direct invocation must not invent a modifier or effect owner.

ActiveMove is a per-use scratch object with id/sourceEffect, flags/runtime flags,
bp/accuracy/type/category/target/priority/crit/multihit, recoil/drain/damage,
ordered effects and self/secondary edits, hit/lastHit/totalDamage, ordered hitTargets,
and slot-keyed hitData. Four secondary entries and four hit targets cover the
closed scope; hitData uses 12 slot keys. Immutable data remains in Dex; chance,
boost and effect edits are local to the use. Passing an existing move-frame id
reuses it (`dex.ts:316-322`); passing a base id allocates a fresh frame. Magic Bounce
uses a fresh base-id move, while Dancer calls runMove with externalMove. Instruct
queues a prioritized move (`moves.ts:9663`), rather than recursively executing it.

The global active move/pokemon/target follow explicit setActiveMove/clearActiveMove
calls; pushing/popping a nested move does not automatically restore those globals.
Only events restore their parent context. A queued move stores its pre-run
priority/Prankster/ignoreAbility edits, then initializes that use's scratch frame.
lastMove can retain an id: scoped consumers use id/name or immutable failinstruct/
charge/recharge flags, not mutable type or bp. Snapshots require no surviving
active frame; generic scratch stack storage is a Stage 2 implementation task.

## Event execution and generated dispatch

Port singleEvent, runEvent, priorityEvent, eachEvent and fieldEvent separately,
following `battle.ts:433-578,580-954,1022-1235`. Collection order is authoritative:
status, ordered volatiles, ability, item, base species, slot conditions; target
relationships, source relationships, sides, field, format. Preserve duplicate
entries. Do not pre-filter suppression, fainted or duration-only listeners.

Resolve order/priority/subOrder from the generated key metadata. Zero order is
false (sort as 2^32). Default subOrder: Status/Terrain/Pokemon 0; Condition 2, or
3 for a Side target with slot flag, 4 for other Side targets, 5 for Field targets;
Weather/Rule/Format 5; Poison Touch 6; other abilities 7; items 8. Explicit zero
subOrder falls back, exactly as TS. Callback names ending SwitchIn or RedirectTarget
add effectOrder. Capture speed, including Magic Bounce's raw stored speed and
SwitchIn's speed-order quarter-unit subtraction (signed speed*4 minus rank).

Port selection sort and tie shuffle literally. The stable left-to-right family is
Invulnerability/TryHit/DamagingHit/EntryHazard; priorityEvent uses stable redirect
ordering and returns on the first defined result. Recheck each dispatcher's
suppression rules separately. Apply numeric nonnegative-integer relay modifiers
at the runEvent epilogue. Array targets keep independent relays; only the prescribed
falsy/fastExit updates propagate. fieldEvent has distinct Side/Field hook names,
location checks, duration expiry before callback, and its exact faint-message sites.

Generate static dispatch by effect then HookId with generic direct calls. Metadata
and constants are always supplied by the manifest. No effect file sets priorities,
no function-pointer trait object in the initial hot path. Unknown function hooks
panic with effect/hook/site; declarative effects need no file. Later benchmark
per-event partitioning without changing ordering or RNG.

## Effect-file contract for fan-out

One file per behavioural effect under `effects/{moves,abilities,items,conditions,
rules,species}/<canonical-id>.rs`. File authors edit only that file and its local
tests. Stage 2 build.rs discovers files in lexical directory order and generates
module declarations and the outer dispatcher; no author edits a shared registry.
Each file exports:

```rust
pub const ID: EffectId = dex::MOVE_DIRECLAW;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_DIRECLAW_SECONDARY_ONHIT,
    dex::HOOK_MOVE_DIRECLAW_SECONDARIES_0_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_DIRECLAW_SECONDARY_ONHIT |
        dex::HOOK_MOVE_DIRECLAW_SECONDARIES_0_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Dire Claw hook"),
    }
}
```

Payload accessors and their word/bit documentation live in the file, with a
compile-time <=4-word assertion. Return Relay::Undefined for JS return-without-
value; call shared mutators for status/damage/forme changes and the exact PRNG
helper at the exact source point. Preserve sparse boost order. Generic callbacks
must work for every L. No formatting, allocation, external writes or independent
random generator in a handler. Tests compare declared HookIds to manifest function
sites; constants and absent callbacks are handled centrally. Rule aliases and
base-species inherited callbacks can delegate in their own files. Validator-only
and ignored-mechanic sites require explicit machine-readable waivers with reasons;
missing reached callbacks are never waived implicitly.

## Construction, choices, requests and logs

`TeamDef::unpack` implements the packed delimiter grammar, ability slot shorthand,
EV/IV defaults/clamping, neutral/known natures, gender, shiny, level, happiness and
misc fields. Accept one through six mons and one through four scoped moves;
reject empty teams, unknown/out-of-scope ids and oversized teams/move lists.
JSON-array teams and automatic team generation are outside this explicit-packed
API. Immutable TeamDefs are owned separately, exposed read-only; construction
resolves gender once before they are frozen. Names and protocol details stay there
or are formatted at output boundaries. Oversized computed stats fail, not truncate.

`Battle::new(seed,p1Packed,p2Packed)` builds the constructor/pre-start state. Install
only Sleep Clause as targetless pseudo-weather, initialize side p1 then p2, and
sample M/F for each missing/invalid gender only when the species has no fixed
gender. This is a uniform sample, not the species ratio. N becomes empty protocol
gender. Retain PP boosts, stored stats, species types/weight, details exceptions
Greninja-Bond/Rockruff-Dusk, and zero effectOrder/no active mons/no request.
`setPlayer` ordinarily auto-starts on its second call (`battle.ts:3256`); the fixture
script suspends only that instance's start method to observe this boundary.

SideChoice stores two slot choices, chosen switch mask, forced switches/passes,
Tera and cantUndo. Request structs retain field-presence/hidden-disable bits,
move id/PP/maxPP/target/disable source, trapping hypotheses, Tera, forceSwitch,
wait/update/noCancel. Team/condition/details/stat fields are derived at request
emission from state plus TeamDefs. No team-preview/Z/Mega/Dynamax action API.
Stage 3 ports text choice validation and default choice, joint switch/Tera rules,
request generation and mid-turn queue continuation before adding policy action masks.

`Battle<L=NoLog>` uses `LogSink::ENABLED`. Call sites gate emit/edit with that
constant; NoLog never formats. The trait receives state, TeamDefs and typed
LogEntry/LogArg, and supports MoveLineEdit. Stage 4 implements TextLog owning its
protocol buffer, split secret/public pairs and mutable last move line. Constructor
logs, HP percentages, Illusion details, and time-line normalization belong to that
sink/difftest boundary, not alternate mechanics.

## Spec corrections and boundaries

- 01 §12's instruction to implement every Gen 9 handler for arbitrary legal teams
  contradicts this single generated-team scope. Its RD column is explicitly
  approximate and excludes lockedmove, while the closed catalog conservatively
  includes it as an engine source effect. Use scope.json plus actual creation sites;
  catalog membership alone does not prove a volatile can be installed.
- 01 §10 item 14 omits RedirectTarget and Dancer when summarizing effectOrder's
  ordering uses. `battle.ts:998-1004,417-423` and `battle-actions.ts:331-332` include
  them; the spec's earlier sections correctly describe them.
- No additional factual error was established in 02/04's construction/stat formulas.
  04's census sizes and suggested packing are estimates, not a safe allocation
  specification. Its ~3.5 KiB proposal excludes alignment and generic arena storage.

No implementation of start/turns/move damage/text logs or full differential battle
replay is claimed at this stage. Stages 2/3/4/5 implement event registry, lifecycle,
move pipeline, and battle difftest before distributing effects. Capacity high-water
checks and snapshot-copy benchmarks decide later state compaction, including a
possible four-active-block split, under regression coverage.
