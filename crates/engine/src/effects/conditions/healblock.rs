//! Ports data/moves.ts:8290-8352 (embedded condition of Heal Block): durationCallback, onStart,
//! onDisableMove, onBeforeMove, onModifyMove, onEnd, onTryHeal, onRestart.
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none. `onResidual` (order 20) is ordering metadata
//! only: the countdown is the generic residual machinery.
//! Out-of-format branches: `effect.id === 'zpower' || effect.isZ` (onTryHeal; Z-Power does not exist) and
//! `!move.isZ && !move.isMax` (always true). `source.hasAbility('persistent')` is kept through the static key
//! query (Persistent is outside the scoped ability list, so it can never match).
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        registry::conditions_choicelock::support,
        support::{mon_arg, move_arg, move_overlay},
    },
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, MoveLineEdit},
    state::ResultFlag,
};
pub const ID: EffectId = dex::CONDITION_HEALBLOCK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_HEALBLOCK_DURATIONCALLBACK,
    dex::HOOK_CONDITION_HEALBLOCK_ONSTART,
    dex::HOOK_CONDITION_HEALBLOCK_ONDISABLEMOVE,
    dex::HOOK_CONDITION_HEALBLOCK_ONBEFOREMOVE,
    dex::HOOK_CONDITION_HEALBLOCK_ONMODIFYMOVE,
    dex::HOOK_CONDITION_HEALBLOCK_ONEND,
    dex::HOOK_CONDITION_HEALBLOCK_ONTRYHEAL,
    dex::HOOK_CONDITION_HEALBLOCK_ONRESTART,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// `effect?.name === "Psychic Noise"` for an optional effect argument.
fn is_psychic_noise<L: LogSink>(b: &Battle<L>, effect: EffectRef) -> bool {
    effect != EffectRef::None && b.event_effect_id(effect) == dex::MOVE_PSYCHICNOISE
}
fn effect_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> EffectRef {
    match b.event_arg(cx, index) {
        EventArg::Effect(e) => e,
        _ => EffectRef::None,
    }
}
/// Shared body of onBeforeMove (data/moves.ts:8311) and onModifyMove (:8317): a move with the `heal` flag
/// is blocked with `|cant|pokemon|move: Heal Block|<move>` and `return false`.
fn block_heal_move<L: LogSink>(b: &mut Battle<L>, mv: u8, pokemon: crate::ids::MonId) -> Relay {
    if move_overlay(b, mv).flags & dex::FLAG_HEAL != 0 {
        b.add(LogEntry::new(
            "cant",
            &[
                LogArg::Mon(pokemon),
                LogArg::Text("move: Heal Block"),
                LogArg::Effect(EffectRef::ActiveMove(mv)),
            ],
            &[],
        ));
        return Relay::FAIL;
    }
    Relay::Undefined
}
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:8291-8300 durationCallback(target, source, effect): direct call, args
        // [target, source, effect]. Psychic Noise -> 2; a Persistent source logs
        // `|-activate|source|ability: Persistent|[move] Heal Block` and gives 7; otherwise 5. PRNG: none.
        dex::HOOK_CONDITION_HEALBLOCK_DURATIONCALLBACK => {
            let source = support::opt_mon_arg(b, cx, 1);
            let effect = effect_arg(b, cx, 2);
            if is_psychic_noise(b, effect) {
                return Relay::Number(2.0);
            }
            if let Some(source) = source {
                if b.query_has_ability(source, "persistent") {
                    b.add(LogEntry::new(
                        "-activate",
                        &[
                            LogArg::Mon(source),
                            LogArg::Text("ability: Persistent"),
                            LogArg::Text("[move] Heal Block"),
                        ],
                        &[],
                    ));
                    return Relay::Number(7.0);
                }
            }
            Relay::Number(5.0)
        }
        // data/moves.ts:8301-8304 onStart(pokemon, source): `|-start|pokemon|move: Heal Block`, then
        // `source.moveThisTurnResult = true`. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_HEALBLOCK_ONSTART => {
            let pokemon = mon_arg(b, cx, 0);
            let source = mon_arg(b, cx, 1);
            b.add(LogEntry::new(
                "-start",
                &[LogArg::Mon(pokemon), LogArg::Text("move: Heal Block")],
                &[],
            ));
            b.state.pokemon[source.0 as usize].move_this_turn_result = ResultFlag::True;
            Relay::Undefined
        }
        // data/moves.ts:8305-8311 onDisableMove(pokemon): `pokemon.disableMove(id)` (not hidden, source effect =
        // this condition) for every slot whose dex move has the `heal` flag. PRNG: none.
        dex::HOOK_CONDITION_HEALBLOCK_ONDISABLEMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            support::disable_matching(b, pokemon, |_, id| {
                dex::move_data(id).flags & dex::FLAG_HEAL != 0
            });
            Relay::Undefined
        }
        // data/moves.ts:8312-8318 onBeforeMove(pokemon, target, move) (priority 6). PRNG: none.
        dex::HOOK_CONDITION_HEALBLOCK_ONBEFOREMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let mv = move_arg(b, cx, 2);
            block_heal_move(b, mv, pokemon)
        }
        // data/moves.ts:8319-8325 onModifyMove(move, pokemon): run-event args (move relay, pokemon, target).
        // Same check and message as onBeforeMove. PRNG: none.
        dex::HOOK_CONDITION_HEALBLOCK_ONMODIFYMOVE => {
            let mv = move_arg(b, cx, 0);
            let pokemon = mon_arg(b, cx, 1);
            block_heal_move(b, mv, pokemon)
        }
        // data/moves.ts:8327-8329 onEnd(pokemon): `|-end|pokemon|move: Heal Block`. PRNG: none.
        dex::HOOK_CONDITION_HEALBLOCK_ONEND => {
            let pokemon = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(pokemon), LogArg::Text("move: Heal Block")],
                &[],
            ));
            Relay::Undefined
        }
        // data/moves.ts:8330-8341 onTryHeal(damage, target, source, effect): run-event args
        // [damage (relay), target, source, effect]. A third-party Pollen Puff on a non-full-HP target is
        // reported (`[still]`, `|cant|source|move: Heal Block|<effect>`) and nullified (`return null`); every
        // other heal is cancelled with `return false`. The Z-Power early `return damage` is outside the format.
        // PRNG: none.
        dex::HOOK_CONDITION_HEALBLOCK_ONTRYHEAL => {
            let target = mon_arg(b, cx, 1);
            let source = support::opt_mon_arg(b, cx, 2);
            let effect = effect_arg(b, cx, 3);
            if let Some(source) = source {
                let t = &b.state.pokemon[target.0 as usize];
                if target != source
                    && t.hp != t.max_hp
                    && effect != EffectRef::None
                    && b.event_effect_id(effect) == dex::MOVE_POLLENPUFF
                {
                    b.attr_last_move(MoveLineEdit::Still);
                    b.add(LogEntry::new(
                        "cant",
                        &[
                            LogArg::Mon(source),
                            LogArg::Text("move: Heal Block"),
                            LogArg::Effect(effect),
                        ],
                        &[],
                    ));
                    return Relay::Null;
                }
            }
            Relay::FAIL
        }
        // data/moves.ts:8342-8350 onRestart(target, source, effect): Psychic Noise leaves the existing block
        // alone. Otherwise `|-fail|target|move: Heal Block` ("succeeds to suppress downstream messages") and
        // `if (!source.moveThisTurnResult) source.moveThisTurnResult = false` (undefined/null become false).
        // Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_HEALBLOCK_ONRESTART => {
            let target = mon_arg(b, cx, 0);
            let source = mon_arg(b, cx, 1);
            let effect = effect_arg(b, cx, 2);
            if is_psychic_noise(b, effect) {
                return Relay::Undefined;
            }
            b.add(LogEntry::new(
                "-fail",
                &[LogArg::Mon(target), LogArg::Text("move: Heal Block")],
                &[],
            ));
            let result = &mut b.state.pokemon[source.0 as usize].move_this_turn_result;
            if *result != ResultFlag::True {
                *result = ResultFlag::False;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Heal Block condition function site"),
    }
}
