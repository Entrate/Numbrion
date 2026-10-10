//! Ports data/moves.ts:3658-3706 (embedded condition of Disable): onStart, onEnd, onBeforeMove, onDisableMove.
//! Payload: word 0 = `effectState.move` (raw EffectId, present bit 8; see `locktrap/mod.rs`).
//! Direct PRNG draws: none. `onResidual` (order 17) is ordering metadata only: the duration countdown is the
//! generic residual machinery and Disable has no residual callback.
use crate::{
    Battle,
    actions::MoveHandle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        registry::conditions_choicelock::support,
        support::{mon_arg, move_arg, move_overlay},
    },
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::scratch::move_runtime as rt,
};
pub const ID: EffectId = dex::CONDITION_DISABLE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_DISABLE_ONSTART,
    dex::HOOK_CONDITION_DISABLE_ONEND,
    dex::HOOK_CONDITION_DISABLE_ONBEFOREMOVE,
    dex::HOOK_CONDITION_DISABLE_ONDISABLEMOVE,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:3662-3684 onStart(pokemon, source, effect). PRNG: none.
        // 1. If the target has not moved yet (queue.willMove) or is the active Pokemon of a non-external move
        //    (Cursed Body), the countdown starts one turn short: `effectState.duration--` (5 -> 4).
        // 2. No lastMove, or lastMove's slot out of PP: return false (addVolatile removes the volatile).
        // 3. `|-start|pokemon|Disable|<lastMove name>`, plus `|[from] ability: X|[of] source` for an Ability
        //    effect (Cursed Body); then `effectState.move = lastMove.id`.
        dex::HOOK_CONDITION_DISABLE_ONSTART => {
            let pokemon = mon_arg(b, cx, 0);
            let source = b.event_arg(cx, 1);
            let effect = match b.event_arg(cx, 2) {
                EventArg::Effect(e) => e,
                _ => EffectRef::None,
            };
            let active = b.scratch.active_move;
            let will_move = b.queue_will_move(pokemon).is_some()
                || (pokemon == b.scratch.active_pokemon
                    && active != MoveHandle::NONE
                    && b.active_move(active).runtime_flags & rt::EXTERNAL == 0);
            if will_move {
                b.hook_state_mut(cx).duration -= 1;
            }
            let last_move = b.state.pokemon[pokemon.0 as usize].last_move;
            if last_move == EffectId::NONE {
                // this.debug(`Pokemon hasn't moved yet`)
                return Relay::FAIL;
            }
            let out_of_pp = b.state.pokemon[pokemon.0 as usize]
                .move_slots()
                .iter()
                .any(|slot| slot.id == last_move && slot.pp == 0);
            if out_of_pp {
                // this.debug('Move out of PP')
                return Relay::FAIL;
            }
            let args = [
                LogArg::Mon(pokemon),
                LogArg::Text("Disable"),
                LogArg::Effect(EffectRef::Dex(last_move)),
            ];
            if effect != EffectRef::None && b.event_effect_type(effect) == dex::EffectType::Ability
            {
                let source = Battle::<L>::arg_mon(source).expect("Disable source is a Pokemon");
                b.add(LogEntry::new(
                    "-start",
                    &args,
                    &[LogTag::From(effect), LogTag::Of(source)],
                ));
            } else {
                b.add(LogEntry::new("-start", &args, &[]));
            }
            support::store_move(b, cx, last_move);
            Relay::Undefined
        }
        // data/moves.ts:3686-3688 onEnd(pokemon): `|-end|pokemon|Disable`. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_DISABLE_ONEND => {
            let pokemon = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(pokemon), LogArg::Text("Disable")],
                &[],
            ));
            Relay::Undefined
        }
        // data/moves.ts:3689-3694 onBeforeMove(attacker, defender, move) (priority 7). PRNG: none.
        // `!(move.isZ && move.isZOrMaxPowered)` is always true here; the disabled move is blocked with
        // `|cant|attacker|Disable|<move>` and `return false`.
        dex::HOOK_CONDITION_DISABLE_ONBEFOREMOVE => {
            let attacker = mon_arg(b, cx, 0);
            let mv = move_arg(b, cx, 2);
            if move_overlay(b, mv).id == support::stored_move(b, cx) {
                b.add(LogEntry::new(
                    "cant",
                    &[
                        LogArg::Mon(attacker),
                        LogArg::Text("Disable"),
                        LogArg::Effect(EffectRef::ActiveMove(mv)),
                    ],
                    &[],
                ));
                return Relay::FAIL;
            }
            Relay::Undefined
        }
        // data/moves.ts:3695-3701 onDisableMove(pokemon): disable every slot holding `effectState.move`
        // (`pokemon.disableMove(id)`: not hidden, source effect = this condition). PRNG: none.
        dex::HOOK_CONDITION_DISABLE_ONDISABLEMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let disabled = support::stored_move(b, cx);
            support::disable_matching(b, pokemon, |_, id| id == disabled);
            Relay::Undefined
        }
        _ => panic!("unexpected Disable condition function site"),
    }
}
