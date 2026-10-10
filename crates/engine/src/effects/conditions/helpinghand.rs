//! Ports data/moves.ts:8586-8601 (embedded volatile condition of Helping Hand).
//!
//! Payload map (PAYLOAD_WORDS = 2): words 0..1 = `effectState.multiplier` as f64 bits
//! (`words[0]` low half, `words[1]` high half); custom presence bit 8 (`MULTIPLIER_PRESENT`).
//! Direct PRNG draws: none.
use crate::effects::registry::conditions_protect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{EffectCell, present},
};
pub const ID: EffectId = dex::CONDITION_HELPINGHAND;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_HELPINGHAND_ONSTART,
    dex::HOOK_CONDITION_HELPINGHAND_ONRESTART,
    dex::HOOK_CONDITION_HELPINGHAND_ONBASEPOWER,
];
pub const PAYLOAD_WORDS: usize = 2;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// Custom presence bit 8: `effectState.multiplier` has been assigned.
const MULTIPLIER_PRESENT: u32 = 1 << present::CUSTOM_START;

/// `this.effectState.multiplier`; an unassigned property is `undefined`, i.e. NaN arithmetic.
fn multiplier(state: &EffectCell) -> f64 {
    if state.present & MULTIPLIER_PRESENT != 0 {
        f64::from_bits(
            u64::from(state.payload.words[0]) | (u64::from(state.payload.words[1]) << 32),
        )
    } else {
        f64::NAN
    }
}
fn set_multiplier(state: &mut EffectCell, value: f64) {
    let bits = value.to_bits();
    state.payload.words[0] = bits as u32;
    state.payload.words[1] = (bits >> 32) as u32;
    state.present |= MULTIPLIER_PRESENT;
}
/// `this.add('-singleturn', target, 'Helping Hand', `[of] ${source}`)`.
fn singleturn<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) {
    let target = support::mon(b, cx, 0);
    let source = support::mon(b, cx, 1);
    b.add(LogEntry::new(
        "-singleturn",
        &[LogArg::Mon(target), LogArg::Text("Helping Hand")],
        &[LogTag::Of(source)],
    ));
}
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:8588-8591 onStart(target, source): effectState.multiplier = 1.5, then the
        // `-singleturn|target|Helping Hand|[of] source` line. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_HELPINGHAND_ONSTART => {
            set_multiplier(b.hook_state_mut(cx), 1.5);
            singleturn(b, cx);
            Relay::Undefined
        }
        // data/moves.ts:8592-8595 onRestart(target, source): effectState.multiplier *= 1.5 (a
        // second Helping Hand in the same turn stacks to 2.25, then 3.375), same line again.
        // Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_HELPINGHAND_ONRESTART => {
            let state = b.hook_state_mut(cx);
            let next = multiplier(state) * 1.5;
            set_multiplier(state, next);
            singleturn(b, cx);
            Relay::Undefined
        }
        // data/moves.ts:8597-8600 onBasePower(basePower), onBasePowerPriority 10:
        // `return this.chainModify(this.effectState.multiplier)` - mutates the BasePower frame
        // (numerator multiplier, denominator 1) and returns undefined. PRNG: none.
        dex::HOOK_CONDITION_HELPINGHAND_ONBASEPOWER => {
            let value = multiplier(b.hook_state(cx));
            b.chain_modify(value, 1.0);
            Relay::Undefined
        }
        _ => panic!("unexpected Helping Hand condition function site"),
    }
}
