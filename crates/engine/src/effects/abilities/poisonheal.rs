//! Ports data/abilities.ts:3332 (Poison Heal). No direct PRNG draws; the `heal` runs
//! TryHeal/Heal events whose listeners may speed-sort ties.
use crate::effects::registry::abilities_baddreams::support::{base_max_hp, effect_at};
use crate::{
    Battle,
    actions::HealEffect,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_POISONHEAL;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_POISONHEAL_ONDAMAGE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_POISONHEAL_ONDAMAGE => on_damage(b, cx),
        _ => panic!("unexpected poisonheal hook"),
    }
}

// data/abilities.ts:3334-3339 onDamage(damage, target, source, effect). PRNG: none directly.
fn on_damage<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 1);
    let effect = b.event_effect_id(effect_at(b, cx, 3));
    // if (effect.id === 'psn' || effect.id === 'tox')
    if effect == dex::CONDITION_PSN || effect == dex::CONDITION_TOX {
        // this.heal(target.baseMaxhp / 8);  target/source/effect default from the Damage event
        let amount = base_max_hp(b, target) / 8.0;
        b.heal(amount, None, None, HealEffect::Context);
        // return false;
        return Relay::Bool(false);
    }
    Relay::Undefined
}
