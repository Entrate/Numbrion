//! Ports data/abilities.ts:4188 (Shed Skin). PRNG: one `randomChance(33, 100)`, drawn only when
//! the holder is alive and statused (JS `&&` short-circuits). `cureStatus` itself draws nothing.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
    state::Status,
};
pub const ID: EffectId = dex::ABILITY_SHEDSKIN;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_SHEDSKIN_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_SHEDSKIN_ONRESIDUAL => on_residual(b, cx),
        _ => panic!("unexpected shedskin hook"),
    }
}

// data/abilities.ts:4191-4197 onResidual(pokemon). PRNG: randomChance(33, 100) if hp && status.
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    let mon = &b.state.pokemon[pokemon.0 as usize];
    // pokemon.hp && pokemon.status && this.randomChance(33, 100)
    if mon.hp != 0 && mon.status != Status::None && b.state.prng.random_chance(33, 100) {
        // this.debug('shed skin');  -- debug lines only exist in debug mode
        // this.add('-activate', pokemon, 'ability: Shed Skin');
        b.add(LogEntry::new(
            "-activate",
            &[
                LogArg::Mon(pokemon),
                LogArg::EffectFullName(EffectRef::Dex(ID)),
            ],
            &[],
        ));
        // pokemon.cureStatus();
        b.cure_status(pokemon, false);
    }
    Relay::Undefined
}
