//! Ports data/moves.ts:18885-18909 (embedded side condition of Tailwind).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none.
//! Persistent is outside the generated ability closure: `query_has_ability` matches by dex key and
//! can never be true here, but the branches are ported so a wider scope stays exact.
use crate::effects::registry::conditions_reflect::support::{self, opt_mon_arg, side_arg};
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::CONDITION_TAILWIND;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_TAILWIND_DURATIONCALLBACK,
    dex::HOOK_CONDITION_TAILWIND_ONSIDESTART,
    dex::HOOK_CONDITION_TAILWIND_ONMODIFYSPE,
    dex::HOOK_CONDITION_TAILWIND_ONSIDEEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:18887 durationCallback(target, source, effect): direct call, args
        // [side.active[0], source, sourceEffect]. `source?.hasAbility('persistent')` logs
        // `-activate|source|ability: Persistent|[move] Tailwind` and returns 6; otherwise 4.
        dex::HOOK_CONDITION_TAILWIND_DURATIONCALLBACK => match opt_mon_arg(b, cx, 1) {
            Some(source) if b.query_has_ability(source, "persistent") => {
                b.add(LogEntry::new(
                    "-activate",
                    &[
                        LogArg::Mon(source),
                        LogArg::Text("ability: Persistent"),
                        LogArg::Text("[move] Tailwind"),
                    ],
                    &[],
                ));
                Relay::Number(6.0)
            }
            _ => Relay::Number(4.0),
        },
        // data/moves.ts:18894 onSideStart(side, source): `-sidestart|side|move: Tailwind` plus
        // `|[persistent]` when the source has Persistent. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_TAILWIND_ONSIDESTART => {
            let side = side_arg(b, cx, 0);
            let persistent = match opt_mon_arg(b, cx, 1) {
                Some(source) => b.query_has_ability(source, "persistent"),
                None => false,
            };
            if persistent {
                b.add(LogEntry::new(
                    "-sidestart",
                    &[LogArg::Side(side), LogArg::Text("move: Tailwind")],
                    &[LogTag::Bare("persistent")],
                ));
            } else {
                b.add(LogEntry::new(
                    "-sidestart",
                    &[LogArg::Side(side), LogArg::Text("move: Tailwind")],
                    &[],
                ));
            }
            Relay::Undefined
        }
        // data/moves.ts:18901 onModifySpe(spe, pokemon): this.chainModify(2), which mutates the
        // ModifySpe frame and returns undefined. PRNG: none.
        dex::HOOK_CONDITION_TAILWIND_ONMODIFYSPE => {
            b.chain_modify(2.0, 1.0);
            Relay::Undefined
        }
        // data/moves.ts:18906 onSideEnd: `-sideend|side|move: Tailwind`. PRNG: none.
        dex::HOOK_CONDITION_TAILWIND_ONSIDEEND => {
            support::side_line(b, cx, "-sideend", "move: Tailwind")
        }
        _ => panic!("unexpected Tailwind function site"),
    }
}
