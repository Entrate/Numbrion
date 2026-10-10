//! Ports data/conditions.ts:364-380 (mustrecharge; `duration: 2` is declarative and `onLockMove:
//! 'recharge'` is a manifest constant that the core turns into the synthetic Recharge move).
//! No payload. PRNG: none directly.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::CONDITION_MUSTRECHARGE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_MUSTRECHARGE_ONBEFOREMOVE,
    dex::HOOK_CONDITION_MUSTRECHARGE_ONSTART,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:367-373 onBeforeMove(pokemon) (manifest priority 11):
        //   this.add('cant', pokemon, 'recharge');
        //   pokemon.removeVolatile('mustrecharge'); pokemon.removeVolatile('truant');
        //   return null;
        // BeforeMove args are (pokemon, target, move). PRNG: none directly.
        dex::HOOK_CONDITION_MUSTRECHARGE_ONBEFOREMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "cant",
                &[LogArg::Mon(pokemon), LogArg::Text("recharge")],
                &[],
            ));
            b.remove_volatile(pokemon, dex::CONDITION_MUSTRECHARGE);
            b.remove_volatile(pokemon, dex::CONDITION_TRUANT);
            Relay::Null
        }
        // data/conditions.ts:374-376 onStart(pokemon): `this.add('-mustrecharge', pokemon)`. PRNG: none.
        dex::HOOK_CONDITION_MUSTRECHARGE_ONSTART => {
            let pokemon = mon_arg(b, cx, 0);
            b.add(LogEntry::new("-mustrecharge", &[LogArg::Mon(pokemon)], &[]));
            Relay::Undefined
        }
        _ => panic!("unexpected mustrecharge hook"),
    }
}
