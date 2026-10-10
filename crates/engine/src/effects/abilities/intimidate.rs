//! Ports data/abilities.ts:2194 (Intimidate). No direct PRNG draws; each Atk drop runs
//! ChangeBoost/TryBoost/AfterEachBoost/AfterBoost, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::Stat,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_INTIMIDATE;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_INTIMIDATE_ONSTART];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_INTIMIDATE_ONSTART => on_start(b, cx),
        _ => panic!("unexpected intimidate hook"),
    }
}

// data/abilities.ts:2195-2208 onStart(pokemon). PRNG: none directly.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = support::mon(b, cx, 0);
    // for (const target of pokemon.adjacentFoes()): the list is captured once, before any drop.
    let foes = b.adjacent_foes(pokemon);
    let mut activated = false;
    for entry in &foes.entries[..foes.len as usize] {
        let target = support::foe(*entry);
        if !activated {
            // this.add('-ability', pokemon, 'Intimidate', 'boost');
            support::add_ability(b, pokemon, ID, true);
            activated = true;
        }
        if b.get_volatile(target, dex::CONDITION_SUBSTITUTE).is_some() {
            // this.add('-immune', target);
            support::add_immune(b, target);
        } else {
            // this.boost({ atk: -1 }, target, pokemon, null, true);
            b.boost(
                support::boosts([(Stat::Atk, -1)]),
                Some(target),
                support::by(pokemon),
                true,
                false,
            );
        }
    }
    Relay::Undefined
}
