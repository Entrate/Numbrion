//! Ports data/abilities.ts:320 (Bad Dreams). No direct PRNG draws; each foe's `damage` runs
//! Damage/AfterDamage events whose listeners may speed-sort ties.
//!
//! Also owns the batch's private helper module (`healingresidual/`), shared by the other
//! `healing_residual` files through `registry::abilities_baddreams::support`.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::Status,
};
#[path = "healingresidual/mod.rs"]
pub(super) mod support;
use support::{base_max_hp, listed_mon};
pub const ID: EffectId = dex::ABILITY_BADDREAMS;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_BADDREAMS_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_BADDREAMS_ONRESIDUAL => on_residual(b, cx),
        _ => panic!("unexpected baddreams hook"),
    }
}

// data/abilities.ts:323-330 onResidual(pokemon). PRNG: none directly (damage events only).
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // if (!pokemon.hp) return;
    if b.state.pokemon[pokemon.0 as usize].hp == 0 {
        return Relay::Undefined;
    }
    // for (const target of pokemon.foes()): the list is captured once, before any damage.
    let foes = b.foes(pokemon, false);
    for entry in &foes.entries[..foes.len as usize] {
        let target = listed_mon(*entry);
        // target.status === 'slp' || target.hasAbility('comatose')
        let asleep = b.state.pokemon[target.0 as usize].status == Status::Sleep;
        if asleep || b.has_ability(target, &[dex::ABILITY_COMATOSE]) {
            // this.damage(target.baseMaxhp / 8, target, pokemon);  effect defaults to this.effect
            b.damage(
                base_max_hp(b, target) / 8.0,
                Some(target),
                Attribution::from_move(pokemon, EffectRef::None),
            );
        }
    }
    Relay::Undefined
}
