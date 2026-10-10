//! Ports data/abilities.ts:4515 (Stamina). No direct PRNG draws; the Def boost runs the
//! usual boost events, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_STAMINA;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_STAMINA_ONDAMAGINGHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_STAMINA_ONDAMAGINGHIT => on_damaging_hit(b, cx),
        _ => panic!("unexpected stamina hook"),
    }
}

// data/abilities.ts:4516-4518 onDamagingHit(damage, target, source, effect). PRNG: none directly.
fn on_damaging_hit<L: LogSink>(b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    // this.boost({ def: 1 });  (target = event target, source = event source, effect = Stamina)
    b.boost(
        support::boosts([(Stat::Def, 1)]),
        None,
        Attribution::DEFAULT,
        false,
        false,
    );
    Relay::Undefined
}
