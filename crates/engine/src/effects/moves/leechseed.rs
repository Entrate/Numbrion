//! Ports data/moves.ts:10229 (Leech Seed `onTryImmunity`). PRNG: none directly; `hasType` runs
//! the Type event, whose listeners may speed-sort ties. The embedded volatile condition is the
//! separate `conditions:leechseed` file.
use crate::effects::registry::abilities_baddreams::support::TYPE_GRASS;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_LEECHSEED;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_LEECHSEED_ONTRYIMMUNITY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_LEECHSEED_ONTRYIMMUNITY => on_try_immunity(b, cx),
        _ => panic!("unexpected leechseed hook"),
    }
}

// data/moves.ts:10229-10231 onTryImmunity(target). PRNG: none directly.
fn on_try_immunity<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    // return !target.hasType('Grass');   (a boolean; false fails the hit, true continues)
    Relay::Bool(!b.has_type(target, &[TYPE_GRASS]))
}
