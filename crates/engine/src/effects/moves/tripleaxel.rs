//! Ports data/moves.ts:20005-20024 (Triple Axel; `multihit: 3` and `multiaccuracy` are declarative).
//! PRNG: none.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::move_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_TRIPLEAXEL;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_TRIPLEAXEL_BASEPOWERCALLBACK];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:20009-20011 basePowerCallback(pokemon, target, move): `20 * move.hit`.
        // Direct callback, arguments (pokemon, target, move). PRNG: none.
        dex::HOOK_MOVE_TRIPLEAXEL_BASEPOWERCALLBACK => {
            let mv = move_arg(b, cx, 2);
            let hit = b.scratch.moves[mv as usize]
                .as_ref()
                .expect("released active move")
                .hit;
            Relay::Number(20.0 * f64::from(hit))
        }
        _ => panic!("unexpected Triple Axel hook"),
    }
}
