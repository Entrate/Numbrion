//! Ports data/items.ts:7596 (Weakness Policy). No direct PRNG draws; useItem's events/callbacks
//! (UseItem, boost, Use, AfterUseItem) retain theirs.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    actions::{Attribution, MoveHandle},
    dex::{self, HookId, HookRel},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::{EffectId, EventId},
    log::LogSink,
};
pub const ID: EffectId = dex::ITEM_WEAKNESSPOLICY;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_WEAKNESSPOLICY_ONDAMAGINGHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:7602 onDamagingHit(damage, target, source, move):
        //   if (!move.damage && !move.damageCallback && target.getMoveHitData(move).typeMod > 0)
        //       target.useItem();
        // `move.damage` is truthy for 'level' and any non-zero fixed amount; `damageCallback` is
        // the move's direct DamageCallback hook. getMoveHitData lazily creates the per-target
        // record exactly like TS. The item's `boosts: { atk: 2, spa: 2 }` are applied by useItem.
        // Returns undefined. PRNG: none directly.
        dex::HOOK_ITEM_WEAKNESSPOLICY_ONDAMAGINGHIT => {
            let target = support::mon(b, cx, 1);
            let handle = move_arg(b, cx, 3);
            let no_fixed_damage = match move_overlay(b, handle).damage {
                dex::DamageSpec::None => true,
                dex::DamageSpec::Fixed(n) => n == 0,
                dex::DamageSpec::Level => false,
            };
            if no_fixed_damage
                && b.event_hook(
                    EffectRef::ActiveMove(handle),
                    EventId::DamageCallback,
                    HookRel::Direct,
                )
                .is_none()
                && b.move_hit_data(target, MoveHandle(handle)).type_mod > 0
            {
                b.use_item(target, Attribution::DEFAULT);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Weakness Policy hook"),
    }
}
