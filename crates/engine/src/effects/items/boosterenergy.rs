//! Pinned Showdown 7332b60 effect port. See handler comments for source and draws.
#![allow(unused_imports)]
use crate::effects::registry::abilities_battlebond::support::{
    boosts, effect_at, effect_has_status, effect_is_move, foe, mon_opt, type_named,
};
use crate::{
    Battle,
    actions::{
        Attribution, FormeOptions, ImmunityMessage, ImmunitySource, MoveHandle, Stat, StatOptions,
        Types,
    },
    dex::{self, Category, DataValue, HookId, MoveTarget},
    effects::{
        HookWaiver,
        support::{mon_arg as mon, move_arg, optional_id, relay_number},
    },
    event::{CallArgs, EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{
        mon_flags,
        scratch::{MoveEffectsScratch, move_runtime},
    },
};
pub const ID: EffectId = dex::ITEM_BOOSTERENERGY;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ITEM_BOOSTERENERGY_ONSTART,
    dex::HOOK_ITEM_BOOSTERENERGY_ONUPDATE,
    dex::HOOK_ITEM_BOOSTERENERGY_ONTAKEITEM,
];
// Payload: 1 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:628-631. PRNG: direct onUpdate uses the same effect/event/state.
        dex::HOOK_ITEM_BOOSTERENERGY_ONSTART => {
            b.hook_state_mut(cx).payload.words[0] = 1;
            b.hook_state_mut(cx).present |= 1 << 8;
            b.call_hook(
                dex::HOOK_ITEM_BOOSTERENERGY_ONUPDATE,
                CallArgs {
                    values: [
                        b.event_arg(cx, 0),
                        EventArg::Undefined,
                        EventArg::Undefined,
                        EventArg::Undefined,
                    ],
                    len: 1,
                },
            );
            Relay::Undefined
        }
        // data/items.ts:632-641. PRNG: useItem and addVolatile nested events only.
        // word0=started, bit8=present. The second if is evaluated even after consuming the item.
        dex::HOOK_ITEM_BOOSTERENERGY_ONUPDATE => {
            let m = mon(b, cx, 0);
            if b.hook_state(cx).payload.words[0] == 0
                || b.state.pokemon[m.0 as usize].flags & mon_flags::TRANSFORMED != 0
            {
                return Relay::Undefined;
            }
            if b.has_ability(m, &[dex::ABILITY_PROTOSYNTHESIS])
                && !b.is_weather(&[dex::CONDITION_SUNNYDAY])
                && b.use_item(m, Attribution::DEFAULT)
            {
                b.add_volatile(m, dex::CONDITION_PROTOSYNTHESIS, Attribution::DEFAULT, None);
            }
            if b.has_ability(m, &[dex::ABILITY_QUARKDRIVE])
                && !b.is_terrain(&[dex::CONDITION_ELECTRICTERRAIN], None)
                && b.use_item(m, Attribution::DEFAULT)
            {
                b.add_volatile(m, dex::CONDITION_QUARKDRIVE, Attribution::DEFAULT, None);
            }
            Relay::Undefined
        }
        // data/items.ts:642-645. PRNG: none. Raw baseSpecies tags, no suppression checks.
        dex::HOOK_ITEM_BOOSTERENERGY_ONTAKEITEM => {
            let m = mon(b, cx, 1);
            Relay::Bool(
                dex::species(b.state.pokemon[m.0 as usize].base_species).tags
                    & dex::SPECIES_TAG_PARADOX
                    == 0,
            )
        }
        _ => panic!("unexpected boosterenergy hook"),
    }
}
