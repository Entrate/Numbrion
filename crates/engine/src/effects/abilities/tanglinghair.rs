//! Ports data/abilities.ts:4906 (Tangling Hair). No direct PRNG draws; the Spe drop runs the
//! usual boost events, whose listeners may shuffle speed ties.
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::{MoveHandle, Stat},
    dex::{self, HookId},
    effects::{HookWaiver, support::move_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_TANGLINGHAIR;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_TANGLINGHAIR_ONDAMAGINGHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_TANGLINGHAIR_ONDAMAGINGHIT => on_damaging_hit(b, cx),
        _ => panic!("unexpected tanglinghair hook"),
    }
}

// data/abilities.ts:4907-4912 onDamagingHit(damage, target, source, move). PRNG: none directly.
fn on_damaging_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = support::mon(b, cx, 1);
    let source = support::mon(b, cx, 2);
    let mv = MoveHandle(move_arg(b, cx, 3));
    // if (this.checkMoveMakesContact(move, source, target, true))
    if b.check_move_makes_contact(mv, source, target, true) {
        // this.add('-ability', target, 'Tangling Hair');
        support::add_ability(b, target, ID, false);
        // this.boost({ spe: -1 }, source, target, null, true);
        b.boost(
            support::boosts([(Stat::Spe, -1)]),
            Some(source),
            support::by(target),
            true,
            false,
        );
    }
    Relay::Undefined
}
