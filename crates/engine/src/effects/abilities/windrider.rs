//! Ports data/abilities.ts:5530 (Wind Rider). No direct PRNG draws; the Atk boosts run the
//! usual boost events, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::Stat,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_WINDRIDER;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_WINDRIDER_ONSTART,
    dex::HOOK_ABILITY_WINDRIDER_ONTRYHIT,
    dex::HOOK_ABILITY_WINDRIDER_ONSIDECONDITIONSTART,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_WINDRIDER_ONSTART => on_start(b, cx),
        dex::HOOK_ABILITY_WINDRIDER_ONTRYHIT => on_try_hit(b, cx),
        dex::HOOK_ABILITY_WINDRIDER_ONSIDECONDITIONSTART => on_side_condition_start(b, cx),
        _ => panic!("unexpected windrider hook"),
    }
}

// data/abilities.ts:5531-5535 onStart(pokemon). PRNG: none directly.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = support::mon(b, cx, 0);
    // if (pokemon.side.sideConditions['tailwind'])
    if b.get_side_condition(pokemon.side(), dex::CONDITION_TAILWIND)
        .is_some()
    {
        // this.boost({ atk: 1 }, pokemon, pokemon);
        b.boost(
            support::boosts([(Stat::Atk, 1)]),
            Some(pokemon),
            support::by(pokemon),
            false,
            false,
        );
    }
    Relay::Undefined
}

// data/abilities.ts:5536-5543 onTryHit(target, source, move). PRNG: none directly.
fn on_try_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = support::mon(b, cx, 0);
    let source = support::mon_opt(b, cx, 1);
    let flags = move_overlay(b, move_arg(b, cx, 2)).flags;
    // if (target !== source && move.flags['wind'])
    if Some(target) != source && flags & dex::FLAG_WIND != 0 {
        // if (!this.boost({ atk: 1 }, target, target)) this.add('-immune', target, '[from] ability: Wind Rider');
        let boosted = b.boost(
            support::boosts([(Stat::Atk, 1)]),
            Some(target),
            support::by(target),
            false,
            false,
        );
        if !boosted.truthy() {
            support::add_immune_from(b, target, ID);
        }
        // return null;
        return Relay::Null;
    }
    Relay::Undefined
}

// data/abilities.ts:5544-5549 onSideConditionStart(side, source, sideCondition). PRNG: none directly.
fn on_side_condition_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // const pokemon = this.effectState.target;
    let pokemon = support::owner(b, cx);
    // if (sideCondition.id === 'tailwind')
    if support::effect_id_at(b, cx, 2) == dex::CONDITION_TAILWIND {
        // this.boost({ atk: 1 }, pokemon, pokemon);
        b.boost(
            support::boosts([(Stat::Atk, 1)]),
            Some(pokemon),
            support::by(pokemon),
            false,
            false,
        );
    }
    Relay::Undefined
}
