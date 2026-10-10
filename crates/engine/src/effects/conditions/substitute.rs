//! Faithful port of pinned Showdown 7332b60. Source/draw notes at each handler.
#![allow(unused_imports)]
use crate::effects::registry::moves_batonpass::support::{self, effect, handle, volatile};
use crate::{
    Battle,
    actions::*,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg as mon, optional_id},
    },
    event::{EffectRef, EventArg, HookCtx, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::{
        mon_flags,
        scratch::{OrderedBoosts, move_runtime},
    },
};
pub const ID: EffectId = dex::CONDITION_SUBSTITUTE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_SUBSTITUTE_ONSTART,
    dex::HOOK_CONDITION_SUBSTITUTE_ONTRYPRIMARYHIT,
    dex::HOOK_CONDITION_SUBSTITUTE_ONEND,
];
// Payload words: word0 = Substitute HP (u32), present bit8.
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:18329-18341. PRNG: none.
        dex::HOOK_CONDITION_SUBSTITUTE_ONSTART => {
            let t = mon(b, cx, 0);
            let e = effect(b, cx, 2);
            if b.event_effect_id(e) == dex::MOVE_SHEDTAIL {
                b.add(LogEntry::new(
                    "-start",
                    &[LogArg::Mon(t), LogArg::Text("Substitute")],
                    &[LogTag::From(EffectRef::Dex(dex::MOVE_SHEDTAIL))],
                ));
            } else {
                b.add(LogEntry::new(
                    "-start",
                    &[LogArg::Mon(t), LogArg::Text("Substitute")],
                    &[],
                ));
            }
            b.hook_state_mut(cx).payload.words[0] =
                u32::from(b.state.pokemon[t.0 as usize].max_hp) / 4;
            b.hook_state_mut(cx).present |= 1 << 8;
            if let Some(c) = b.get_volatile(t, dex::CONDITION_PARTIALLYTRAPPED) {
                let e = b.state.effects.cells[c.0 as usize].source_effect;
                b.add(LogEntry::new(
                    "-end",
                    &[LogArg::Mon(t), LogArg::Effect(EffectRef::Dex(e))],
                    &[LogTag::Bare("partiallytrapped"), LogTag::Bare("silent")],
                ));
                // Source deletes the volatile directly, without End; retire its original cell.
                let i = b.state.pokemon[t.0 as usize]
                    .volatiles
                    .as_slice()
                    .iter()
                    .position(|&x| x == c)
                    .unwrap();
                b.state.pokemon[t.0 as usize].volatiles.remove(i);
                b.state.effects.release(c);
            }
            Relay::Undefined
        }
        // data/moves.ts:18343-18374. PRNG: getDamage (crit/damage/events), recoil/heal/AfterSubDamage events.
        dex::HOOK_CONDITION_SUBSTITUTE_ONTRYPRIMARYHIT => {
            let t = mon(b, cx, 0);
            let s = mon(b, cx, 1);
            let h = handle(b, cx, 2);
            let m = *b.active_move(h);
            if t == s
                || m.flags & dex::FLAG_BYPASSSUB != 0
                || m.runtime_flags & move_runtime::INFILTRATES != 0
            {
                return Relay::Undefined;
            }
            let r = b.get_damage(
                s,
                t,
                DamageInput::Move(MoveInput::Active(h)),
                DamageOptions::default(),
            );
            if !r.truthy() && r != Relay::Number(0.) {
                b.add(LogEntry::new("-fail", &[LogArg::Mon(s)], &[]));
                b.attr_last_move(MoveLineEdit::Still);
                return Relay::Null;
            }
            let Relay::Number(n) = r else {
                panic!("Substitute requires numeric damage")
            };
            let damage = n.min(f64::from(b.hook_state(cx).payload.words[0]));
            b.hook_state_mut(cx).payload.words[0] -= damage as u32;
            // source.lastDamage is a dead store in scoped gen9; attack records are owned by hit execution.
            if b.hook_state(cx).payload.words[0] == 0 {
                if false
                /* OHKO moves are absent from the pinned gen9 random doubles closure */
                {
                    b.add(LogEntry::new("-ohko", &[], &[]));
                }
                b.remove_volatile(t, ID);
            } else {
                b.add(LogEntry::new(
                    "-activate",
                    &[LogArg::Mon(t), LogArg::Text("move: Substitute")],
                    &[LogTag::Bare("damage")],
                ));
            }
            if damage != 0. {
                b.apply_recoil_damage(damage, h, s);
            }
            if let Some(d) = m.drain {
                b.heal(
                    (damage * f64::from(d[0]) / f64::from(d[1])).ceil(),
                    Some(s),
                    Some(t),
                    HealEffect::Drain,
                );
            }
            b.single_event(
                EventId::AfterSubDamage,
                EffectRef::ActiveMove(h.0),
                None,
                support::mon(t),
                support::mon(s),
                EffectRef::ActiveMove(h.0),
                Relay::Number(damage),
                None,
            );
            b.run_event(
                EventId::AfterSubDamage,
                support::mon(t),
                support::mon(s),
                EffectRef::ActiveMove(h.0),
                Relay::Number(damage),
                RunEventOptions::default(),
            );
            Relay::Number(0.)
        }
        // data/moves.ts:18375-18377. PRNG: none.
        dex::HOOK_CONDITION_SUBSTITUTE_ONEND => {
            let t = mon(b, cx, 0);
            b.add(LogEntry::new(
                "-end",
                &[LogArg::Mon(t), LogArg::Text("Substitute")],
                &[],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected Substitute hook"),
    }
}
