# Effect authoring contract (frozen in Stage 2A)

This is the contract for the later parallel effect ports. The lead implements the
event/registry behavior and expands the worked examples in 2B. Do not start a
batch before its required core methods and the first vertical slice work. The
source oracle is commit `7332b60e22b9e8194bb53549549eba241d73cc9a`; generated HookId
metadata and source file:line outrank prose specs.

## Files and exports

One canonical lowercase alphanumeric filename per effect under
`crates/engine/src/effects/{moves,abilities,items,conditions,rules,species}/`.
The exact batch assignments are in EFFECT-BATCHES.json. Parents and embedded
conditions have **distinct** IDs/files, assigned together when they share mechanics.
Files with no function hooks are declarative and require no implementation file.
build.rs discovers direct `.rs` children in sorted family/filename order and emits
module declarations, EffectImpl metadata and generic dispatch calls into OUT_DIR.
Do not edit mod.rs, registry.rs, build.rs, dex or another agent's files.

Every behavioral file exports exactly this shape (the listed constants are real):

```rust
use crate::{Battle, dex, dex::HookId, ids::EffectId,
    effects::HookWaiver, event::{HookCtx, Relay}, log::LogSink};
pub const ID: EffectId = dex::MOVE_DIRECLAW;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_DIRECLAW_SECONDARY_ONHIT,
    dex::HOOK_MOVE_DIRECLAW_SECONDARIES_0_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_DIRECLAW_SECONDARY_ONHIT |
        dex::HOOK_MOVE_DIRECLAW_SECONDARIES_0_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Dire Claw function site"),
    }
}
// This deliberate stub is replaced with the complete source port by the owner.
// Ports data/moves.ts:3640-3643. PRNG: sample one status; status events may draw.
fn on_hit<L: LogSink>(_b: &mut Battle<L>, _cx: HookCtx) -> Relay { todo!() }
```

Use the HookId's effect/key/nested `site` to locate its function in the pinned
TypeScript, and put that file:line in each function comment. `Hook.site` is a
nested object path, not a TypeScript line number. Include direct
callbacks and every nested `secondary`/`secondaries.N`/self function site, even if
TS aliases a function and both Rust match arms delegate to the same local helper.
HOOKS contains function sites only: constant/absent ordering-only entries belong
to the manifest and central dispatcher. Files can contain private helpers and
inline `#[cfg(test)]` tests; auxiliary files go in a nested private subdirectory
so discovery does not mistake them for effects. Never export a trait object or
store a generic function pointer in EffectImpl.

## State, calls and return values

Every handler is generic over LogSink. It receives a copied HookCtx, reads typed
arguments through `b.event_arg(cx,index)` and accesses its exact captured state
through `b.hook_state[_mut](cx)`. Consult the source call site for argument order;
relay is present only when TS passed it. `cx.relay` is the current relay. Direct
callbacks use CallArgs, retain parent event/effect/state and do not gain a new
owner or modifier frame; an initial empty context has frame 255 and no owned
state object.

Drop all references into Battle/frame/move/state before a nested Battle call.
Copy the needed fields, call the method, then reacquire state. Retired listener
state remains valid while pinned; do not check arena.is_live and silently skip it.
Never retain scratch handles in effect payload, Copy snapshot, queue or team data.

Return Relay::Undefined for JS `return;` or falling off the function. Return
Relay::Null for null, Relay::Bool(false) for false, Relay::Number(0.0) for zero,
and Relay::NotFail for the empty-string sentinel. HIT_SUBSTITUTE is numeric zero.
Use `.truthy()` only where TS performs a truthiness check. `return
this.chainModify(...)` mutates the frame and returns undefined: call
`b.chain_modify(num,den)` then return Relay::Undefined. No substituting true for
undefined; no collapsing null/false/zero into a bool.

Use the core mutators rather than copying status/item/HP/volatile algorithms.
Shared entry points live in actions/mutators/, damage/, moves/, sim/lifecycle/
and event/. Attribution preserves source Undefined/Null/defaulting and EffectRef
retains synthetic/condition/live-move identity. Use `MoveHandle` to access an
ActiveMove and release only frames you acquired; Active input aliases the same
object. Mutable properties are complete overlays: setting an ID to NONE or
removing boosts/selfSwitch must not resurrect static dex data. Primary boosts
are `active_move.effects.boosts`; selfBoost is separate. Deleted hook bits index
that MoveEffectsScratch's `base.hooks`, never a global EventId bit.

OrderedBoosts stores sparse values plus insertion order/presence. Preserve TS
Object.keys order and spreads; do not reorder boosts into stat-index order. Relay
Types/Boosts/Secondaries refer to lead-owned scratch slots. Use stash/access/release
helpers, drop their borrows before events, and do not retain the slot afterwards.

## Effect payload

Document a local word/bit map in the file. EffectPayload contains four u32 words;
PAYLOAD_WORDS is the maximum used, asserted <=4. Custom `EffectCell.present` bits
8..30 distinguish missing from false/zero. Common id/source/sourceSlot/target,
sourceEffect/duration/link fields have shared named bits. RETIRED (31) is arena-owned.
Do not repurpose common fields or free/pin bookkeeping. f64 payloads use two words
with to_bits/from_bits; MonId, EffectId and enums use documented compact integers.
No strings, references, Box/Vec or scratch indexes in payload. Universal stat/HP/
boost/form/PP/trapping state goes through core APIs rather than hidden payloads.

Initialization/restart/end must match TS property creation/deletion order. The
mutator creates the cell and invokes Start; durationCallback is a direct call and
can fail/rollback. Restart keeps object identity and insertion position. Cleanup
uses the core removal methods so captured cells remain pinned correctly.

## Draws and output

Use only `b.state.prng`'s verified helpers at the exact source statement. Document
direct draws plus nested event draws beside every handler. Sampling a singleton,
100%/absent chance rolls and tie shuffles still consume draws when TS calls them.
Do not combine repeated rolls or memoize repeated event/stat queries to save draws.
Source constraints only eliminate proven unreachable format branches.

Output uses structural LogEntry/LogArg/LogTag and Battle add/edit methods. Use dex
IDs for names/fullnames, MonId for idents, Health/Details for split formatting and
ordered From/Of tags. LogArg::Parts concatenates a stack slice within one argument,
including dynamic hint numbers/effect names. Static prose is allowed as &'static str. No format!, String,
normalization, external IO or a second RNG in a handler. No protocol lines assembled
by effects; NoLog must compile out formatting. Request/errors are C-owned side
updates, never log entries.

## Waivers and acceptance

`WAIVERS` is explicit coverage metadata with HookId, WaiverReason and explanation.
ValidationOnly covers source team-validator hooks never called by the explicit
packed-team battle API. OutsideFormat requires pinned source evidence of the
unreachable branch/view; the lead reviews it. A waiver never becomes a runtime
no-op: a reached waived site must panic. Rule aliases and inherited crowned
species views must route correctly through canonical dispatch; do not independently
implement an alias with different state or mark a callback unreachable casually.

The manifest test checks every registered file for duplicate IDs/sites, four-word
limit and exactly all function sites accounted for by HOOKS plus WAIVERS. Missing
whole files are discovered by reached-hook replay; passing the registry test alone
is not full coverage. Each batch reports files, source references, payload maps,
waivers, required core APIs, targeted tests/seeds and unresolved integration cases.
Tests cover the mechanic's meaningful successful/failed/suppressed/restart/expiry
branches and exact PRNG/log deltas. Full acceptance is the common sample-50 and
fuzz replay gate in IMPLEMENTATION-PLAN.md. Ask the owning module/lead before
changing any cross-module signature; add private local helpers freely.
