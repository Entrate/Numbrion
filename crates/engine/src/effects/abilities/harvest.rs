//! Ports data/abilities.ts:1801 (Harvest).
//!
//! PRNG: one `randomChance(1, 2)` per Residual unless the effective weather is Sun
//! (`isWeather([...]) ||` short-circuits). Nested setItem events may add draws.
use crate::effects::registry::items_aguavberry::support;
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::HookWaiver,
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, LogTag},
};
pub const ID: EffectId = dex::ABILITY_HARVEST;
pub const HOOKS: &[HookId] = &[dex::HOOK_ABILITY_HARVEST_ONRESIDUAL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:1804 onResidual(pokemon) (order 28 / sub-order 2 are manifest
        // metadata):
        //   if (this.field.isWeather(['sunnyday', 'desolateland']) || this.randomChance(1, 2)) {
        //       if (pokemon.hp && !pokemon.item && this.dex.items.get(pokemon.lastItem).isBerry) {
        //           pokemon.setItem(pokemon.lastItem);
        //           pokemon.lastItem = '';
        //           this.add('-item', pokemon, pokemon.getItem(), '[from] ability: Harvest');
        //       }
        //   }
        // Desolate Land is outside the scoped dex (weather_terrain report), so only Sunny Day can
        // satisfy isWeather. Returns undefined. PRNG: randomChance(1, 2) only when not sunny.
        dex::HOOK_ABILITY_HARVEST_ONRESIDUAL => {
            let pokemon = support::mon(b, cx, 0);
            if b.is_weather(&[dex::CONDITION_SUNNYDAY]) || b.state.prng.random_chance(1, 2) {
                let (hp, item, last_item) = {
                    let p = &b.state.pokemon[pokemon.0 as usize];
                    (p.hp, p.item, p.last_item)
                };
                if hp != 0 && item == EffectId::NONE && support::item_is_berry(last_item) {
                    // setItem(item) with source/sourceEffect omitted.
                    b.set_item(pokemon, last_item, Attribution::DEFAULT);
                    b.state.pokemon[pokemon.0 as usize].last_item = EffectId::NONE;
                    // getItem() is evaluated after setItem (a Start handler may have used it up).
                    let current = b.state.pokemon[pokemon.0 as usize].item;
                    b.add(LogEntry::new(
                        "-item",
                        &[
                            LogArg::Mon(pokemon),
                            LogArg::Effect(EffectRef::Dex(current)),
                        ],
                        &[LogTag::From(EffectRef::Dex(dex::ABILITY_HARVEST))],
                    ));
                }
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Harvest hook"),
    }
}
