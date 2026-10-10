//! Shared private helpers for the `hazards_screens` effect batch (screens, entry hazards, Tailwind,
//! Trick Room and the moves that clear/shatter/swap them). Hosted by `conditions/reflect.rs`; sibling
//! effect files reach it through `crate::effects::registry::conditions_reflect::support`.
//!
//! Pinned source: data/moves.ts (condition objects embedded in each move), sim/side.ts:413-462,
//! sim/battle.ts:488-575 (fieldEvent SwitchIn args). No allocation, no `String`/`format!`, no direct
//! PRNG draws; every nested core call (damage/boost/status/removal) retains its own event draws.
//!
//! Argument layouts used throughout (battle.ts:575-655 singleEvent, :762-944 runEvent):
//!   singleEvent without relay -> [target, source, effect]      (SideStart, SideEnd, SwitchIn, TryHit, AfterHit...)
//!   singleEvent/runEvent with relay -> [relay, target, source, effect] (AfterSubDamage, ModifyDamage, ModifySpe)
//!   direct durationCallback.call -> side: [active[0], source, sourceEffect]; field: [source, effect]
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{Attribution, MoveHandle, MoveInput},
    dex,
    effects::support::{mon_arg, move_arg, move_overlay},
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        CellId, EffectCell, present,
        scratch::{OrderedBoosts, move_runtime},
    },
};

#[cfg(test)]
mod tests;

/// `TypeId` = index in `dex::TYPE_NAMES` + 1 (checked by `tests::type_ids_match_dex`).
pub const TYPE_FIGHTING: TypeId = TypeId(2);
pub const TYPE_POISON: TypeId = TypeId(4);
pub const TYPE_STEEL: TypeId = TypeId(9);
pub const TYPE_FIRE: TypeId = TypeId(10);
pub const TYPE_WATER: TypeId = TypeId(11);

/// `battle.activePerHalf` (battle.ts:221): gen9randomdoublesbattle is a `doubles` game, so 2.
/// Screens therefore always use the 2732/4096 branch (data/moves.ts:861,10339,14866).
pub const ACTIVE_PER_HALF: u8 = 2;

/// Spikes / Toxic Spikes `effectState.layers`: payload word 0, custom presence bit 8. This is the
/// layout the reactive-contact batch already reads for Toxic Debris (reactivecontact/host.rs).
pub const LAYERS_PRESENT: u32 = 1 << 8;

/// Mirrors the `HookCtx`-free side holder encoding (`Holder::side`, ids.rs).
pub fn side_of(holder: Holder) -> SideId {
    assert!(
        holder.0 == 12 || holder.0 == 13,
        "side-condition callback requires a Side holder"
    );
    SideId(holder.0 - 12)
}
/// Side-typed callback argument (SideStart/SideEnd/SideRestart target).
pub fn side_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> SideId {
    match b.event_arg(cx, index) {
        EventArg::Holder(h) => side_of(h),
        _ => panic!("callback requires Side argument"),
    }
}
/// `source?.` optional-chaining read of a Pokemon argument (undefined/null stay None).
pub fn opt_mon_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> Option<MonId> {
    Battle::<L>::arg_mon(b.event_arg(cx, index))
}
/// Doubles has exactly one foe side (`side.foe`, `foeSidesWithConditions()` = [foe]; side.ts:376-380).
pub const fn foe_of(side: SideId) -> SideId {
    SideId(side.0 ^ 1)
}
/// `side.foe.active[0]` as an attribution source (may be an empty slot -> explicit null).
pub fn foe_active0_source<L: LogSink>(b: &Battle<L>, side: SideId) -> EventArg {
    let m = b.state.sides[foe_of(side).0 as usize].active[0];
    if m == MonId::NONE {
        EventArg::Null
    } else {
        EventArg::Holder(Holder::mon(m))
    }
}
pub fn has_sheer_force<L: LogSink>(b: &Battle<L>, mv: u8) -> bool {
    move_overlay(b, mv).runtime_flags & move_runtime::HAS_SHEER_FORCE != 0
}
/// Sparse `{stat: delta, ...}` object keeping JS insertion order (index: atk0 def1 spa2 spd3 spe4).
pub fn boosts_of(entries: &[(u8, i8)]) -> OrderedBoosts {
    let mut o = OrderedBoosts::default();
    for &(stat, delta) in entries {
        o.values[stat as usize] = delta;
        o.order[o.len as usize] = stat;
        o.len += 1;
        o.present |= 1 << stat;
    }
    o
}
pub const STAT_ATK: u8 = 0;
pub const STAT_SPE: u8 = 4;

// ---------------------------------------------------------------------------------------------
// Screens: Reflect / Light Screen / Aurora Veil
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Reflect,
    LightScreen,
    AuroraVeil,
}

/// durationCallback(target, source, effect) for the three screens (data/moves.ts:845,10327,14854).
/// Direct call: args [side.active[0], source, sourceEffect]. `source?.hasItem('lightclay')`.
/// PRNG: none.
pub fn screen_duration<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match opt_mon_arg(b, cx, 1) {
        Some(source) if b.has_item(source, &[dex::ITEM_LIGHTCLAY]) => Relay::Number(8.0),
        _ => Relay::Number(5.0),
    }
}

/// onAnyModifyDamage(damage, source, target, move) (data/moves.ts:851,10333,14860).
/// runEvent('ModifyDamage', attacker, defender, move, damage) => args [damage, attacker, defender,
/// move]; the handler's `source` is the attacker and its `target` the defender. `this.effectState.target`
/// is the screen's own Side. `chainModify` mutates the frame and returns undefined. PRNG: none.
pub fn screen_any_modify_damage<L: LogSink>(
    screen: Screen,
    b: &mut Battle<L>,
    cx: HookCtx,
) -> Relay {
    let attacker = mon_arg(b, cx, 1);
    let defender = mon_arg(b, cx, 2);
    let mv = move_arg(b, cx, 3);
    // target !== source
    if defender == attacker {
        return Relay::Undefined;
    }
    // this.effectState.target.hasAlly(target): no allySide outside multi battles.
    let screen_side = side_of(b.hook_state(cx).target);
    if defender.side() != screen_side {
        return Relay::Undefined;
    }
    // this.getCategory(move): ActiveMove.category (never falsy).
    let category = move_overlay(b, mv).category;
    match screen {
        Screen::Reflect => {
            if category != dex::Category::Physical {
                return Relay::Undefined;
            }
        }
        Screen::LightScreen => {
            if category != dex::Category::Special {
                return Relay::Undefined;
            }
        }
        Screen::AuroraVeil => {
            // Reflect/Light Screen on the defender's side already weaken that category.
            let side = defender.side();
            if (b.get_side_condition(side, dex::CONDITION_REFLECT).is_some()
                && category == dex::Category::Physical)
                || (b
                    .get_side_condition(side, dex::CONDITION_LIGHTSCREEN)
                    .is_some()
                    && category == dex::Category::Special)
            {
                return Relay::Undefined;
            }
        }
    }
    // !target.getMoveHitData(move).crit && !move.infiltrates (hit data is created on first read).
    let crit = b.move_hit_data(defender, MoveHandle(mv)).crit;
    if !crit && move_overlay(b, mv).runtime_flags & move_runtime::INFILTRATES == 0 {
        // this.debug('... weaken') is not a protocol line.
        if ACTIVE_PER_HALF > 1 {
            b.chain_modify(2732.0, 4096.0);
        } else {
            b.chain_modify(0.5, 1.0);
        }
    }
    Relay::Undefined
}

/// onSideStart(side) / onSideEnd(side) of the screens: a single literal line.
/// `command` is "-sidestart" or "-sideend"; `text` is the call-site literal (Reflect has no
/// "move: " prefix: data/moves.ts:14870 vs 862,10343). PRNG: none.
pub fn side_line<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    command: &'static str,
    text: &'static str,
) -> Relay {
    let side = side_arg(b, cx, 0);
    b.add(LogEntry::new(
        command,
        &[LogArg::Side(side), LogArg::Text(text)],
        &[],
    ));
    Relay::Undefined
}

// ---------------------------------------------------------------------------------------------
// Entry hazards
// ---------------------------------------------------------------------------------------------

/// Spikes/Toxic Spikes onSideStart: `-sidestart`, then `effectState.layers = 1`.
pub fn layered_side_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, text: &'static str) -> Relay {
    let side = side_arg(b, cx, 0);
    b.add(LogEntry::new(
        "-sidestart",
        &[LogArg::Side(side), LogArg::Text(text)],
        &[],
    ));
    set_layers(b.hook_state_mut(cx), 1);
    Relay::Undefined
}
/// Spikes/Toxic Spikes onSideRestart: `layers >= max` returns false (the whole move fails);
/// otherwise `-sidestart` and `layers++`.
pub fn layered_side_restart<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    text: &'static str,
    max_layers: u32,
) -> Relay {
    if layers(b.hook_state(cx)) >= max_layers {
        return Relay::Bool(false);
    }
    let side = side_arg(b, cx, 0);
    b.add(LogEntry::new(
        "-sidestart",
        &[LogArg::Side(side), LogArg::Text(text)],
        &[],
    ));
    let next = layers(b.hook_state(cx)) + 1;
    set_layers(b.hook_state_mut(cx), next);
    Relay::Undefined
}
pub fn layers(cell: &EffectCell) -> u32 {
    cell.payload.words[0]
}
pub fn set_layers(cell: &mut EffectCell, layers: u32) {
    cell.payload.words[0] = layers;
    cell.present |= LAYERS_PRESENT;
}

/// Spikes onSwitchIn(pokemon) (data/moves.ts:17521). PRNG: none directly (damage events).
pub fn spikes_switch_in<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if !b.is_grounded(m, false).truthy() || b.has_item(m, &[dex::ITEM_HEAVYDUTYBOOTS]) {
        return Relay::Undefined;
    }
    let damage_amounts = [0.0, 3.0, 4.0, 6.0]; // 1/8, 1/6, 1/4
    let layer_index = layers(b.hook_state(cx)) as usize;
    let max_hp = f64::from(b.state.pokemon[m.0 as usize].max_hp);
    // this.damage(x): target/source/effect default from the SwitchIn event (target = pokemon).
    b.damage(
        damage_amounts[layer_index] * max_hp / 24.0,
        None,
        Attribution::DEFAULT,
    );
    Relay::Undefined
}

/// Stealth Rock onSwitchIn(pokemon) (data/moves.ts:17829). PRNG: none directly.
pub fn stealth_rock_switch_in<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if b.has_item(m, &[dex::ITEM_HEAVYDUTYBOOTS]) {
        return Relay::Undefined;
    }
    // this.dex.getActiveMove('stealthrock') is a fresh scratch frame, released after the query.
    let mv = b.get_active_move(MoveInput::Dex(dex::MOVE_STEALTHROCK));
    let effectiveness = b.run_effectiveness(m, mv);
    b.release_active_move(mv);
    // this.clampIntRange(..., -6, 6)
    let type_mod = effectiveness.clamp(-6, 6);
    let max_hp = f64::from(b.state.pokemon[m.0 as usize].max_hp);
    b.damage(
        max_hp * 2f64.powi(i32::from(type_mod)) / 8.0,
        None,
        Attribution::DEFAULT,
    );
    Relay::Undefined
}

/// Sticky Web onSwitchIn(pokemon) (data/moves.ts:17949). PRNG: none directly (boost events).
pub fn sticky_web_switch_in<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if !b.is_grounded(m, false).truthy() || b.has_item(m, &[dex::ITEM_HEAVYDUTYBOOTS]) {
        return Relay::Undefined;
    }
    b.add(LogEntry::new(
        "-activate",
        &[LogArg::Mon(m), LogArg::Text("move: Sticky Web")],
        &[],
    ));
    // this.boost({ spe: -1 }, pokemon, pokemon.side.foe.active[0], this.dex.getActiveMove('stickyweb'))
    let source = foe_active0_source(b, m.side());
    let mv = b.get_active_move(MoveInput::Dex(dex::MOVE_STICKYWEB));
    b.boost(
        boosts_of(&[(STAT_SPE, -1)]),
        Some(m),
        Attribution {
            source,
            effect: EffectRef::ActiveMove(mv.0),
        },
        false,
        false,
    );
    b.release_active_move(mv);
    Relay::Undefined
}

/// Toxic Spikes onSwitchIn(pokemon) (data/moves.ts:19770). PRNG: none directly (status events).
pub fn toxic_spikes_switch_in<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if !b.is_grounded(m, false).truthy() {
        return Relay::Undefined;
    }
    if b.has_type(m, &[TYPE_POISON]) {
        b.add(LogEntry::new(
            "-sideend",
            &[LogArg::Side(m.side()), LogArg::Text("move: Toxic Spikes")],
            &[LogTag::Of(m)],
        ));
        b.remove_side_condition(m.side(), dex::CONDITION_TOXICSPIKES);
    } else if b.has_type(m, &[TYPE_STEEL]) || b.has_item(m, &[dex::ITEM_HEAVYDUTYBOOTS]) {
        // do nothing
    } else if layers(b.hook_state(cx)) >= 2 {
        let source = foe_active0_source(b, m.side());
        b.try_set_status(
            m,
            dex::CONDITION_TOX,
            Attribution {
                source,
                effect: EffectRef::None,
            },
        );
    } else {
        let source = foe_active0_source(b, m.side());
        b.try_set_status(
            m,
            dex::CONDITION_PSN,
            Attribution {
                source,
                effect: EffectRef::None,
            },
        );
    }
    Relay::Undefined
}

// ---------------------------------------------------------------------------------------------
// Move-side helpers
// ---------------------------------------------------------------------------------------------

/// Brick Break / Psychic Fangs / Raging Bull onTryHit(pokemon) (data/moves.ts:1831,14069,14642).
/// `pokemon` is the first argument = the move's *target*; its side loses the three screens, in this
/// order, each firing SideEnd (`-sideend`). Returns undefined. PRNG: none.
pub fn shatter_screens<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let side = target.side();
    b.remove_side_condition(side, dex::CONDITION_REFLECT);
    b.remove_side_condition(side, dex::CONDITION_LIGHTSCREEN);
    b.remove_side_condition(side, dex::CONDITION_AURORAVEIL);
    Relay::Undefined
}

/// Ceaseless Edge / Stone Axe onAfterHit / onAfterSubDamage (data/moves.ts:2229-2243,18078-18092).
/// `after_sub` selects the (damage, target, source, move) argument layout and the extra `source.hp`
/// guard. `side.addSideCondition(id)` passes no source: SideStart sees the event target (see report).
/// PRNG: none directly.
pub fn add_foe_hazard<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    hazard: EffectId,
    after_sub: bool,
) -> Relay {
    let (source_index, move_index) = if after_sub { (2, 3) } else { (1, 2) };
    let source = mon_arg(b, cx, source_index);
    let mv = move_arg(b, cx, move_index);
    if has_sheer_force(b, mv) {
        return Relay::Undefined;
    }
    if after_sub && b.state.pokemon[source.0 as usize].hp == 0 {
        return Relay::Undefined;
    }
    // for (const side of source.side.foeSidesWithConditions()) side.addSideCondition(hazard)
    b.add_side_condition(foe_of(source.side()), hazard, Attribution::DEFAULT);
    Relay::Undefined
}

/// Rapid Spin / Mortal Spin onAfterHit / onAfterSubDamage (data/moves.ts:12323-12357,14703-14737).
/// AfterSubDamage re-checks `pokemon.hp` before every step; AfterHit does not. `mover` supplies the
/// `[from] move: X` tag. `gmaxsteelsurge` is outside the generated dex and can never be present, so its
/// removeSideCondition is a no-op that logs nothing. PRNG: none directly.
pub fn spin_clear<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    mover: EffectId,
    after_sub: bool,
) -> Relay {
    let (user_index, move_index) = if after_sub { (2, 3) } else { (1, 2) };
    let user = mon_arg(b, cx, user_index);
    let mv = move_arg(b, cx, move_index);
    if has_sheer_force(b, mv) {
        return Relay::Undefined;
    }
    let alive = |b: &Battle<L>| !after_sub || b.state.pokemon[user.0 as usize].hp != 0;
    if alive(b) && b.remove_volatile(user, dex::CONDITION_LEECHSEED) {
        b.add(LogEntry::new(
            "-end",
            &[LogArg::Mon(user), LogArg::Text("Leech Seed")],
            &[LogTag::From(EffectRef::Dex(mover)), LogTag::Of(user)],
        ));
    }
    let side = user.side();
    for condition in [
        dex::CONDITION_SPIKES,
        dex::CONDITION_TOXICSPIKES,
        dex::CONDITION_STEALTHROCK,
        dex::CONDITION_STICKYWEB,
    ] {
        if alive(b) && b.remove_side_condition(side, condition) {
            b.add(LogEntry::new(
                "-sideend",
                &[
                    LogArg::Side(side),
                    LogArg::Effect(EffectRef::Dex(condition)),
                ],
                &[LogTag::From(EffectRef::Dex(mover)), LogTag::Of(user)],
            ));
        }
    }
    if alive(b)
        && b.get_volatile(user, dex::CONDITION_PARTIALLYTRAPPED)
            .is_some()
    {
        b.remove_volatile(user, dex::CONDITION_PARTIALLYTRAPPED);
    }
    Relay::Undefined
}

/// Court Change's `sideConditions` allow-list intersected with the generated side-condition set
/// (data/moves.ts:3042-3044). Wide Guard is the only in-scope side condition that is not swapped.
pub fn court_change_swaps(id: EffectId) -> bool {
    id == dex::CONDITION_LIGHTSCREEN
        || id == dex::CONDITION_REFLECT
        || id == dex::CONDITION_SPIKES
        || id == dex::CONDITION_TAILWIND
        || id == dex::CONDITION_TOXICSPIKES
        || id == dex::CONDITION_STEALTHROCK
        || id == dex::CONDITION_STICKYWEB
        || id == dex::CONDITION_AURORAVEIL
}

/// The non-free-for-all branch of Court Change (data/moves.ts:3066-3099). Returns `success`.
/// JS objects iterate in insertion order and `delete` keeps the survivors' order, so the allow-listed
/// cells leave each side's list in order and are appended to the other side's list in that same order.
/// Each moved state keeps its identity, effectOrder, payload and generation; only `target` (and
/// the physical holder) changes. PRNG: none; no events fire.
pub fn swap_side_conditions<L: LogSink>(b: &mut Battle<L>, source_side: SideId) -> bool {
    let a = source_side.0 as usize;
    let f = foe_of(source_side).0 as usize;
    let mut success = false;
    let mut taken = [([CellId::NONE; 9], 0usize); 2];
    for (slot, side) in [a, f].into_iter().enumerate() {
        let mut list = b.state.sides[side].conditions;
        let mut i = 0;
        while i < list.len as usize {
            let cell = list.cells[i];
            if court_change_swaps(b.state.effects.cells[cell.0 as usize].id) {
                list.remove(i);
                taken[slot].0[taken[slot].1] = cell;
                taken[slot].1 += 1;
                success = true;
            } else {
                i += 1;
            }
        }
        b.state.sides[side].conditions = list;
    }
    // targetSideConditions[id] = sourceTemp[id]; .target = foe  (then the reverse).
    for (slot, dest) in [(0usize, f), (1usize, a)] {
        let holder = Holder::side(SideId(dest as u8));
        for k in 0..taken[slot].1 {
            let cell = taken[slot].0[k];
            let c = &mut b.state.effects.cells[cell.0 as usize];
            c.target = holder;
            c.owner = holder;
            c.present |= present::TARGET;
            b.state.sides[dest].conditions.push(cell);
        }
    }
    success
}

/// Tidy Up's removeAll list (data/moves.ts:19638); `gmaxsteelsurge` is never present in this scope.
pub const TIDY_UP_HAZARDS: [EffectId; 4] = [
    dex::CONDITION_SPIKES,
    dex::CONDITION_TOXICSPIKES,
    dex::CONDITION_STEALTHROCK,
    dex::CONDITION_STICKYWEB,
];
