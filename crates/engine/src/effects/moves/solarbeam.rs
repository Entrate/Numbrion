//! Ports data/moves.ts:17224-17265 (Solar Beam). PRNG: no direct draws; ChargeMove (Power Herb) and
//! the twoturnmove Start run nested core events.
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_SOLARBEAM;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_SOLARBEAM_ONTRYMOVE,
    dex::HOOK_MOVE_SOLARBEAM_ONBASEPOWER,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:17233-17248 onTryMove(attacker, defender, move): removeVolatile(move.id) ->
        // return; -prepare; sunnyday/desolateland (`attacker.effectiveWeather(undefined, true)`)
        // -> [still] + -anim, return; runEvent('ChargeMove') falsy -> return;
        // addVolatile('twoturnmove', defender); return null. Desolate Land is outside the scope, and
        // the `true` message flag only matters for Mega Sol (outside the scope too).
        dex::HOOK_MOVE_SOLARBEAM_ONTRYMOVE => {
            cp::charge_try_move(b, cx, false, Some(dex::CONDITION_SUNNYDAY))
        }
        // data/moves.ts:17249-17255 onBasePower(basePower, pokemon, target): rain, sand and snow
        // (primordialsea/hail are outside the scope) weaken it with chainModify(0.5). PRNG: none.
        dex::HOOK_MOVE_SOLARBEAM_ONBASEPOWER => {
            let pokemon = mon_arg(b, cx, 1);
            let weather = b.effective_weather(pokemon);
            if weather == dex::CONDITION_RAINDANCE
                || weather == dex::CONDITION_SANDSTORM
                || weather == dex::CONDITION_SNOWSCAPE
            {
                b.chain_modify(0.5, 1.0);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Solar Beam hook"),
    }
}
