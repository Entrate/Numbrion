//! Ports data/moves.ts:3266-3314 (`curse`): onModifyMove, onTryHit, onHit.
//! Payload: none (PAYLOAD_WORDS = 0). Direct PRNG draws: none. Nested: `hasType` Type events may sort ties.
//! The embedded condition (Ghost-type curse) is `conditions/curse.rs`.
use crate::{
    Battle,
    actions::{Attribution, MoveHandle, moves::EMPTY_EFFECTS},
    dex::{self, HookId, MoveTarget},
    effects::{
        HookWaiver,
        registry::conditions_choicelock::support::{TYPE_GHOST, opt_mon_arg},
        support::{mon_arg, move_arg},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::{EffectId, MonId},
    log::LogSink,
    state::scratch::{MoveEffectsScratch, OrderedBoosts},
};
pub const ID: EffectId = dex::MOVE_CURSE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_CURSE_ONMODIFYMOVE,
    dex::HOOK_MOVE_CURSE_ONTRYHIT,
    dex::HOOK_MOVE_CURSE_ONHIT,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// `move.self = { boosts: { spe: -1, atk: 1, def: 1 } }`: boost keys are stat index - 1 (atk 0, def 1, spe 4)
/// and object insertion order is spe, atk, def (data/moves.ts:3287).
fn curse_self_effect() -> MoveEffectsScratch {
    let mut boosts = OrderedBoosts::default();
    for (key, delta) in [(4u8, -1i8), (0, 1), (1, 1)] {
        boosts.values[key as usize] = delta;
        boosts.order[boosts.len as usize] = key;
        boosts.len += 1;
        boosts.present |= 1 << key;
    }
    MoveEffectsScratch {
        base: &EMPTY_EFFECTS,
        boosts,
        status: EffectId::NONE,
        volatile_status: EffectId::NONE,
        side_condition: EffectId::NONE,
        slot_condition: EffectId::NONE,
        weather: EffectId::NONE,
        terrain: EffectId::NONE,
        pseudo_weather: EffectId::NONE,
        chance: None,
        heal: None,
        force_switch: false,
        self_switch: dex::SelfSwitch::None,
        suppressed_hooks: 0,
    }
}
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:3277-3283 onModifyMove(move, source, target). PRNG: none directly (Type event).
        // Non-Ghost users retarget to self; a Ghost with no target (or an ally) picks randomNormal.
        dex::HOOK_MOVE_CURSE_ONMODIFYMOVE => {
            let mv = move_arg(b, cx, 0);
            let source = mon_arg(b, cx, 1);
            let target = opt_mon_arg(b, cx, 2);
            if !b.has_type(source, &[TYPE_GHOST]) {
                b.active_move_mut(MoveHandle(mv)).target = MoveTarget::SelfTarget;
            } else if target.is_none() || b.is_ally(source, target) {
                b.active_move_mut(MoveHandle(mv)).target = MoveTarget::RandomNormal;
            }
            Relay::Undefined
        }
        // data/moves.ts:3284-3293 onTryHit(target, source, move). PRNG: none directly (Type event).
        // Non-Ghost: `delete move.volatileStatus; delete move.onHit; move.self = {boosts}` (the deleted onHit is
        // bit i of `effects.suppressed_hooks`, i = its index in `effects.base.hooks`); returns undefined.
        // Ghost already cursing the target: return false (the move fails after its `|move|` line).
        dex::HOOK_MOVE_CURSE_ONTRYHIT => {
            let target = mon_arg(b, cx, 0);
            let source = mon_arg(b, cx, 1);
            let mv = move_arg(b, cx, 2);
            if !b.has_type(source, &[TYPE_GHOST]) {
                let m = b.active_move_mut(MoveHandle(mv));
                m.effects.volatile_status = EffectId::NONE;
                if let Some(i) = m
                    .effects
                    .base
                    .hooks
                    .iter()
                    .position(|h| *h == dex::HOOK_MOVE_CURSE_ONHIT)
                {
                    m.effects.suppressed_hooks |= 1 << i;
                }
                m.self_effect = Some(curse_self_effect());
            } else {
                let has_volatile_status =
                    b.active_move(MoveHandle(mv)).effects.volatile_status != EffectId::NONE;
                if has_volatile_status && b.get_volatile(target, dex::CONDITION_CURSE).is_some() {
                    return Relay::FAIL;
                }
            }
            Relay::Undefined
        }
        // data/moves.ts:3294-3296 onHit(target, source): `this.directDamage(source.maxhp / 2, source, source)`.
        // The Ghost-type cost: half of the user's max HP, rounded down (min 1), logged as plain `-damage`.
        // PRNG: none directly; a faint is queued by the core mutator.
        dex::HOOK_MOVE_CURSE_ONHIT => {
            let source: MonId = mon_arg(b, cx, 1);
            let half = f64::from(b.state.pokemon[source.0 as usize].max_hp) / 2.0;
            b.direct_damage(
                half,
                Some(source),
                Attribution::from_move(source, EffectRef::None),
            );
            Relay::Undefined
        }
        _ => panic!("unexpected Curse function site"),
    }
}
