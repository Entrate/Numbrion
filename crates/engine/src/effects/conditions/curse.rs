//! Ports data/moves.ts:3297-3306 (embedded condition of Curse, the Ghost-type curse). Payload: none.
//! Direct PRNG draws: none. `onResidual` damage may raise Damage/Faint events whose handlers own their draws.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::CONDITION_CURSE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_CURSE_ONSTART,
    dex::HOOK_CONDITION_CURSE_ONRESIDUAL,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:3298 onStart(pokemon, source): `|-start|pokemon|Curse|[of] source`
        // (the TS argument `[of] ${source}` is the same text as an `[of]` tag). Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_CURSE_ONSTART => {
            let pokemon = mon_arg(b, cx, 0);
            let source = mon_arg(b, cx, 1);
            b.add(LogEntry::new(
                "-start",
                &[LogArg::Mon(pokemon), LogArg::Text("Curse")],
                &[LogTag::Of(source)],
            ));
            Relay::Undefined
        }
        // data/moves.ts:3302 onResidual(pokemon) (onResidualOrder 12): `this.damage(pokemon.baseMaxhp / 4)`.
        // baseMaxhp == maxhp in this format; target/source/effect default from the running event
        // (target = pokemon, source = null, effect = curse). Returns undefined. PRNG: none directly.
        dex::HOOK_CONDITION_CURSE_ONRESIDUAL => {
            let pokemon = mon_arg(b, cx, 0);
            let amount = f64::from(b.state.pokemon[pokemon.0 as usize].max_hp) / 4.0;
            b.damage(amount, Some(pokemon), Attribution::DEFAULT);
            Relay::Undefined
        }
        _ => panic!("unexpected Curse condition function site"),
    }
}
