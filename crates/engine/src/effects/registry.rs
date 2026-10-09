//! Static generic dispatch and honest, whole-manifest port accounting.
//! Source: battle.ts:641-646,903-910. Registry bookkeeping makes no PRNG draws.
use super::{EffectImpl, HookCoverage, ManifestCoverage, PendingPortWaiver, RegistryAuditError};
use crate::{
    Battle,
    dex::{self, DataValue, HookId, HookValue},
    event::{HookCtx, Relay, SyntheticEffect},
    ids::EffectKind,
    log::LogSink,
};
include!(concat!(env!("OUT_DIR"), "/effects_registry.rs"));

/// Explicit stage-wide waiver for files that have not yet been ported.
/// Remove this policy after the fan-out is complete. Reached gaps always panic.
pub const INCOMPLETE_PORT_WAIVER: Option<PendingPortWaiver> = Some(PendingPortWaiver {
    stage: "Stage 2B initial effects",
    explanation: "Remaining effect families have not been ported; this registry is incomplete.",
    plan: "docs/design/EFFECT-BATCHES.json and docs/design/IMPLEMENTATION-PLAN.md",
});

// One byte per hook, no registration lookup on the hot path. Each sink retains
// monomorphized effect calls. Invalid declarations get detailed audit errors.
static IMPLEMENTED: [bool; dex::HOOKS.len()] = declared_functions();
const fn declared_functions() -> [bool; dex::HOOKS.len()] {
    let mut result = [false; dex::HOOKS.len()];
    let mut effect_index = 0;
    while effect_index < IMPLEMENTATIONS.len() {
        let hooks = IMPLEMENTATIONS[effect_index].hooks;
        let mut hook_index = 0;
        while hook_index < hooks.len() {
            let index = hooks[hook_index].0 as usize;
            if index < result.len() {
                result[index] = true;
            }
            hook_index += 1;
        }
        effect_index += 1;
    }
    result
}

/// Ports callback value/function selection in battle.ts:641-646,903-910.
/// PRNG: constant/absent sites draw nothing; functions make their source draws.
/// Event entry/exit and undefined-relay propagation belong to event::dispatch.
#[inline]
pub fn dispatch_hook<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let entry = dex::HOOKS
        .get(usize::from(hook.0))
        .unwrap_or_else(|| panic!("invalid generated HookId {}", hook.0));
    match entry.value {
        HookValue::Absent => Relay::Undefined,
        HookValue::Constant(value) => {
            #[cfg(feature = "trace-hooks")]
            record_reached_hook(hook);
            constant_relay(value)
        }
        HookValue::Function => {
            #[cfg(feature = "trace-hooks")]
            record_reached_hook(hook);
            if !IMPLEMENTED[usize::from(hook.0)] {
                reached_unimplemented_hook(hook);
            }
            dispatch_function(hook, b, cx)
        }
    }
}

/// Translate the closed constant-value set without allocating or drawing.
/// Ports battle.ts:645,909; constant recharge is conditions.ts:371-376.
fn constant_relay(value: DataValue) -> Relay {
    match value {
        DataValue::Null => Relay::Null,
        DataValue::Bool(value) => Relay::Bool(value),
        DataValue::Number(value) => Relay::Number(value),
        DataValue::Text("") => Relay::NotFail,
        DataValue::Text("recharge") => Relay::PseudoMove(SyntheticEffect::Recharge),
        _ => panic!("unsupported generated constant callback value: {value:?}"),
    }
}

#[cold]
#[inline(never)]
fn reached_unimplemented_hook(hook: HookId) -> ! {
    let entry = &dex::HOOKS[usize::from(hook.0)];
    let effect = dex::effect(entry.effect);
    let classification = match hook_coverage(hook) {
        HookCoverage::Waived(waiver) => format!(
            "explicit {:?} waiver was reached: {}",
            waiver.reason, waiver.explanation
        ),
        HookCoverage::Pending(waiver) => format!(
            "{}: {}; assigned in {}",
            waiver.stage, waiver.explanation, waiver.plan
        ),
        _ => "function declaration is missing; run effects::audit_registry()".into(),
    };
    panic!(
        "unimplemented effect hook {}: {}:{} {} site={:?}; {}",
        hook.0,
        kind_name(entry.effect.kind().expect("generated effect kind")),
        effect.key,
        entry.key,
        entry.site,
        classification
    )
}

/// Disposition of any manifest site. PRNG: none; boundary/debug operation.
/// It is source-port status, not a reachability claim.
pub fn hook_coverage(hook: HookId) -> HookCoverage {
    let entry = dex::HOOKS
        .get(usize::from(hook.0))
        .unwrap_or_else(|| panic!("invalid generated HookId {}", hook.0));
    match entry.value {
        HookValue::Absent => HookCoverage::MetadataOnly,
        HookValue::Constant(value) => HookCoverage::Constant(value),
        HookValue::Function if IMPLEMENTED[usize::from(hook.0)] => HookCoverage::Implemented,
        HookValue::Function => {
            if let Some(waiver) = IMPLEMENTATIONS
                .iter()
                .flat_map(|effect| effect.waivers)
                .find(|waiver| waiver.hook == hook)
            {
                HookCoverage::Waived(*waiver)
            } else {
                HookCoverage::Pending(INCOMPLETE_PORT_WAIVER.unwrap_or(PendingPortWaiver {
                    stage: "Unaccounted manifest gap",
                    explanation: "No implementation or explicit waiver exists.",
                    plan: "effects::require_complete_registry()",
                }))
            }
        }
    }
}

/// Iterate pending functions with exact generated effect/key/site identity.
/// PRNG: none; no allocation. Use at a boundary for coverage reports.
pub fn pending_hooks() -> impl Iterator<Item = (HookId, &'static dex::Hook)> {
    dex::HOOKS.iter().enumerate().filter_map(|(index, entry)| {
        let hook = HookId(index as u16);
        matches!(hook_coverage(hook), HookCoverage::Pending(_)).then_some((hook, entry))
    })
}

/// Audit the full manifest, filenames, duplicate declarations and payloads.
/// PRNG: none; boundary-only allocation gives actionable diagnostics.
/// A whole-file pending waiver is reported, never counted as implemented.
pub fn audit_registry() -> Result<ManifestCoverage, RegistryAuditError> {
    audit_implementations(IMPLEMENTATIONS, INCOMPLETE_PORT_WAIVER)
}

fn audit_implementations(
    implementations: &[EffectImpl],
    incomplete: Option<PendingPortWaiver>,
) -> Result<ManifestCoverage, RegistryAuditError> {
    let mut owners = vec![None; dex::MANIFESTS.len()];
    let mut assigned = vec![0u8; dex::HOOKS.len()];
    let mut coverage = ManifestCoverage {
        registered_effects: implementations.len(),
        ..Default::default()
    };
    for effect in implementations {
        let index = usize::from(effect.id.0);
        let Some(kind) = effect.id.kind() else {
            return Err(RegistryAuditError(format!(
                "{} declares invalid EffectId {}",
                effect.file, effect.id.0
            )));
        };
        if let Some(prior) = owners[index].replace(effect.file) {
            return Err(RegistryAuditError(format!(
                "duplicate EffectId {} in {} and {}",
                effect.id.0, prior, effect.file
            )));
        }
        let expected_file = format!(
            "effects/{}/{}.rs",
            kind_name(kind),
            dex::effect(effect.id).key
        );
        if effect.file != expected_file {
            return Err(RegistryAuditError(format!(
                "{} declares {}, expected filename {}",
                effect.file, effect.id.0, expected_file
            )));
        }
        if effect.payload_words > 4 {
            return Err(RegistryAuditError(format!(
                "{} uses {} payload words; capacity is four",
                effect.file, effect.payload_words
            )));
        }
        for &hook in effect.hooks {
            assign_hook(effect, hook, 1, &mut assigned)?;
            coverage.implemented_functions += 1;
        }
        for waiver in effect.waivers {
            if waiver.explanation.trim().is_empty() {
                return Err(RegistryAuditError(format!(
                    "{} waives hook {} without a source explanation",
                    effect.file, waiver.hook.0
                )));
            }
            assign_hook(effect, waiver.hook, 2, &mut assigned)?;
            coverage.waived_functions += 1;
        }
        for (index, hook) in dex::HOOKS.iter().enumerate() {
            if hook.effect == effect.id
                && matches!(hook.value, HookValue::Function)
                && assigned[index] == 0
            {
                return Err(RegistryAuditError(format!(
                    "{} omits function HookId {} {} site={:?}; declare it in HOOKS or an evidenced WAIVERS entry",
                    effect.file, index, hook.key, hook.site
                )));
            }
        }
    }
    if let Some(waiver) = incomplete {
        if waiver.stage.trim().is_empty()
            || waiver.explanation.trim().is_empty()
            || waiver.plan.trim().is_empty()
        {
            return Err(RegistryAuditError(
                "central incomplete-port waiver requires stage, explanation and assignment plan"
                    .into(),
            ));
        }
    }
    for (index, hook) in dex::HOOKS.iter().enumerate() {
        match hook.value {
            HookValue::Function => {
                coverage.functions += 1;
                if assigned[index] == 0 {
                    coverage.pending_functions += 1;
                    if incomplete.is_none() {
                        return Err(RegistryAuditError(format!(
                            "unaccounted function HookId {} {}:{} {} site={:?}; add the assigned file or an explicit source waiver",
                            index,
                            kind_name(hook.effect.kind().unwrap()),
                            dex::effect(hook.effect).key,
                            hook.key,
                            hook.site
                        )));
                    }
                }
            }
            HookValue::Constant(value) => {
                coverage.constants += 1;
                if !matches!(
                    value,
                    DataValue::Null
                        | DataValue::Bool(_)
                        | DataValue::Number(_)
                        | DataValue::Text("" | "recharge")
                ) {
                    return Err(RegistryAuditError(format!(
                        "HookId {index} has an unsupported constant {value:?}"
                    )));
                }
            }
            HookValue::Absent => coverage.metadata_only += 1,
        }
    }
    Ok(coverage)
}

fn assign_hook(
    effect: &EffectImpl,
    hook: HookId,
    disposition: u8,
    assigned: &mut [u8],
) -> Result<(), RegistryAuditError> {
    let Some(entry) = dex::HOOKS.get(usize::from(hook.0)) else {
        return Err(RegistryAuditError(format!(
            "{} declares invalid HookId {}",
            effect.file, hook.0
        )));
    };
    if entry.effect != effect.id {
        return Err(RegistryAuditError(format!(
            "{} claims HookId {} owned by EffectId {}",
            effect.file, hook.0, entry.effect.0
        )));
    }
    if entry.value != HookValue::Function {
        return Err(RegistryAuditError(format!(
            "{} claims non-function HookId {}; constants/metadata belong to the manifest",
            effect.file, hook.0
        )));
    }
    if assigned[usize::from(hook.0)] != 0 {
        return Err(RegistryAuditError(format!(
            "{} declares HookId {} twice or overlaps HOOKS/WAIVERS",
            effect.file, hook.0
        )));
    }
    assigned[usize::from(hook.0)] = disposition;
    Ok(())
}

const fn kind_name(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Species => "species",
        EffectKind::Move => "moves",
        EffectKind::Ability => "abilities",
        EffectKind::Item => "items",
        EffectKind::Condition => "conditions",
        EffectKind::Rule => "rules",
    }
}

/// Completion gate independent of the temporary accounting waiver.
/// PRNG: none. The error lists every pending HookId/effect/key/site and its plan.
pub fn require_complete_registry() -> Result<ManifestCoverage, RegistryAuditError> {
    let coverage = audit_registry()?;
    if coverage.complete() {
        return Ok(coverage);
    }
    use core::fmt::Write;
    let mut report = format!(
        "effect registry incomplete: {}/{} function sites implemented, {} explicit source waivers, {} pending; constants={}, metadata-only={}.\nAssignments: docs/design/EFFECT-BATCHES.json\n",
        coverage.implemented_functions,
        coverage.functions,
        coverage.waived_functions,
        coverage.pending_functions,
        coverage.constants,
        coverage.metadata_only
    );
    for (id, hook) in pending_hooks() {
        writeln!(
            report,
            "  HookId {} {}:{} {} site={:?}",
            id.0,
            kind_name(hook.effect.kind().unwrap()),
            dex::effect(hook.effect).key,
            hook.key,
            hook.site
        )
        .unwrap();
    }
    Err(RegistryAuditError(report))
}

/// True only with `--features trace-hooks`; default dispatch has no tracing code.
pub const HOOK_TRACING_ENABLED: bool = cfg!(feature = "trace-hooks");
#[cfg(feature = "trace-hooks")]
static REACHED: [std::sync::atomic::AtomicU64; dex::HOOKS.len().div_ceil(64)] =
    [const { std::sync::atomic::AtomicU64::new(0) }; dex::HOOKS.len().div_ceil(64)];

#[cfg(feature = "trace-hooks")]
#[inline]
fn record_reached_hook(hook: HookId) {
    use std::sync::atomic::Ordering;
    let index = usize::from(hook.0);
    REACHED[index / 64].fetch_or(1u64 << (index % 64), Ordering::Relaxed);
}

/// Sorted process-wide union of invoked function/constant HookIds, including gaps.
/// PRNG: none. Boundary allocation; disabled builds return an empty Vec.
pub fn reached_hooks() -> Vec<HookId> {
    #[cfg(feature = "trace-hooks")]
    {
        use std::sync::atomic::Ordering;
        REACHED
            .iter()
            .enumerate()
            .flat_map(|(word_index, word)| {
                let bits = word.load(Ordering::Relaxed);
                (0..64).filter_map(move |bit| {
                    let index = word_index * 64 + bit;
                    (index < dex::HOOKS.len() && bits & (1u64 << bit) != 0)
                        .then_some(HookId(index as u16))
                })
            })
            .collect()
    }
    #[cfg(not(feature = "trace-hooks"))]
    {
        Vec::new()
    }
}

/// Clear debug process-wide union only while all battle workers are paused.
/// PRNG: none. Disabled builds do nothing; tracing is not in a battle snapshot.
pub fn clear_reached_hooks() {
    #[cfg(feature = "trace-hooks")]
    for word in &REACHED {
        word.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_effect_files_match_the_whole_manifest() {
        let coverage = audit_registry().unwrap();
        assert_eq!(
            coverage.functions,
            coverage.implemented_functions + coverage.waived_functions + coverage.pending_functions
        );
        assert_eq!(
            coverage.functions + coverage.constants + coverage.metadata_only,
            dex::HOOKS.len()
        );
        assert_eq!(coverage.pending_functions, pending_hooks().count());
        for (index, hook) in dex::HOOKS.iter().enumerate() {
            assert_eq!(
                matches!(hook.value, HookValue::Function),
                matches!(
                    hook_coverage(HookId(index as u16)),
                    HookCoverage::Implemented | HookCoverage::Waived(_) | HookCoverage::Pending(_)
                )
            );
        }
        if !coverage.complete() {
            let report = require_complete_registry().unwrap_err().0;
            assert!(report.contains("effect registry incomplete"));
            assert!(report.contains("EFFECT-BATCHES.json"));
            let first = pending_hooks().next().unwrap().0;
            assert!(report.contains(&format!("HookId {} ", first.0)));
        }
    }

    #[test]
    fn incomplete_policy_does_not_cover_partial_files_or_bad_declarations() {
        let effect = EffectImpl {
            file: "effects/abilities/adaptability.rs",
            id: dex::ABILITY_ADAPTABILITY,
            hooks: &[],
            payload_words: 0,
            waivers: &[],
        };
        assert!(
            audit_implementations(&[effect], INCOMPLETE_PORT_WAIVER)
                .unwrap_err()
                .0
                .contains("omits function")
        );
        assert!(
            audit_implementations(&[], None)
                .unwrap_err()
                .0
                .contains("unaccounted function")
        );
        let bad_file = EffectImpl {
            file: "effects/abilities/wrong.rs",
            ..effect
        };
        assert!(
            audit_implementations(&[bad_file], INCOMPLETE_PORT_WAIVER)
                .unwrap_err()
                .0
                .contains("expected filename")
        );
        let overflow = EffectImpl {
            payload_words: 5,
            ..effect
        };
        assert!(
            audit_implementations(&[overflow], INCOMPLETE_PORT_WAIVER)
                .unwrap_err()
                .0
                .contains("capacity is four")
        );
    }

    #[test]
    fn constant_values_preserve_source_sentinels() {
        assert_eq!(constant_relay(DataValue::Null), Relay::Null);
        assert_eq!(constant_relay(DataValue::Bool(false)), Relay::FAIL);
        assert_eq!(
            constant_relay(DataValue::Number(0.0)),
            Relay::HIT_SUBSTITUTE
        );
        assert_eq!(constant_relay(DataValue::Text("")), Relay::NotFail);
        assert_eq!(
            constant_relay(DataValue::Text("recharge")),
            Relay::PseudoMove(SyntheticEffect::Recharge)
        );
        for entry in dex::HOOKS {
            if let HookValue::Constant(value) = entry.value {
                let _ = constant_relay(value);
            }
        }
    }

    #[test]
    fn unsupported_constant_is_never_silently_coerced() {
        assert!(std::panic::catch_unwind(|| constant_relay(DataValue::Text("unknown"))).is_err());
    }

    #[cfg(feature = "trace-hooks")]
    #[test]
    fn tracing_records_hook_ids_across_word_boundaries() {
        // No clear: other tests may concurrently populate the process-wide union.
        for index in [0, 63, 64, dex::HOOKS.len() - 1] {
            let id = HookId(index as u16);
            record_reached_hook(id);
            assert!(reached_hooks().contains(&id));
        }
    }
}
