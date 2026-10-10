//! Ports data/abilities.ts:4157-4175 (Shadow Tag): onFoeTrapPokemon, onFoeMaybeTrapPokemon.
//! Payload: none (PAYLOAD_WORDS = 0); the holder is the ability cell's `effectState.target`.
//! Direct PRNG draws: none (tryTrap runs an immunity event that may sort ties).
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, registry::conditions_choicelock::support, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::ABILITY_SHADOWTAG;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_SHADOWTAG_ONFOETRAPPOKEMON,
    dex::HOOK_ABILITY_SHADOWTAG_ONFOEMAYBETRAPPOKEMON,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:4158-4162 onFoeTrapPokemon(pokemon): `if (!pokemon.hasAbility('shadowtag') &&
        // pokemon.isAdjacent(effectState.target)) pokemon.tryTrap(true)`. Returns undefined. PRNG: none directly.
        dex::HOOK_ABILITY_SHADOWTAG_ONFOETRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            if !b.has_ability(pokemon, &[dex::ABILITY_SHADOWTAG]) {
                let Some(holder) = support::state_target(b, cx) else {
                    // `pokemon.isAdjacent(undefined)` would throw in TS; the effect target is always set.
                    panic!("Shadow Tag without an effect target");
                };
                if b.is_adjacent(pokemon, holder) {
                    b.try_trap(pokemon, true);
                }
            }
            Relay::Undefined
        }
        // data/abilities.ts:4163-4168 onFoeMaybeTrapPokemon(pokemon, source): `if (!source) source =
        // effectState.target; if (!source || !pokemon.isAdjacent(source)) return;` then
        // `if (!pokemon.hasAbility('shadowtag')) pokemon.maybeTrapped = true`. Returns undefined.
        // PRNG: none directly.
        dex::HOOK_ABILITY_SHADOWTAG_ONFOEMAYBETRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            let source = support::opt_mon_arg(b, cx, 1).or_else(|| support::state_target(b, cx));
            let Some(source) = source else {
                return Relay::Undefined;
            };
            if !b.is_adjacent(pokemon, source) {
                return Relay::Undefined;
            }
            if !b.has_ability(pokemon, &[dex::ABILITY_SHADOWTAG]) {
                support::set_flag(b, pokemon, support::MAYBE_TRAPPED);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Shadow Tag function site"),
    }
}
