//! Ports data/abilities.ts:216-228 (Armor Tail onFoeTryMove). No PRNG draws.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_ARMORTAIL;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_ARMORTAIL_ONFOETRYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:216-228 onFoeTryMove(target, source, move): foeSide moves and `all`
        // moves (outside the perishsong / flowershield / rototiller exceptions) are ignored; a
        // priority move aimed at an ally of the holder (or any `all` move) is stopped:
        // `attrLastMove('[still]')`, `cant|holder|ability: Armor Tail|move|[of] target`, return
        // false. Args: [mover (event target), the move's target (event source), move]. PRNG: none.
        dex::HOOK_ABILITY_ARMORTAIL_ONFOETRYMOVE => support::foe_try_move(ID, b, cx),
        _ => panic!("unexpected Armor Tail function site"),
    }
}
