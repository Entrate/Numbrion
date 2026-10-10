//! Shared private helpers for the `protect_redirection` effect batch (Protect family, Stall,
//! Wide Guard, Follow Me / Rage Powder, Helping Hand and the doubles-support abilities).
//! Hosted by `conditions/protect.rs`; sibling files reach it through
//! `crate::effects::registry::conditions_protect::support`.
//!
//! Pinned source: data/moves.ts (moves and their embedded `condition` objects), data/abilities.ts,
//! data/conditions.ts:437-460. No allocation, no `String`/`format!`, no direct PRNG draws (Stall's
//! `randomChance` is drawn in `conditions/stall.rs`); every nested core call retains its own event
//! draws.
//!
//! Argument layouts used throughout (battle.ts:575-655 singleEvent, :762-944 runEvent):
//!   singleEvent / runEvent without relay -> [target, source, effect]
//!     (Start, SideStart, Try, PrepareHit, Hit, HitSide, TryHit, TryMove, StallMove, HitProtect)
//!   runEvent with relay -> [relay, target, source, effect]
//!     (BasePower, ModifyDamage, RedirectTarget; ModifyMove has the move itself as relay)
//!   Be careful with the event direction: ModifyDamage / BasePower fire as
//!   `runEvent(ev, attacker, defender, move, value)`, so callback arg 1 is the attacker and
//!   arg 2 the defender; RedirectTarget fires as `priorityEvent('RedirectTarget', user, user,
//!   move, target)`, so arg 0 is the current target and arg 1 the user.
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{Attribution, MoveHandle, MoveInput, Stat},
    dex,
    effects::support::{move_arg, move_overlay, optional_id},
    event::{EffectRef, EventArg, HookCtx, Relay, RunEventOptions},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag, MoveLineEdit},
    state::scratch::{OrderedBoosts, move_runtime},
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// `battle.activePerHalf` (battle.ts:221): gen9randomdoublesbattle is a `doubles` game, so 2.
pub const ACTIVE_PER_HALF: u32 = 2;

/// Compile-time lookup of a type by its display name, mirroring `dex::type_id`
/// (`TypeId` = index in `dex::TYPE_NAMES` + 1).
pub const fn type_named(name: &str) -> TypeId {
    let want = name.as_bytes();
    let mut i = 0;
    while i < dex::TYPE_NAMES.len() {
        let have = dex::TYPE_NAMES[i].as_bytes();
        if have.len() == want.len() {
            let mut j = 0;
            let mut same = true;
            while j < have.len() {
                if have[j] != want[j] {
                    same = false;
                }
                j += 1;
            }
            if same {
                return TypeId(i as u8 + 1);
            }
        }
        i += 1;
    }
    panic!("unknown type name")
}
pub const TYPE_ELECTRIC: TypeId = type_named("Electric");
pub const TYPE_STEEL: TypeId = type_named("Steel");
pub const TYPE_WATER: TypeId = type_named("Water");

/// Required Pokemon callback argument.
pub fn mon<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> MonId {
    Battle::<L>::arg_mon(b.event_arg(cx, index)).expect("callback requires Pokemon argument")
}

/// Nullable Pokemon callback argument.
pub fn mon_opt<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> Option<MonId> {
    Battle::<L>::arg_mon(b.event_arg(cx, index))
}

/// `this.effectState.target` of a volatile / ability whose holder is a Pokemon.
pub fn state_target<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> MonId {
    let holder = b.hook_state(cx).target;
    assert!(holder.0 < 12, "effect state target is not a Pokemon");
    MonId(holder.0)
}

/// `this.add(command, pokemon, text)` with a static text argument.
pub fn add_text<L: LogSink>(
    b: &mut Battle<L>,
    command: &'static str,
    pokemon: MonId,
    text: &'static str,
) {
    b.add(LogEntry::new(
        command,
        &[LogArg::Mon(pokemon), LogArg::Text(text)],
        &[],
    ));
}

/// `move.smartTarget` truthiness (present and true).
pub fn smart_target<L: LogSink>(b: &Battle<L>, mv: u8) -> bool {
    let flags = move_overlay(b, mv).runtime_flags;
    flags & move_runtime::SMART_TARGET_PRESENT != 0 && flags & move_runtime::SMART_TARGET != 0
}

/// `move.smartTarget = false`: present and false, never deleted.
pub fn clear_smart_target<L: LogSink>(b: &mut Battle<L>, mv: u8) {
    let m = b.active_move_mut(MoveHandle(mv));
    m.runtime_flags |= move_runtime::SMART_TARGET_PRESENT;
    m.runtime_flags &= !move_runtime::SMART_TARGET;
}

/// pokemon.ts `isSkyDropped()`: the volatile itself, or a foe's Sky Drop volatile naming this
/// Pokemon as its source. Sky Drop is outside the scoped dex (06-scope.md:127), so the
/// volatile key can never be present; the query stays literal, by key, like
/// `is_semi_invulnerable`.
pub fn is_sky_dropped<L: LogSink>(b: &Battle<L>, m: MonId) -> bool {
    if b.query_has_volatile(m, dex::key_ids!("skydrop")) {
        return true;
    }
    let foe = (m.side().0 ^ 1) as usize;
    for &active in &b.state.sides[foe].active {
        if active == MonId::NONE {
            continue;
        }
        let list = &b.state.pokemon[active.0 as usize].volatiles;
        if list.as_slice().iter().any(|c| {
            let cell = &b.state.effects.cells[c.0 as usize];
            dex::key_ids!("skydrop").contains(cell.id) && cell.source == m
        }) {
            return true;
        }
    }
    false
}

/// `const lockedmove = source.getVolatile('lockedmove'); if (lockedmove) { if
/// (source.volatiles['lockedmove'].duration === 2) delete source.volatiles['lockedmove']; }`
/// (data/moves.ts:1015-1021 and its Protect-family / Wide Guard copies). `delete` is a raw map
/// removal: no End event, no linked cleanup, so the volatile is erased, not removed.
pub fn reset_lockedmove<L: LogSink>(b: &mut Battle<L>, source: MonId) {
    if let Some(cell) = b.get_volatile(source, dex::CONDITION_LOCKEDMOVE) {
        if b.state.effects.cells[cell.0 as usize].duration == 2 {
            b.erase_volatile(source, dex::CONDITION_LOCKEDMOVE);
        }
    }
}

// ---------------------------------------------------------------------------------
// Protect family (Protect, Spiky Shield, Baneful Bunker, Burning Bulwark) and Stall.
// ---------------------------------------------------------------------------------

/// `onPrepareHit(pokemon)` of the four stalling moves (data/moves.ts:996-998, 2031-2033,
/// 13972-13974, 17543-17545): `return !!this.queue.willAct() && this.runEvent('StallMove',
/// pokemon);`. PRNG: none directly; the Stall condition's `onStallMove` draws one
/// `randomChance(1, counter)` when the volatile exists.
pub fn stall_prepare_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon(b, cx, 0);
    // !!this.queue.willAct() && ...: a falsy left side yields the boolean false.
    if b.queue_will_act().is_none() {
        return Relay::Bool(false);
    }
    // runEvent('StallMove', pokemon): source and sourceEffect omitted, relay defaults to true.
    b.run_event(
        EventId::StallMove,
        EventArg::Holder(Holder::mon(pokemon)),
        EventArg::Undefined,
        EffectRef::None,
        Relay::Undefined,
        RunEventOptions::default(),
    )
}

/// `pokemon.addVolatile('stall')` (source / sourceEffect default to null, then to the event's).
pub fn add_stall<L: LogSink>(b: &mut Battle<L>, pokemon: MonId) {
    b.add_volatile(pokemon, dex::CONDITION_STALL, Attribution::NONE, None);
}

/// `onHit(pokemon) { pokemon.addVolatile('stall'); }` of the four stalling moves
/// (data/moves.ts:999-1001, 2034-2036, 13975-13977, 17546-17548).
pub fn stall_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon(b, cx, 0);
    add_stall(b, pokemon);
    Relay::Undefined
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shield {
    Protect,
    SpikyShield,
    BanefulBunker,
    BurningBulwark,
}

/// `onStart(target)` of the four shield conditions (data/moves.ts:1004, 2039, 13980, 17551):
/// Protect logs `'Protect'`, the other three `'move: Protect'`.
pub fn shield_start<L: LogSink>(shield: Shield, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon(b, cx, 0);
    let text = if shield == Shield::Protect {
        "Protect"
    } else {
        "move: Protect"
    };
    add_text(b, "-singleturn", target, text);
    Relay::Undefined
}

/// `source.trySetStatus(status, target, this.dex.getActiveMove(<shield move>))`.
fn try_set_status_via<L: LogSink>(
    b: &mut Battle<L>,
    source: MonId,
    target: MonId,
    status: EffectId,
    via: EffectId,
) {
    // A fresh ActiveMove frame, released once the status attempt (and its events) finished.
    let mv = b.get_active_move(MoveInput::Dex(via));
    b.try_set_status(
        source,
        status,
        Attribution::from_move(target, EffectRef::ActiveMove(mv.0)),
    );
    b.release_active_move(mv);
}

/// `onTryHit(target, source, move)` of the four shield conditions (data/moves.ts:1008-1026,
/// 2043-2061, 13984-13999, 17555-17573). Event priority 3 (metadata). Returns
/// `this.NOT_FAIL` after blocking, `undefined` when the move bypasses protection.
/// PRNG: none directly (HitProtect / status / damage events may sort ties).
pub fn shield_try_hit<L: LogSink>(shield: Shield, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon(b, cx, 0);
    let source = mon(b, cx, 1);
    let mv = move_arg(b, cx, 2);
    // checkMoveBypassesProtect(move, source, target[, false for Burning Bulwark])
    if b.check_move_bypasses_protect(
        MoveHandle(mv),
        source,
        target,
        shield != Shield::BurningBulwark,
    ) {
        return Relay::Undefined;
    }
    if smart_target(b, mv) {
        // move.smartTarget = false;
        clear_smart_target(b, mv);
    } else {
        add_text(b, "-activate", target, "move: Protect");
    }
    reset_lockedmove(b, source);
    match shield {
        Shield::Protect => {}
        Shield::SpikyShield => {
            if b.check_move_makes_contact(MoveHandle(mv), source, target, false) {
                // this.damage(source.baseMaxhp / 8, source, target): target is the attacker,
                // the source is the shield holder, the effect defaults to this.effect.
                let amount = f64::from(b.state.pokemon[source.0 as usize].max_hp) / 8.0;
                b.damage(
                    amount,
                    Some(source),
                    Attribution::from_move(target, EffectRef::None),
                );
            }
        }
        Shield::BanefulBunker => {
            if b.check_move_makes_contact(MoveHandle(mv), source, target, false) {
                try_set_status_via(
                    b,
                    source,
                    target,
                    dex::CONDITION_PSN,
                    dex::MOVE_BANEFULBUNKER,
                );
            }
        }
        Shield::BurningBulwark => {
            if b.check_move_makes_contact(MoveHandle(mv), source, target, false) {
                try_set_status_via(
                    b,
                    source,
                    target,
                    dex::CONDITION_BRN,
                    dex::MOVE_BURNINGBULWARK,
                );
            }
        }
    }
    Relay::NotFail
}

/// `onHit(target, source, move)` of Spiky Shield / Baneful Bunker / Burning Bulwark conditions
/// (data/moves.ts:1027-1031, 2062-2066, 17574-17578): `move.isZOrMaxPowered && ...`.
/// Z-moves and Max moves do not exist in gen9randomdoublesbattle (the ActiveMove has no such
/// property), so the short-circuit always yields false and nothing runs. PRNG: none.
pub fn shield_hit_z_or_max<L: LogSink>(_b: &mut Battle<L>, _cx: HookCtx) -> Relay {
    Relay::Undefined
}

// ---------------------------------------------------------------------------------
// Abilities: Lightning Rod / Storm Drain, Armor Tail / Queenly Majesty, tracking moves.
// ---------------------------------------------------------------------------------

/// A boost object holding only `spa: 1` (`this.boost({ spa: 1 })`).
fn spa_plus_one() -> OrderedBoosts {
    let mut out = OrderedBoosts::default();
    let i = Stat::SpA as usize;
    out.values[i] = 1;
    out.order[0] = i as u8;
    out.len = 1;
    out.present = 1 << i;
    out
}

/// `onTryHit(target, source, move)` of Lightning Rod (data/abilities.ts:2345-2352) and Storm
/// Drain (:4638-4645). Returns null after absorbing. PRNG: none directly (boost events).
pub fn absorb_try_hit<L: LogSink>(
    ability: EffectId,
    move_type: TypeId,
    b: &mut Battle<L>,
    cx: HookCtx,
) -> Relay {
    let target = mon(b, cx, 0);
    let source = mon_opt(b, cx, 1);
    let mv = move_arg(b, cx, 2);
    // target !== source && move.type === 'Electric' / 'Water'
    if Some(target) != source && move_overlay(b, mv).move_type == move_type {
        // this.boost({ spa: 1 }): target, source and effect all default from the event/effect.
        if !b
            .boost(spa_plus_one(), None, Attribution::DEFAULT, false, false)
            .truthy()
        {
            b.add(LogEntry::new(
                "-immune",
                &[LogArg::Mon(target)],
                &[LogTag::From(EffectRef::Dex(ability))],
            ));
        }
        return Relay::Null;
    }
    Relay::Undefined
}

/// `onAnyRedirectTarget(target, source, source2, move)` of Lightning Rod (data/abilities.ts:
/// 2353-2363) and Storm Drain (:4646-4656). Args: [current target (relay), user, user, move].
/// PRNG: none (the event ordering itself is stable, fastExit).
pub fn absorb_redirect_target<L: LogSink>(
    ability: EffectId,
    move_type: TypeId,
    b: &mut Battle<L>,
    cx: HookCtx,
) -> Relay {
    let current = mon_opt(b, cx, 0);
    let user = mon(b, cx, 1);
    let mv = move_arg(b, cx, 3);
    let (ty, kind) = {
        let m = move_overlay(b, mv);
        (m.move_type, m.target)
    };
    // move.type !== 'Electric' || move.flags['pledgecombo']: no scoped move has pledgecombo
    // (pledge moves are outside the scope), so only the type test remains.
    if ty != move_type {
        return Relay::Undefined;
    }
    // ['randomNormal', 'adjacentFoe'].includes(move.target) ? 'normal' : move.target
    // ('adjacentFoe' is not a scoped target kind.)
    let redirect_target = if kind == dex::MoveTarget::RandomNormal {
        dex::MoveTarget::Normal
    } else {
        kind
    };
    let holder = state_target(b, cx);
    if b.valid_target(holder, user, redirect_target) {
        if smart_target(b, mv) {
            clear_smart_target(b, mv);
        }
        if Some(holder) != current {
            b.add(LogEntry::new(
                "-activate",
                &[
                    LogArg::Mon(holder),
                    LogArg::EffectFullName(EffectRef::Dex(ability)),
                ],
                &[],
            ));
        }
        return Relay::Pokemon(holder);
    }
    Relay::Undefined
}

/// `targetAllExceptions.includes(move.id)` with `['perishsong', 'flowershield', 'rototiller']`.
/// None of the three is in the scoped dex, so `optional_id` yields NONE (which never matches).
fn is_all_exception(id: EffectId) -> bool {
    const PERISHSONG: EffectId = optional_id(dex::MOVES_DATA, "perishsong");
    const FLOWERSHIELD: EffectId = optional_id(dex::MOVES_DATA, "flowershield");
    const ROTOTILLER: EffectId = optional_id(dex::MOVES_DATA, "rototiller");
    (PERISHSONG != EffectId::NONE && id == PERISHSONG)
        || (FLOWERSHIELD != EffectId::NONE && id == FLOWERSHIELD)
        || (ROTOTILLER != EffectId::NONE && id == ROTOTILLER)
}

/// `onFoeTryMove(target, source, move)` of Armor Tail (data/abilities.ts:216-228) and Queenly
/// Majesty (:3718-3730). Args: [mover (event target), the move's target (event source), move].
/// PRNG: none.
pub fn foe_try_move<L: LogSink>(ability: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon(b, cx, 0);
    let source = mon(b, cx, 1);
    let mv = move_arg(b, cx, 2);
    let (kind, id, priority) = {
        let m = move_overlay(b, mv);
        (m.target, m.id, m.priority)
    };
    // move.target === 'foeSide' || (move.target === 'all' && !targetAllExceptions.includes(move.id))
    if kind == dex::MoveTarget::FoeSide || (kind == dex::MoveTarget::All && !is_all_exception(id)) {
        return Relay::Undefined;
    }
    // const armorTailHolder = this.effectState.target;
    let holder = state_target(b, cx);
    // (source.isAlly(holder) || move.target === 'all') && move.priority > 0.1
    if (b.is_ally(source, Some(holder)) || kind == dex::MoveTarget::All)
        && f64::from(priority) > 0.1
    {
        b.attr_last_move(MoveLineEdit::Still);
        b.add(LogEntry::new(
            "cant",
            &[
                LogArg::Mon(holder),
                LogArg::EffectFullName(EffectRef::Dex(ability)),
                LogArg::Effect(EffectRef::ActiveMove(mv)),
            ],
            &[LogTag::Of(target)],
        ));
        return Relay::Bool(false);
    }
    Relay::Undefined
}

/// `onModifyMove(move)` of Propeller Tail (data/abilities.ts:3489-3492) and Stalwart (:4506-4509):
/// `move.tracksTarget = move.target !== 'scripted'`. 'scripted' is not a scoped target kind
/// (MoveTarget has no such variant), so the assignment is always `true`. PRNG: none.
pub fn modify_move_tracks_target<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 0);
    b.active_move_mut(MoveHandle(mv)).runtime_flags |= move_runtime::TRACKS_TARGET;
    Relay::Undefined
}
