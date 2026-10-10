//! Ports data/moves.ts:6052-6069 (embedded volatile condition of Follow Me).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay, optional_id},
    },
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::CONDITION_FOLLOWME;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_FOLLOWME_ONSTART,
    dex::HOOK_CONDITION_FOLLOWME_ONFOEREDIRECTTARGET,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// `effect?.id === 'zpower'`: Z-moves do not exist in this format, so there is no such dex row
/// and the id is NONE (which never matches a real effect).
const ZPOWER: EffectId = optional_id(dex::CONDITIONS_DATA, "zpower");
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:6054-6060 onStart(target, source, effect): `-singleturn|target|move:
        // Follow Me`, plus `[zeffect]` when the effect is the Z-power pseudo effect (never here).
        // Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_FOLLOWME_ONSTART => {
            let target = support::mon(b, cx, 0);
            let effect_id = match b.event_arg(cx, 2) {
                EventArg::Effect(e) if e != EffectRef::None => b.event_effect_id(e),
                _ => EffectId::NONE,
            };
            if ZPOWER != EffectId::NONE && effect_id == ZPOWER {
                b.add(LogEntry::new(
                    "-singleturn",
                    &[LogArg::Mon(target), LogArg::Text("move: Follow Me")],
                    &[LogTag::Bare("zeffect")],
                ));
            } else {
                support::add_text(b, "-singleturn", target, "move: Follow Me");
            }
            Relay::Undefined
        }
        // data/moves.ts:6062-6068 onFoeRedirectTarget(target, source, source2, move), priority 1
        // (+ effectOrder). Args: [current target (relay), user, user, move]. If the holder is not
        // Sky Dropped and is a valid target of the move for the user, clear smartTarget and
        // redirect to the holder; otherwise undefined. PRNG: none.
        dex::HOOK_CONDITION_FOLLOWME_ONFOEREDIRECTTARGET => {
            let holder = support::state_target(b, cx);
            let user = support::mon(b, cx, 1);
            let mv = move_arg(b, cx, 3);
            let kind = move_overlay(b, mv).target;
            if !support::is_sky_dropped(b, holder) && b.valid_target(holder, user, kind) {
                if support::smart_target(b, mv) {
                    support::clear_smart_target(b, mv);
                }
                return Relay::Pokemon(holder);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Follow Me condition function site"),
    }
}
