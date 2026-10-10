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
pub const ID: EffectId = dex::ABILITY_TRACE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_ABILITY_TRACE_ONSTART,
    dex::HOOK_ABILITY_TRACE_ONUPDATE,
];
// Payload: 1 word(s); local fields documented at writes below.
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/abilities.ts:5121-5136. PRNG: nested Update event and sample only.
        dex::HOOK_ABILITY_TRACE_ONSTART => {
            let user = mon(b, cx, 0);
            b.hook_state_mut(cx).payload.words[0] = 1;
            b.hook_state_mut(cx).present |= 1 << 8;
            const NOABILITY: EffectId = optional_id(dex::ABILITIES_DATA, "noability");
            const SHIELD: EffectId = optional_id(dex::ITEMS_DATA, "abilityshield");
            let foes = b.adjacent_foes(user);
            if foes.entries[..foes.len as usize].iter().any(|&t| {
                NOABILITY != EffectId::NONE
                    && b.state.pokemon[foe(t).0 as usize].ability == NOABILITY
            }) {
                b.hook_state_mut(cx).payload.words[0] = 0;
            }
            if SHIELD != EffectId::NONE && b.has_item(user, &[SHIELD]) {
                b.add(LogEntry::new(
                    "-block",
                    &[
                        LogArg::Mon(user),
                        LogArg::EffectFullName(EffectRef::Dex(SHIELD)),
                    ],
                    &[],
                ));
                b.hook_state_mut(cx).payload.words[0] = 0;
            }
            if b.hook_state(cx).payload.words[0] != 0 {
                b.single_event(
                    EventId::Update,
                    EffectRef::Dex(ID),
                    Some(cx.state),
                    b.event_arg(cx, 0),
                    EventArg::Undefined,
                    EffectRef::None,
                    Relay::Undefined,
                    None,
                );
            }
            Relay::Undefined
        }
        // data/abilities.ts:5137-5150. PRNG: sample exactly once, even one candidate; setAbility events may draw.
        // word0=seek, bit8=present. No scratch handles retained.
        dex::HOOK_ABILITY_TRACE_ONUPDATE => {
            if b.hook_state(cx).payload.words[0] == 0 {
                return Relay::Undefined;
            }
            let user = mon(b, cx, 0);
            let foes = b.adjacent_foes(user);
            let mut candidates = [MonId::NONE; 2];
            let mut len = 0;
            for &t in &foes.entries[..foes.len as usize] {
                let target = foe(t);
                let ability = b.state.pokemon[target.0 as usize].ability;
                if ability != EffectId::NONE
                    && dex::ABILITIES[(ability.0 - dex::ABILITY_START) as usize].flags
                        & dex::FLAG_NOTRACE
                        == 0
                {
                    candidates[len] = target;
                    len += 1;
                }
            }
            if len == 0 {
                return Relay::Undefined;
            }
            let target = b.state.prng.sample(&candidates[..len]);
            let ability = b.state.pokemon[target.0 as usize].ability;
            b.set_ability(
                user,
                ability,
                Attribution::from_move(target, EffectRef::None),
                false,
                false,
            );
            Relay::Undefined
        }
        _ => panic!("unexpected trace hook"),
    }
}
