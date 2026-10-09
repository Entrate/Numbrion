//! data/conditions.ts:189-222. words[0]=time(bit8). Start random(min,6), BeforeMove randomChance(33,100) after decrement; self-hit D draws damage randomizer.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::CONDITION_CONFUSION;
pub const HOOKS: &[HookId] = &[HookId(657), HookId(658), HookId(659)];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const ONSTART: HookId = HookId(657);
const ONEND: HookId = HookId(658);
const ONBEFOREMOVE: HookId = HookId(659);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        ONSTART => {
            let m = mon_arg(b, cx, 0);
            let effect = match b.event_arg(cx, 2) {
                EventArg::Effect(e) => e,
                _ => EffectRef::None,
            };
            let id = b.event_effect_id(effect);
            if id == dex::CONDITION_LOCKEDMOVE {
                b.add(LogEntry::new(
                    "-start",
                    &[LogArg::Mon(m), LogArg::Text("confusion")],
                    &[crate::log::LogTag::Bare("fatigue")],
                ));
            } else if effect != EffectRef::None
                && b.event_effect_type(effect) == dex::EffectType::Ability
            {
                let source = mon_arg(b, cx, 1);
                b.add(LogEntry::new(
                    "-start",
                    &[LogArg::Mon(m), LogArg::Text("confusion")],
                    &[
                        crate::log::LogTag::From(effect),
                        crate::log::LogTag::Of(source),
                    ],
                ));
            } else {
                b.add(LogEntry::new(
                    "-start",
                    &[LogArg::Mon(m), LogArg::Text("confusion")],
                    &[],
                ));
            } /* Axe Kick is outside the pinned format scope. */
            let time = b.state.prng.random_range(2, 6);
            let c = b.hook_state_mut(cx);
            c.payload.words[0] = time;
            c.present |= 1 << 8;
            Relay::Undefined
        }
        ONEND => {
            let m = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(m), LogArg::Text("confusion")],
                &[],
            ));
            Relay::Undefined
        }
        ONBEFOREMOVE => {
            let m = mon_arg(b, cx, 0);
            let c = b.hook_state_mut(cx);
            c.payload.words[0] = c.payload.words[0].wrapping_sub(1);
            if c.payload.words[0] == 0 {
                b.remove_volatile(m, ID);
                return Relay::Undefined;
            }
            b.add(LogEntry::new(
                "-activate",
                &[LogArg::Mon(m), LogArg::Text("confusion")],
                &[],
            ));
            if !b.state.prng.random_chance(33, 100) {
                return Relay::Undefined;
            }
            b.scratch.active_target = m;
            let damage = b.get_confusion_damage(m, 40.0);
            b.damage(
                damage,
                Some(m),
                crate::actions::Attribution::from_move(
                    m,
                    EffectRef::Synthetic(SyntheticEffect::Confused),
                ),
            );
            Relay::FAIL
        }
        _ => unreachable!(),
    }
}
