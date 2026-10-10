//! Ports data/abilities.ts:414 (Berserk). No direct PRNG draws; the Sp. Atk boost runs the
//! usual boost events, whose listeners may shuffle speed ties.
//!
//! Payload map (ability state): word 0 = `checkedBerserk` (0/1), presence bit 8.
//! Bit 8 clear means the property was never assigned (JS `undefined`), which is
//! observable: `onTryEatItem` then returns `undefined` rather than a boolean.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::Stat,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay, optional_id},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::{present, scratch::move_runtime},
};
pub const ID: EffectId = dex::ABILITY_BERSERK;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_BERSERK_ONDAMAGE,
    dex::HOOK_ABILITY_BERSERK_ONTRYEATITEM,
    dex::HOOK_ABILITY_BERSERK_ONAFTERMOVESECONDARY,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

/// Custom presence bit for `checkedBerserk` (EffectCell.present bits 8..30 are custom).
const CHECKED_PRESENT: u32 = 1 << present::CUSTOM_START;

/// `healingItems` of onTryEatItem. A name absent from the scoped dex resolves to
/// `EffectId::NONE`, which is filtered out below (an eaten item always has a real id).
const HEALING_ITEMS: [EffectId; 9] = [
    optional_id(dex::ITEMS_DATA, "aguavberry"),
    optional_id(dex::ITEMS_DATA, "enigmaberry"),
    optional_id(dex::ITEMS_DATA, "figyberry"),
    optional_id(dex::ITEMS_DATA, "iapapaberry"),
    optional_id(dex::ITEMS_DATA, "magoberry"),
    optional_id(dex::ITEMS_DATA, "sitrusberry"),
    optional_id(dex::ITEMS_DATA, "wikiberry"),
    optional_id(dex::ITEMS_DATA, "oranberry"),
    optional_id(dex::ITEMS_DATA, "berryjuice"),
];

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_BERSERK_ONDAMAGE => on_damage(b, cx),
        dex::HOOK_ABILITY_BERSERK_ONTRYEATITEM => on_try_eat_item(b, cx),
        dex::HOOK_ABILITY_BERSERK_ONAFTERMOVESECONDARY => on_after_move_secondary(b, cx),
        _ => panic!("unexpected berserk hook"),
    }
}

fn set_checked<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, value: bool) {
    let state = b.hook_state_mut(cx);
    state.present |= CHECKED_PRESENT;
    state.payload.words[0] = u32::from(value);
}

// data/abilities.ts:415-420 onDamage(damage, target, source, effect). PRNG: none.
fn on_damage<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let effect = support::effect_at(b, cx, 3);
    // effect.effectType === "Move" && !effect.multihit && !this.suppressingSecondaries()
    let checked = !(support::effect_is_move(b, effect)
        && !support::effect_has_multihit(b, effect)
        && !b.suppressing_secondaries());
    set_checked(b, cx, checked);
    Relay::Undefined
}

// data/abilities.ts:421-429 onTryEatItem(item). PRNG: none.
fn on_try_eat_item<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let item = support::effect_id_at(b, cx, 0);
    if item != EffectId::NONE && HEALING_ITEMS.contains(&item) {
        // return this.effectState.checkedBerserk;  (undefined when never assigned)
        let state = b.hook_state(cx);
        return if state.present & CHECKED_PRESENT != 0 {
            Relay::Bool(state.payload.words[0] != 0)
        } else {
            Relay::Undefined
        };
    }
    Relay::Bool(true)
}

// data/abilities.ts:430-439 onAfterMoveSecondary(target, source, move). PRNG: none directly.
fn on_after_move_secondary<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // this.effectState.checkedBerserk = true;
    set_checked(b, cx, true);
    let target = support::mon(b, cx, 0);
    let Some(source) = support::mon_opt(b, cx, 1) else {
        return Relay::Undefined;
    };
    let mv = move_arg(b, cx, 2);
    let m = move_overlay(b, mv);
    let (total_damage, smart_target) = (
        m.total_damage,
        m.runtime_flags & move_runtime::SMART_TARGET != 0,
    );
    let multihit = support::effect_has_multihit(b, EffectRef::ActiveMove(mv));
    // if (!source || source === target || !target.hp || !move.totalDamage) return;
    if source == target || b.state.pokemon[target.0 as usize].hp == 0 || total_damage == 0 {
        return Relay::Undefined;
    }
    // const lastAttackedBy = target.getLastAttackedBy(); if (!lastAttackedBy) return;
    let Some((_, last)) = b.state.last_attacked_by(target) else {
        return Relay::Undefined;
    };
    let last_damage = f64::from(last.last_damage);
    // const damage = move.multihit && !move.smartTarget ? move.totalDamage : lastAttackedBy.damage;
    let damage = if multihit && !smart_target {
        f64::from(total_damage)
    } else {
        last_damage
    };
    let hp = f64::from(b.state.pokemon[target.0 as usize].hp);
    let half = f64::from(b.state.pokemon[target.0 as usize].max_hp) / 2.0;
    if hp <= half && hp + damage > half {
        // this.boost({ spa: 1 }, target, target);
        b.boost(
            support::boosts([(Stat::SpA, 1)]),
            Some(target),
            support::by(target),
            false,
            false,
        );
    }
    Relay::Undefined
}
