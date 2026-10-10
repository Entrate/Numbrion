//! Ports data/abilities.ts:4992 (Thermal Exchange). No direct PRNG draws; the Atk boost runs
//! the usual boost events and the burn cure runs the status-clear events (listeners may
//! shuffle speed ties).
use crate::effects::registry::abilities_battlebond::support;
use crate::{
    Battle,
    actions::{Attribution, Stat},
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{move_arg, move_overlay},
    },
    event::{HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::Status,
};
pub const ID: EffectId = dex::ABILITY_THERMALEXCHANGE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_THERMALEXCHANGE_ONDAMAGINGHIT,
    dex::HOOK_ABILITY_THERMALEXCHANGE_ONUPDATE,
    dex::HOOK_ABILITY_THERMALEXCHANGE_ONSETSTATUS,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_ABILITY_THERMALEXCHANGE_ONDAMAGINGHIT => on_damaging_hit(b, cx),
        dex::HOOK_ABILITY_THERMALEXCHANGE_ONUPDATE => on_update(b, cx),
        dex::HOOK_ABILITY_THERMALEXCHANGE_ONSETSTATUS => on_set_status(b, cx),
        _ => panic!("unexpected thermalexchange hook"),
    }
}

// data/abilities.ts:4993-4997 onDamagingHit(damage, target, source, move). PRNG: none directly.
fn on_damaging_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // if (move.type === 'Fire')  (the mutable overlay type)
    let move_type = move_overlay(b, move_arg(b, cx, 3)).move_type;
    if move_type == support::TYPE_FIRE {
        // this.boost({ atk: 1 });
        b.boost(
            support::boosts([(Stat::Atk, 1)]),
            None,
            Attribution::DEFAULT,
            false,
            false,
        );
    }
    Relay::Undefined
}

// data/abilities.ts:4998-5003 onUpdate(pokemon). PRNG: none directly.
fn on_update<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = support::mon(b, cx, 0);
    // if (pokemon.status === 'brn')
    if b.state.pokemon[pokemon.0 as usize].status == Status::Burn {
        // this.add('-activate', pokemon, 'ability: Thermal Exchange');
        support::add_activate(b, pokemon, ID);
        // pokemon.cureStatus();
        b.cure_status(pokemon, false);
    }
    Relay::Undefined
}

// data/abilities.ts:5004-5010 onSetStatus(status, target, source, effect). PRNG: none.
// Returns false (blocks the status) for every burn, with the message only when the effect
// is a move whose own `status` field is set (including Synchronize's synthetic source).
fn on_set_status<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // if (status.id !== 'brn') return;
    if support::effect_id_at(b, cx, 0) != dex::CONDITION_BRN {
        return Relay::Undefined;
    }
    // if ((effect as Move)?.status)
    let effect = support::effect_at(b, cx, 3);
    if support::effect_has_status(b, effect) {
        // this.add('-immune', target, '[from] ability: Thermal Exchange');
        let target = support::mon(b, cx, 1);
        support::add_immune_from(b, target, ID);
    }
    // return false;
    Relay::Bool(false)
}
