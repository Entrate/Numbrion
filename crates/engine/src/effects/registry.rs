//! Static generic dispatcher. OWNER lead engine author, implemented in stage 2B.
use crate::{
    Battle,
    dex::HookId,
    event::{HookCtx, Relay},
    log::LogSink,
};
include!(concat!(env!("OUT_DIR"), "/effects_registry.rs"));
/// Ports battle.ts:641-646,855-859. PRNG: function's exact source draws only;
/// constants draw nothing. Absent metadata never invokes an effect function.
pub fn dispatch_hook<L: LogSink>(_hook: HookId, _b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    todo!("stage 2B: manifest value or static effect dispatch")
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dex::{self, HookValue};
    #[test]
    fn registered_effect_files_match_manifest() {
        let mut ids = std::collections::BTreeSet::new();
        for effect in IMPLEMENTATIONS {
            assert!(ids.insert(effect.id.0), "duplicate effect registration");
            assert!(effect.payload_words <= 4, "effect payload overflow");
            let mut declared = std::collections::BTreeSet::new();
            for hook in effect.hooks {
                assert!(declared.insert(hook.0), "duplicate function declaration");
                let h = &dex::HOOKS[hook.0 as usize];
                assert_eq!(h.effect, effect.id);
                assert_eq!(
                    h.value,
                    HookValue::Function,
                    "only function sites belong in effect files"
                );
            }
            for waiver in effect.waivers {
                assert!(!waiver.explanation.is_empty());
                assert!(declared.insert(waiver.hook.0), "waiver overlaps a function");
                let h = &dex::HOOKS[waiver.hook.0 as usize];
                assert_eq!(h.effect, effect.id);
                assert_eq!(h.value, HookValue::Function);
            }
            let expected: std::collections::BTreeSet<_> = dex::HOOKS
                .iter()
                .enumerate()
                .filter(|(_, h)| h.effect == effect.id && matches!(h.value, HookValue::Function))
                .map(|(i, _)| i as u16)
                .collect();
            assert_eq!(
                declared, expected,
                "registered file must declare every function site or explicit waiver"
            );
        }
    }
}
