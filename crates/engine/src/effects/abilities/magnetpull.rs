//! Ports data/abilities.ts:2517-2535 (Magnet Pull): onFoeTrapPokemon, onFoeMaybeTrapPokemon.
//! Payload: none (PAYLOAD_WORDS = 0); the holder is the ability cell's `effectState.target`.
//! Direct PRNG draws: none (hasType/tryTrap run Type and immunity events that may sort ties).
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, registry::conditions_choicelock::support, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::mon_flags,
};
pub const ID: EffectId = dex::ABILITY_MAGNETPULL;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_MAGNETPULL_ONFOETRAPPOKEMON,
    dex::HOOK_ABILITY_MAGNETPULL_ONFOEMAYBETRAPPOKEMON,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:2518-2522 onFoeTrapPokemon(pokemon): `if (pokemon.hasType('Steel') &&
        // pokemon.isAdjacent(effectState.target)) pokemon.tryTrap(true)`. `hasType` (a Type event) runs
        // before the adjacency test. Returns undefined. PRNG: none directly.
        dex::HOOK_ABILITY_MAGNETPULL_ONFOETRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            if b.has_type(pokemon, &[support::TYPE_STEEL]) {
                let Some(holder) = support::state_target(b, cx) else {
                    // `pokemon.isAdjacent(undefined)` would throw in TS; the effect target is always set.
                    panic!("Magnet Pull without an effect target");
                };
                if b.is_adjacent(pokemon, holder) {
                    b.try_trap(pokemon, true);
                }
            }
            Relay::Undefined
        }
        // data/abilities.ts:2523-2528 onFoeMaybeTrapPokemon(pokemon, source): `if (!source) source =
        // effectState.target; if (!source || !pokemon.isAdjacent(source)) return;` then
        // `if (!pokemon.knownType || pokemon.hasType('Steel')) pokemon.maybeTrapped = true`
        // (`hasType` is only evaluated while the type is known). Returns undefined. PRNG: none directly.
        dex::HOOK_ABILITY_MAGNETPULL_ONFOEMAYBETRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            let source = support::opt_mon_arg(b, cx, 1).or_else(|| support::state_target(b, cx));
            let Some(source) = source else {
                return Relay::Undefined;
            };
            if !b.is_adjacent(pokemon, source) {
                return Relay::Undefined;
            }
            let known_type = b.state.pokemon[pokemon.0 as usize].flags & mon_flags::KNOWN_TYPE != 0;
            if !known_type || b.has_type(pokemon, &[support::TYPE_STEEL]) {
                support::set_flag(b, pokemon, support::MAYBE_TRAPPED);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Magnet Pull function site"),
    }
}
