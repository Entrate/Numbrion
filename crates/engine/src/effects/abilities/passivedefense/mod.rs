//! Private common bodies for the passive-defense batch. Each caller records its
//! pinned data/abilities.ts location; no direct draws occur in any body.
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{Attribution, HealEffect},
    dex::{self, Category, ImmunityId},
    effects::support::*,
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        Status, mon_flags,
        scratch::{MoveAccuracy, OrderedBoosts, move_runtime},
    },
};
fn effect<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> EffectRef {
    match b.event_arg(cx, i) {
        EventArg::Effect(e) => e,
        EventArg::Move(m) | EventArg::Relay(Relay::ActiveMove(m)) => EffectRef::ActiveMove(m),
        EventArg::Relay(Relay::Effect(e)) => EffectRef::Dex(e),
        EventArg::Undefined | EventArg::Null => EffectRef::None,
        _ => panic!("effect argument"),
    }
}
fn effect_id<L: LogSink>(b: &Battle<L>, cx: HookCtx, i: usize) -> EffectId {
    b.event_effect_id(effect(b, cx, i))
}
fn owner<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> MonId {
    let h = b.hook_state(cx).target;
    assert!(h.0 < 12);
    MonId(h.0)
}
fn log<L: LogSink>(b: &mut Battle<L>, command: &'static str, m: MonId, id: EffectId, full: bool) {
    let e = EffectRef::Dex(id);
    b.add(LogEntry::new(
        command,
        &[
            LogArg::Mon(m),
            if full {
                LogArg::EffectFullName(e)
            } else {
                LogArg::Effect(e)
            },
        ],
        &[],
    ));
}
fn immune<L: LogSink>(b: &mut Battle<L>, m: MonId, id: EffectId) {
    b.add(LogEntry::new(
        "-immune",
        &[LogArg::Mon(m)],
        &[LogTag::From(EffectRef::Dex(id))],
    ));
}
fn block<L: LogSink>(b: &mut Battle<L>, m: MonId, id: EffectId, holder: MonId) {
    b.add(LogEntry::new(
        "-block",
        &[LogArg::Mon(m), LogArg::EffectFullName(EffectRef::Dex(id))],
        &[LogTag::Of(holder)],
    ));
}
fn boost_slot<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> u8 {
    match b.event_arg(cx, 0) {
        EventArg::Relay(Relay::Boosts(i)) => i,
        _ => panic!("boost relay argument"),
    }
}
fn delete(boost: &mut OrderedBoosts, s: usize) {
    if boost.present & (1 << s) == 0 {
        return;
    }
    let i = boost.order[..boost.len as usize]
        .iter()
        .position(|x| *x as usize == s)
        .unwrap();
    boost.order.copy_within(i + 1..boost.len as usize, i);
    boost.len -= 1;
    boost.present &= !(1 << s);
    boost.values[s] = 0;
}
fn set(boost: &mut OrderedBoosts, s: usize, n: i8) {
    if boost.present & (1 << s) == 0 {
        boost.order[boost.len as usize] = s as u8;
        boost.len += 1;
        boost.present |= 1 << s;
    }
    boost.values[s] = n;
}
fn singleton(s: usize, n: i8) -> OrderedBoosts {
    let mut x = OrderedBoosts::default();
    set(&mut x, s, n);
    x
}
fn secondary<L: LogSink>(b: &Battle<L>, e: EffectRef) -> bool {
    match e {
        EffectRef::ActiveMove(i) => move_overlay(b, i).secondaries_present,
        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
            !dex::move_data(id).secondaries.is_empty()
        }
        _ => false,
    }
}
fn status_field<L: LogSink>(b: &Battle<L>, e: EffectRef) -> bool {
    match e {
        EffectRef::ActiveMove(i) => move_overlay(b, i).effects.status != EffectId::NONE,
        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
            dex::move_data(id).effects.status != EffectId::NONE
        }
        EffectRef::Synchronize(_) => true,
        _ => false,
    }
}
fn key(id: EffectId, name: &str) -> bool {
    id != EffectId::NONE && dex::effect(id).key == name
}
fn has_volatile<L: LogSink>(b: &Battle<L>, m: MonId, name: &str) -> bool {
    b.state.pokemon[m.0 as usize]
        .volatiles
        .as_slice()
        .iter()
        .any(|c| key(b.state.effects.cells[c.0 as usize].id, name))
}
fn hit_type<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> i8 {
    let m = mon_arg(b, cx, 2);
    let slot = m.side().0 as usize * 6 + b.state.pokemon[m.0 as usize].position as usize;
    let mov = move_arg(b, cx, 3);
    let data = &mut b.scratch.moves[mov as usize].as_mut().unwrap().hit_data[slot];
    data.present = true;
    data.type_mod
}
fn chain<L: LogSink>(b: &mut Battle<L>, n: u32, d: u32) -> Relay {
    b.chain_modify(n as f64, d as f64);
    Relay::Undefined
}

/// Ports data/abilities.ts:1285,1460,2028,2762,3476,3598,4146,4416. PRNG: none directly; nested core events retain draws.
pub fn source_modify_damage<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mov = move_arg(b, cx, 3);
    let mv = *move_overlay(b, mov);
    match id {
        dex::ABILITY_FILTER | dex::ABILITY_PRISMARMOR | dex::ABILITY_SOLIDROCK => {
            if hit_type(b, cx) > 0 {
                return chain(b, 3, 4);
            }
        }
        dex::ABILITY_FLUFFY => {
            return chain(
                b,
                if mv.move_type == dex::TYPE_FIRE {
                    2
                } else {
                    1
                },
                if mv.flags & dex::FLAG_CONTACT != 0 {
                    2
                } else {
                    1
                },
            );
        }
        dex::ABILITY_ICESCALES => {
            if mv.category == Category::Special {
                return chain(b, 1, 2);
            }
        }
        dex::ABILITY_MULTISCALE | dex::ABILITY_SHADOWSHIELD => {
            let m = mon_arg(b, cx, 2);
            let p = &b.state.pokemon[m.0 as usize];
            if p.hp >= p.max_hp {
                return chain(b, 1, 2);
            }
        }
        dex::ABILITY_PUNKROCK => {
            if mv.flags & dex::FLAG_SOUND != 0 {
                return chain(b, 1, 2);
            }
        }
        _ => panic!("unhandled damage modifier"),
    };
    Relay::Undefined
}
// The source relation is encoded by each wrapper, not a second event frame.
/// Ports data/abilities.ts:5037. PRNG: none directly; nested core events retain draws.
pub fn modify_damage<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if hit_type(b, cx) < 0 {
        return chain(b, 2, 1);
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:1581. PRNG: none directly; nested core events retain draws.
pub fn modify_def<L: LogSink>(_: EffectId, b: &mut Battle<L>, _: HookCtx) -> Relay {
    chain(b, 2, 1)
}
/// Ports data/abilities.ts:1912,4329,5392,1361. PRNG: none directly; nested core events retain draws.
pub fn modify_atk<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match id {
        dex::ABILITY_HUSTLE => Relay::Number(b.modify(relay_number(b, cx, 0), 3.0, 2.0)),
        dex::ABILITY_SLOWSTART => {
            if counter(b, cx) > 0 {
                chain(b, 1, 2)
            } else {
                Relay::Undefined
            }
        }
        dex::CONDITION_FLASHFIRE => flash_boost(b, cx),
        dex::ABILITY_WATERBUBBLE => water_boost(b, cx),
        _ => panic!("unhandled atk"),
    }
}
/// Ports data/abilities.ts:5397,1368. PRNG: none directly; nested core events retain draws.
pub fn modify_sp_a<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if id == dex::CONDITION_FLASHFIRE {
        flash_boost(b, cx)
    } else {
        water_boost(b, cx)
    }
}
fn water_boost<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if move_overlay(b, move_arg(b, cx, 3)).move_type == dex::TYPE_WATER {
        chain(b, 2, 1)
    } else {
        Relay::Undefined
    }
}
fn flash_boost<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if move_overlay(b, move_arg(b, cx, 3)).move_type == dex::TYPE_FIRE
        && b.has_ability(mon_arg(b, cx, 1), &[dex::ABILITY_FLASHFIRE])
    {
        chain(b, 3, 2)
    } else {
        Relay::Undefined
    }
}
/// Ports data/abilities.ts:3633,5018,5381. PRNG: none directly; nested core events retain draws.
pub fn source_modify_atk<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let t = move_overlay(b, move_arg(b, cx, 3)).move_type;
    let yes = match id {
        dex::ABILITY_PURIFYINGSALT => t == dex::TYPE_GHOST,
        dex::ABILITY_THICKFAT => {
            t == dex::TYPE_FIRE || t == dex::TYPE_ICE
        }
        dex::ABILITY_WATERBUBBLE => t == dex::TYPE_FIRE,
        _ => panic!("unhandled source atk"),
    };
    if yes {
        chain(b, 1, 2)
    } else {
        Relay::Undefined
    }
}
/// Ports data/abilities.ts:3640,5025,5387. PRNG: none directly; nested core events retain draws.
pub fn source_modify_sp_a<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    source_modify_atk(id, b, cx)
}
/// Ports data/abilities.ts:668,1916. PRNG: none directly; nested core events retain draws.
pub fn source_modify_accuracy<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if !matches!(
        b.event_arg(cx, 0),
        EventArg::Number(_) | EventArg::Relay(Relay::Number(_))
    ) {
        return Relay::Undefined;
    }
    if id == dex::ABILITY_COMPOUNDEYES {
        chain(b, 5325, 4096)
    } else if move_overlay(b, move_arg(b, cx, 3)).category == Category::Physical {
        chain(b, 3277, 4096)
    } else {
        Relay::Undefined
    }
}
/// Ports data/abilities.ts:1611,3310,3592. PRNG: none directly; nested core events retain draws.
pub fn base_power<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let mv = move_overlay(b, move_arg(b, cx, 3));
    if id == dex::ABILITY_PUNKROCK {
        if mv.flags & dex::FLAG_SOUND != 0 {
            return chain(b, 5325, 4096);
        }
    } else if mv.type_changer_boosted == id {
        return chain(b, 4915, 4096);
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:1600,2419,3299. PRNG: none directly; nested core events retain draws.
pub fn modify_type<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let i = move_arg(b, cx, 0);
    let m = mon_arg(b, cx, 1);
    let mv = b.scratch.moves[i as usize].as_mut().unwrap();
    if id == dex::ABILITY_LIQUIDVOICE {
        // Dynamax is unavailable in the fixed Gen 9 format.
        if mv.flags & dex::FLAG_SOUND != 0 {
            mv.move_type = dex::TYPE_WATER
        }
    } else if mv.move_type == dex::TYPE_NORMAL
        && !matches!(
            dex::effect(mv.id).key,
            "judgment"
                | "multiattack"
                | "naturalgift"
                | "revelationdance"
                | "technoblast"
                | "terrainpulse"
                | "weatherball"
        )
        && !(mv.id == dex::MOVE_TERABLAST
            && b.state.pokemon[m.0 as usize].terastallized != TypeId(0))
    {
        // Z/Max moves cannot exist in this format.
        mv.move_type = dex::type_id(if id == dex::ABILITY_GALVANIZE {
            "Electric"
        } else {
            "Fairy"
        })
        .unwrap();
        mv.type_changer_boosted = id;
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:2271,2631,2694,4082,4984,5208. PRNG: none directly; nested core events retain draws.
pub fn modify_move<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let i = move_arg(b, cx, 0);
    let mv = b.scratch.moves[i as usize].as_mut().unwrap();
    match id {
        dex::ABILITY_MOLDBREAKER | dex::ABILITY_TERAVOLT | dex::ABILITY_TURBOBLAZE => {
            mv.runtime_flags |= move_runtime::IGNORE_ABILITY
        }
        dex::ABILITY_KEENEYE => mv.runtime_flags |= move_runtime::IGNORE_EVASION,
        dex::ABILITY_MINDSEYE | dex::ABILITY_SCRAPPY => {
            if id == dex::ABILITY_MINDSEYE {
                mv.runtime_flags |= move_runtime::IGNORE_EVASION
            }
            if mv.runtime_flags & move_runtime::IGNORE_IMMUNITY == 0 {
                mv.runtime_flags |= move_runtime::IGNORE_IMMUNITY_PRESENT;
                mv.ignore_immunity_types |= (1 << dex::TYPE_NORMAL.0)
                    | (1 << dex::TYPE_FIGHTING.0)
            }
        }
        _ => panic!("unhandled modify move"),
    };
    Relay::Undefined
}
/// Ports data/abilities.ts:524,1421,1560,2158,2262,2621,2659,3031,3160,4089. PRNG: none directly; nested core events retain draws.
pub fn try_boost<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let s = boost_slot(b, cx);
    let t = mon_arg(b, cx, 1);
    let source = Battle::<L>::arg_mon(b.event_arg(cx, 2));
    let e = effect(b, cx, 3);
    if id == dex::ABILITY_MIRRORARMOR {
        if source.is_none() || source == Some(t) || b.event_effect_id(e) == dex::ABILITY_MIRRORARMOR
        {
            return Relay::Undefined;
        }
        let a = source.unwrap();
        host::mirror(&mut host::MirrorBattleHost {
            b,
            boosts: s,
            target: t,
            source: a,
        });
        return Relay::Undefined;
    }
    if matches!(
        id,
        dex::ABILITY_INNERFOCUS
            | dex::ABILITY_OBLIVIOUS
            | dex::ABILITY_OWNTEMPO
            | dex::ABILITY_SCRAPPY
    ) {
        if b.event_effect_id(e) == dex::ABILITY_INTIMIDATE
            && b.scratch_boosts(s).values[0] != 0
            && b.scratch_boosts(s).present & 1 != 0
        {
            delete(b.scratch_boosts(s), 0);
            fail_boost(b, t, id, Some("atk"));
        }
        return Relay::Undefined;
    }
    if source == Some(t) {
        return Relay::Undefined;
    }
    if id == dex::ABILITY_FLOWERVEIL && !b.has_type(t, &[dex::TYPE_GRASS]) {
        return Relay::Undefined;
    }
    let mut changed = false;
    let acc = matches!(id, dex::ABILITY_KEENEYE | dex::ABILITY_MINDSEYE);
    let order = *b.scratch_boosts(s);
    for stat in order.order[..order.len as usize].iter().copied() {
        let stat = stat as usize;
        if (!acc || stat == 5) && b.scratch_boosts(s).values[stat] < 0 {
            delete(b.scratch_boosts(s), stat);
            changed = true
        }
    }
    if changed && !secondary(b, e) {
        if id == dex::ABILITY_FLOWERVEIL {
            block(b, t, id, owner(b, cx));
        } else if acc || !key(b.event_effect_id(e), "octolock") {
            fail_boost(b, t, id, if acc { Some("accuracy") } else { None });
        }
    }
    Relay::Undefined
}
fn fail_boost<L: LogSink>(b: &mut Battle<L>, t: MonId, id: EffectId, stat: Option<&'static str>) {
    let args = [
        LogArg::Mon(t),
        LogArg::Text("unboost"),
        LogArg::Text(stat.unwrap_or("")),
    ];
    b.add(LogEntry::new(
        "-fail",
        &args[..if stat.is_some() { 3 } else { 2 }],
        &[LogTag::From(EffectRef::Dex(id)), LogTag::Of(t)],
    ));
}
/// Ports data/abilities.ts:679. PRNG: none directly; nested core events retain draws.
pub fn change_boost<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if key(effect_id(b, cx, 3), "zpower") {
        return Relay::Undefined;
    }
    let s = boost_slot(b, cx);
    let boost = b.scratch_boosts(s);
    for stat in boost.order[..boost.len as usize].iter().copied() {
        boost.values[stat as usize] *= -1;
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:5217. PRNG: none directly; nested core events retain draws.
pub fn modify_boost<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let t = mon_arg(b, cx, 1);
    let o = owner(b, cx);
    if o == t {
        return Relay::Undefined;
    }
    let s = boost_slot(b, cx);
    if o == b.scratch.active_pokemon && t == b.scratch.active_target {
        for stat in [1, 3, 6] {
            set(b.scratch_boosts(s), stat, 0)
        }
    }
    if t == b.scratch.active_pokemon && o == b.scratch.active_target {
        for stat in [0, 1, 2, 5] {
            set(b.scratch_boosts(s), stat, 0)
        }
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:4231. PRNG: none directly; nested core events retain draws.
pub fn modify_secondaries<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let s = match b.event_arg(cx, 0) {
        EventArg::Relay(Relay::Secondaries(s)) => s,
        _ => panic!("secondaries relay"),
    };
    let mut kept = [None; 4];
    let mut n = 0;
    for sec in b.scratch_secondaries(s).iter().flatten() {
        if sec.self_effect.is_some() {
            kept[n] = Some(*sec);
            n += 1
        }
    }
    Relay::Secondaries(b.stash_secondaries(kept))
}

/// Ports data/abilities.ts:235,1446,2155,2183,3152,3626,5331. PRNG: none directly; nested core events retain draws.
pub fn try_add_volatile<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let status = effect_id(b, cx, 0);
    let t = mon_arg(b, cx, 1);
    match id {
        dex::ABILITY_AROMAVEIL => {
            if matches!(
                dex::effect(status).key,
                "attract" | "disable" | "encore" | "healblock" | "taunt" | "torment"
            ) {
                if b.event_effect_type(effect(b, cx, 3)) == dex::EffectType::Move {
                    block(b, t, id, owner(b, cx))
                }
                return Relay::Null;
            }
        }
        dex::ABILITY_FLOWERVEIL => {
            if b.has_type(t, &[dex::TYPE_GRASS]) && status == dex::CONDITION_YAWN {
                block(b, t, id, owner(b, cx));
                return Relay::Null;
            }
        }
        dex::ABILITY_INNERFOCUS => {
            if status == dex::CONDITION_FLINCH {
                return Relay::Null;
            }
        }
        dex::ABILITY_OWNTEMPO => {
            if status == dex::CONDITION_CONFUSION {
                return Relay::Null;
            }
        }
        dex::ABILITY_INSOMNIA | dex::ABILITY_VITALSPIRIT | dex::ABILITY_PURIFYINGSALT => {
            if status == dex::CONDITION_YAWN {
                immune(b, t, id);
                return Relay::Null;
            }
        }
        _ => panic!("unhandled volatile guard"),
    };
    Relay::Undefined
}
/// Ports data/abilities.ts:592,1436,2176,3620,5324,5408,5438. PRNG: none directly; nested core events retain draws.
pub fn set_status<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let status = effect_id(b, cx, 0);
    let t = mon_arg(b, cx, 1);
    let e = effect(b, cx, 3);
    if id == dex::ABILITY_FLOWERVEIL {
        let source = Battle::<L>::arg_mon(b.event_arg(cx, 2));
        if b.has_type(t, &[dex::TYPE_GRASS])
            && source.is_some()
            && source != Some(t)
            && e != EffectRef::None
            && b.event_effect_id(e) != dex::CONDITION_YAWN
        {
            if matches!(e, EffectRef::Synchronize(_))
                || (b.event_effect_type(e) == dex::EffectType::Move && !secondary(b, e))
            {
                block(b, t, id, owner(b, cx))
            }
            return Relay::Null;
        }
        return Relay::Undefined;
    }
    if matches!(id, dex::ABILITY_INSOMNIA | dex::ABILITY_VITALSPIRIT)
        && status != dex::CONDITION_SLP
    {
        return Relay::Undefined;
    }
    if matches!(id, dex::ABILITY_WATERVEIL | dex::ABILITY_WATERBUBBLE)
        && status != dex::CONDITION_BRN
    {
        return Relay::Undefined;
    }
    if status_field(b, e) {
        immune(b, t, id)
    }
    Relay::Bool(false)
}
/// Ports data/abilities.ts:2170,3010,3146,5318,5402,5432. PRNG: none directly; nested core events retain draws.
pub fn update<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    host::update(&mut host::UpdateBattleHost { b, target: m, id }, id);
    Relay::Undefined
}
/// Ports data/abilities.ts:3155. PRNG: none directly; nested core events retain draws.
pub fn hit<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if let EventArg::Move(i) | EventArg::Effect(EffectRef::ActiveMove(i)) = b.event_arg(cx, 2) {
        if move_overlay(b, i).effects.volatile_status == dex::CONDITION_CONFUSION {
            let m = mon_arg(b, cx, 0);
            b.add(LogEntry::new(
                "-immune",
                &[LogArg::Mon(m), LogArg::Text("confusion")],
                &[LogTag::From(EffectRef::Dex(id))],
            ));
        }
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:3022,3110. PRNG: none directly; nested core events retain draws.
pub fn immunity<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let arg = b.event_arg(cx, 0);
    let yes = if id == dex::ABILITY_OBLIVIOUS {
        matches!(arg, EventArg::StaticText("attract"))
    } else {
        matches!(
            arg,
            EventArg::StatusImmunity(ImmunityId::Sandstorm | ImmunityId::Hail | ImmunityId::Powder)
                | EventArg::Relay(Relay::StatusImmunity(
                    ImmunityId::Sandstorm | ImmunityId::Hail | ImmunityId::Powder
                ))
        )
    };
    if yes {
        Relay::Bool(false)
    } else {
        Relay::Undefined
    }
}
/// Ports data/abilities.ts:481,1134,1343,1632,2737,3025,3114,4026,4438,4675,4934,5343,5366. PRNG: none directly; nested core events retain draws.
pub fn try_hit<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let t = mon_arg(b, cx, 0);
    let s = mon_arg(b, cx, 1);
    let i = move_arg(b, cx, 2);
    let mv = *move_overlay(b, i);
    let self_hit = t == s;
    let absorb_type = match id {
        dex::ABILITY_EARTHEATER => Some("Ground"),
        dex::ABILITY_VOLTABSORB | dex::ABILITY_MOTORDRIVE => Some("Electric"),
        dex::ABILITY_WATERABSORB => Some("Water"),
        dex::ABILITY_SAPSIPPER => Some("Grass"),
        dex::ABILITY_FLASHFIRE => Some("Fire"),
        _ => None,
    };
    if let Some(typ) = absorb_type {
        if !self_hit && mv.move_type == dex::type_id(typ).unwrap() {
            return absorb(&mut BattleHost { b, move_index: i }, id, t);
        }
        return Relay::Undefined;
    }
    let guarded = match id {
        dex::ABILITY_BULLETPROOF => mv.flags & dex::FLAG_BULLET != 0,
        dex::ABILITY_GOODASGOLD => mv.category == Category::Status && !self_hit,
        dex::ABILITY_SOUNDPROOF => !self_hit && mv.flags & dex::FLAG_SOUND != 0,
        // No scoped move has ohko; verified by the pinned numeric probe.
        dex::ABILITY_STURDY => false,
        dex::ABILITY_OBLIVIOUS => {
            matches!(dex::effect(mv.id).key, "attract" | "captivate" | "taunt")
        }
        dex::ABILITY_TELEPATHY => {
            !self_hit && t.side() == s.side() && mv.category != Category::Status
        }
        dex::ABILITY_OVERCOAT => {
            !self_hit && mv.flags & dex::FLAG_POWDER != 0 && dex_powder_immunity(b, t)
        }
        _ => panic!("unhandled hit immunity"),
    };
    if guarded {
        if id == dex::ABILITY_TELEPATHY {
            log(b, "-activate", t, id, true)
        } else {
            immune(b, t, id)
        }
        return Relay::Null;
    }
    Relay::Undefined
}
// Dex.getImmunity('powder', target) reads target.getTypes(), not runStatusImmunity.
fn dex_powder_immunity<L: LogSink>(b: &mut Battle<L>, m: MonId) -> bool {
    let types = b.get_types(m, false, false);
    types.values[..types.len as usize]
        .iter()
        .all(|t| dex::IMMUNITY_CHART[(t.0 - 1) as usize][ImmunityId::Powder as usize] != 3)
}
/// Ports data/abilities.ts:4034,4444. PRNG: none directly; nested core events retain draws.
pub fn try_hit_side<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let t = mon_arg(b, cx, 0);
    let s = mon_arg(b, cx, 1);
    let i = move_arg(b, cx, 2);
    if id == dex::ABILITY_SOUNDPROOF {
        if move_overlay(b, i).flags & dex::FLAG_SOUND != 0 {
            log_immune_owner(b, cx, id)
        }
    } else {
        let o = owner(b, cx);
        if s == o || t.side() != s.side() {
            return Relay::Undefined;
        }
        if move_overlay(b, i).move_type == dex::TYPE_GRASS {
            b.boost(singleton(0, 1), Some(o), Attribution::DEFAULT, false, false);
        }
    }
    Relay::Undefined
}
fn log_immune_owner<L: LogSink>(b: &mut Battle<L>, cx: HookCtx, id: EffectId) {
    immune(b, owner(b, cx), id)
}
/// Ports data/abilities.ts:2467,3908,4682. PRNG: none directly; nested core events retain draws.
pub fn damage<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let e = effect(b, cx, 3);
    match id {
        dex::ABILITY_MAGICGUARD => {
            if b.event_effect_type(e) != dex::EffectType::Move {
                if b.event_effect_type(e) == dex::EffectType::Ability {
                    log(
                        b,
                        "-activate",
                        mon_arg(b, cx, 2),
                        b.event_effect_id(e),
                        true,
                    )
                }
                return Relay::Bool(false);
            }
        }
        dex::ABILITY_ROCKHEAD => {
            if e == EffectRef::Synthetic(crate::state::scratch::SyntheticEffect::Recoil) {
                let m = b.scratch.active_move;
                assert!(m.0 != 255, "Battle.activeMove is null");
                if move_overlay(b, m.0).id != dex::MOVE_STRUGGLE {
                    return Relay::Null;
                }
            }
        }
        dex::ABILITY_STURDY => {
            let t = mon_arg(b, cx, 1);
            let hp = b.state.pokemon[t.0 as usize].hp;
            if hp == b.state.pokemon[t.0 as usize].max_hp
                && relay_number(b, cx, 0) >= hp as f64
                && e != EffectRef::None
                && b.event_effect_type(e) == dex::EffectType::Move
            {
                log(b, "-ability", t, id, false);
                return Relay::Number(hp as f64 - 1.0);
            }
        }
        _ => panic!("unhandled damage guard"),
    };
    Relay::Undefined
}
fn truthy(arg: EventArg) -> bool {
    match arg {
        EventArg::Undefined | EventArg::Null | EventArg::Bool(false) => false,
        EventArg::Number(n) => n != 0.0 && !n.is_nan(),
        EventArg::Relay(r) => r.truthy(),
        _ => true,
    }
}
/// Ports data/abilities.ts:2973. PRNG: none directly; nested core events retain draws.
pub fn invulnerability<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let o = owner(b, cx);
    if truthy(b.event_arg(cx, 2))
        && (Battle::<L>::arg_mon(b.event_arg(cx, 0)) == Some(o)
            || Battle::<L>::arg_mon(b.event_arg(cx, 1)) == Some(o))
    {
        Relay::Number(0.0)
    } else {
        Relay::Undefined
    }
}
/// Ports data/abilities.ts:2976. PRNG: none directly; nested core events retain draws.
pub fn accuracy<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let o = owner(b, cx);
    if truthy(b.event_arg(cx, 3))
        && (Battle::<L>::arg_mon(b.event_arg(cx, 1)) == Some(o)
            || Battle::<L>::arg_mon(b.event_arg(cx, 2)) == Some(o))
    {
        Relay::Bool(true)
    } else {
        match b.event_arg(cx, 0) {
            EventArg::Relay(r) => r,
            EventArg::Number(n) => Relay::Number(n),
            EventArg::Bool(x) => Relay::Bool(x),
            EventArg::Null => Relay::Null,
            EventArg::Undefined => Relay::Undefined,
            _ => panic!("accuracy relay"),
        }
    }
}
/// Ports data/abilities.ts:3442. PRNG: none directly; nested core events retain draws.
pub fn deduct_pp<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if mon_arg(b, cx, 0).side() == mon_arg(b, cx, 1).side() {
        Relay::Undefined
    } else {
        Relay::Number(1.0)
    }
}
// Slow Start payload: word0 counter, custom present bit8; deletion clears presence.
fn counter<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> u32 {
    let s = b.hook_state(cx);
    if s.present & (1 << 8) != 0 {
        s.payload.words[0]
    } else {
        0
    }
}
/// Ports data/abilities.ts:589,2691,3439,4313,4981,5205,1357. PRNG: none directly; nested core events retain draws.
pub fn start<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    if id == dex::ABILITY_SLOWSTART {
        log(b, "-start", m, id, true);
        let s = b.hook_state_mut(cx);
        s.payload.words[0] = 5;
        s.present |= 1 << 8;
    } else if id == dex::CONDITION_FLASHFIRE {
        log(b, "-start", m, dex::ABILITY_FLASHFIRE, true);
    } else {
        log(b, "-ability", m, id, false)
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:4319. PRNG: none directly; nested core events retain draws.
pub fn residual<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    let n = counter(b, cx);
    if b.state.pokemon[m.0 as usize].active_turns != 0 && n != 0 {
        b.hook_state_mut(cx).payload.words[0] = n - 1;
        if n == 1 {
            log(b, "-end", m, dex::ABILITY_SLOWSTART, false);
            b.hook_state_mut(cx).present &= !(1 << 8);
        }
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:4334,5249. PRNG: none directly; nested core events retain draws.
pub fn modify_spe<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    if id == dex::ABILITY_SLOWSTART {
        if counter(b, cx) > 0 {
            return chain(b, 1, 2);
        }
    } else {
        let m = mon_arg(b, cx, 1);
        if b.state.pokemon[m.0 as usize].item == EffectId::NONE && !b.ignoring_ability(m) {
            return chain(b, 2, 1);
        }
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:1352,4339,5245,1374. PRNG: none directly; nested core events retain draws.
pub fn end<L: LogSink>(id: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 0);
    match id {
        dex::ABILITY_FLASHFIRE => {
            b.remove_volatile(m, dex::CONDITION_FLASHFIRE);
        }
        dex::ABILITY_UNBURDEN => {
            b.remove_volatile(m, dex::CONDITION_UNBURDEN);
        }
        dex::CONDITION_FLASHFIRE => {
            b.add(LogEntry::new(
                "-end",
                &[
                    LogArg::Mon(m),
                    LogArg::EffectFullName(EffectRef::Dex(dex::ABILITY_FLASHFIRE)),
                ],
                &[LogTag::Bare("silent")],
            ));
        }
        dex::ABILITY_SLOWSTART => {
            if b.state.pokemon[m.0 as usize].flags & mon_flags::BEING_CALLED_BACK == 0
                && counter(b, cx) != 0
            {
                b.add(LogEntry::new(
                    "-end",
                    &[LogArg::Mon(m), LogArg::Effect(EffectRef::Dex(id))],
                    &[LogTag::Bare("silent")],
                ));
            }
        }
        _ => panic!("unhandled end"),
    };
    Relay::Undefined
}
/// Ports data/abilities.ts:5238. PRNG: none directly; nested core events retain draws.
pub fn after_use_item<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 1);
    if m == owner(b, cx) {
        b.add_volatile(m, dex::CONDITION_UNBURDEN, Attribution::DEFAULT, None);
    }
    Relay::Undefined
}
/// Ports data/abilities.ts:5242. PRNG: none directly; nested core events retain draws.
pub fn take_item<L: LogSink>(_: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let m = mon_arg(b, cx, 1);
    b.add_volatile(m, dex::CONDITION_UNBURDEN, Attribution::DEFAULT, None);
    Relay::Undefined
}
mod host;
use host::{BattleHost, absorb};
#[cfg(test)]
mod tests;
