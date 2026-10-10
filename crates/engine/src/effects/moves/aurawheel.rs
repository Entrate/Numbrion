//! Pinned Showdown 7332b60 effect port. See handler comments for source and draws.
#![allow(unused_imports)]
use crate::effects::registry::abilities_battlebond::support::{
    boosts, effect_at, effect_has_status, effect_is_move, foe, mon_opt, type_named,
};
use crate::{
    Battle,
    actions::{
        Attribution, FormeOptions, ImmunityMessage, ImmunitySource, MoveHandle, Stat, StatOptions,
        Types,
    },
    dex::{self, Category, DataValue, HookId, MoveTarget},
    effects::{
        HookWaiver,
        support::{mon_arg as mon, move_arg, optional_id, relay_number},
    },
    event::{CallArgs, EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{
        mon_flags,
        scratch::{MoveEffectsScratch, move_runtime},
    },
};
pub const ID: EffectId = dex::MOVE_AURAWHEEL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_AURAWHEEL_ONTRY,
    dex::HOOK_MOVE_AURAWHEEL_ONMODIFYTYPE,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:792-800. PRNG: none. Failure retains Null sentinel.
        dex::HOOK_MOVE_AURAWHEEL_ONTRY => {
            let source = mon(b, cx, 0);
            if dex::species(b.state.pokemon[source.0 as usize].species).base_species
                == dex::SPECIES_MORPEKO
            {
                return Relay::Undefined;
            }
            b.attr_last_move(MoveLineEdit::Still);
            b.add(LogEntry::new(
                "-fail",
                &[LogArg::Mon(source), LogArg::Text("move: Aura Wheel")],
                &[],
            ));
            b.hint(
                LogArg::Text(
                    "Only a Pokemon whose form is Morpeko or Morpeko-Hangry can use this move.",
                ),
                false,
                None,
            );
            Relay::Null
        }
        // data/moves.ts:801-807. PRNG: none. Current species, even after Transform.
        dex::HOOK_MOVE_AURAWHEEL_ONMODIFYTYPE => {
            let h = MoveHandle(move_arg(b, cx, 0));
            let m = mon(b, cx, 1);
            b.active_move_mut(h).move_type =
                if b.state.pokemon[m.0 as usize].species == dex::SPECIES_MORPEKOHANGRY {
                    type_named("Dark")
                } else {
                    type_named("Electric")
                };
            Relay::Undefined
        }
        _ => panic!("unexpected aurawheel hook"),
    }
}
