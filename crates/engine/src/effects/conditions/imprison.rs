//! Ports data/moves.ts:9500-9524 (embedded condition of Imprison): onStart, onFoeDisableMove, onFoeBeforeMove.
//! Payload: none (PAYLOAD_WORDS = 0); the imprisoner is the shared `effectState.source`.
//! Direct PRNG draws: none.
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
pub const ID: EffectId = dex::CONDITION_IMPRISON;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_IMPRISON_ONSTART,
    dex::HOOK_CONDITION_IMPRISON_ONFOEDISABLEMOVE,
    dex::HOOK_CONDITION_IMPRISON_ONFOEBEFOREMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:9503-9505 onStart(target): `|-start|target|move: Imprison`. Returns undefined.
        dex::HOOK_CONDITION_IMPRISON_ONSTART => {
            let target = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-start",
                &[LogArg::Mon(target), LogArg::Text("move: Imprison")],
                &[],
            ));
            Relay::Undefined
        }
        // data/moves.ts:9506-9512 onFoeDisableMove(pokemon) (the imprisoner's foes): every non-Struggle slot of
        // `effectState.source` is hidden-disabled on `pokemon` (`disableMove(id, true)`, source effect = this
        // condition), then `pokemon.maybeDisabled = true`. PRNG: none.
        dex::HOOK_CONDITION_IMPRISON_ONFOEDISABLEMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let source = support::state_source(b, cx).expect("Imprison source");
            let (ids, len) = support::move_slot_ids(b, source);
            for &id in &ids[..len] {
                if id == dex::MOVE_STRUGGLE {
                    continue;
                }
                b.disable_move(pokemon, id, true, EffectRef::None);
            }
            support::set_flag(b, pokemon, support::MAYBE_DISABLED);
            Relay::Undefined
        }
        // data/moves.ts:9513-9519 onFoeBeforeMove(attacker, defender, move) (priority 4). PRNG: none.
        // A non-Struggle move that the imprisoner also knows is blocked with
        // `|cant|attacker|move: Imprison|<move>` and `return false` (Z/Max-powered moves do not exist here).
        dex::HOOK_CONDITION_IMPRISON_ONFOEBEFOREMOVE => {
            let attacker = mon_arg(b, cx, 0);
            let mv = move_arg(b, cx, 2);
            let move_id = move_overlay(b, mv).id;
            let source = support::state_source(b, cx).expect("Imprison source");
            if move_id != dex::MOVE_STRUGGLE && b.has_move(source, move_id).truthy() {
                b.add(LogEntry::new(
                    "cant",
                    &[
                        LogArg::Mon(attacker),
                        LogArg::Text("move: Imprison"),
                        LogArg::Effect(EffectRef::ActiveMove(mv)),
                    ],
                    &[],
                ));
                return Relay::FAIL;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Imprison condition function site"),
    }
}
