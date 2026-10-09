# Stage 2A implementation plan

The API skeleton compiles; battle execution is still `todo!()`. The pinned oracle is
Showdown `7332b60e22b9e8194bb53549549eba241d73cc9a`. Source references and draw notes
are attached to each port in the Rust files. Stage 1 construction, dex generation,
PRNG and math remain implemented and tested. Read ARCHITECTURE.md and DIFFTEST.md
alongside this document; TypeScript resolves disagreements.

## Ownership and worktrees

| Owner | Exclusive write paths | Deliverable |
|---|---|---|
| Lead, 2B | `src/event/`, `src/effects/{mod.rs,contract.rs,registry.rs}`, `build.rs`, shared interfaces | Events, ordering, suppression, registry, effect authoring support |
| L | `src/sim/lifecycle/` | Queue, start/actions/turns, switching, faint/end, Tera |
| C | `src/sim/choices/` | Text/typed choices, errors, cached requests/JSON, legal actions |
| M | `src/actions/moves/` | ActiveMove, targeting, move callers/hit stages/effects, Dancer |
| D | `src/actions/damage/`, `src/actions/mutators/` | Damage/stats/immunity and all core mutation/query helpers |
| T | `src/log/{text.rs,format.rs,battle.rs}` | Exact protocol formatting, splits, edits, drain/flush |
| Lead integration | `src/battle.rs`, `src/lib.rs`, `src/ids.rs`, `src/state/`, `src/actions/{mod.rs,types.rs}`, `src/sim/mod.rs`, `src/log/mod.rs`, shared docs/tests | Frozen interfaces and integration fixes |
| Effect batch owner | Only assigned effect files, per EFFECT-BATCHES.json | Generic functions, local payload accessors, inline tests |
| Harness owner | Separate `crates/difftest` workspace | Adapter and replay harness; excluded from these agents' write scope |

Paths above are relative to `crates/engine/`. Every child declaration is already
present. Owners can add private files under their directories and change their
local declarations. They do not edit shared parents, generated dex, another
owner's files, or the pinned checkout. Tests should be inline or under the owner's
dir, rather than adding competing shared integration-test files.

Create one branch/worktree per core owner from the same reviewed Stage 2A base;
record that base SHA in each task. This stage creates the base files, not commits
or worktrees. Do not branch before the base is saved by the integrator. Integrate
owner branches one at a time into an integration worktree, compile after each,
then replay there. Do not run competing builds against a shared target directory.
The PC's build budget is **six concurrent Rust jobs total** (for example three
worktrees at `-j 2`); each command is also capped at six. No network or new engine
dependencies. Preserve other agents' protected docs/tools/harness edits.

A signature change goes to the owning module and lead before implementation:
send the existing signature, proposed signature, pinned source evidence and
known callers. The lead updates the shared base and notifies every affected owner;
owners rebase that small interface change before integration. Additional private
helpers need no coordination. Never compensate for an unavailable dependency with
a silent no-op or a different random draw. Reached missing behavior panics with its
owner/function; difftest catches it per battle.

## Dependency order

All five core owners can start immediately against the compiling stubs. The lead
implements event/registry in 2B while they implement their directories. Tests can
use the working constructor/state/dex without invoking unfinished methods.

1. Lead: exact comparators/selection sort, scratch frames/pins, event collectors,
   constants/direct calls, suppression and modifier epilogues, registry dispatch.
   T can finish formatting independently; C can finish boundary JSON/parsing.
2. D: raw HP/PP/boost/type helpers, cell installation/removal, stats/immunity and
   damage. Event dispatch is the only shared mechanics dependency. M and L can
   implement control flow against these signatures before D lands.
3. M: ActiveMove initialization, targets, outside/inside callers and ordered hit
   stages; calls D mutators, event dispatch, L faint/switch queries, and T logging.
4. L: queue/start/turn/switch/faint/residual; calls M move execution, D mutations,
   C request construction and choice conversion, lead sorting/events and T flush.
   C calls L's synchronous commit when the final required choice is accepted.
5. Merge the first effects and run constrained vertical-slice fixtures. Widen
   effect coverage batch by batch; then run sample-50 and the complete fuzz corpus.

Cycles are method calls on `Battle<L>`, not ownership cycles. No module needs to
edit a caller's files to fill in its body. Owners must land complete behavior for
their paths before whole-engine acceptance; local PRs can retain explicitly
unreached stubs while dependencies are being implemented.

## Frozen contracts

The code is the exhaustive signature list. These are the cross-owner rules.

### Battle and adapter

`Battle<L: LogSink = NoLog>` owns the Copy state, immutable `TeamDefs`, player names,
a sink and worker scratch. Names/teams/scratch are outside snapshots. `new` and
`with_log` still stop before start. `from_players(seed, (name, packed), (name,
packed), sink)` is the named pre-start constructor. `set_names` is valid only in
Created phase. The difftest adapter can use:

```rust
let mut battle = Battle::from_players(seed, p1, p2, TextLog::default())?;
battle.start()?;                          // synchronous to first request, turn 1
battle.seed();                           // [u16; 4]
battle.turn();                           // u32; adapter returns Some(...)
battle.drain_log(&mut entries);           // one raw log entry per String
battle.request_json(side);               // usize 0/1 -> Option<String>
battle.choose(side, text);               // Result<(), ChoiceError>
battle.outcome();                        // Option<sim::Outcome>
```

Map TeamError/BattleError to String at adapter start. Map
`ChoiceError { text, resent_request_json }` to harness Rejection; map Outcome's
winner/tie/turns/pokemon_left directly. Adapter edits belong to its harness owner,
not to the engine skeleton task. `start`, text logs, requests and choices currently
panic deliberately; the sample-50 replay is not expected to pass this stage.

Accepted noncommitting choices and every rejection leave PRNG/log untouched.
Rejections may reveal hidden disabled/trapped information and mutate the cached
request. Errors/resends are side updates, never battle log entries. Both sides
have requests at every live boundary, including `{wait:true,side:...}`; only an
ended battle has no requests. Outcome ties have winner None.

### Events and effects

`event` reexports Relay/EventArg/EffectRef/EventFrame/HookCtx from the existing
scratch types, with lead ownership. `HookCtx` is copied, never borrowed across a
nested event. Every effect has the single generic entry point
`dispatch<L: LogSink>(HookId, &mut Battle<L>, HookCtx) -> Relay`.

- `single_event(event,effect,state:Option<CellRef>,target,source,source_effect,
  relay,custom_hook)` returns Relay. Undefined defaults; explicit null does not.
- `run_event(event,target:EventArg,source,source_effect,relay,RunEventOptions)`
  returns Relay. Undefined/null default and omit the relay argument.
- `run_event_spread` accepts EventTargets and TargetResults; preserves separate
  per-target relays and array-target order. `on_effect` cannot inject a handler on
  an array invocation, matching the oracle error path.
- `priority_event(...,on_effect:bool)` uses stable redirect ordering and fastExit;
  `each_event` and `field_event` retain their distinct collection/expiry behavior.
- `call_hook(HookId,CallArgs)` makes a **direct** call. It preserves the parent
  event/effect/state and does not increment eventDepth. Duration, base-power,
  damage, before-turn/priority-charge and rule callbacks use this path where TS
  calls `.call/.apply`, rather than inventing a singleEvent context.
- `event_arg`, `event_frame`, `current_hook_context`, `hook_state[_mut]` give
  effects typed access. Explicit callback relay arguments use EventArg::Relay.
  `stash_*`, `scratch_*`, `release_relay` own temporary boosts/types/secondaries.
- `chain_modify(numerator,denominator)`, `modify(value,...)`, `final_modify` use
  the current frame and verified fixed-point primitives. Do not cache a frame
  reference while calling another Battle method.

Relay preserves Undefined, Null, Bool(false), Number(0), NotFail (`''`) and object
identities; HIT_SUBSTITUTE is numeric zero. No coercing returned relays to bool
before the source does so. EffectRef includes a live ActiveMove handle, preserving
runtime source metadata. It must not survive its move frame. `EffectToken(u16)`
keeps dex/condition views and synthetic attribution in the faint queue without
putting scratch handles into a snapshot; D projects live moves when enqueuing.

`event::speed_sort(&mut Prng, &mut [T: SpeedSortable], SortOrder)` is the shared
literal selection sort/tie shuffle for L and event dispatch. JS comparator signs,
falsy zero order and quarter speeds are retained. Redirect and left-to-right
families use their source-prescribed stable sort. L includes fractional priority in Action.priority before sorting
(battle.ts:2651); the sort must not add it a second time. Queue insertion performs its own single inclusive tie-roll;
it does not sort the old queue. Collection pins each CellRef: runEvent executes a
removed captured listener; fieldEvent checks its current location before calling.

Scratch allocates fixed-capacity buffers once, then uses eight event frames,
eight move frames, sixteen direct-call argument slots and eight 2,304-listener
buffers. Capacity/depth overflow fails loudly. Active globals follow explicit
set/clear calls; popping a nested move does not restore them automatically.
Snapshots are legal only after the matching input flush with no live frames.

### Queue, choices and training actions

L's `ActionChoice` contains action kind, mon/target, resolved move ID/kind,
optional targetLoc, EffectRef source, Tera and optional EventId.
`ResolvedActions` holds four expanded actions. `QueueHandle` is an **ephemeral
index** invalidated by every queue mutation; copy the action before a callback.
Prioritization accepts the copied action plus optional existing handle.
`ResidualSnapshot` retains original HP for all four active mons.

In Copy state, SlotChoice retains the resolved move and ChosenMoveKind (Dex or
synthetic Recharge), rather than trying to reconstruct a locked/Struggle move from
a slot alone. Slot 255 means no original PP slot. QueuedMove preserves that kind
plus pre-run priority/flags. Action keeps optional EventId and targetLoc presence.
Tera execution/eligibility belongs to **L**, using D's type/forme mutators.
Before-turn action wrappers in L delegate to M's direct callback APIs.

C owns exact parser/check order, legacy modifier tokens and rejection strings even
for disabled mechanics, partial choices/auto-pass, hidden-information patches and
JSON escaping. `get_requests` / `get_move_request_data` may mutate hypotheses;
`request_json(&self)` serializes cached requests and never rebuilds them. Presence
bits distinguish full/locked/Struggle/Recharge shapes, omitted fields, hidden
trapping and appended disabledSource/update keys. JSON key order follows TS
insertion order, including stats order and side.party order after swaps.

`legal_actions(&self,SideId) -> LegalActions` is authoritative **accepted-action**
legality for training. Per-slot arrays include move ID/kind/slot, target mask,
Tera, switch/revival masks and pass. Target-mask bits are -2,-1,0,+1,+2. Joint
constraints enforce distinct incoming mons, one Tera and forced replacement/pass
limits. Enumeration never runs mutating hooks or changes requests/PRNG. Public
requests can advertise hidden-disabled/trapped options which the validator rejects;
the authoritative training API excludes those options. `choose_typed` uses the
same ordered validation as text, and commits synchronously through L.

### Moves, damage and mutation

M receives `MoveInput::{Dex,Active,Recharge}`; Active reuses the existing mutable
object, not an automatic clone. `get_active_move`, `copy_active_move`,
`active_move[_mut]`, `release_active_move` own frame lifetimes. Numeric damage
sources use `DamageInput::BasePower(f64)` (D). `run_move` takes optional targetLoc;
`use_move[_inner]` takes EventArg so undefined/null/Pokemon targets remain distinct.
MoveTarget helpers return separate hit/Pressure target lists.

`Targets { entries:[HitTarget;4],len }` preserves Pokemon/False/Null; Null continues
self effects after Substitute, False stops that target. `TargetResults` keeps one
Relay per index. Do not compress either array before the source filters it.
Hit stages' `Option<TargetResults>` distinguishes an undefined whole-array no-op
from per-target failures. `HitEffect` distinguishes primary/self/selfBoost/each
secondary/secondary-self/custom payload without creating a new move frame.

ActiveMove overlays copy **all** mutable typed properties on initialization.
Explicit zero/None/empty edits delete fields; they never fall back to static data.
`MoveEffectsScratch.base` supplies immutable hook identities, with a local
suppressed-hooks mask for deleted callbacks (Curse). It also retains mutable
self chance, heal, force/self-switch, boosts and condition IDs. Runtime bits
preserve absent/ranged/fixed multihit, absent/false smartTarget and willCrit,
mutable multiaccuracy, immunity, bounce, suppression and per-use bookkeeping.
Type-changer and four Ruin holder identities are typed scratch fields.

D's `Attribution { source:EventArg,effect:EffectRef }` distinguishes default
undefined and explicit null. Each mutator applies its own TS `||=`/`??=` default
order using the active event frame; do not normalize attribution globally.
Damage amounts/modifiers are f64 until TS truncates them. `get_damage`, damage,
heal/raw_heal, boost and set/trySetStatus retain Relay sentinels. Item/ability and
species return values likewise retain object/false/null rather than Option IDs.
`set_species(...,is_transform)` preserves base stats correctly.

D can reuse the verified crate-visible battle::compute_stats for constructor/form
stat calculation. D owns condition installation/restart/rollback/retirement, duration direct calls,
item/ability lifecycle, PP/disabled slots, type/form/Transform helpers and faint
queueing. L owns faint processing/checkWin and actual random drags. M marks
forceSwitch, executes drain/crash/recoil and invokes D for HP mutation. Confusion,
Struggle recoil, Recharge and faint synthetic names remain distinct from dex
conditions with similar names.

### Structural logs

`LogSink` is generic with const ENABLED. T implements TextLog; NoLog emits/edits
nothing and never formats. Callers send `LogEntry { command:&'static str,args,
tags,split_side,secret_only }` via `Battle::add/add_move/attr_last_move`.
LogArg covers idents, sides/player names, health/details, effect name/fullname,
types/status, numbers, blank fields, spread targets and Parts slices for
concatenation inside an argument (dynamic hints need no caller formatting).
LogTag expresses ordered
[from]/[of]/bare/value tags. No caller assembles protocol lines or suffix strings.

`LogView` gives T read-only state, teams, names and live move overlays. Explicit
split entries create exactly three raw entries, including an empty public line
for a secret-only message. Health/FullDetails choose the secret/shared view.
TextLog retains history and mutable last-move index; draining advances its cursor,
not deleting entries that attrLastMove might edit. `drain_log` does not replace
sendUpdates; T's `send_updates` resets the **logical** unsent count even with NoLog.
The pre-start count is four; `emit_opening_log` realizes those buffered lines once
and avoids counting them twice. Review the concurrently authored read-only
05-protocol.md and 05-log-shapes.txt when implementing T.

## Owner acceptance

| Owner | Focused checks before integration |
|---|---|
| Lead 2B | Oracle parity for scalar/array/default relay and constants; all prefix collection orders; durations without callbacks; removed/restarted pinned cells; suppression exceptions; parent restoration/direct calls; limit paths; literal tie-sort seeds including singleton samples |
| L | Queue expansion/order/seeds and inclusive insert ties; saved midturn queue; start through turn 1; double KO/result; forced/Revival switches; residual captures; 1000-turn guard; Tera and initial switch ordering |
| C | Exact ordered errors for malformed/legacy/partial choices; auto-pass, duplicate switches/Tera and hidden updates; JSON bytes/property order; legal/text acceptance consistency; no draws/log before commit or on rejection |
| M | Outside versus inside calls and move identity; undefined/null targets; Pressure/redirection; all hit gates; Substitute sentinels; multihit/Loaded Dice/Skill Link/Dragon Darts; unconditional secondary/self chance rolls; nested Magic Bounce/Sleep Talk/Instruct/Dancer |
| D | Fixed-point modifier order/overflow and damage/crit seeds; type/status immunity; sparse boost sentinel/order; condition restart/rollback/expiry; retired cells; item/ability veto/defaulting; Transform PP aliasing/base stats; faint synthetic attribution |
| T | Every applicable catalog line shape byte-exact; opening/end/blank lines; split secret/public HP and details; Illusion idents; empty args; ordered tags; all move edits; names; cursor drain and logical count/flush |
| Each effect batch | Every manifest function site declared or explicitly waived; local payload <=4 words; targeted successful/failed/restarted/suppressed/expired cases; before/after seeds and exact logs; no allocation/strings/extra RNG in hooks |

Keep tests meaningful: oracle comparisons for semantic branch points, not copies
of the implementation. Constructor/declarative/parser regressions remain in the
existing Stage 1 suite. Core tests that require another owner are marked as
integration gates in that owner's report and run after dependencies land; they
must not be silently treated as passing. No full-engine performance claim until
parity. After parity, benchmark NoLog single-core versus the pinned baseline and
record capacities/high-water marks; retain every required draw while optimizing.

End-to-end gate, run by the integration/harness owner without changing fixtures:

```
export PATH=$HOME/.cargo/bin:$PATH
cargo build -j 6
cargo test -j 6
cd crates/difftest
cargo build --release -j 6
./target/release/difftest replay ../../data/fixtures/sample-50.jsonl --strict-order
./target/release/difftest replay ../../fuzz-out/fuzz-2000.jsonl --strict-order
```

Use the current documented fuzz path if the harness owner supplies another corpus;
include accepted and rejected fuzz choices. Every boundary must match log, turn,
PRNG, cached requests and errors/resends; end seed/outcome/no requests must match.
Zero panics/divergences/order warnings is the completion gate. Whole corpus replay
requires the full reachable effect set, not just the slice below.

## First effects for a vertical slice

Use deliberately constrained packed teams, **no items**, and a scoped ability with
no callbacks (Dancer is inert when the teams have no dance moves). Pick declarative
single-target/spread damage, primary boosts, drain/recoil and common status moves;
do not rely on random generated teams to avoid effects. Build/record corresponding
oracle fixtures through the existing harness/oracle owner.

First files: rule Begin hooks for PotD, Species Clause, HP Percentage Mod, Cancel
Mod, Illusion Level Mod and Sleep Clause Mod, plus Sleep Clause SetStatus; status
conditions brn/par/slp/frz/psn/tox and flinch; Struggle's ModifyMove. Confusion is the
next small addition (required once confuse moves enter the fixture set). Preserve
rule iteration/order, Sleep Clause aliasing, synthetic Struggle recoil and the
sleep/status PRNG points. Validation-only rule sites use explicit waivers, not
empty battle handlers. No behavioral file is needed for an effect with no function
sites. Opening logs, requests/choices, crit/accuracy/secondary rolls, faint/switch
and outcome must already work for this slice.

Next widen with Protect+stall, Leftovers, Sitrus Berry, Life Orb, Choice items plus
choicelock, Intimidate, Pressure, Levitate, Regenerator and Adaptability. This gives
useful turn/status/item/switch coverage quickly. Sample-50/fuzz contains far more
mechanics and remains the final gate.

## Effect fan-out

[EFFECT-BATCHES.json](EFFECT-BATCHES.json) is the exact file assignment inventory.
It uses scope.json hardEffects plus the **generated manifest**, which is the file
contract: 470 effect records carry 879 entries (852 functions, four constants,
23 ordering-only absent callbacks). scope's aggregate fnCount is not a substitute
for inherited/nested generated HookIds. The remaining dex records are declarative
or engine plumbing and need no placeholder effect files.

| Batch | Records | Function sites | Main shared mechanics/hard cases |
|---|---:|---:|---|
| rules_status | 22 | 42 | Rules, sleep/freeze/confusion/flinch, Rest/Yawn/Tri Attack/Dire Claw |
| weather_terrain | 44 | 88 | Weather/terrain setters, sun/snow immunity, Psychic Terrain priority |
| protect_redirection | 27 | 50 | Protect+stall, Wide Guard, Follow Me/Rage Powder, Helping Hand, Storm Drain |
| disable_lock_trap | 19 | 58 | Disable/Encore/Curse, Taunt/Throat Chop, choice lock, trapping |
| charge_priority_multihit | 26 | 41 | Two-turn/recharge/locked moves, Truant, priority, Power Herb/Loaded Dice |
| hazards_screens | 20 | 47 | Trick Room, screens/Tailwind, hazards, cleanup/Court Change |
| passive_offense | 25 | 34 | Numeric offensive/speed/STAB modifiers |
| passive_defense | 60 | 109 | Immunities, suppression/Mold Breaker, Flash Fire/Unburden links |
| reactive_stats | 27 | 39 | Intimidate/reactive boosts/KO boosts and four Ruin holders |
| reactive_contact | 13 | 13 | Contact damage/status and status reactions |
| healing_residual | 21 | 32 | Natural Cure/Regenerator, drain/healing, residual HP/status |
| items_modifiers | 40 | 73 | Passive item modifiers, take-item guards, Rusted Sword fixed forme |
| consumables | 16 | 42 | Berries, White Herb/policies, eat/use/recycle bookkeeping |
| item_exchange | 10 | 13 | Trick/Knock Off/Magician and item transfer/removal |
| formes_tera | 48 | 85 | Tera/forms, Paradox+Booster Energy, species views |
| identity_abilities | 6 | 16 | Illusion/Transform/Imposter/Trace and identity |
| recursive_switch | 8 | 15 | Magic Bounce/Instruct, Substitute, Baton Pass/Shed Tail/Revival |
| move_callbacks | 38 | 55 | Remaining variable power/type/damage move callbacks |

These are 18 disjoint **file** batches; nested condition records go with their
parent mechanic, and aliases/inherited species sites remain explicit. Each agent
owns its listed canonical files across families and does not edit shared helper
files. The defense/form families are heavier and should go to stronger agents or
be divided into disjoint IDs by the lead after the slice passes; never split a
parent/condition pair between owners. The small reactive/item batches suit cheaper
agents. Review cross-family interactions after integrating both batches.

`hardNoHooks` explicitly flags core work: Dancer and Dragon Darts/Population Bomb
belong to M; Dragon Tail's marking/drag spans M/L; Tera Shell effectiveness belongs
to D. Encore and Trick Room have callbacks on condition records, not on their
zero-hook parent moves. These tasks are part of the relevant core/batch acceptance,
not fake no-op effect implementations. Tera Shell's ordering-only manifest entry
still matters even though no function file is required.

## Remaining boundaries

No new factual spec correction was established during 2A. The state spec's dropped
Gen 9 dead stores were rechecked rather than bulk-added: baseMaxhp aliases maxhp;
lastDamage/moveThisTurn/lastMoveUsed/subFainted/isStarted/duringMove and the scoped
hurtThisTurn consumers do not require snapshot fields. Existing v1 corrections
remain recorded in ARCHITECTURE.md. Source signatures returning mixed sentinels
(boost/heal/SetStatus) still require Relay even when a caller tests only truthiness.

Open integration questions are empirical: listener/frame high-water counts,
registry code size/compile time after fan-out, and whether the final dispatch needs
per-event partitioning to reach 50x. They do not block any owner. The current
registry test audits registered files only; an empty registry does **not** certify
full effect coverage. Full runtime coverage is established by replay/fuzz and the
lead's reached-hook coverage report in 2B.
