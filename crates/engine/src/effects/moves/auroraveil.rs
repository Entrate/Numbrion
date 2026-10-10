//! Ports data/moves.ts:840-842 (move Aurora Veil onTry). Its side condition is conditions/auroraveil.rs.
//! Payload: none. No PRNG draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_AURORAVEIL;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_AURORAVEIL_ONTRY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:840 onTry(): `return this.field.isWeather(['hail', 'snowscape'])`.
        // Called from singleEvent('Try', move, null, pokemon, targets[0], move) (battle-actions.ts:586);
        // a false result prints `-fail` there. Returns a real boolean (never undefined). Hail has no
        // dex entry in this format (nothing in gen9randomdoublesbattle can set it), so only
        // Snowscape can match. PRNG: none.
        dex::HOOK_MOVE_AURORAVEIL_ONTRY => Relay::Bool(b.is_weather(&[dex::CONDITION_SNOWSCAPE])),
        _ => panic!("unexpected Aurora Veil function site"),
    }
}
