//! Ports data/abilities.ts:1098 (Dry Skin). No direct PRNG draws; `heal`/`damage` run their
//! TryHeal/Heal/Damage events, whose listeners may speed-sort ties.
//!
//! Primordial Sea and Desolate Land do not exist in the scoped dex (no condition record), so the
//! `effect.id === 'primordialsea' / 'desolateland'` alternatives of `onWeather` can never match.
use crate::effects::registry::abilities_baddreams::support::{
    TYPE_FIRE, TYPE_WATER, base_max_hp, effect_at,
};
use crate::{
    Battle,
    actions::{Attribution, HealEffect},
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg, move_overlay},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::ABILITY_DRYSKIN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_DRYSKIN_ONTRYHIT,
    dex::HOOK_ABILITY_DRYSKIN_ONSOURCEBASEPOWER,
    dex::HOOK_ABILITY_DRYSKIN_ONWEATHER,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_DRYSKIN_ONTRYHIT => on_try_hit(b, cx),
        dex::HOOK_ABILITY_DRYSKIN_ONSOURCEBASEPOWER => on_source_base_power(b, cx),
        dex::HOOK_ABILITY_DRYSKIN_ONWEATHER => on_weather(b, cx),
        _ => panic!("unexpected dryskin hook"),
    }
}

// data/abilities.ts:1099-1107 onTryHit(target, source, move). PRNG: none directly.
fn on_try_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let mv = move_arg(b, cx, 2);
    // target !== source && move.type === 'Water'
    if target != source && move_overlay(b, mv).move_type == TYPE_WATER {
        // if (!this.heal(target.baseMaxhp / 4)) this.add('-immune', target, '[from] ability: Dry Skin');
        let amount = base_max_hp(b, target) / 4.0;
        if !b.heal(amount, None, None, HealEffect::Context).truthy() {
            b.add(LogEntry::new(
                "-immune",
                &[LogArg::Mon(target)],
                &[LogTag::From(EffectRef::Dex(ID))],
            ));
        }
        // return null;
        return Relay::Null;
    }
    Relay::Undefined
}

// data/abilities.ts:1108-1112 onSourceBasePower(basePower, attacker, defender, move). PRNG: none.
fn on_source_base_power<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 3);
    if move_overlay(b, mv).move_type == TYPE_FIRE {
        // return this.chainModify(1.25);  (mutates the frame, returns undefined)
        b.chain_modify(1.25, 1.0);
    }
    Relay::Undefined
}

// data/abilities.ts:1113-1121 onWeather(target, source, effect). PRNG: none directly.
fn on_weather<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let weather = b.event_effect_id(effect_at(b, cx, 2));
    // if (target.effectiveWeather() !== effect.id) return;
    if b.effective_weather(target) != weather {
        return Relay::Undefined;
    }
    if weather == dex::CONDITION_RAINDANCE {
        // this.heal(target.baseMaxhp / 8);
        let amount = base_max_hp(b, target) / 8.0;
        b.heal(amount, None, None, HealEffect::Context);
    } else if weather == dex::CONDITION_SUNNYDAY {
        // this.damage(target.baseMaxhp / 8, target, target);
        let amount = base_max_hp(b, target) / 8.0;
        b.damage(
            amount,
            Some(target),
            Attribution::from_move(target, EffectRef::None),
        );
    }
    Relay::Undefined
}
