//! Ports data/moves.ts:9426-9433 (Ice Spinner onAfterHit + onAfterSubDamage).
//! Payload: none. No direct PRNG draws; terrain End events retain their own.
use crate::effects::support::mon_arg;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_ICESPINNER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_ICESPINNER_ONAFTERHIT,
    dex::HOOK_MOVE_ICESPINNER_ONAFTERSUBDAMAGE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:9426 onAfterHit(target, source): `this.field.clearTerrain()` (result ignored;
        // FieldEnd logs `-fieldend` when a terrain was active). Returns undefined.
        dex::HOOK_MOVE_ICESPINNER_ONAFTERHIT => {
            b.clear_terrain();
            Relay::Undefined
        }
        // data/moves.ts:9429 onAfterSubDamage(damage, target, source): args [damage, target, source,
        // move]; clears terrain only while `source.hp` is truthy. Returns undefined.
        dex::HOOK_MOVE_ICESPINNER_ONAFTERSUBDAMAGE => {
            let source = mon_arg(b, cx, 2);
            if b.state.pokemon[source.0 as usize].hp != 0 {
                b.clear_terrain();
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Ice Spinner function site"),
    }
}
