//! Ports the `charge` condition embedded in the move Charge, data/moves.ts:2264-2316 (the volatile
//! is added by Electromorphosis, `target.addVolatile('charge')`; the Charge move itself and Wind
//! Power are outside the generated scope, so their ids below are `NONE` and never match).
//! No payload. PRNG: none directly (removeVolatile/addVolatile events may tie-shuffle).
use crate::effects::registry::moves_electroshot::cp;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg, move_overlay, optional_id},
    },
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::CONDITION_CHARGE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_CHARGE_ONSTART,
    dex::HOOK_CONDITION_CHARGE_ONRESTART,
    dex::HOOK_CONDITION_CHARGE_ONBASEPOWER,
    dex::HOOK_CONDITION_CHARGE_ONMOVEABORTED,
    dex::HOOK_CONDITION_CHARGE_ONAFTERMOVE,
    dex::HOOK_CONDITION_CHARGE_ONEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

/// The move Charge (`move.id !== 'charge'`): not in the generated move list.
const MOVE_CHARGE: EffectId = optional_id(dex::MOVES_DATA, "charge");
/// Wind Power (`['Electromorphosis', 'Wind Power'].includes(effect.name)`): not in the ability list.
const ABILITY_WINDPOWER: EffectId = optional_id(dex::ABILITIES_DATA, "windpower");

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:2275-2281 onStart(pokemon, source, effect) and 2282-2288 onRestart(...) share
        // one body. PRNG: none.
        dex::HOOK_CONDITION_CHARGE_ONSTART | dex::HOOK_CONDITION_CHARGE_ONRESTART => {
            let pokemon = mon_arg(b, cx, 0);
            let effect = match b.event_arg(cx, 2) {
                EventArg::Effect(e) => e,
                _ => EffectRef::None,
            };
            let id = b.event_effect_id(effect);
            if effect != EffectRef::None
                && (id == dex::ABILITY_ELECTROMORPHOSIS
                    || (ABILITY_WINDPOWER != EffectId::NONE && id == ABILITY_WINDPOWER))
            {
                // '-start', pokemon, 'Charge', this.activeMove!.name, '[from] ability: ' + effect.name
                let active = b.scratch.active_move;
                assert!(
                    active != crate::actions::MoveHandle::NONE,
                    "Charge started by an ability outside a move"
                );
                b.add(LogEntry::new(
                    "-start",
                    &[
                        LogArg::Mon(pokemon),
                        LogArg::Text("Charge"),
                        LogArg::Effect(EffectRef::ActiveMove(active.0)),
                    ],
                    &[LogTag::From(EffectRef::Dex(id))],
                ));
            } else {
                b.add(LogEntry::new(
                    "-start",
                    &[LogArg::Mon(pokemon), LogArg::Text("Charge")],
                    &[],
                ));
            }
            Relay::Undefined
        }
        // data/moves.ts:2289-2295 onBasePower(basePower, attacker, defender, move) (manifest priority
        // 9): Electric moves get chainModify(2). Args: (relay, attacker, defender, move). PRNG: none.
        dex::HOOK_CONDITION_CHARGE_ONBASEPOWER => {
            let mv = move_arg(b, cx, 3);
            if move_overlay(b, mv).move_type == cp::TYPE_ELECTRIC {
                b.chain_modify(2.0, 1.0);
            }
            Relay::Undefined
        }
        // data/moves.ts:2296-2300 onMoveAborted(pokemon, target, move) and 2301-2305
        // onAfterMove(pokemon, target, move): an Electric move other than Charge consumes the
        // volatile. Both events pass (pokemon, target, move). PRNG: none.
        dex::HOOK_CONDITION_CHARGE_ONMOVEABORTED | dex::HOOK_CONDITION_CHARGE_ONAFTERMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let mv = move_arg(b, cx, 2);
            let m = move_overlay(b, mv);
            if m.move_type == cp::TYPE_ELECTRIC && m.id != MOVE_CHARGE {
                b.remove_volatile(pokemon, dex::CONDITION_CHARGE);
            }
            Relay::Undefined
        }
        // data/moves.ts:2306-2308 onEnd(pokemon): `this.add('-end', pokemon, 'Charge', '[silent]')`.
        dex::HOOK_CONDITION_CHARGE_ONEND => {
            let pokemon = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[
                    LogArg::Mon(pokemon),
                    LogArg::Text("Charge"),
                    LogArg::Text("[silent]"),
                ],
                &[],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected charge hook"),
    }
}
