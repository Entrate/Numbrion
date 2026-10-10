//! Ports data/moves.ts:19400-19430 (embedded condition of Throat Chop): onStart, onDisableMove, onBeforeMove,
//! onModifyMove, onEnd.
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none. `onResidual` (order 22) is ordering metadata
//! only: the 2-turn countdown is the generic residual machinery.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        registry::conditions_choicelock::support,
        support::{mon_arg, move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::CONDITION_THROATCHOP;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_THROATCHOP_ONSTART,
    dex::HOOK_CONDITION_THROATCHOP_ONDISABLEMOVE,
    dex::HOOK_CONDITION_THROATCHOP_ONBEFOREMOVE,
    dex::HOOK_CONDITION_THROATCHOP_ONMODIFYMOVE,
    dex::HOOK_CONDITION_THROATCHOP_ONEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// Shared body of onBeforeMove (data/moves.ts:19416) and onModifyMove (:19422): a sound move (the ActiveMove's
/// flags) is blocked with `|cant|pokemon|move: Throat Chop` (no move name) and `return false`.
fn block_sound<L: LogSink>(b: &mut Battle<L>, mv: u8, pokemon: crate::ids::MonId) -> Relay {
    if move_overlay(b, mv).flags & dex::FLAG_SOUND != 0 {
        b.add(LogEntry::new(
            "cant",
            &[LogArg::Mon(pokemon), LogArg::Text("move: Throat Chop")],
            &[],
        ));
        return Relay::FAIL;
    }
    Relay::Undefined
}
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:19403-19405 onStart(target): `|-start|target|Throat Chop|[silent]`. PRNG: none.
        dex::HOOK_CONDITION_THROATCHOP_ONSTART => {
            let target = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-start",
                &[LogArg::Mon(target), LogArg::Text("Throat Chop")],
                &[LogTag::Bare("silent")],
            ));
            Relay::Undefined
        }
        // data/moves.ts:19406-19412 onDisableMove(pokemon): `pokemon.disableMove(id)` (not hidden, source
        // effect = this condition) for every slot whose move has the `sound` flag (dex flags). PRNG: none.
        dex::HOOK_CONDITION_THROATCHOP_ONDISABLEMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            support::disable_matching(b, pokemon, |_, id| {
                dex::move_data(id).flags & dex::FLAG_SOUND != 0
            });
            Relay::Undefined
        }
        // data/moves.ts:19413-19419 onBeforeMove(pokemon, target, move) (priority 6). PRNG: none.
        dex::HOOK_CONDITION_THROATCHOP_ONBEFOREMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let mv = move_arg(b, cx, 2);
            block_sound(b, mv, pokemon)
        }
        // data/moves.ts:19420-19426 onModifyMove(move, pokemon, target): run-event args (move relay, pokemon,
        // target). Same check and message as onBeforeMove. PRNG: none.
        dex::HOOK_CONDITION_THROATCHOP_ONMODIFYMOVE => {
            let mv = move_arg(b, cx, 0);
            let pokemon = mon_arg(b, cx, 1);
            block_sound(b, mv, pokemon)
        }
        // data/moves.ts:19428-19430 onEnd(target): `|-end|target|Throat Chop|[silent]`. PRNG: none.
        dex::HOOK_CONDITION_THROATCHOP_ONEND => {
            let target = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(target), LogArg::Text("Throat Chop")],
                &[LogTag::Bare("silent")],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected Throat Chop condition function site"),
    }
}
