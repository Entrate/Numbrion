//! Ports data/moves.ts:20926 (Wish's embedded slot condition). No direct PRNG draws; `heal`
//! runs TryHeal/Heal events, whose listeners may speed-sort ties.
//!
//! Payload map (slot-condition cell, `PAYLOAD_WORDS = 3`):
//! * words[0..2] = `effectState.hp` (`source.maxhp / 2`, a JS float: x.5 for odd max HP) as
//!   `f64::to_bits` (low word first), presence bit 8;
//! * words[2] = `effectState.startingTurn` (`getOverflowedTurnCount()`, 0..=255), presence bit 9.
//!
//! The wisher is the common `source`/`sourceSlot` pair that addSlotCondition stores
//! (side.ts:464-494); `duration` is absent (Wish has none), so it never expires by count.
use crate::{
    Battle,
    actions::HealEffect,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::{EffectId, MonId, SlotId},
    log::{LogArg, LogEntry, LogSink, LogTag},
    math,
    state::{mon_flags, present},
};
pub const ID: EffectId = dex::CONDITION_WISH;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_CONDITION_WISH_ONSTART,
    dex::HOOK_CONDITION_WISH_ONRESIDUAL,
    dex::HOOK_CONDITION_WISH_ONEND,
];
pub const PAYLOAD_WORDS: usize = 3;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);

/// Custom presence bits (EffectCell.present bits 8..30 are per-effect).
const HP_PRESENT: u32 = 1 << present::CUSTOM_START;
const STARTING_TURN_PRESENT: u32 = 1 << (present::CUSTOM_START + 1);

pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_CONDITION_WISH_ONSTART => on_start(b, cx),
        dex::HOOK_CONDITION_WISH_ONRESIDUAL => on_residual(b, cx),
        dex::HOOK_CONDITION_WISH_ONEND => on_end(b, cx),
        _ => panic!("unexpected wish hook"),
    }
}

/// `this.getOverflowedTurnCount()` (battle.ts:3316): `trunc(turn - 1, 8)` in Gen 8+.
fn overflowed_turn_count<L: LogSink>(b: &Battle<L>) -> u32 {
    math::trunc_bits(i64::from(b.state.turn) - 1, 8)
}

fn stored_hp<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> f64 {
    let state = b.hook_state(cx);
    debug_assert!(state.present & HP_PRESENT != 0, "Wish hp was never set");
    f64::from_bits(u64::from(state.payload.words[0]) | (u64::from(state.payload.words[1]) << 32))
}

// data/moves.ts:20926-20932 onStart(pokemon, source). PRNG: none.
fn on_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let source = mon_arg(b, cx, 1);
    // this.effectState.hp = source.maxhp / 2;
    let hp = f64::from(b.state.pokemon[source.0 as usize].max_hp) / 2.0;
    // this.effectState.startingTurn = this.getOverflowedTurnCount();
    let starting_turn = overflowed_turn_count(b);
    let bits = hp.to_bits();
    let state = b.hook_state_mut(cx);
    state.payload.words[0] = bits as u32;
    state.payload.words[1] = (bits >> 32) as u32;
    state.payload.words[2] = starting_turn;
    state.present |= HP_PRESENT | STARTING_TURN_PRESENT;
    // if (this.effectState.startingTurn === 255) this.hint(`In Gen 8+, Wish will never resolve
    // when used on the ${this.turn}th turn.`);
    if starting_turn == 255 {
        let turn = b.turn();
        b.hint(
            LogArg::Parts(&[
                LogArg::Text("In Gen 8+, Wish will never resolve when used on the "),
                LogArg::Number(turn as i32),
                LogArg::Text("th turn."),
            ]),
            false,
            None,
        );
    }
    Relay::Undefined
}

// data/moves.ts:20934-20937 onResidual(target). PRNG: none directly. Residual order 4 is
// manifest metadata.
fn on_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let state = b.hook_state(cx);
    debug_assert!(state.present & STARTING_TURN_PRESENT != 0);
    let (starting_turn, source_slot) = (state.payload.words[2], state.source_slot);
    // if (this.getOverflowedTurnCount() <= this.effectState.startingTurn) return;
    if overflowed_turn_count(b) <= starting_turn {
        return Relay::Undefined;
    }
    // target.side.removeSlotCondition(this.getAtSlot(this.effectState.sourceSlot), 'wish');
    // removeSlotCondition(Pokemon) uses `pokemon.position` (an undefined occupant would throw).
    let wisher_slot = b
        .get_at_slot(source_slot)
        .expect("Wish sourceSlot has no active Pokemon (removeSlotCondition would throw)");
    debug_assert_eq!(wisher_slot.side(), target.side());
    let slot: SlotId = b.get_slot(wisher_slot);
    b.remove_slot_condition(slot, ID);
    Relay::Undefined
}

// data/moves.ts:20938-20945 onEnd(target). PRNG: none directly.
fn on_end<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    // if (target && !target.fainted)
    let Some(target) = Battle::<L>::arg_mon(b.event_arg(cx, 0)) else {
        return Relay::Undefined;
    };
    if b.state.pokemon[target.0 as usize].flags & mon_flags::FAINTED != 0 {
        return Relay::Undefined;
    }
    // const damage = this.heal(this.effectState.hp, target, target);  (effect defaults to this.effect,
    // the Wish condition, for which heal itself prints nothing)
    let hp = stored_hp(b, cx);
    let wisher = b.hook_state(cx).source;
    let damage = b.heal(hp, Some(target), Some(target), HealEffect::Context);
    // if (damage) this.add('-heal', target, target.getHealth, '[from] move: Wish',
    //   '[wisher] ' + this.effectState.source.name);
    if damage.truthy() {
        assert_ne!(wisher, MonId::NONE, "Wish state has no source");
        // pokemon.name is the (<= 20 UTF-16 unit) team-set nickname: at most 60 UTF-8 bytes.
        // Copy it to the stack so the log entry does not borrow the battle; NoLog skips the copy.
        let mut name = [0u8; 64];
        let mut len = 0;
        if L::ENABLED {
            let text = b.teams().sides[wisher.side().0 as usize].sets[wisher.0 as usize % 6]
                .name
                .as_bytes();
            len = text.len();
            name[..len].copy_from_slice(text);
        }
        let name = core::str::from_utf8(&name[..len]).expect("Pokemon name is valid UTF-8");
        b.add(LogEntry::split(
            "-heal",
            &[LogArg::Mon(target), LogArg::Health(target)],
            &[
                LogTag::From(EffectRef::Dex(dex::MOVE_WISH)),
                LogTag::Value("wisher", LogArg::Text(name)),
            ],
            target.side(),
            false,
        ));
    }
    Relay::Undefined
}
