//! Ports data/moves.ts:12803-12814 (embedded condition of No Retreat): onStart, onTrapPokemon.
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none. `tryTrap` runs the Trapped immunity event.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::CONDITION_NORETREAT;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_NORETREAT_ONSTART,
    dex::HOOK_CONDITION_NORETREAT_ONTRAPPOKEMON,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:12804 onStart(pokemon): `|-start|pokemon|move: No Retreat`. Returns undefined.
        dex::HOOK_CONDITION_NORETREAT_ONSTART => {
            let pokemon = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-start",
                &[LogArg::Mon(pokemon), LogArg::Text("move: No Retreat")],
                &[],
            ));
            Relay::Undefined
        }
        // data/moves.ts:12807 onTrapPokemon(pokemon): `pokemon.tryTrap()` (not hidden). Returns undefined.
        dex::HOOK_CONDITION_NORETREAT_ONTRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            b.try_trap(pokemon, false);
            Relay::Undefined
        }
        _ => panic!("unexpected No Retreat condition function site"),
    }
}
