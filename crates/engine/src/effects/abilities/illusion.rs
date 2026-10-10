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
pub const ID: EffectId = dex::ABILITY_ILLUSION;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_ILLUSION_ONBEFORESWITCHIN,
    dex::HOOK_ABILITY_ILLUSION_ONDAMAGINGHIT,
    dex::HOOK_ABILITY_ILLUSION_ONEND,
    dex::HOOK_ABILITY_ILLUSION_ONFAINT,
];
// Payload: 0 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:2057-2071. PRNG: none; party order, never original MonId order.
        dex::HOOK_ABILITY_ILLUSION_ONBEFORESWITCHIN => {
            let user = mon(b, cx, 0);
            b.state.pokemon[user.0 as usize].illusion = MonId::NONE;
            let p = b.state.pokemon[user.0 as usize];
            let side = b.state.sides[user.side().0 as usize];
            for i in ((p.position as usize + 1)..side.pokemon_count as usize).rev() {
                let target = side.party[i];
                let t = b.state.pokemon[target.0 as usize];
                if t.flags & mon_flags::FAINTED == 0 {
                    // Search bookkeeping, not Showdown: the target is displayed, or (while
                    // terastallized) its species decides whether a disguise appears.
                    b.mark_revealed(target);
                    if p.terastallized == TypeId::NONE
                        || !matches!(
                            dex::species(t.species).base_species,
                            dex::SPECIES_OGERPON | dex::SPECIES_TERAPAGOS
                        )
                    {
                        b.state.pokemon[user.0 as usize].illusion = target;
                    }
                    break;
                }
            }
            Relay::Undefined
        }
        // data/abilities.ts:2072-2076. PRNG: nested End event only; same captured ability state.
        dex::HOOK_ABILITY_ILLUSION_ONDAMAGINGHIT => {
            let target = mon(b, cx, 1);
            if b.state.pokemon[target.0 as usize].illusion != MonId::NONE {
                let state = b
                    .state
                    .effects
                    .capture(b.state.pokemon[target.0 as usize].ability_state);
                b.single_event(
                    EventId::End,
                    EffectRef::Dex(ID),
                    Some(state),
                    b.event_arg(cx, 1),
                    b.event_arg(cx, 2),
                    effect_at(b, cx, 3),
                    Relay::Undefined,
                    None,
                );
            }
            Relay::Undefined
        }
        // data/abilities.ts:2077-2088. PRNG: none. Illusion Level Mod is a format rule.
        dex::HOOK_ABILITY_ILLUSION_ONEND => {
            let user = mon(b, cx, 0);
            let p = b.state.pokemon[user.0 as usize];
            if p.illusion != MonId::NONE && p.flags & mon_flags::BEING_CALLED_BACK == 0 {
                b.state.pokemon[user.0 as usize].illusion = MonId::NONE;
                b.add(LogEntry::new(
                    "replace",
                    &[LogArg::Mon(user), LogArg::Details(user)],
                    &[],
                ));
                b.add(LogEntry::new(
                    "-end",
                    &[LogArg::Mon(user), LogArg::Text("Illusion")],
                    &[],
                ));
                b.hint(
                    LogArg::Text(
                        "Illusion Level Mod is active, so this Pokémon's true level was hidden.",
                    ),
                    true,
                    None,
                );
            }
            Relay::Undefined
        }
        // data/abilities.ts:2089-2091. PRNG: none; no reveal logs on faint.
        dex::HOOK_ABILITY_ILLUSION_ONFAINT => {
            let m = mon(b, cx, 0);
            b.state.pokemon[m.0 as usize].illusion = MonId::NONE;
            Relay::Undefined
        }
        _ => panic!("unexpected illusion hook"),
    }
}
