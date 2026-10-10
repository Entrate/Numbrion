//! Ports data/moves.ts:10547 (Lunar Blessing). No direct PRNG draws; `heal` and `cureStatus`
//! run their events, whose listeners may speed-sort ties.
use crate::{
    Battle,
    actions::HealEffect,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_LUNARBLESSING;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_LUNARBLESSING_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_LUNARBLESSING_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected lunarblessing hook"),
    }
}

// data/moves.ts:10556-10559 onHit(pokemon). PRNG: none directly.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // const success = !!this.heal(this.modify(pokemon.maxhp, 0.25));
    let amount = b.modify(
        f64::from(b.state.pokemon[pokemon.0 as usize].max_hp),
        0.25,
        1.0,
    );
    let success = b.heal(amount, None, None, HealEffect::Context).truthy();
    // return pokemon.cureStatus() || success;   (cureStatus runs first; `||` yields a boolean)
    let cured = b.cure_status(pokemon, false);
    Relay::Bool(cured || success)
}
