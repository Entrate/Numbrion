//! Ports data/abilities.ts:196-210 (Arena Trap): onFoeTrapPokemon, onFoeMaybeTrapPokemon.
//! Payload: none (PAYLOAD_WORDS = 0); the holder is the ability cell's `effectState.target`.
//! Direct PRNG draws: none (isGrounded/tryTrap run Type and immunity events that may sort ties).
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, registry::conditions_choicelock::support, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::mon_flags,
};
pub const ID: EffectId = dex::ABILITY_ARENATRAP;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_ARENATRAP_ONFOETRAPPOKEMON,
    dex::HOOK_ABILITY_ARENATRAP_ONFOEMAYBETRAPPOKEMON,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:197-202 onFoeTrapPokemon(pokemon): an adjacent grounded foe is hard-trapped
        // (`pokemon.tryTrap(true)`: hidden trap). `isGrounded()` is null (Levitate) / false -> no trap.
        // Returns undefined. PRNG: none directly.
        dex::HOOK_ABILITY_ARENATRAP_ONFOETRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            let Some(holder) = support::state_target(b, cx) else {
                // `pokemon.isAdjacent(undefined)` would throw in TS; the effect target is always set.
                panic!("Arena Trap without an effect target");
            };
            if !b.is_adjacent(pokemon, holder) {
                return Relay::Undefined;
            }
            if b.is_grounded(pokemon, false).truthy() {
                b.try_trap(pokemon, true);
            }
            Relay::Undefined
        }
        // data/abilities.ts:203-208 onFoeMaybeTrapPokemon(pokemon, source): `if (!source) source =
        // effectState.target; if (!source || !pokemon.isAdjacent(source)) return;` then, when the foe is
        // grounded (`isGrounded(!pokemon.knownType)`: immunities are negated while its type is unknown),
        // `pokemon.maybeTrapped = true`. Returns undefined. PRNG: none directly.
        dex::HOOK_ABILITY_ARENATRAP_ONFOEMAYBETRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            let source = support::opt_mon_arg(b, cx, 1).or_else(|| support::state_target(b, cx));
            let Some(source) = source else {
                return Relay::Undefined;
            };
            if !b.is_adjacent(pokemon, source) {
                return Relay::Undefined;
            }
            let known_type = b.state.pokemon[pokemon.0 as usize].flags & mon_flags::KNOWN_TYPE != 0;
            if b.is_grounded(pokemon, !known_type).truthy() {
                support::set_flag(b, pokemon, support::MAYBE_TRAPPED);
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Arena Trap function site"),
    }
}
