//! Ports data/conditions.ts:208-221 (`trapped`, added by Spirit Shackle's secondary).
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none. `tryTrap` runs the Trapped immunity event.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::CONDITION_TRAPPED;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_TRAPPED_ONTRAPPOKEMON,
    dex::HOOK_CONDITION_TRAPPED_ONSTART,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:211 onTrapPokemon(pokemon): `pokemon.tryTrap()` (not hidden). Returns undefined.
        dex::HOOK_CONDITION_TRAPPED_ONTRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            b.try_trap(pokemon, false);
            Relay::Undefined
        }
        // data/conditions.ts:214 onStart(target): `|-activate|target|trapped`. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_TRAPPED_ONSTART => {
            let target = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-activate",
                &[LogArg::Mon(target), LogArg::Text("trapped")],
                &[],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected trapped function site"),
    }
}
