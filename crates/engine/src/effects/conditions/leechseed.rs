//! Ports data/moves.ts:10212 (Leech Seed's embedded volatile condition). No direct PRNG draws;
//! the drain's `damage`/`heal` run their events, whose listeners may speed-sort ties.
//!
//! State: stateless (`PAYLOAD_WORDS = 0`). The seeder is the common `sourceSlot` field of the
//! volatile's effect cell (set by addVolatile, pokemon.ts:1969-2027), exactly what
//! `pokemon.volatiles['leechseed'].sourceSlot` reads: the *slot* is stored, not the Pokemon, so a
//! replacement that later occupies the seeder's slot receives the HP.
use crate::effects::registry::abilities_baddreams::support::relay_amount;
use crate::{
    Battle,
    actions::{Attribution, HealEffect},
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::{EffectId, SlotId},
    log::{LogArg, LogEntry, LogSink},
    state::mon_flags,
};
pub const ID: EffectId = dex::CONDITION_LEECHSEED;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_LEECHSEED_ONSTART,
    dex::HOOK_CONDITION_LEECHSEED_ONRESIDUAL,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_CONDITION_LEECHSEED_ONSTART => on_start(b, cx),
        dex::HOOK_CONDITION_LEECHSEED_ONRESIDUAL => on_residual(b, cx),
        _ => panic!("unexpected leechseed condition hook"),
    }
}

// data/moves.ts:10213-10215 onStart(target). PRNG: none.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    // this.add('-start', target, 'move: Leech Seed');
    b.add(LogEntry::new(
        "-start",
        &[LogArg::Mon(target), LogArg::Text("move: Leech Seed")],
        &[],
    ));
    Relay::Undefined
}

// data/moves.ts:10217-10226 onResidual(pokemon). PRNG: none directly (damage/heal events only).
// Residual order 8 is manifest metadata.
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon_arg(b, cx, 0);
    // const target = this.getAtSlot(pokemon.volatiles['leechseed'].sourceSlot);
    // The running effect state is that volatile (Residual skips removed/replaced states), so
    // its common sourceSlot field is the same property.
    let slot = b.hook_state(cx).source_slot;
    let target = if slot == SlotId::NONE {
        None
    } else {
        b.get_at_slot(slot)
    };
    // if (!target || target.fainted || target.hp <= 0) { this.debug('Nothing to leech into'); return; }
    let Some(target) = target else {
        return Relay::Undefined;
    };
    let seeder = &b.state.pokemon[target.0 as usize];
    if seeder.flags & mon_flags::FAINTED != 0 || seeder.hp == 0 {
        return Relay::Undefined;
    }
    // const damage = this.damage(pokemon.baseMaxhp / 8, pokemon, target);   (effect defaults to
    // this.effect, the Leech Seed volatile)
    let amount = f64::from(b.state.pokemon[pokemon.0 as usize].max_hp) / 8.0;
    let damage = b.damage(
        amount,
        Some(pokemon),
        Attribution::from_move(target, EffectRef::None),
    );
    // if (damage) this.heal(damage, target, pokemon);
    if damage.truthy() {
        b.heal(
            relay_amount(damage),
            Some(target),
            Some(pokemon),
            HealEffect::Context,
        );
    }
    Relay::Undefined
}
