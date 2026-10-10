//! Ports data/moves.ts:19950-19975 (embedded pseudo-weather of Trick Room).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none.
//! The speed reversal itself lives in Pokemon.getActionSpeed (sim/pokemon.ts:644), already ported by
//! the core stat module (`pokemon_action_speed`); this file only owns the condition's callbacks.
//! Persistent is outside the generated ability closure: `query_has_ability` matches by dex key and
//! can never be true here, but the branches are ported so a wider scope stays exact.
use crate::effects::registry::conditions_reflect::support::opt_mon_arg;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::CONDITION_TRICKROOM;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_TRICKROOM_DURATIONCALLBACK,
    dex::HOOK_CONDITION_TRICKROOM_ONFIELDSTART,
    dex::HOOK_CONDITION_TRICKROOM_ONFIELDRESTART,
    dex::HOOK_CONDITION_TRICKROOM_ONFIELDEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:19952 durationCallback(source, effect): direct call, args [source, effect].
        // `source?.hasAbility('persistent')` logs `-activate|source|ability: Persistent|[move]
        // Trick Room` and returns 7; otherwise 5.
        dex::HOOK_CONDITION_TRICKROOM_DURATIONCALLBACK => match opt_mon_arg(b, cx, 0) {
            Some(source) if b.query_has_ability(source, dex::key_ids!("persistent")) => {
                b.add(LogEntry::new(
                    "-activate",
                    &[
                        LogArg::Mon(source),
                        LogArg::Text("ability: Persistent"),
                        LogArg::Text("[move] Trick Room"),
                    ],
                    &[],
                ));
                Relay::Number(7.0)
            }
            _ => Relay::Number(5.0),
        },
        // data/moves.ts:19959 onFieldStart(target, source): `-fieldstart|move: Trick Room|[of] source`
        // plus `|[persistent]` when the source has Persistent. `[of] ${source}` is the same text as an
        // `[of]` tag. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_TRICKROOM_ONFIELDSTART => {
            let source = mon_arg(b, cx, 1);
            let persistent = b.query_has_ability(source, dex::key_ids!("persistent"));
            if persistent {
                b.add(LogEntry::new(
                    "-fieldstart",
                    &[LogArg::Text("move: Trick Room")],
                    &[LogTag::Of(source), LogTag::Bare("persistent")],
                ));
            } else {
                b.add(LogEntry::new(
                    "-fieldstart",
                    &[LogArg::Text("move: Trick Room")],
                    &[LogTag::Of(source)],
                ));
            }
            Relay::Undefined
        }
        // data/moves.ts:19966 onFieldRestart(target, source): this.field.removePseudoWeather
        // ('trickroom') (its FieldEnd logs `-fieldend`). Returns undefined, so the restart reports
        // success: using Trick Room again toggles it off. PRNG: none.
        dex::HOOK_CONDITION_TRICKROOM_ONFIELDRESTART => {
            b.remove_pseudo_weather(dex::CONDITION_TRICKROOM);
            Relay::Undefined
        }
        // data/moves.ts:19972 onFieldEnd: `-fieldend|move: Trick Room`. PRNG: none.
        dex::HOOK_CONDITION_TRICKROOM_ONFIELDEND => {
            b.add(LogEntry::new(
                "-fieldend",
                &[LogArg::Text("move: Trick Room")],
                &[],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected Trick Room function site"),
    }
}
