//! Ports data/abilities.ts:3843 (Ripen). No direct PRNG draws.
//!
//! # Payload map (ability state)
//!
//! `pokemon.abilityState.berryWeaken`: presence bit `BERRY_WEAKEN_FLAG` (custom bit 8) of the
//! eater's ability state cell; set means `true`. `onEatItem` assigns it every time a berry is
//! eaten (`true` for a resist berry, otherwise `false`); `onSourceModifyDamage` consumes it.
//!
//! # Scope
//!
//! Berry Juice and the eighteen type-resist berries are outside the scoped dex, so the name
//! comparisons below resolve those ids to `EffectId::NONE` at compile time and can never match
//! (`support::is_named` / `support::is_listed` reject NONE). Leftovers and the in-scope healing
//! berries are matched normally.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{EventArg, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::ABILITY_RIPEN;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_RIPEN_ONTRYHEAL,
    dex::HOOK_ABILITY_RIPEN_ONCHANGEBOOST,
    dex::HOOK_ABILITY_RIPEN_ONSOURCEMODIFYDAMAGE,
    dex::HOOK_ABILITY_RIPEN_ONTRYEATITEM,
    dex::HOOK_ABILITY_RIPEN_ONEATITEM,
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

const BERRY_JUICE: EffectId = support::item_named("berryjuice");
/// data/abilities.ts:3872 `weakenBerries`.
const WEAKEN_BERRIES: [EffectId; 18] = [
    support::item_named("babiriberry"),
    support::item_named("chartiberry"),
    support::item_named("chilanberry"),
    support::item_named("chopleberry"),
    support::item_named("cobaberry"),
    support::item_named("colburberry"),
    support::item_named("habanberry"),
    support::item_named("kasibberry"),
    support::item_named("kebiaberry"),
    support::item_named("occaberry"),
    support::item_named("passhoberry"),
    support::item_named("payapaberry"),
    support::item_named("rindoberry"),
    support::item_named("roseliberry"),
    support::item_named("shucaberry"),
    support::item_named("tangaberry"),
    support::item_named("wacanberry"),
    support::item_named("yacheberry"),
];

/// `this.add('-activate', pokemon, 'ability: Ripen')`.
fn add_activate<L: LogSink>(b: &mut Battle<L>, pokemon: crate::ids::MonId) {
    b.add(LogEntry::new(
        "-activate",
        &[
            LogArg::Mon(pokemon),
            LogArg::EffectFullName(crate::event::EffectRef::Dex(ID)),
        ],
        &[],
    ));
}

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:3844 onTryHeal(damage, target, source, effect):
        //   if (!effect) return;
        //   if (effect.name === 'Berry Juice' || effect.name === 'Leftovers')
        //       this.add('-activate', target, 'ability: Ripen');
        //   if ((effect as Item).isBerry) return this.chainModify(2);
        // Returns undefined (chainModify only edits the event modifier). PRNG: none.
        dex::HOOK_ABILITY_RIPEN_ONTRYHEAL => {
            let effect = support::effect_at(b, cx, 3);
            if effect == crate::event::EffectRef::None {
                return Relay::Undefined;
            }
            let id = b.event_effect_id(effect);
            if support::is_named(id, BERRY_JUICE) || support::is_named(id, dex::ITEM_LEFTOVERS) {
                let target = support::mon(b, cx, 1);
                add_activate(b, target);
            }
            if support::effect_is_berry(b, effect) {
                b.chain_modify(2.0, 1.0);
            }
            Relay::Undefined
        }
        // data/abilities.ts:3851 onChangeBoost(boost, target, source, effect):
        //   if (effect && (effect as Item).isBerry) { for (b in boost) boost[b] *= 2; }
        // Doubles every present key of the relayed boost object in place (insertion order and
        // presence untouched). Returns undefined. PRNG: none.
        dex::HOOK_ABILITY_RIPEN_ONCHANGEBOOST => {
            let effect = support::effect_at(b, cx, 3);
            if support::effect_is_berry(b, effect) {
                let EventArg::Relay(Relay::Boosts(slot)) = b.event_arg(cx, 0) else {
                    panic!("Ripen onChangeBoost requires the boost object relay");
                };
                let boosts = b.scratch_boosts(slot);
                for i in 0..boosts.len as usize {
                    let stat = boosts.order[i] as usize;
                    boosts.values[stat] = boosts.values[stat].saturating_mul(2);
                }
            }
            Relay::Undefined
        }
        // data/abilities.ts:3860 onSourceModifyDamage(damage, source, target, move) (priority -1
        // is manifest metadata): `target` (argument 2) is the defender holding Ripen.
        //   if (target.abilityState.berryWeaken) {
        //       target.abilityState.berryWeaken = false;
        //       return this.chainModify(0.5);
        //   }
        // Returns undefined. PRNG: none.
        dex::HOOK_ABILITY_RIPEN_ONSOURCEMODIFYDAMAGE => {
            let target = support::mon(b, cx, 2);
            let cell = b.state.pokemon[target.0 as usize].ability_state;
            if b.state.effects.cells[cell.0 as usize].present & support::BERRY_WEAKEN_FLAG != 0 {
                b.state.effects.cells[cell.0 as usize].present &= !support::BERRY_WEAKEN_FLAG;
                b.chain_modify(0.5, 1.0);
            }
            Relay::Undefined
        }
        // data/abilities.ts:3867 onTryEatItem(item, pokemon) (priority -1 is manifest metadata):
        // `this.add('-activate', pokemon, 'ability: Ripen')`. Returns undefined, so eating
        // proceeds. PRNG: none.
        dex::HOOK_ABILITY_RIPEN_ONTRYEATITEM => {
            let pokemon = support::mon(b, cx, 1);
            add_activate(b, pokemon);
            Relay::Undefined
        }
        // data/abilities.ts:3870 onEatItem(item, pokemon): record whether the eaten berry is one
        // of the resist berries: `pokemon.abilityState.berryWeaken = weakenBerries.includes(
        // item.name)`. Returns undefined. PRNG: none.
        dex::HOOK_ABILITY_RIPEN_ONEATITEM => {
            let effect = support::effect_at(b, cx, 0);
            let item = b.event_effect_id(effect);
            let pokemon = support::mon(b, cx, 1);
            let cell = b.state.pokemon[pokemon.0 as usize].ability_state;
            let present = &mut b.state.effects.cells[cell.0 as usize].present;
            if support::is_listed(item, &WEAKEN_BERRIES) {
                *present |= support::BERRY_WEAKEN_FLAG;
            } else {
                *present &= !support::BERRY_WEAKEN_FLAG;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Ripen hook"),
    }
}
