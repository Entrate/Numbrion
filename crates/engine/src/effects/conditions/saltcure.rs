//! Ports data/moves.ts:15641 (Salt Cure's embedded volatile condition). No direct PRNG draws;
//! `damage` and `hasType` run their events, whose listeners may speed-sort ties.
//!
//! State: stateless (`PAYLOAD_WORDS = 0`); `noCopy: true` and the 100% `secondary.volatileStatus`
//! are declarative data on the move/condition rows, handled by the generic pipeline.
use crate::effects::registry::abilities_baddreams::support::{TYPE_STEEL, TYPE_WATER, base_max_hp};
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::CONDITION_SALTCURE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_SALTCURE_ONSTART,
    dex::HOOK_CONDITION_SALTCURE_ONRESIDUAL,
    dex::HOOK_CONDITION_SALTCURE_ONEND,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_CONDITION_SALTCURE_ONSTART => on_start(b, cx),
        dex::HOOK_CONDITION_SALTCURE_ONRESIDUAL => on_residual(b, cx),
        dex::HOOK_CONDITION_SALTCURE_ONEND => on_end(b, cx),
        _ => panic!("unexpected saltcure hook"),
    }
}

// data/moves.ts:15643-15645 onStart(pokemon). PRNG: none.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // this.add('-start', pokemon, 'Salt Cure');
    b.add(LogEntry::new(
        "-start",
        &[LogArg::Mon(pokemon), LogArg::Text("Salt Cure")],
        &[],
    ));
    Relay::Undefined
}

// data/moves.ts:15647-15649 onResidual(pokemon). PRNG: none directly. Residual order 13 is
// manifest metadata.
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // this.damage(pokemon.baseMaxhp / (pokemon.hasType(['Water', 'Steel']) ? 4 : 8));
    // target/source/effect default from the Residual event (effect = the Salt Cure volatile).
    let divisor = if b.has_type(pokemon, &[TYPE_WATER, TYPE_STEEL]) {
        4.0
    } else {
        8.0
    };
    let amount = base_max_hp(b, pokemon) / divisor;
    b.damage(amount, None, Attribution::DEFAULT);
    Relay::Undefined
}

// data/moves.ts:15650-15652 onEnd(pokemon). PRNG: none.
fn on_end<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // this.add('-end', pokemon, 'Salt Cure');
    b.add(LogEntry::new(
        "-end",
        &[LogArg::Mon(pokemon), LogArg::Text("Salt Cure")],
        &[],
    ));
    Relay::Undefined
}
