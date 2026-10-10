//! Derived at compile time from the pinned hook metadata. Effect authors still
//! declare hooks in their effect files; nested move sites stay out of this index.
use super::*;

const REL_COUNT: usize = HookRel::Direct as usize + 1;
const MISSING: u16 = u16::MAX;
static INDEX: [[[u16; REL_COUNT]; EVENT_COUNT]; MANIFESTS.len()] = {
    let mut index = [[[MISSING; REL_COUNT]; EVENT_COUNT]; MANIFESTS.len()];
    let mut i = 0;
    while i < HOOKS.len() {
        let h = &HOOKS[i];
        let slot = &mut index[h.effect.0 as usize][h.event as usize][h.rel as usize];
        // Keep the first exact match, including ordering-only (Absent) hooks.
        if h.site.is_empty() && *slot == MISSING {
            assert!(i < MISSING as usize);
            *slot = i as u16;
        }
        i += 1;
    }
    index
};

#[inline]
pub fn event_hook(id: EffectId, event: EventId, rel: HookRel) -> Option<HookId> {
    let h = INDEX[id.0 as usize][event as usize][rel as usize];
    (h != MISSING).then_some(HookId(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_matches_manifest_scan() {
        for (id, manifest) in MANIFESTS.iter().enumerate() {
            for hook in HOOKS {
                for rel in [
                    HookRel::On,
                    HookRel::Source,
                    HookRel::Ally,
                    HookRel::Foe,
                    HookRel::Any,
                    HookRel::Direct,
                ] {
                    let scanned = manifest
                        .hooks()
                        .iter()
                        .position(|h| h.event == hook.event && h.rel == rel && h.site.is_empty())
                        .map(|i| HookId(manifest.hooks_start + i as u16));
                    assert_eq!(event_hook(EffectId(id as u16), hook.event, rel), scanned);
                }
            }
        }
    }
}
