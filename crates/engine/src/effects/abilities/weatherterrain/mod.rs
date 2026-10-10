//! Private shared bodies for the weather_terrain batch (weathers, terrains, setters
//! and their abilities/moves). Every caller records its pinned source location.
//! No body in this module draws directly from the PRNG: the only draws are the
//! speedSort tie shuffles and handler draws of nested core events
//! (`each_event`, `is_grounded`/`has_type` Type events, `heal`, `damage`,
//! `set_weather`, `set_terrain`), and each caller documents which of those it can reach.
//!
//! Scope facts used throughout (data/scope.json, docs/showdown/06-scope.md:131):
//! Hail, Primordial Sea, Desolate Land, Delta Stream and Misty Terrain, Hydro Steam,
//! Bulldoze/Magnitude and the Damp/Heat/Smooth/Icy Rock, Terrain Extender, Blue/Red Orb
//! and Utility Umbrella items are outside the generated dex. Source branches naming them
//! are therefore unreachable for the scoped weather ids; they are noted at each use and
//! no executable id is fabricated for them. `gen <= 5` branches are unreachable in Gen 9.
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{Attribution, HealEffect, MoveHandle},
    dex::{self, EffectType, ImmunityId, MoveTarget},
    effects::support::{mon_arg, move_arg},
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        Status, present,
        scratch::{ActiveMove, MoveAccuracy},
    },
};
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

// ---------------------------------------------------------------------------------
// Compile-time type ids (TYPE_NAMES order is the TypeId order, TypeId = index + 1).
// ---------------------------------------------------------------------------------
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
pub const fn type_named(name: &str) -> TypeId {
    let mut i = 0;
    while i < dex::TYPE_NAMES.len() {
        if same(dex::TYPE_NAMES[i], name) {
            return TypeId(i as u8 + 1);
        }
        i += 1;
    }
    panic!("unknown type name")
}
pub const TYPE_ELECTRIC: TypeId = type_named("Electric");
pub const TYPE_FIRE: TypeId = type_named("Fire");
pub const TYPE_GRASS: TypeId = type_named("Grass");
pub const TYPE_GROUND: TypeId = type_named("Ground");
pub const TYPE_ICE: TypeId = type_named("Ice");
pub const TYPE_PSYCHIC: TypeId = type_named("Psychic");
pub const TYPE_ROCK: TypeId = type_named("Rock");
pub const TYPE_STEEL: TypeId = type_named("Steel");
pub const TYPE_WATER: TypeId = type_named("Water");

// ---------------------------------------------------------------------------------
// Argument access.
// ---------------------------------------------------------------------------------
/// Pokemon argument that TS may leave undefined/null (e.g. a ModifyMove target).
pub fn opt_mon<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> Option<MonId> {
    Battle::<L>::arg_mon(b.event_arg(cx, i))
}
/// Effect-shaped argument (source effect, Condition relay or ActiveMove effect).
pub fn effect_ref<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> EffectRef {
    match b.event_arg(cx, i) {
        EventArg::Effect(e) => e,
        EventArg::Move(m) | EventArg::Relay(Relay::ActiveMove(m)) => EffectRef::ActiveMove(m),
        EventArg::Relay(Relay::Effect(e) | Relay::Move(e)) => EffectRef::Dex(e),
        EventArg::Undefined | EventArg::Null => EffectRef::None,
        _ => panic!("effect argument"),
    }
}
pub fn effect_id<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> EffectId {
    b.event_effect_id(effect_ref(b, cx, i))
}
pub fn number_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> f64 {
    match b.event_arg(cx, i) {
        EventArg::Relay(Relay::Number(n)) | EventArg::Number(n) => n,
        _ => panic!("callback requires numeric argument"),
    }
}
fn move_ref<L: LogSink>(b: &Battle<L>, i: u8) -> &ActiveMove {
    b.scratch.moves[i as usize]
        .as_ref()
        .expect("released active move")
}
/// `type === 'sandstorm'`-style test of the Immunity event's relay (pokemon.ts:2269-2294).
pub fn immunity_is<L: LogSink>(b: &Battle<L>, cx: HookCtx, which: ImmunityId) -> bool {
    matches!(
        b.event_arg(cx, 0),
        EventArg::StatusImmunity(i) | EventArg::Relay(Relay::StatusImmunity(i)) if i == which
    )
}
/// `effect?.effectType === 'Ability'` (conditions.ts:496, moves.ts:4539 ...).
pub fn is_ability_effect<L: LogSink>(b: &Battle<L>, e: EffectRef) -> bool {
    e != EffectRef::None && b.event_effect_type(e) == EffectType::Ability
}
/// `(effect as Move)?.status` truthiness (abilities.ts:2295): an ActiveMove or dex move
/// with a primary `status`, or Synchronize's synthetic `{id, status}` object.
pub fn has_status_field<L: LogSink>(b: &Battle<L>, e: EffectRef) -> bool {
    match e {
        EffectRef::ActiveMove(i) => move_ref(b, i).effects.status != EffectId::NONE,
        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
            dex::move_data(id).effects.status != EffectId::NONE
        }
        EffectRef::Synchronize(_) => true,
        _ => false,
    }
}
/// `effect.secondaries` truthiness for a Move effect (moves.ts:4518).
pub fn has_secondaries<L: LogSink>(b: &Battle<L>, e: EffectRef) -> bool {
    match e {
        EffectRef::ActiveMove(i) => move_ref(b, i).secondaries_present,
        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
            !dex::move_data(id).secondaries.is_empty()
        }
        _ => false,
    }
}
pub fn move_type<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> TypeId {
    move_ref(b, move_arg(b, cx, i)).move_type
}

// ---------------------------------------------------------------------------------
// Field queries. All preserve the source call order (Type/TryTerrain events can run).
// ---------------------------------------------------------------------------------
/// `pokemon.isGrounded()` truthiness: false and null (Levitate) both fail `!isGrounded()`.
pub fn grounded<L: LogSink>(b: &mut Battle<L>, m: MonId) -> bool {
    b.is_grounded(m, false).truthy()
}
/// `this.field.isTerrain(id)` with the implicit `this.battle.event.target` (field.ts:169-184):
/// every caller here is an event handler whose event target is `m`.
pub fn terrain_is<L: LogSink>(b: &mut Battle<L>, terrain: EffectId, m: MonId) -> bool {
    b.is_terrain(&[terrain], Some(m))
}
/// `['sunnyday', 'desolateland'].includes(pokemon.effectiveWeather())`; Desolate Land is
/// outside the scoped dex, so only Sunny Day can match.
pub fn in_sun<L: LogSink>(b: &mut Battle<L>, m: MonId) -> bool {
    b.effective_weather(m) == dex::CONDITION_SUNNYDAY
}
/// `['raindance', 'primordialsea'].includes(pokemon.effectiveWeather())` (Primordial Sea outside scope).
pub fn in_rain<L: LogSink>(b: &mut Battle<L>, m: MonId) -> bool {
    b.effective_weather(m) == dex::CONDITION_RAINDANCE
}

// ---------------------------------------------------------------------------------
// Weather and terrain conditions.
// ---------------------------------------------------------------------------------
/// durationCallback(source, effect): conditions.ts:478,548,630,698; moves.ts:4510,7686,14108.
/// The five extender items are outside the scoped item list, so hasItem is false for scoped
/// teams, but the query is still performed (static key; no executable id is invented).
/// PRNG: none.
pub fn duration_callback<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, item: dex::KeyIds) -> Relay {
    if let Some(m) = opt_mon(b, cx, 0)
        && b.query_has_item(m, item)
    {
        return Relay::Number(8.0);
    }
    Relay::Number(5.0)
}
/// onFieldStart(field, source, effect) for weathers: conditions.ts:495-503,569-576,644-652,710-718.
/// The `gen <= 5` duration reset is unreachable. PRNG: none.
pub fn weather_field_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, name: &'static str) {
    let effect = effect_ref(b, cx, 2);
    if is_ability_effect(b, effect) {
        let source = opt_mon(b, cx, 1).expect("weather ability source is a Pokemon");
        b.add(LogEntry::new(
            "-weather",
            &[LogArg::Text(name)],
            &[LogTag::From(effect), LogTag::Of(source)],
        ));
    } else {
        b.add(LogEntry::new("-weather", &[LogArg::Text(name)], &[]));
    }
}
/// onFieldResidual(): conditions.ts:504-507,582-585,653-656,719-722. Sandstorm and Snowscape
/// guard the Weather event with `isWeather`; Rain and Sun do not (`gate` is None).
/// PRNG: none directly; eachEvent speedSort ties and Weather handlers (damage/heal) may draw.
pub fn weather_residual<L: LogSink>(b: &mut Battle<L>, name: &'static str, gate: Option<EffectId>) {
    b.add(LogEntry::new(
        "-weather",
        &[LogArg::Text(name)],
        &[LogTag::Bare("upkeep")],
    ));
    if let Some(weather) = gate
        && !b.is_weather(&[weather])
    {
        return;
    }
    b.each_event(EventId::Weather, EffectRef::None, Relay::Undefined);
}
/// onFieldEnd(): conditions.ts:508-510,586-588,660-662,723-725. PRNG: none.
pub fn weather_end<L: LogSink>(b: &mut Battle<L>) {
    b.add(LogEntry::new("-weather", &[LogArg::Text("none")], &[]));
}
/// onFieldStart for terrains: moves.ts:4538-4544,7704-7710,14137-14143. PRNG: none.
pub fn terrain_field_start<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, name: &'static str) {
    let effect = effect_ref(b, cx, 2);
    if is_ability_effect(b, effect) {
        let source = opt_mon(b, cx, 1).expect("terrain ability source is a Pokemon");
        b.add(LogEntry::new(
            "-fieldstart",
            &[LogArg::Text(name)],
            &[LogTag::From(effect), LogTag::Of(source)],
        ));
    } else {
        b.add(LogEntry::new("-fieldstart", &[LogArg::Text(name)], &[]));
    }
}
/// onFieldEnd for terrains: moves.ts:4547-4549,7722-7724,14146-14148. PRNG: none.
pub fn terrain_end<L: LogSink>(b: &mut Battle<L>, name: &'static str) {
    b.add(LogEntry::new("-fieldend", &[LogArg::Text(name)], &[]));
}
/// onWeatherModifyDamage(damage, attacker, defender, move): conditions.ts:484-494 (Rain) and
/// :554-568 (Sun). `up`/`down` are the boosted/suppressed move types. Hydro Steam (the Sun
/// attacker branch, :555) is outside the scoped dex and cannot match. PRNG: none.
pub fn weather_modify_damage<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    weather: EffectId,
    up: TypeId,
    down: TypeId,
) -> Relay {
    let defender = mon_arg(b, cx, 2);
    if b.effective_weather(defender) != weather {
        return Relay::Undefined;
    }
    let ty = move_type(b, cx, 3);
    if ty == up {
        b.chain_modify(1.5, 1.0);
    } else if ty == down {
        b.chain_modify(0.5, 1.0);
    }
    Relay::Undefined
}
/// Sandstorm onModifySpD / Snowscape onModifyDef: conditions.ts:639-643,705-709. Returns
/// `this.modify(stat, 1.5)` as a number, not a chained modifier. hasType (Type event) is
/// evaluated before effectiveWeather. PRNG: none directly.
pub fn type_weather_stat<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    ty: TypeId,
    weather: EffectId,
) -> Relay {
    let stat = number_arg(b, cx, 0);
    let m = mon_arg(b, cx, 1);
    if b.has_type(m, &[ty]) && b.effective_weather(m) == weather {
        return Relay::Number(b.modify(stat, 1.5, 1.0));
    }
    Relay::Undefined
}
/// Sandstorm onWeather(target): conditions.ts:657-659. `this.damage(baseMaxhp / 16)` with
/// source and effect defaulted from the event. PRNG: none directly (Damage handlers may draw).
pub fn sandstorm_weather<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let amount = b.state.pokemon[target.0 as usize].max_hp as f64 / 16.0;
    b.damage(amount, Some(target), Attribution::DEFAULT);
    Relay::Undefined
}

// ---------------------------------------------------------------------------------
// Weather/terrain-dependent healing moves.
// ---------------------------------------------------------------------------------
/// Shared tail of Moonlight/Morning Sun/Synthesis/Shore Up (moves.ts:12266-12271,
/// 12302-12307,16375-16380,18749-18754): `heal(modify(maxhp, factor))`, `-fail ... heal` and
/// NOT_FAIL on failure, `true` otherwise. PRNG: none directly (TryHeal/Heal handlers may draw).
pub fn heal_fraction_move<L: LogSink>(b: &mut Battle<L>, m: MonId, factor: f64) -> Relay {
    let max_hp = b.state.pokemon[m.0 as usize].max_hp as f64;
    let amount = b.modify(max_hp, factor, 1.0);
    heal_result(b, m, amount)
}
/// `const success = !!this.heal(amount); if (!success) {add('-fail', m, 'heal'); return NOT_FAIL} return success`.
pub fn heal_result<L: LogSink>(b: &mut Battle<L>, m: MonId, amount: f64) -> Relay {
    let success = b.heal(amount, Some(m), None, HealEffect::Context).truthy();
    if !success {
        b.add(LogEntry::new(
            "-fail",
            &[LogArg::Mon(m), LogArg::Text("heal")],
            &[],
        ));
        return Relay::NotFail;
    }
    Relay::Bool(true)
}
/// Factor table shared by Moonlight, Morning Sun and Synthesis: sun 0.667; rain, sandstorm,
/// snow 0.25 (hail/Desolate Land/Primordial Sea outside scope); otherwise 0.5.
pub fn sun_heal_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    let weather = b.effective_weather(m);
    let factor = if weather == dex::CONDITION_SUNNYDAY {
        0.667
    } else if weather == dex::CONDITION_RAINDANCE
        || weather == dex::CONDITION_SANDSTORM
        || weather == dex::CONDITION_SNOWSCAPE
    {
        0.25
    } else {
        0.5
    };
    heal_fraction_move(b, m, factor)
}

// ---------------------------------------------------------------------------------
// ModifyMove helpers.
// ---------------------------------------------------------------------------------
/// Bleakwind/Sandsear/Wildbolt Storm onModifyMove (moves.ts:1477,15687,20861):
/// `target && rain -> accuracy = true`. PRNG: none.
pub fn rain_always_hits<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 0);
    if let Some(target) = opt_mon(b, cx, 2)
        && in_rain(b, target)
    {
        b.active_move_mut(MoveHandle(mv)).accuracy = MoveAccuracy::Always;
    }
    Relay::Undefined
}
/// Hurricane/Thunder onModifyMove (moves.ts:9035,19447): rain -> true, sun -> 50, from the
/// single `target?.effectiveWeather()` switch. PRNG: none.
pub fn rain_sun_accuracy<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_arg(b, cx, 0);
    let weather = opt_mon(b, cx, 2).map(|target| b.effective_weather(target));
    if weather == Some(dex::CONDITION_RAINDANCE) {
        b.active_move_mut(MoveHandle(mv)).accuracy = MoveAccuracy::Always;
    } else if weather == Some(dex::CONDITION_SUNNYDAY) {
        b.active_move_mut(MoveHandle(mv)).accuracy = MoveAccuracy::Percent(50.0);
    }
    Relay::Undefined
}

// ---------------------------------------------------------------------------------
// Weather / terrain setting abilities.
// ---------------------------------------------------------------------------------
/// `source.species.id === species && source.item === item` (abilities.ts:1080,1090). The raw
/// item field is compared without the ignoringItem check, exactly like the source; the orbs
/// are outside the scoped item list, so the key comparison is never true for scoped teams.
pub fn primal_orb_holder<L: LogSink>(
    b: &Battle<L>,
    m: MonId,
    species: EffectId,
    orb: dex::KeyIds,
) -> bool {
    let p = &b.state.pokemon[m.0 as usize];
    p.species == species && orb.contains(p.item)
}
/// Drizzle/Drought onStart (abilities.ts:1079,1089). PRNG: none directly (setWeather events).
pub fn set_weather_on_start<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    weather: EffectId,
    blocked_by_orb: Option<(EffectId, dex::KeyIds)>,
) -> Relay {
    if let Some((species, orb)) = blocked_by_orb {
        let source = mon_arg(b, cx, 0);
        if primal_orb_holder(b, source, species, orb) {
            return Relay::Undefined;
        }
    }
    b.set_weather(weather, Attribution::DEFAULT);
    Relay::Undefined
}
/// Electric/Grassy/Psychic Surge onStart and Seed Sower onDamagingHit
/// (abilities.ts:1180,1709,3582,4121). PRNG: none directly (setTerrain events).
pub fn set_terrain_default<L: LogSink>(b: &mut Battle<L>, terrain: EffectId) -> Relay {
    b.set_terrain(terrain, Attribution::DEFAULT);
    Relay::Undefined
}

// ---------------------------------------------------------------------------------
// Air Lock / Cloud Nine.
// ---------------------------------------------------------------------------------
fn set_ability_ending<L: LogSink>(b: &mut Battle<L>, m: MonId, ending: bool) {
    let cell = b.state.pokemon[m.0 as usize].ability_state;
    let c = &mut b.state.effects.cells[cell.0 as usize];
    if ending {
        c.present |= present::ENDING;
    } else {
        c.present &= !present::ENDING;
    }
}
/// onStart(pokemon): abilities.ts:96-99,549-552. `abilityState.ending = false`, then
/// `eachEvent('WeatherChange', this.effect)`. PRNG: eachEvent speedSort ties / handlers.
pub fn suppress_weather_start<L: LogSink>(b: &mut Battle<L>, m: MonId) -> Relay {
    set_ability_ending(b, m, false);
    let effect = b.scratch.current_effect;
    b.each_event(EventId::WeatherChange, effect, Relay::Undefined);
    Relay::Undefined
}
/// onEnd(pokemon): abilities.ts:100-103,553-556. `abilityState.ending = true`, then WeatherChange.
pub fn suppress_weather_end<L: LogSink>(b: &mut Battle<L>, m: MonId) -> Relay {
    set_ability_ending(b, m, true);
    let effect = b.scratch.current_effect;
    b.each_event(EventId::WeatherChange, effect, Relay::Undefined);
    Relay::Undefined
}
/// onSwitchIn(pokemon): abilities.ts:91-95,544-548. Announces, then directly calls the same
/// ability's onStart with the same `this.effect`. PRNG: as onStart.
pub fn suppress_weather_switch_in<L: LogSink>(
    b: &mut Battle<L>,
    cx: HookCtx,
    name: &'static str,
) -> Relay {
    let m = mon_arg(b, cx, 0);
    b.add(LogEntry::new(
        "-ability",
        &[LogArg::Mon(m), LogArg::Text(name)],
        &[],
    ));
    suppress_weather_start(b, m)
}

// ---------------------------------------------------------------------------------
// Weather-damage abilities.
// ---------------------------------------------------------------------------------
/// Ice Body onWeather(target, source, effect): abilities.ts:1957-1961. Hail is outside scope.
/// `this.heal(baseMaxhp / 16)` defaults target/source/effect from the event. PRNG: none directly.
pub fn ice_body_weather<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    if effect_id(b, cx, 2) == dex::CONDITION_SNOWSCAPE {
        let amount = b.state.pokemon[target.0 as usize].max_hp as f64 / 16.0;
        b.heal(amount, Some(target), None, HealEffect::Context);
    }
    Relay::Undefined
}
/// Solar Power onWeather(target, source, effect): abilities.ts:4404-4409. Desolate Land is
/// outside scope. `this.damage(baseMaxhp / 8, target, target)`. PRNG: none directly.
pub fn solar_power_weather<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let weather = effect_id(b, cx, 2);
    if b.effective_weather(target) != weather {
        return Relay::Undefined;
    }
    if weather == dex::CONDITION_SUNNYDAY {
        let amount = b.state.pokemon[target.0 as usize].max_hp as f64 / 8.0;
        b.damage(
            amount,
            Some(target),
            Attribution::from_move(target, EffectRef::None),
        );
    }
    Relay::Undefined
}
/// Hydration onResidual(pokemon): abilities.ts:1929-1935. `pokemon.status` truthiness includes
/// the faint pseudo-status. PRNG: none directly (cureStatus events).
pub fn hydration_residual<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if b.state.pokemon[m.0 as usize].status != Status::None && in_rain(b, m) {
        b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(m), LogArg::Text("ability: Hydration")],
            &[],
        ));
        b.cure_status(m, false);
    }
    Relay::Undefined
}

// ---------------------------------------------------------------------------------
// Priority and target helpers for Psychic Terrain / Expanding Force.
// ---------------------------------------------------------------------------------
/// `(effect.priority, effect.target)` of a move-shaped effect.
pub fn move_priority_target<L: LogSink>(b: &Battle<L>, e: EffectRef) -> (f64, MoveTarget) {
    match e {
        EffectRef::ActiveMove(i) => {
            let m = move_ref(b, i);
            (m.priority as f64, m.target)
        }
        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
            let m = dex::move_data(id);
            (m.priority as f64, m.target)
        }
        _ => panic!("Psychic Terrain TryHit requires a move effect"),
    }
}
