//! Ports data/moves.ts:18773 (Syrup Bomb's embedded volatile condition). No direct PRNG draws;
//! `boost` runs ChangeBoost/TryBoost/AfterEachBoost/AfterBoost, whose listeners may speed-sort ties.
//!
//! State: stateless (`PAYLOAD_WORDS = 0`). The bomber is the common `source` field of the
//! volatile's effect cell (set by addVolatile from the secondary's source); `duration: 4` and
//! `noCopy: true` are declarative data, and the duration countdown runs in the generic
//! Residual pass.
use crate::effects::registry::abilities_baddreams::support::boosts;
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::{EffectId, MonId},
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::mon_flags,
};
pub const ID: EffectId = dex::CONDITION_SYRUPBOMB;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_SYRUPBOMB_ONSTART,
    dex::HOOK_CONDITION_SYRUPBOMB_ONUPDATE,
    dex::HOOK_CONDITION_SYRUPBOMB_ONRESIDUAL,
    dex::HOOK_CONDITION_SYRUPBOMB_ONEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_CONDITION_SYRUPBOMB_ONSTART => on_start(b, cx),
        dex::HOOK_CONDITION_SYRUPBOMB_ONUPDATE => on_update(b, cx),
        dex::HOOK_CONDITION_SYRUPBOMB_ONRESIDUAL => on_residual(b, cx),
        dex::HOOK_CONDITION_SYRUPBOMB_ONEND => on_end(b, cx),
        _ => panic!("unexpected syrupbomb hook"),
    }
}

/// `this.effectState.source`: the Pokemon that applied the volatile, `None` when the property
/// was never set (undefined).
fn effect_source<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> Option<MonId> {
    let source = b.hook_state(cx).source;
    (source != MonId::NONE).then_some(source)
}

// data/moves.ts:18773-18775 onStart(pokemon). PRNG: none.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // this.add('-start', pokemon, 'Syrup Bomb');
    b.add(LogEntry::new(
        "-start",
        &[LogArg::Mon(pokemon), LogArg::Text("Syrup Bomb")],
        &[],
    ));
    Relay::Undefined
}

// data/moves.ts:18776-18780 onUpdate(pokemon). PRNG: none directly (End runs on removal).
fn on_update<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // if (this.effectState.source && !this.effectState.source.isActive) pokemon.removeVolatile('syrupbomb');
    if let Some(source) = effect_source(b, cx) {
        if b.state.pokemon[source.0 as usize].flags & mon_flags::ACTIVE == 0 {
            b.remove_volatile(pokemon, ID);
        }
    }
    Relay::Undefined
}

// data/moves.ts:18782-18784 onResidual(pokemon). PRNG: none directly. Residual order 14 is
// manifest metadata.
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // this.boost({ spe: -1 }, pokemon, this.effectState.source);  effect defaults to this.effect.
    // An unset source (undefined) falls back to the event source (null for Residual).
    let attribution = match effect_source(b, cx) {
        Some(source) => Attribution::from_move(source, EffectRef::None),
        None => Attribution::DEFAULT,
    };
    b.boost(
        boosts([(Stat::Spe, -1)]),
        Some(pokemon),
        attribution,
        false,
        false,
    );
    Relay::Undefined
}

// data/moves.ts:18785-18787 onEnd(pokemon). PRNG: none.
fn on_end<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // this.add('-end', pokemon, 'Syrup Bomb', '[silent]');
    b.add(LogEntry::new(
        "-end",
        &[LogArg::Mon(pokemon), LogArg::Text("Syrup Bomb")],
        &[LogTag::Bare("silent")],
    ));
    Relay::Undefined
}
