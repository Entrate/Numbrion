//! Ports data/abilities.ts:3718-3730 (Queenly Majesty onFoeTryMove). No PRNG draws.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_QUEENLYMAJESTY;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_QUEENLYMAJESTY_ONFOETRYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3718-3730 onFoeTryMove(target, source, move): identical to Armor Tail
        // except for the message (`cant|holder|ability: Queenly Majesty|move|[of] target`).
        // Args: [mover (event target), the move's target (event source), move]. PRNG: none.
        dex::HOOK_ABILITY_QUEENLYMAJESTY_ONFOETRYMOVE => support::foe_try_move(ID, b, cx),
        _ => panic!("unexpected Queenly Majesty function site"),
    }
}
