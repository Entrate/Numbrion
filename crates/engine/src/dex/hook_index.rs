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

// A conservative, immutable union across all effects. False proves collection
// is empty; true still requires the normal holder and effect lookup.
static CALLBACK_RELS: [u8; EVENT_COUNT] = {
    let mut masks = [0; EVENT_COUNT];
    let mut i = 0;
    while i < HOOKS.len() {
        let h = &HOOKS[i];
        if h.site.is_empty() && !matches!(h.value, HookValue::Absent) {
            masks[h.event as usize] |= 1 << h.rel as usize;
            if h.event as usize == EventId::Start as usize
                && h.rel as usize == HookRel::On as usize
                && matches!(
                    MANIFESTS[h.effect.0 as usize].effect_type,
                    EffectType::Ability | EffectType::Item
                )
            {
                masks[EventId::SwitchIn as usize] |= 1 << HookRel::On as usize;
            }
        }
        i += 1;
    }
    masks
};

// Includes the possible onStart fallback for a Pokemon's SwitchIn. This is
// conservative when onAnySwitchIn disables that fallback; false remains exact.
// Rows are indexed by raw id: alias rows hold their canonical effect's masks.
static EFFECT_CALLBACK_RELS: [[u8; EVENT_COUNT]; MANIFESTS.len()] = {
    let mut masks = [[0; EVENT_COUNT]; MANIFESTS.len()];
    let mut i = 0;
    while i < HOOKS.len() {
        let h = &HOOKS[i];
        if h.site.is_empty() && !matches!(h.value, HookValue::Absent) {
            masks[h.effect.0 as usize][h.event as usize] |= 1 << h.rel as usize;
            if h.event as usize == EventId::Start as usize
                && h.rel as usize == HookRel::On as usize
                && matches!(
                    MANIFESTS[h.effect.0 as usize].effect_type,
                    EffectType::Ability | EffectType::Item
                )
            {
                masks[h.effect.0 as usize][EventId::SwitchIn as usize] |= 1 << HookRel::On as usize;
            }
        }
        i += 1;
    }
    // canonical_effect is a single lookup, so copy canonical rows after all hooks.
    let canonical = masks;
    let mut a = 0;
    while a < CONDITION_RULE_ALIASES.len() {
        let (from, to) = CONDITION_RULE_ALIASES[a];
        masks[from.0 as usize] = canonical[to.0 as usize];
        a += 1;
    }
    masks
};

#[inline]
pub fn effect_has_callback(id: EffectId, event: EventId, rel: HookRel) -> bool {
    effect_callback_relations(id, event) & (1 << rel as usize) != 0
}

/// All relation bits of `effect_has_callback` for one effect and event.
#[inline]
pub fn effect_callback_relations(id: EffectId, event: EventId) -> u8 {
    EFFECT_CALLBACK_RELS[id.0 as usize][event as usize]
}

#[inline]
pub fn callback_relations(event: EventId) -> u8 {
    CALLBACK_RELS[event as usize]
}

#[inline]
pub fn has_callback(event: EventId, rel: HookRel) -> bool {
    callback_relations(event) & (1 << rel as usize) != 0
}

#[inline]
pub fn event_hook(id: EffectId, event: EventId, rel: HookRel) -> Option<HookId> {
    let h = INDEX[id.0 as usize][event as usize][rel as usize];
    (h != MISSING).then_some(HookId(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_union_covers_exact_hooks_and_species_views() {
        for h in HOOKS.iter().filter(|h| h.site.is_empty()) {
            if !matches!(h.value, HookValue::Absent) {
                assert!(has_callback(h.event, h.rel));
                assert!(effect_has_callback(h.effect, h.event, h.rel));
            }
        }
        for view in SPECIES_CONDITION_VIEWS {
            for id in view.hooks {
                let h = &HOOKS[id.0 as usize];
                if !matches!(h.value, HookValue::Absent) {
                    assert!(has_callback(h.event, h.rel));
                }
            }
        }
    }

    #[test]
    fn alias_rows_match_canonical_rows() {
        for id in 0..MANIFESTS.len() {
            let id = EffectId(id as u16);
            for h in HOOKS {
                assert_eq!(
                    effect_callback_relations(id, h.event),
                    effect_callback_relations(canonical_effect(id), h.event)
                );
            }
        }
    }

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
