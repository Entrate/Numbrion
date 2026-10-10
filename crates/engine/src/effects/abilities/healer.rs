//! Ports data/abilities.ts:1818 (Healer). PRNG: one `randomChance(3, 10)` per adjacent ally
//! that has a status, in `adjacentAllies()` order (the draw is skipped for healthy allies
//! because JS `&&` short-circuits). `cureStatus` itself draws nothing.
use crate::effects::registry::abilities_baddreams::support::listed_mon;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
    state::Status,
};
pub const ID: EffectId = dex::ABILITY_HEALER;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_HEALER_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_HEALER_ONRESIDUAL => on_residual(b, cx),
        _ => panic!("unexpected healer hook"),
    }
}

// data/abilities.ts:1821-1828 onResidual(pokemon). PRNG: randomChance(3, 10) per statused ally.
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // for (const allyActive of pokemon.adjacentAllies()): captured once, before any cure.
    let allies = b.adjacent_allies(pokemon);
    for entry in &allies.entries[..allies.len as usize] {
        let ally = listed_mon(*entry);
        // allyActive.status && this.randomChance(3, 10)
        if b.state.pokemon[ally.0 as usize].status != Status::None
            && b.state.prng.random_chance(3, 10)
        {
            // this.add('-activate', pokemon, 'ability: Healer');
            b.add(LogEntry::new(
                "-activate",
                &[
                    LogArg::Mon(pokemon),
                    LogArg::EffectFullName(EffectRef::Dex(ID)),
                ],
                &[],
            ));
            // allyActive.cureStatus();
            b.cure_status(ally, false);
        }
    }
    Relay::Undefined
}
