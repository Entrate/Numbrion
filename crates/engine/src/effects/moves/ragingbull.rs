//! Ports data/moves.ts:14642-14660 (Raging Bull onTryHit + onModifyType). Payload: none. No PRNG draws.
use crate::effects::registry::conditions_reflect::support::{
    self, TYPE_FIGHTING, TYPE_FIRE, TYPE_WATER,
};
use crate::{
    Battle,
    actions::MoveHandle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_RAGINGBULL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_RAGINGBULL_ONTRYHIT,
    dex::HOOK_MOVE_RAGINGBULL_ONMODIFYTYPE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:14642 onTryHit(pokemon): identical to Brick Break; arg 0 is the event target.
        // Removes Reflect, Light Screen, Aurora Veil from the target's side. Returns undefined.
        // PRNG: none.
        dex::HOOK_MOVE_RAGINGBULL_ONTRYHIT => support::shatter_screens(b, cx),
        // data/moves.ts:14648 onModifyType(move, pokemon): singleEvent('ModifyType', move, null,
        // pokemon, target, move, move; battle-actions.ts:395,403,430) => args [move(relay), pokemon,
        // target, move]. Switches on pokemon.species.name: Paldea Combat -> Fighting, Blaze -> Fire,
        // Aqua -> Water, anything else leaves the type alone. Returns undefined (the relay stays the
        // move). PRNG: none.
        dex::HOOK_MOVE_RAGINGBULL_ONMODIFYTYPE => {
            let mv = move_arg(b, cx, 0);
            let pokemon = mon_arg(b, cx, 1);
            let species = b.state.pokemon[pokemon.0 as usize].species;
            let new_type = if species == dex::SPECIES_TAUROSPALDEACOMBAT {
                Some(TYPE_FIGHTING)
            } else if species == dex::SPECIES_TAUROSPALDEABLAZE {
                Some(TYPE_FIRE)
            } else if species == dex::SPECIES_TAUROSPALDEAAQUA {
                Some(TYPE_WATER)
            } else {
                None
            };
            if let Some(t) = new_type {
                b.active_move_mut(MoveHandle(mv)).move_type = t;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Raging Bull function site"),
    }
}
