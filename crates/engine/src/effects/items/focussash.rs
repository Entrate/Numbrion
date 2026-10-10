//! Ports data/items.ts:2269 (Focus Sash). No direct PRNG draws; useItem's events/callbacks
//! (UseItem, Use, AfterUseItem) retain theirs.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::relay_number},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ITEM_FOCUSSASH;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_FOCUSSASH_ONDAMAGE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:2276 onDamage(damage, target, source, effect) (priority -40 is manifest
        // metadata):
        //   if (target.hp === target.maxhp && damage >= target.hp && effect &&
        //       effect.effectType === 'Move') {
        //       if (target.useItem()) return target.hp - 1;
        //   }
        // Returns the new damage number or undefined. PRNG: none directly.
        dex::HOOK_ITEM_FOCUSSASH_ONDAMAGE => {
            let damage = relay_number(b, cx, 0);
            let target = support::mon(b, cx, 1);
            let effect = support::effect_at(b, cx, 3);
            let (hp, max_hp) = {
                let p = &b.state.pokemon[target.0 as usize];
                (p.hp, p.max_hp)
            };
            if hp == max_hp && damage >= f64::from(hp) && support::effect_is_move(b, effect) {
                // useItem(): source/sourceEffect default from the Damage event / this.effect.
                if b.use_item(target, Attribution::DEFAULT) {
                    // `target.hp - 1`, read after the item was used.
                    return Relay::Number(f64::from(b.state.pokemon[target.0 as usize].hp) - 1.0);
                }
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Focus Sash hook"),
    }
}
