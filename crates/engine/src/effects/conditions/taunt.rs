//! Ports data/moves.ts:18985-19016 (embedded condition of Taunt): onStart, onEnd, onDisableMove, onBeforeMove.
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none. `onResidual` (order 15) is ordering metadata
//! only: the 3-turn countdown is the generic residual machinery.
//! Me First (`move.id !== 'mefirst'`) is outside the scoped dex, so every Status move is affected.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        registry::conditions_choicelock::support,
        support::{mon_arg, move_arg, move_overlay},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::CONDITION_TAUNT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_TAUNT_ONSTART,
    dex::HOOK_CONDITION_TAUNT_ONEND,
    dex::HOOK_CONDITION_TAUNT_ONDISABLEMOVE,
    dex::HOOK_CONDITION_TAUNT_ONBEFOREMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:18988-18993 onStart(target). PRNG: none.
        // A Pokemon that already spent a turn on the field (`activeTurns`) and has no move queued yet gets one
        // extra turn (`effectState.duration++`, 3 -> 4); then `|-start|target|move: Taunt`. Returns undefined.
        dex::HOOK_CONDITION_TAUNT_ONSTART => {
            let target = mon_arg(b, cx, 0);
            if b.state.pokemon[target.0 as usize].active_turns != 0
                && b.queue_will_move(target).is_none()
            {
                b.hook_state_mut(cx).duration += 1;
            }
            b.add(LogEntry::new(
                "-start",
                &[LogArg::Mon(target), LogArg::Text("move: Taunt")],
                &[],
            ));
            Relay::Undefined
        }
        // data/moves.ts:18995-18997 onEnd(target): `|-end|target|move: Taunt`. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_TAUNT_ONEND => {
            let target = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(target), LogArg::Text("move: Taunt")],
                &[],
            ));
            Relay::Undefined
        }
        // data/moves.ts:18998-19005 onDisableMove(pokemon): `pokemon.disableMove(id)` (not hidden, source
        // effect = this condition) for every Status-category slot. PRNG: none.
        dex::HOOK_CONDITION_TAUNT_ONDISABLEMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            support::disable_matching(b, pokemon, |_, id| {
                dex::move_data(id).category == dex::Category::Status
            });
            Relay::Undefined
        }
        // data/moves.ts:19006-19012 onBeforeMove(attacker, defender, move) (priority 5). PRNG: none.
        // `!(move.isZ && move.isZOrMaxPowered)` is always true here. A Status move is blocked with
        // `|cant|attacker|move: Taunt|<move>` and `return false`. Category is the ActiveMove overlay's.
        dex::HOOK_CONDITION_TAUNT_ONBEFOREMOVE => {
            let attacker = mon_arg(b, cx, 0);
            let mv = move_arg(b, cx, 2);
            if move_overlay(b, mv).category == dex::Category::Status {
                b.add(LogEntry::new(
                    "cant",
                    &[
                        LogArg::Mon(attacker),
                        LogArg::Text("move: Taunt"),
                        LogArg::Effect(EffectRef::ActiveMove(mv)),
                    ],
                    &[],
                ));
                return Relay::FAIL;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Taunt condition function site"),
    }
}
