//! Yawn volatile (conditions:yawn). Ports the embedded `condition` of data/moves.ts:21146-21157
//! (`noCopy: true` and `duration: 2` are declarative; `onResidualOrder: 23` is an ordering
//! constant carried by the manifest, not a function site).
//!
//! Payload: none. The Yawn user is the cell's common `source` field (set by `addVolatile`).
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::{EffectId, Holder, MonId},
    log::{LogArg, LogEntry, LogSink, LogTag},
};

pub const ID: EffectId = dex::CONDITION_YAWN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_YAWN_ONSTART,
    dex::HOOK_CONDITION_YAWN_ONEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_CONDITION_YAWN_ONSTART => on_start(b, cx),
        dex::HOOK_CONDITION_YAWN_ONEND => on_end(b, cx),
        _ => panic!("unexpected Yawn condition function site"),
    }
}

/// data/moves.ts:21149-21151, `onStart(target, source)`:
/// `this.add('-start', target, 'move: Yawn', `[of] ${source}`)`. PRNG: none. Returns undefined.
/// `addVolatile` always supplies a source (defaulting to the target), so the `[of] undefined`
/// branch is only a defensive rendering of `${undefined}`.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = Battle::<L>::arg_mon(b.event_arg(cx, 1));
    let of = source.map_or(LogTag::Value("of", LogArg::Text("undefined")), LogTag::Of);
    b.add(LogEntry::new(
        "-start",
        &[LogArg::Mon(target), LogArg::Text("move: Yawn")],
        &[of],
    ));
    Relay::Undefined
}

/// data/moves.ts:21153-21156, `onEnd(target)` (called when `duration` runs out in the
/// residual, or by removeVolatile):
/// `this.add('-end', target, 'move: Yawn', '[silent]'); target.trySetStatus('slp', this.effectState.source);`
/// PRNG: none directly; a successful sleep draws `random(2,5)` in the Sleep onStart.
/// The source effect is left unset so setStatus defaults it to the running effect (the Yawn
/// condition), which is why the resulting `-status|slp` carries no `[from]`.
fn on_end<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    b.add(LogEntry::new(
        "-end",
        &[LogArg::Mon(target), LogArg::Text("move: Yawn")],
        &[LogTag::Bare("silent")],
    ));
    let source = b.hook_state(cx).source;
    let source = if source == MonId::NONE {
        EventArg::Undefined
    } else {
        EventArg::Holder(Holder::mon(source))
    };
    b.try_set_status(
        target,
        dex::CONDITION_SLP,
        Attribution {
            source,
            effect: EffectRef::None,
        },
    );
    Relay::Undefined
}

#[cfg(test)]
#[path = "rulesstatus/tests.rs"]
mod scenario_tests;
