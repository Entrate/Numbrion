//! Ports data/conditions.ts:222-258 (`partiallytrapped`: Bind, Wrap, Fire Spin, Whirlpool, Clamp, Sand Tomb,
//! Magma Storm, Infestation, Snap Trap, Thunder Cage): durationCallback, onStart, onResidual, onEnd,
//! onTrapPokemon.
//! Payload: word 0 = `effectState.boundDivisor` (6 with Binding Band, else 8; present bit 8).
//! `effectState.source` / `effectState.sourceEffect` are the shared source fields written by addVolatile
//! (the trapper and the trapping move). Direct PRNG draws: `random(5, 7)` in durationCallback only.
//! G-Max Centiferno / G-Max Sandblast (`gmaxEffect`) do not exist in this format, so `gmaxEffect` is false.
//! Grip Claw / Binding Band are outside the scoped item list; they are kept through the static key query
//! (`query_has_item`), which can never match here.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, registry::conditions_choicelock::support, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{mon_flags, present},
};
pub const ID: EffectId = dex::CONDITION_PARTIALLYTRAPPED;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_PARTIALLYTRAPPED_DURATIONCALLBACK,
    dex::HOOK_CONDITION_PARTIALLYTRAPPED_ONSTART,
    dex::HOOK_CONDITION_PARTIALLYTRAPPED_ONRESIDUAL,
    dex::HOOK_CONDITION_PARTIALLYTRAPPED_ONEND,
    dex::HOOK_CONDITION_PARTIALLYTRAPPED_ONTRAPPOKEMON,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
/// `effectState.boundDivisor` present bit (first custom bit); the divisor is `payload.words[0]`.
const DIVISOR_PRESENT: u32 = 1 << present::CUSTOM_START;
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:225-228 durationCallback(target, source): direct call, args [target, source, effect].
        // `if (source?.hasItem('gripclaw')) return 8; return this.random(5, 7);`
        // PRNG: one `random(5, 7)` draw unless the source holds a Grip Claw.
        dex::HOOK_CONDITION_PARTIALLYTRAPPED_DURATIONCALLBACK => {
            if let Some(source) = support::opt_mon_arg(b, cx, 1) {
                if b.query_has_item(source, dex::key_ids!("gripclaw")) {
                    return Relay::Number(8.0);
                }
            }
            Relay::Number(f64::from(b.state.prng.random_range(5, 7)))
        }
        // data/conditions.ts:229-232 onStart(pokemon, source): `|-activate|pokemon|move: <sourceEffect>|[of] source`
        // (`'move: ' + effectState.sourceEffect`, `[of] ${source}`), then
        // `effectState.boundDivisor = source.hasItem('bindingband') ? 6 : 8`. Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_PARTIALLYTRAPPED_ONSTART => {
            let pokemon = mon_arg(b, cx, 0);
            let source = mon_arg(b, cx, 1);
            let source_effect = support::state_source_effect(b, cx);
            b.add(LogEntry::new(
                "-activate",
                &[
                    LogArg::Mon(pokemon),
                    LogArg::Parts(&[LogArg::Text("move: "), LogArg::Effect(source_effect)]),
                ],
                &[LogTag::Of(source)],
            ));
            let divisor = if b.query_has_item(source, dex::key_ids!("bindingband")) {
                6
            } else {
                8
            };
            let c = b.hook_state_mut(cx);
            c.payload.words[0] = divisor;
            c.present |= DIVISOR_PRESENT;
            Relay::Undefined
        }
        // data/conditions.ts:233-243 onResidual(pokemon) (order 13). PRNG: none directly (Damage events).
        // When the trapper has left the field, fainted, or has not yet been active a turn
        // (`!source.isActive || source.hp <= 0 || !source.activeTurns`) the volatile is deleted *without* an
        // End event and `|-end|pokemon|<sourceEffect>|[partiallytrapped]|[silent]` is logged; otherwise the
        // victim takes `baseMaxhp / boundDivisor` damage (default source/effect from the running event).
        dex::HOOK_CONDITION_PARTIALLYTRAPPED_ONRESIDUAL => {
            let pokemon = mon_arg(b, cx, 0);
            let source_effect = support::state_source_effect(b, cx);
            if let Some(source) = support::state_source(b, cx) {
                let s = &b.state.pokemon[source.0 as usize];
                if s.flags & mon_flags::ACTIVE == 0 || s.hp == 0 || s.active_turns == 0 {
                    b.erase_volatile(pokemon, dex::CONDITION_PARTIALLYTRAPPED);
                    b.add(LogEntry::new(
                        "-end",
                        &[LogArg::Mon(pokemon), LogArg::Effect(source_effect)],
                        &[LogTag::Bare("partiallytrapped"), LogTag::Bare("silent")],
                    ));
                    return Relay::Undefined;
                }
            }
            let divisor = f64::from(b.hook_state(cx).payload.words[0]);
            let amount = f64::from(b.state.pokemon[pokemon.0 as usize].max_hp) / divisor;
            b.damage(amount, Some(pokemon), Attribution::DEFAULT);
            Relay::Undefined
        }
        // data/conditions.ts:244-246 onEnd(pokemon): `|-end|pokemon|<sourceEffect>|[partiallytrapped]`.
        // Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_PARTIALLYTRAPPED_ONEND => {
            let pokemon = mon_arg(b, cx, 0);
            let source_effect = support::state_source_effect(b, cx);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(pokemon), LogArg::Effect(source_effect)],
                &[LogTag::Bare("partiallytrapped")],
            ));
            Relay::Undefined
        }
        // data/conditions.ts:247-250 onTrapPokemon(pokemon): `if (effectState.source?.isActive || gmaxEffect)
        // pokemon.tryTrap()` (not hidden). Returns undefined. PRNG: none.
        dex::HOOK_CONDITION_PARTIALLYTRAPPED_ONTRAPPOKEMON => {
            let pokemon = mon_arg(b, cx, 0);
            if let Some(source) = support::state_source(b, cx) {
                if b.state.pokemon[source.0 as usize].flags & mon_flags::ACTIVE != 0 {
                    b.try_trap(pokemon, false);
                }
            }
            Relay::Undefined
        }
        _ => panic!("unexpected partiallytrapped function site"),
    }
}
