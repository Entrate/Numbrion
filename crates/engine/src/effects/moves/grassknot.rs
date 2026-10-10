//! Ports data/moves.ts:7537-7577 (Grass Knot). No PRNG draws; `getWeight` runs a ModifyWeight event.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
#[path = "movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::MOVE_GRASSKNOT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_GRASSKNOT_BASEPOWERCALLBACK,
    dex::HOOK_MOVE_GRASSKNOT_ONTRYHIT,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_GRASSKNOT_BASEPOWERCALLBACK => base_power_callback(b, cx),
        dex::HOOK_MOVE_GRASSKNOT_ONTRYHIT => on_try_hit(b, cx),
        _ => panic!("unexpected Grass Knot function site"),
    }
}

// data/moves.ts:7541-7559 basePowerCallback(pokemon, target): direct call, args
// [pokemon, target, move]. `target.getWeight()` (ModifyWeight event, then max(1, weight)) picks
// 120/100/80/60/40/20 at >=2000/1000/500/250/100 hg. `this.debug` prints nothing. PRNG: none.
fn base_power_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 1);
    let target_weight = b.get_weight(target);
    Relay::Number(support::target_weight_base_power(target_weight))
}

// data/moves.ts:7565-7571 onTryHit(target, source, move): only acts for a target holding the
// 'dynamax' volatile (log `-fail|source|move: Grass Knot|[from] Dynamax`, then null). Evidence
// that the branch is unreachable in gen9randomdoublesbattle: side.ts:264 sets
// `dynamaxUsed = gen !== 8` (true here) so canDynamaxNow() (side.ts:311-319) is false and no
// dynamax choice/volatile can exist, and the generated dex has no Dynamax condition. The handler
// therefore returns `undefined` for every target. PRNG: none.
fn on_try_hit<L: LogSink>(_b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    Relay::Undefined
}
