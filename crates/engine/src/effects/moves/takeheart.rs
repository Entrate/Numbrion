//! Ports data/moves.ts:18929 (Take Heart). No direct PRNG draws; `boost` and `cureStatus` run
//! their events, whose listeners may speed-sort ties.
use crate::effects::registry::abilities_baddreams::support::boosts;
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_TAKEHEART;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_TAKEHEART_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_TAKEHEART_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected takeheart hook"),
    }
}

// data/moves.ts:18938-18941 onHit(pokemon). PRNG: none directly.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // const success = !!this.boost({ spa: 1, spd: 1 });  target/source/effect default from the event
    let success = b
        .boost(
            boosts([(Stat::SpA, 1), (Stat::SpD, 1)]),
            None,
            Attribution::DEFAULT,
            false,
            false,
        )
        .truthy();
    // return pokemon.cureStatus() || success;
    let cured = b.cure_status(pokemon, false);
    Relay::Bool(cured || success)
}
