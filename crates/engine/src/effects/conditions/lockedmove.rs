//! Ports data/conditions.ts:253-286 (lockedmove: Outrage, Thrash, Petal Dance...; `duration: 2` is
//! declarative and lives in the common `duration` field of the cell).
//!
//! Payload map (PAYLOAD_WORDS = 2):
//! * `words[0]` = `effectState.trueDuration`, a signed counter stored as the two's-complement of an
//!   i32 (custom presence bit 8). Start draws it from `random(2, 4)`, Residual decrements it (it can
//!   reach zero or below on the way to expiry), Restart/End only compare it.
//! * `words[1]` = `effectState.move`, the EffectId of the locked move (custom presence bit 9).
//! * `effectState.duration` is the cell's common `duration` field (i16; -1 absent).
//!
//! PRNG: Start draws `random(2, 4)` exactly once; the confusion Start in End draws its own
//! `random(2, 6)` (confusion.rs).
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EventArg, HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
    state::{Status, present},
};
pub const ID: EffectId = dex::CONDITION_LOCKEDMOVE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_LOCKEDMOVE_ONRESIDUAL,
    dex::HOOK_CONDITION_LOCKEDMOVE_ONSTART,
    dex::HOOK_CONDITION_LOCKEDMOVE_ONRESTART,
    dex::HOOK_CONDITION_LOCKEDMOVE_ONAFTERMOVE,
    dex::HOOK_CONDITION_LOCKEDMOVE_ONEND,
    dex::HOOK_CONDITION_LOCKEDMOVE_ONLOCKMOVE,
];
pub const PAYLOAD_WORDS: usize = 2;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

const PRESENT_TRUE_DURATION: u32 = 1 << present::CUSTOM_START;
const PRESENT_MOVE: u32 = 1 << (present::CUSTOM_START + 1);

fn true_duration<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> i32 {
    let c = b.hook_state(cx);
    assert!(
        c.present & PRESENT_TRUE_DURATION != 0,
        "lockedmove state has no trueDuration"
    );
    c.payload.words[0] as i32
}

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:257-263 onResidual(target):
        //   if (target.status === 'slp') delete target.volatiles['lockedmove'];   // no End callback
        //   this.effectState.trueDuration--;
        // Residual passes (target, null, undefined). The decrement still happens on the detached
        // state object (it stays valid while this callback pins it). PRNG: none.
        dex::HOOK_CONDITION_LOCKEDMOVE_ONRESIDUAL => {
            let target = mon_arg(b, cx, 0);
            if b.state.pokemon[target.0 as usize].status == Status::Sleep {
                delete_volatile_raw(b, target);
            }
            let remaining = true_duration(b, cx).wrapping_sub(1);
            b.hook_state_mut(cx).payload.words[0] = remaining as u32;
            Relay::Undefined
        }
        // data/conditions.ts:264-267 onStart(target, source, effect):
        //   this.effectState.trueDuration = this.random(2, 4);
        //   this.effectState.move = effect.id;
        // PRNG: exactly one random(2, 4) (value 2 or 3).
        dex::HOOK_CONDITION_LOCKEDMOVE_ONSTART => {
            let duration = b.state.prng.random_range(2, 4);
            let effect = match b.event_arg(cx, 2) {
                EventArg::Effect(e) => e,
                other => panic!("lockedmove Start without an effect: {other:?}"),
            };
            let move_id = b.event_effect_id(effect);
            let c = b.hook_state_mut(cx);
            c.payload.words[0] = duration;
            c.payload.words[1] = u32::from(move_id.0);
            c.present |= PRESENT_TRUE_DURATION | PRESENT_MOVE;
            Relay::Undefined
        }
        // data/conditions.ts:268-272 onRestart():
        //   if (this.effectState.trueDuration >= 2) this.effectState.duration = 2;
        // PRNG: none.
        dex::HOOK_CONDITION_LOCKEDMOVE_ONRESTART => {
            if true_duration(b, cx) >= 2 {
                b.hook_state_mut(cx).duration = 2;
            }
            Relay::Undefined
        }
        // data/conditions.ts:273-277 onAfterMove(pokemon):
        //   if (this.effectState.duration === 1) pokemon.removeVolatile('lockedmove');
        // AfterMove passes (pokemon, target, move). PRNG: none directly.
        dex::HOOK_CONDITION_LOCKEDMOVE_ONAFTERMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            if b.hook_state(cx).duration == 1 {
                b.remove_volatile(pokemon, dex::CONDITION_LOCKEDMOVE);
            }
            Relay::Undefined
        }
        // data/conditions.ts:278-281 onEnd(target):
        //   if (this.effectState.trueDuration > 1) return;
        //   target.addVolatile('confusion');
        // No source/sourceEffect is passed, so the event defaults supply them; the sourceEffect is
        // this lockedmove condition, which confusion's Start tests (conditions.ts:166 `[fatigue]`).
        // PRNG: none directly (confusion Start draws).
        dex::HOOK_CONDITION_LOCKEDMOVE_ONEND => {
            let target = mon_arg(b, cx, 0);
            if true_duration(b, cx) > 1 {
                return Relay::Undefined;
            }
            b.add_volatile(target, dex::CONDITION_CONFUSION, Attribution::DEFAULT, None);
            Relay::Undefined
        }
        // data/conditions.ts:282-285 onLockMove(pokemon):
        //   if (pokemon.volatiles['dynamax']) return;       // no Dynamax in this format
        //   return this.effectState.move;
        // PRNG: none.
        dex::HOOK_CONDITION_LOCKEDMOVE_ONLOCKMOVE => {
            let c = b.hook_state(cx);
            assert!(
                c.present & PRESENT_MOVE != 0,
                "lockedmove state has no move"
            );
            Relay::Move(EffectId(c.payload.words[1] as u16))
        }
        _ => panic!("unexpected lockedmove hook"),
    }
}

/// `delete target.volatiles['lockedmove']`: drop the entry without any End callback. The cell is
/// pinned by the running callback, so it retires instead of being freed (like a JS property delete
/// leaving the captured object alive).
fn delete_volatile_raw<L: LogSink>(b: &mut Battle<L>, target: crate::ids::MonId) {
    let found = b.state.pokemon[target.0 as usize]
        .volatiles
        .as_slice()
        .iter()
        .position(|c| b.state.effects.cells[c.0 as usize].id == ID);
    if let Some(index) = found {
        let cell = b.state.pokemon[target.0 as usize].volatiles.remove(index);
        b.state.effects.release(cell);
    }
}
