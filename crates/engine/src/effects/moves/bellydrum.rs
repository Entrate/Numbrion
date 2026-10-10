//! Ports data/moves.ts:1216-1236 (Belly Drum). No direct PRNG draws; the boost and damage
//! mutators dispatch events whose listeners may speed-sort ties.
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::scratch::OrderedBoosts,
};
pub const ID: EffectId = dex::MOVE_BELLYDRUM;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_BELLYDRUM_ONHIT];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_BELLYDRUM_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected Belly Drum function site"),
    }
}

// data/moves.ts:1225-1231 onHit(target): Hit single event, args [target, source, move].
//   if (target.hp <= target.maxhp / 2 || target.boosts.atk >= 6 || target.maxhp === 1) return false;
//   this.directDamage(target.maxhp / 2);   // target/source/effect default from the running event
//   this.boost({ atk: 12 }, target);       // source/effect default from the running event
// The handler then falls off the end: undefined (the boost result is not returned).
// PRNG: none directly.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let p = &b.state.pokemon[target.0 as usize];
    let half = f64::from(p.max_hp) / 2.0;
    if f64::from(p.hp) <= half || p.boosts[Stat::Atk as usize] >= 6 || p.max_hp == 1 {
        return Relay::Bool(false);
    }
    b.direct_damage(half, None, Attribution::DEFAULT);
    let mut atk = OrderedBoosts::default();
    atk.values[Stat::Atk as usize] = 12;
    atk.order[0] = Stat::Atk as u8;
    atk.len = 1;
    atk.present = 1 << Stat::Atk as u8;
    b.boost(atk, Some(target), Attribution::DEFAULT, false, false);
    Relay::Undefined
}
