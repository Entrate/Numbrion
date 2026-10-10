//! Ports the Roost volatile, data/moves.ts:15441-15458 (the `condition` of the move at
//! 15428-15463; the move itself heals half max HP and sets `self.volatileStatus: 'roost'`).
//! No PRNG draws; `hasType` in `onStart` runs Type events, which may sort ties.
//!
//! Function sites: `onStart` and `onType` (priority -1). `onResidual` is order-only metadata
//! (`onResidualOrder: 25` with no function; the one-turn `duration: 1` expires the volatile).
//!
//! Payload map (`this.effectState.typeWas`, data/moves.ts:15455):
//!   word 0 = the pre-Roost type list packed as `type0 | type1 << 8` (TypeId bytes, NONE = 0);
//!            custom present bit 8 (`present::CUSTOM_START`) marks the property as assigned.
//! The core reads this word in `transform_into` (pokemon.ts:1295: Transform copies
//! `volatiles['roost'].typeWas` instead of the current, Flying-less types), so the layout is
//! part of the contract with actions/mutators/pokemon.rs.
use crate::{
    Battle,
    actions::Types,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink},
    state::present,
};
#[path = "../moves/movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::CONDITION_ROOST;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_ROOST_ONSTART,
    dex::HOOK_CONDITION_ROOST_ONTYPE,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// Custom presence bit for `typeWas` (EffectCell.present bits 8..30 are effect-defined).
const TYPE_WAS_PRESENT: u32 = 1 << present::CUSTOM_START;
const TYPE_FLYING: TypeId = support::type_named("Flying");
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_CONDITION_ROOST_ONSTART => on_start(b, cx),
        dex::HOOK_CONDITION_ROOST_ONTYPE => on_type(b, cx),
        _ => panic!("unexpected Roost function site"),
    }
}

// data/moves.ts:15444-15452 onStart(target): Start single event from addVolatile
// (pokemon.ts:2018), args [target, source, sourceEffect].
//   if (target.terastallized) {
//     if (target.hasType('Flying')) this.add('-hint', "If a Terastallized Pokemon uses Roost, it remains Flying-type.");
//     return false;                       // the volatile is removed again
//   }
//   this.add('-singleturn', target, 'move: Roost');
// PRNG: none directly (hasType runs the Type event).
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    if b.state.pokemon[target.0 as usize].terastallized != TypeId::NONE {
        if b.has_type(target, &[TYPE_FLYING]) {
            b.add(LogEntry::new(
                "-hint",
                &[LogArg::Text(
                    "If a Terastallized Pokemon uses Roost, it remains Flying-type.",
                )],
                &[],
            ));
        }
        return Relay::Bool(false);
    }
    b.add(LogEntry::new(
        "-singleturn",
        &[LogArg::Mon(target), LogArg::Text("move: Roost")],
        &[],
    ));
    Relay::Undefined
}

// data/moves.ts:15453-15457 onType(types, pokemon) at priority -1: runEvent('Type', pokemon, null,
// null, types) from getTypes (pokemon.ts:2142), args [types, pokemon].
//   this.effectState.typeWas = types;                  // the incoming array, snapshotted below
//   return types.filter(type => type !== 'Flying');    // a NEW array: the incoming one is
//                                                      // untouched (getTypes writes it back to
//                                                      // pokemon.types), so stash a new relay slot
// PRNG: none.
fn on_type<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let handle = match b.event_arg(cx, 0) {
        EventArg::Relay(Relay::Types(handle)) => handle,
        other => panic!("Type callback requires a types relay, got {other:?}"),
    };
    let types = *b.scratch_types(handle);
    let second = if types.len > 1 {
        u32::from(types.values[1].0)
    } else {
        0
    };
    let packed = u32::from(types.values[0].0) | second << 8;
    let cell = b.hook_state_mut(cx);
    cell.payload.words[0] = packed;
    cell.present |= TYPE_WAS_PRESENT;
    let mut filtered = Types {
        values: [TypeId::NONE; 3],
        len: 0,
    };
    for &ty in &types.values[..types.len as usize] {
        if ty != TYPE_FLYING {
            filtered.values[filtered.len as usize] = ty;
            filtered.len += 1;
        }
    }
    Relay::Types(b.stash_types(filtered))
}
