//! Yawn (moves:yawn). Ports data/moves.ts:21131-21162; the embedded volatile condition is
//! `conditions:yawn`. `volatileStatus: 'yawn'` is applied by the core move pipeline.
//! Payload: none.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::Status,
};

pub const ID: EffectId = dex::MOVE_YAWN;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_YAWN_ONTRYHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_YAWN_ONTRYHIT => on_try_hit(b, cx),
        _ => panic!("unexpected Yawn function site"),
    }
}

/// data/moves.ts:21141-21145, `onTryHit(target)`:
/// `if (target.status || !target.runStatusImmunity('slp')) return false;`, else undefined.
///
/// PRNG: none directly. `runStatusImmunity('slp')` is `Dex.getImmunity('slp', pokemon)`
/// (a `getTypes()` / Type event; 'slp' has no type-chart entry so it never fails) followed by
/// `runEvent('Immunity', target, null, null, 'slp')` (sim/pokemon.ts:2269-2294). The generated
/// `ImmunityId` enum only covers immunity names that pinned-source callbacks compare against
/// and has no sleep member, so this uses the core's generic condition-id route
/// (`condition_immunity`), which is exactly what `setStatus('slp')` runs for the same check
/// (actions/mutators/status.rs `install_status`). No pinned onImmunity handler tests 'slp'.
fn on_try_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    if b.state.pokemon[target.0 as usize].status != Status::None {
        return Relay::Bool(false);
    }
    if !b.condition_immunity(target, dex::CONDITION_SLP) {
        return Relay::Bool(false);
    }
    Relay::Undefined
}
