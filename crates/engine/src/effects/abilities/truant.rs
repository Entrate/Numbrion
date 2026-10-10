//! Ports data/abilities.ts:5183-5203 (Truant; its `condition: {}` is the bare `truant` volatile
//! whose record needs no file). PRNG: none directly.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
    state::ResultFlag,
};
pub const ID: EffectId = dex::ABILITY_TRUANT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_TRUANT_ONSTART,
    dex::HOOK_ABILITY_TRUANT_ONBEFOREMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5184-5189 onStart(pokemon):
        //   pokemon.removeVolatile('truant');
        //   if (pokemon.activeTurns && (pokemon.moveThisTurnResult !== undefined ||
        //       !this.queue.willMove(pokemon))) pokemon.addVolatile('truant');
        // `moveThisTurnResult` is undefined only for ResultFlag::Undefined (null/false/true are
        // all "defined"). The addVolatile call has no source/sourceEffect (event defaults apply).
        // PRNG: none directly.
        dex::HOOK_ABILITY_TRUANT_ONSTART => {
            let pokemon = mon_arg(b, cx, 0);
            b.remove_volatile(pokemon, dex::CONDITION_TRUANT);
            let p = &b.state.pokemon[pokemon.0 as usize];
            let active_turns = p.active_turns != 0;
            let moved = p.move_this_turn_result != ResultFlag::Undefined;
            if active_turns && (moved || b.queue_will_move(pokemon).is_none()) {
                b.add_volatile(pokemon, dex::CONDITION_TRUANT, Attribution::DEFAULT, None);
            }
            Relay::Undefined
        }
        // data/abilities.ts:5190-5197 onBeforeMove(pokemon) (manifest priority 9):
        //   if (pokemon.removeVolatile('truant')) { this.add('cant', pokemon, 'ability: Truant'); return false; }
        //   pokemon.addVolatile('truant');
        // BeforeMove args are (pokemon, target, move). PRNG: none directly.
        dex::HOOK_ABILITY_TRUANT_ONBEFOREMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            if b.remove_volatile(pokemon, dex::CONDITION_TRUANT) {
                b.add(LogEntry::new(
                    "cant",
                    &[LogArg::Mon(pokemon), LogArg::Text("ability: Truant")],
                    &[],
                ));
                return Relay::FAIL;
            }
            b.add_volatile(pokemon, dex::CONDITION_TRUANT, Attribution::DEFAULT, None);
            Relay::Undefined
        }
        _ => panic!("unexpected Truant hook"),
    }
}
