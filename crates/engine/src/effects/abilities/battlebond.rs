//! Ports data/abilities.ts:362 (Battle Bond). No direct PRNG draws; the Atk/SpA/Spe boost
//! below may sort tied AfterEachBoost/AfterBoost listeners.
//!
//! Also owns the batch's private helper module (`reactivestats/`), shared by the other
//! `reactive_stats` ability files through `registry::abilities_battlebond::support`.
use crate::{
    Battle,
    actions::Stat,
    dex::{self, HookId},
    effects::{HookWaiver, support::move_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::*,
    log::LogSink,
    state::{mon_flags, scratch::move_runtime},
};
#[path = "reactivestats/mod.rs"]
pub(super) mod support;
use support::{add_activate, boosts, effect_at, effect_is_move, mon};
pub const ID: EffectId = dex::ABILITY_BATTLEBOND;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_BATTLEBOND_ONSOURCEAFTERFAINT,
    dex::HOOK_ABILITY_BATTLEBOND_ONMODIFYMOVE,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

/// `move.id === 'watershuriken'` and `species.name === 'Greninja-Ash'`: neither exists in
/// the scoped dex (docs/showdown/06-scope.md rejects greninjaash: no in-scope handler
/// forme-changes to it), so the ModifyMove branch can never be taken. NONE never matches.
const WATERSHURIKEN: EffectId =
    crate::effects::support::optional_id(dex::MOVES_DATA, "watershuriken");
const GRENINJAASH: EffectId =
    crate::effects::support::optional_id(dex::SPECIES_DATA, "greninjaash");

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_BATTLEBOND_ONSOURCEAFTERFAINT => on_source_after_faint(b, cx),
        dex::HOOK_ABILITY_BATTLEBOND_ONMODIFYMOVE => on_modify_move(b, cx),
        _ => panic!("unexpected battlebond hook"),
    }
}

// data/abilities.ts:363-371 onSourceAfterFaint(length, target, source, effect).
// PRNG: none directly; the boost's AfterEachBoost/AfterBoost listeners may draw tie shuffles.
fn on_source_after_faint<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let source = mon(b, cx, 2);
    // if (source.bondTriggered) return;
    if b.state.pokemon[source.0 as usize].flags & mon_flags::BOND_TRIGGERED != 0 {
        return Relay::Undefined;
    }
    // if (effect?.effectType !== 'Move') return;
    let effect = effect_at(b, cx, 3);
    if !effect_is_move(b, effect) {
        return Relay::Undefined;
    }
    let p = &b.state.pokemon[source.0 as usize];
    // source.species.id === 'greninjabond' && source.hp && !source.transformed
    //   && source.side.foePokemonLeft()
    let eligible = p.species == dex::SPECIES_GRENINJABOND
        && p.hp != 0
        && p.flags & mon_flags::TRANSFORMED == 0
        && b.state.sides[(source.side().0 ^ 1) as usize].pokemon_left != 0;
    if eligible {
        // this.boost({ atk: 1, spa: 1, spe: 1 }, source, source, this.effect);
        b.boost(
            boosts([(Stat::Atk, 1), (Stat::SpA, 1), (Stat::Spe, 1)]),
            Some(source),
            crate::actions::Attribution::from_move(source, EffectRef::Dex(ID)),
            false,
            false,
        );
        // this.add('-activate', source, 'ability: Battle Bond');
        add_activate(b, source, ID);
        // source.bondTriggered = true;
        b.state.pokemon[source.0 as usize].flags |= mon_flags::BOND_TRIGGERED;
    }
    Relay::Undefined
}

// data/abilities.ts:373-378 onModifyMove(move, attacker). PRNG: none.
// `move.multihit = 3` is a fixed (non-range) multihit overlay.
fn on_modify_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 0);
    let attacker = mon(b, cx, 1);
    let m = crate::effects::support::move_overlay(b, mv);
    let p = &b.state.pokemon[attacker.0 as usize];
    if WATERSHURIKEN != EffectId::NONE
        && GRENINJAASH != EffectId::NONE
        && m.id == WATERSHURIKEN
        && p.species == GRENINJAASH
        && p.flags & mon_flags::TRANSFORMED == 0
    {
        let m = b.scratch.moves[mv as usize].as_mut().unwrap();
        m.multihit = [3, 3];
        m.runtime_flags |= move_runtime::MULTIHIT_PRESENT;
        m.runtime_flags &= !move_runtime::MULTIHIT_RANGE;
    }
    Relay::Undefined
}
