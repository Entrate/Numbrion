//! Ports data/moves.ts:6158-6177 (Freeze-Dry). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{EventArg, HookCtx, Relay},
    ids::*,
    log::LogSink,
};
#[path = "movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::MOVE_FREEZEDRY;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_FREEZEDRY_ONEFFECTIVENESS];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_FREEZEDRY_ONEFFECTIVENESS => on_effectiveness(b, cx),
        _ => panic!("unexpected Freeze-Dry function site"),
    }
}

// data/moves.ts:6167-6169 onEffectiveness(typeMod, target, type): singleEvent('Effectiveness',
// move, null, target, type, move, typeMod) (pokemon.ts:2208-2234), args
// [typeMod, target, type, move]. `if (type === 'Water') return 1;` else undefined (the relay
// keeps the chart value). PRNG: none.
fn on_effectiveness<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match b.event_arg(cx, 2) {
        EventArg::Type(ty) if ty == support::TYPE_WATER => Relay::Number(1.0),
        EventArg::Type(_) => Relay::Undefined,
        _ => panic!("Effectiveness callback requires a type argument"),
    }
}
