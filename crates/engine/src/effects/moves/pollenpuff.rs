//! Ports data/moves.ts:13571-13590 (move Pollen Puff onTryHit / onTryMove / onHit).
//! Payload: none. Direct PRNG draws: none; the heal runs TryHeal / Heal events (listeners may sort ties).
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    actions::{HealEffect, MoveHandle},
    dex::{self, HookId},
    effects::{HookWaiver, support::move_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, MoveLineEdit},
    state::scratch::move_runtime,
};
pub const ID: EffectId = dex::MOVE_POLLENPUFF;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_POLLENPUFF_ONTRYHIT,
    dex::HOOK_MOVE_POLLENPUFF_ONTRYMOVE,
    dex::HOOK_MOVE_POLLENPUFF_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:13571-13576 onTryHit(target, source, move): against an ally the move
        // becomes a zero-power infiltrating heal: `move.basePower = 0; move.infiltrates = true`.
        // Returns undefined. PRNG: none.
        dex::HOOK_MOVE_POLLENPUFF_ONTRYHIT => {
            let target = support::mon(b, cx, 0);
            let source = support::mon(b, cx, 1);
            let mv = move_arg(b, cx, 2);
            if b.is_ally(source, Some(target)) {
                let m = b.active_move_mut(MoveHandle(mv));
                m.base_power = 0.0;
                m.runtime_flags |= move_runtime::INFILTRATES;
            }
            Relay::Undefined
        }
        // data/moves.ts:13577-13583 onTryMove(source, target, move): an ally target under Heal Block
        // fails: `attrLastMove('[still]')`, `cant|source|move: Heal Block|move`, return false.
        // Args: [user, target, move]. PRNG: none.
        dex::HOOK_MOVE_POLLENPUFF_ONTRYMOVE => {
            let source = support::mon(b, cx, 0);
            let target = support::mon_opt(b, cx, 1);
            let mv = move_arg(b, cx, 2);
            // source.isAlly(target) && source.volatiles['healblock']
            if b.is_ally(source, target)
                && b.get_volatile(source, dex::CONDITION_HEALBLOCK).is_some()
            {
                b.attr_last_move(MoveLineEdit::Still);
                b.add(LogEntry::new(
                    "cant",
                    &[
                        LogArg::Mon(source),
                        LogArg::Text("move: Heal Block"),
                        LogArg::Effect(EffectRef::ActiveMove(mv)),
                    ],
                    &[],
                ));
                return Relay::Bool(false);
            }
            Relay::Undefined
        }
        // data/moves.ts:13584-13590 onHit(target, source, move): against an ally,
        // `this.heal(Math.floor(target.baseMaxhp * 0.5))` (target, source and effect default from
        // the Hit event / this move); a falsy heal (full HP, blocked) returns NOT_FAIL so the move
        // does not report failure. Otherwise undefined. PRNG: none directly.
        dex::HOOK_MOVE_POLLENPUFF_ONHIT => {
            let target = support::mon(b, cx, 0);
            let source = support::mon(b, cx, 1);
            if b.is_ally(source, Some(target)) {
                let max_hp = f64::from(b.state.pokemon[target.0 as usize].max_hp);
                let healed = b.heal((max_hp * 0.5).floor(), None, None, HealEffect::Context);
                if !healed.truthy() {
                    return Relay::NotFail;
                }
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Pollen Puff function site"),
    }
}
