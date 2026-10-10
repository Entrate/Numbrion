//! Ports data/conditions.ts:287-327 (twoturnmove: Solar Beam, Meteor Beam, Electro Shot, Phantom Force,
//! Shadow Force; `duration: 2` is declarative).
//!
//! Payload map (PAYLOAD_WORDS = 1): `words[0]` = `effectState.move`, the EffectId of the charging
//! move (custom presence bit 8; set in Start before anything else can read it).
//!
//! The same Start also stores `targetLoc` on the *move's own volatile* (`attacker.volatiles[effect.id]`,
//! the bare `solarbeam`/`phantomforce`/... condition cell): word 0 of that cell is the signed
//! location as a two's-complement i32 under custom presence bit 8. That is the encoding the
//! choice code reads (`Battle::ch_locked_target_loc`, side.ts:677-679).
//!
//! PRNG: one `sample` draw only in Start for a fainted defender of a move that was called by
//! another move; nested events (addVolatile, PrepareHit, removeVolatile) own their own draws.
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId, MoveTarget},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, EventArg, HookCtx, Relay, RunEventOptions},
    ids::{EffectId, EventId, Holder},
    log::{LogSink, MoveLineEdit},
    state::{mon_flags, present},
};
pub const ID: EffectId = dex::CONDITION_TWOTURNMOVE;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_TWOTURNMOVE_ONSTART,
    dex::HOOK_CONDITION_TWOTURNMOVE_ONEND,
    dex::HOOK_CONDITION_TWOTURNMOVE_ONLOCKMOVE,
    dex::HOOK_CONDITION_TWOTURNMOVE_ONMOVEABORTED,
];
pub const PAYLOAD_WORDS: usize = 1;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

/// Custom presence bit of `effectState.move` (and of the move volatile's `targetLoc`).
const PRESENT_VALUE: u32 = 1 << present::CUSTOM_START;

fn stored_move<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> EffectId {
    let c = b.hook_state(cx);
    assert!(
        c.present & PRESENT_VALUE != 0,
        "twoturnmove state has no move"
    );
    EffectId(c.payload.words[0] as u16)
}

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/conditions.ts:290-312 onStart(attacker, defender, effect). `attacker` is the user of the
        // two-turn move and the Pokemon the condition is applied to; `effect` is the charging move
        // (the ActiveMove that called addVolatile('twoturnmove', defender)).
        dex::HOOK_CONDITION_TWOTURNMOVE_ONSTART => {
            let attacker = mon_arg(b, cx, 0);
            let mut defender = mon_arg(b, cx, 1);
            let effect = match b.event_arg(cx, 2) {
                EventArg::Effect(e) => e,
                _ => panic!("twoturnmove Start without an effect"),
            };
            let move_id = b.event_effect_id(effect);
            // this.effectState.move = effect.id;
            let c = b.hook_state_mut(cx);
            c.payload.words[0] = u32::from(move_id.0);
            c.present |= PRESENT_VALUE;
            // attacker.addVolatile(effect.id);  (no source/sourceEffect: event defaults apply)
            b.add_volatile(attacker, move_id, Attribution::DEFAULT, None);
            // let moveTargetLoc: number = attacker.lastMoveTargetLoc!;
            let mut target_loc = b.state.pokemon[attacker.0 as usize].last_move_target_loc;
            // if (effect.sourceEffect && this.dex.moves.get(effect.id).target !== 'self'): the move
            // was called by another move (Metronome...), so lastMoveTargetLoc is stale.
            let called = match effect {
                EffectRef::ActiveMove(i) => {
                    b.scratch.moves[i as usize]
                        .as_ref()
                        .expect("released active move")
                        .source_effect
                        != EffectRef::None
                }
                _ => false,
            };
            if called && dex::move_data(move_id).target != MoveTarget::SelfTarget {
                // if (defender.fainted) defender = this.sample(attacker.foes(true));
                if b.state.pokemon[defender.0 as usize].flags & mon_flags::FAINTED != 0 {
                    let foes = b.foes(attacker, true);
                    assert!(foes.len != 0, "Cannot sample an empty array");
                    let i = b.state.prng.sample_index(foes.len as usize);
                    defender = match foes.entries[i] {
                        crate::actions::HitTarget::Pokemon(m) => m,
                        _ => panic!("foes() entry is not a Pokemon"),
                    };
                }
                // moveTargetLoc = attacker.getLocOf(defender);
                target_loc = b.get_loc_of(attacker, defender);
            }
            // attacker.volatiles[effect.id].targetLoc = moveTargetLoc;
            let volatile = b.condition_id(move_id);
            let cell = b
                .get_volatile(attacker, volatile)
                .expect("twoturnmove move volatile missing after addVolatile");
            let c = &mut b.state.effects.cells[cell.0 as usize];
            c.payload.words[0] = i32::from(target_loc) as u32;
            c.present |= PRESENT_VALUE;
            // this.attrLastMove('[still]');
            b.attr_last_move(MoveLineEdit::Still);
            // Run side-effects normally associated with hitting (e.g., Protean, Libero)
            // this.runEvent('PrepareHit', attacker, defender, effect);
            b.run_event(
                EventId::PrepareHit,
                EventArg::Holder(Holder::mon(attacker)),
                EventArg::Holder(Holder::mon(defender)),
                effect,
                Relay::Undefined,
                RunEventOptions::default(),
            );
            Relay::Undefined
        }
        // data/conditions.ts:313-315 onEnd(target): `target.removeVolatile(this.effectState.move)`.
        dex::HOOK_CONDITION_TWOTURNMOVE_ONEND => {
            let target = mon_arg(b, cx, 0);
            let move_id = stored_move(b, cx);
            b.remove_volatile(target, move_id);
            Relay::Undefined
        }
        // data/conditions.ts:316-318 onLockMove(): `return this.effectState.move` (a move id, which the
        // core turns into the forced move).
        dex::HOOK_CONDITION_TWOTURNMOVE_ONLOCKMOVE => Relay::Move(stored_move(b, cx)),
        // data/conditions.ts:319-321 onMoveAborted(pokemon): `pokemon.removeVolatile('twoturnmove')`.
        dex::HOOK_CONDITION_TWOTURNMOVE_ONMOVEABORTED => {
            let pokemon = mon_arg(b, cx, 0);
            b.remove_volatile(pokemon, dex::CONDITION_TWOTURNMOVE);
            Relay::Undefined
        }
        _ => panic!("unexpected twoturnmove hook"),
    }
}
