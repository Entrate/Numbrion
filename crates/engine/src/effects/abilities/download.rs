//! Ports data/abilities.ts:1017 (Download). No direct PRNG draws; the getStat reads below
//! are `unmodified` (no ModifyDef/ModifySpD events) and the boost runs the usual boost events.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::{Attribution, Stat, StatOptions},
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_DOWNLOAD;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_DOWNLOAD_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_DOWNLOAD_ONSTART => on_start(b, cx),
        _ => panic!("unexpected download hook"),
    }
}

// data/abilities.ts:1018-1030 onStart(pokemon). PRNG: none directly.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = support::mon(b, cx, 0);
    let mut totaldef = 0.0;
    let mut totalspd = 0.0;
    // for (const target of pokemon.foes())  (living active foes, captured once)
    let foes = b.foes(pokemon, false);
    for entry in &foes.entries[..foes.len as usize] {
        let target = support::foe(*entry);
        // target.getStat('def', false, true)
        let options = StatOptions {
            unboosted: false,
            unmodified: true,
        };
        totaldef += b.get_stat(target, Stat::Def, options);
        totalspd += b.get_stat(target, Stat::SpD, options);
    }
    // `totaldef && totaldef >= totalspd` / `totalspd` are JS truthiness of numbers.
    if totaldef != 0.0 && !f64::is_nan(totaldef) && totaldef >= totalspd {
        // this.boost({ spa: 1 });
        b.boost(
            support::boosts([(Stat::SpA, 1)]),
            None,
            Attribution::DEFAULT,
            false,
            false,
        );
    } else if totalspd != 0.0 && !f64::is_nan(totalspd) {
        // this.boost({ atk: 1 });
        b.boost(
            support::boosts([(Stat::Atk, 1)]),
            None,
            Attribution::DEFAULT,
            false,
            false,
        );
    }
    Relay::Undefined
}
