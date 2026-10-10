//! Ports data/moves.ts:8156-8175 (Haze). No PRNG draws and no nested events.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{HookCtx, Relay},
    ids::*,
    log::{LogEntry, LogSink},
};
pub const ID: EffectId = dex::MOVE_HAZE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_HAZE_ONHITFIELD];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_HAZE_ONHITFIELD => on_hit_field(b, cx),
        _ => panic!("unexpected Haze function site"),
    }
}

// data/moves.ts:8165-8170 onHitField(): singleEvent('HitField', moveData, {}, target, source,
// move) (battle-actions.ts:1258); it reads no argument.
//   this.add('-clearallboost');
//   for (const pokemon of this.getAllActive()) pokemon.clearBoosts();
// getAllActive() = p1a, p1b, p2a, p2b skipping empty and fainted slots (battle.ts:1369-1379).
// Returns undefined. PRNG: none.
fn on_hit_field<L: LogSink>(b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    b.add(LogEntry::new("-clearallboost", &[], &[]));
    let actives = b.get_all_active(false);
    for &pokemon in actives.as_slice() {
        b.clear_boosts(pokemon);
    }
    Relay::Undefined
}
