//! Ports data/abilities.ts:4129; no direct PRNG draws. Core queries retain their event semantics.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_SERENEGRACE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SERENEGRACE_ONMODIFYMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_SERENEGRACE_ONMODIFYMOVE => {
            let mov = move_arg(b, cx, 0);
            let mv = b.scratch.moves[mov as usize].as_mut().unwrap();
            if mv.secondaries_present {
                for sec in mv.secondaries[..mv.secondary_count as usize]
                    .iter_mut()
                    .flatten()
                {
                    if sec.chance_present && sec.chance != 0 {
                        sec.chance = sec
                            .chance
                            .checked_mul(2)
                            .expect("secondary chance overflow");
                    }
                }
            }
            if let Some(effect) = mv.self_effect.as_mut() {
                if let Some(chance) = effect.chance.as_mut() {
                    if *chance != 0 {
                        *chance = chance.checked_mul(2).expect("self chance overflow");
                    }
                }
            }
        }
        _ => unreachable!("unexpected callback for serenegrace"),
    }
    Relay::Undefined
}
