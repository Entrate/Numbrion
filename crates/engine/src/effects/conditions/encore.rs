//! Ports data/moves.ts:4735-4781 (embedded condition of Encore): onStart, onOverrideAction, onResidual,
//! onEnd, onDisableMove.
//! Payload: word 0 = `effectState.move` (raw EffectId, present bit 8; see `locktrap/mod.rs`).
//! Direct PRNG draws: none. `removeVolatile` inside onResidual raises the End event.
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
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::CONDITION_ENCORE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_ENCORE_ONSTART,
    dex::HOOK_CONDITION_ENCORE_ONOVERRIDEACTION,
    dex::HOOK_CONDITION_ENCORE_ONRESIDUAL,
    dex::HOOK_CONDITION_ENCORE_ONEND,
    dex::HOOK_CONDITION_ENCORE_ONDISABLEMOVE,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:4738-4752 onStart(target). PRNG: none.
        // Fails (false -> volatile removed) with no lastMove, or when lastMove is Z/Max, has the `failencore`
        // flag (Struggle, Encore, ...), is missing from the move slots, or has no PP left. Dynamax/Z/Max do not
        // exist in this format. Otherwise `effectState.move = lastMove.id`, `|-start|target|Encore`, and one
        // extra turn of duration when the target has no move queued yet (`!queue.willMove`).
        dex::HOOK_CONDITION_ENCORE_ONSTART => {
            let target = mon_arg(b, cx, 0);
            let last_move = b.state.pokemon[target.0 as usize].last_move;
            if last_move == EffectId::NONE {
                return Relay::FAIL;
            }
            let has_pp = b.get_move_data(target, last_move).is_some_and(|s| s.pp > 0);
            if dex::move_data(last_move).flags & dex::FLAG_FAILENCORE != 0 || !has_pp {
                return Relay::FAIL;
            }
            support::store_move(b, cx, last_move);
            b.add(LogEntry::new(
                "-start",
                &[LogArg::Mon(target), LogArg::Text("Encore")],
                &[],
            ));
            if b.queue_will_move(target).is_none() {
                b.hook_state_mut(cx).duration += 1;
            }
            Relay::Undefined
        }
        // data/moves.ts:4753-4755 onOverrideAction(pokemon, target, move): any other move than the encored one
        // is replaced by it (`return this.effectState.move`, a move id the move executor re-reads); the
        // encored move itself returns undefined. PRNG: none.
        dex::HOOK_CONDITION_ENCORE_ONOVERRIDEACTION => {
            let mv = move_arg(b, cx, 2);
            let encored = support::stored_move(b, cx);
            if move_overlay(b, mv).id != encored {
                return Relay::Move(encored);
            }
            Relay::Undefined
        }
        // data/moves.ts:4757-4763 onResidual(target) (order 16): ends early when the encored move is no
        // longer in the slots or out of PP: `target.removeVolatile('encore')`. PRNG: none directly.
        dex::HOOK_CONDITION_ENCORE_ONRESIDUAL => {
            let target = mon_arg(b, cx, 0);
            let encored = support::stored_move(b, cx);
            let has_pp = b.get_move_data(target, encored).is_some_and(|s| s.pp > 0);
            if !has_pp {
                b.remove_volatile(target, dex::CONDITION_ENCORE);
            }
            Relay::Undefined
        }
        // data/moves.ts:4764-4766 onEnd(target): `|-end|target|Encore`. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_ENCORE_ONEND => {
            let target = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(target), LogArg::Text("Encore")],
                &[],
            ));
            Relay::Undefined
        }
        // data/moves.ts:4767-4776 onDisableMove(pokemon): with a stored move the Pokemon still knows,
        // `pokemon.disableMove(id)` (not hidden, source effect = this condition) on every other slot.
        // PRNG: none.
        dex::HOOK_CONDITION_ENCORE_ONDISABLEMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let encored = support::stored_move(b, cx);
            if encored == EffectId::NONE || !b.has_move(pokemon, encored).truthy() {
                return Relay::Undefined;
            }
            support::disable_matching(b, pokemon, |_, id| id != encored);
            Relay::Undefined
        }
        _ => panic!("unexpected Encore condition function site"),
    }
}
