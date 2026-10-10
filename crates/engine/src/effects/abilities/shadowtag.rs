//! Trapping and hypothetical request callbacks, data/abilities.ts:4163.
//! PRNG: none directly; groundedness/type queries dispatch core events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::mon_flags,
};
pub const ID: EffectId = dex::ABILITY_SHADOWTAG;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SHADOWTAG_ONFOETRAPPOKEMON,
    dex::HOOK_ABILITY_SHADOWTAG_ONFOEMAYBETRAPPOKEMON,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    if hook == HOOKS[0] {
        let holder = b.hook_state(cx).target;
        if holder.0 < 12
            && b.is_adjacent(target, crate::ids::MonId(holder.0))
            && !b.has_ability(target, &[ID])
        {
            b.try_trap(target, true);
        }
        return Relay::Undefined;
    }
    assert_eq!(hook, HOOKS[1]);
    let source = Battle::<L>::arg_mon(b.event_arg(cx, 1)).or_else(|| {
        let holder = b.hook_state(cx).target;
        (holder.0 < 12).then_some(crate::ids::MonId(holder.0))
    });
    let Some(source) = source else {
        return Relay::Undefined;
    };
    if !b.is_adjacent(target, source) {
        return Relay::Undefined;
    };
    if !b.has_ability(target, &[ID]) {
        b.state.pokemon[target.0 as usize].flags |= mon_flags::MAYBE_TRAPPED;
    }
    Relay::Undefined
}
