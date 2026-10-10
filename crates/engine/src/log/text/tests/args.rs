//! Argument and tag stringification: `Array.prototype.join` semantics, effect names versus
//! fullnames, JS number printing.
use super::*;
use crate::log::format::{write_arg, write_effect};
use crate::state::scratch::{
    ActiveMove, HitData, MoveAccuracy, MoveEffectsScratch, OrderedBoosts, SyntheticEffect,
};

fn arg(b: &Battle<TextLog>, a: LogArg<'_>) -> String {
    let mut s = String::new();
    write_arg(&mut s, view(b), a, true);
    s
}

fn effect(b: &Battle<TextLog>, e: EffectRef, fullname: bool) -> String {
    let mut s = String::new();
    write_effect(&mut s, view(b), e, fullname);
    s
}

#[test]
fn js_number_to_string() {
    let b = standard_active();
    for (n, want) in [
        (0.0, "0"),
        (-0.0, "0"),
        (1.0, "1"),
        (-1.0, "-1"),
        (1.5, "1.5"),
        (0.1, "0.1"),
        (100.0, "100"),
        (123456789.0, "123456789"),
        (0.000001, "0.000001"),
        (0.0000001, "1e-7"),
        (1.5e-7, "1.5e-7"),
        (1e21, "1e+21"),
        (1.2345e25, "1.2345e+25"),
        (123456789012345680000.0, "123456789012345680000"),
        (4294967296.0, "4294967296"),
        (f64::NAN, "NaN"),
        (f64::INFINITY, "Infinity"),
        (f64::NEG_INFINITY, "-Infinity"),
        (0.30000000000000004, "0.30000000000000004"),
        (5e-324, "5e-324"),
    ] {
        assert_eq!(arg(&b, LogArg::Decimal(n)), want, "{n:e}");
    }
}

#[test]
fn primitive_arguments() {
    let b = standard_active();
    assert_eq!(arg(&b, LogArg::Text("a|b")), "a|b", "strings are never escaped");
    assert_eq!(arg(&b, LogArg::Empty), "");
    assert_eq!(arg(&b, LogArg::Bool(true)), "true");
    assert_eq!(arg(&b, LogArg::Bool(false)), "false");
    assert_eq!(arg(&b, LogArg::Number(-3)), "-3");
    assert_eq!(arg(&b, LogArg::Number(0)), "0");
    assert_eq!(arg(&b, LogArg::Mon(MonId::NONE)), "", "null Pokemon keeps an empty field");
    assert_eq!(arg(&b, LogArg::Side(SideId(0))), "p1: Alice");
    assert_eq!(arg(&b, LogArg::Side(SideId(1))), "p2: Bob");
    assert_eq!(arg(&b, LogArg::SideId(SideId(1))), "p2");
    assert_eq!(arg(&b, LogArg::PlayerName(SideId(0))), "Alice");
    assert_eq!(arg(&b, LogArg::Type(dex::type_id("Stellar").unwrap())), "Stellar");
    assert_eq!(arg(&b, LogArg::Type(dex::type_id("???").unwrap())), "???");
    assert_eq!(arg(&b, LogArg::Type(TypeId::NONE)), "");
    for (s, want) in [
        (Status::None, ""),
        (Status::Burn, "brn"),
        (Status::Paralysis, "par"),
        (Status::Sleep, "slp"),
        (Status::Freeze, "frz"),
        (Status::Poison, "psn"),
        (Status::Toxic, "tox"),
        (Status::Fainted, "fnt"),
    ] {
        assert_eq!(arg(&b, LogArg::Status(s)), want);
    }
    assert_eq!(arg(&b, LogArg::Species(id(EffectKind::Species, "Mimikyu-Busted"))), "Mimikyu-Busted");
    // Parts concatenate without separators (nested too).
    assert_eq!(
        arg(
            &b,
            LogArg::Parts(&[
                LogArg::Text("Event: "),
                LogArg::Parts(&[LogArg::Text("Fa"), LogArg::Text("int")]),
                LogArg::Number(7),
                LogArg::Empty,
            ])
        ),
        "Event: Faint7"
    );
    assert_eq!(arg(&b, LogArg::Spread { mons: [mon(1, 0), mon(0, 1), MonId::NONE, MonId::NONE], len: 2 }), "p2a,p1b");
    assert_eq!(arg(&b, LogArg::Spread { mons: [MonId::NONE; 4], len: 0 }), "");
}

#[test]
fn effect_name_versus_fullname() {
    let b = standard_active();
    for (kind, name, short, long) in [
        (EffectKind::Move, "Knock Off", "Knock Off", "move: Knock Off"),
        (EffectKind::Ability, "Intimidate", "Intimidate", "ability: Intimidate"),
        (EffectKind::Item, "Life Orb", "Life Orb", "item: Life Orb"),
        (EffectKind::Species, "Mimikyu-Busted", "Mimikyu-Busted", "pokemon: Mimikyu-Busted"),
        // Conditions (statuses, volatiles, weathers, move-owned) have bare names in both forms.
        (EffectKind::Condition, "brn", "brn", "brn"),
        (EffectKind::Condition, "confusion", "confusion", "confusion"),
        (EffectKind::Condition, "RainDance", "RainDance", "RainDance"),
        (EffectKind::Condition, "Stealth Rock", "Stealth Rock", "Stealth Rock"),
        (EffectKind::Condition, "Recoil", "Recoil", "Recoil"),
        (EffectKind::Condition, "Drain", "Drain", "Drain"),
        (EffectKind::Rule, "Sleep Clause Mod", "Sleep Clause Mod", "Sleep Clause Mod"),
    ] {
        let e = fx(kind, name);
        assert_eq!(effect(&b, e, false), short, "{name}");
        assert_eq!(effect(&b, e, true), long, "{name}");
        assert_eq!(arg(&b, LogArg::Effect(e)), short);
        assert_eq!(arg(&b, LogArg::EffectFullName(e)), long);
    }
    // Condition *views* of a move / ability / item / species record: the display name, no prefix.
    let mv = id(EffectKind::Move, "Reflect");
    let ab = id(EffectKind::Ability, "Unburden");
    let it = id(EffectKind::Item, "Leftovers");
    let sp = id(EffectKind::Species, "Mimikyu-Busted");
    for e in [
        EffectRef::MoveCondition(mv),
        EffectRef::AbilityCondition(ab),
        EffectRef::ItemCondition(it),
        EffectRef::SpeciesCondition(sp),
    ] {
        assert_eq!(effect(&b, e, false), effect(&b, e, true));
    }
    assert_eq!(effect(&b, EffectRef::MoveCondition(mv), true), "Reflect");
    assert_eq!(effect(&b, EffectRef::SpeciesCondition(sp), true), "Mimikyu-Busted");
    // Synthetic effect objects.
    assert_eq!(effect(&b, EffectRef::Synthetic(SyntheticEffect::Recoil), true), "Recoil");
    assert_eq!(effect(&b, EffectRef::None, true), "");
}

#[test]
fn active_move_effects_print_the_move() {
    let mut b = standard_active();
    let m = dex::move_data(dex::MOVE_UTURN);
    b.scratch.moves[2] = Some(ActiveMove {
        flags: m.flags,
        traits: m.traits,
        runtime_flags: 0,
        total_damage: 0,
        hit_targets: [MonId::NONE; 4],
        hit_target_len: 0,
        id: m.id,
        source_effect: EffectRef::None,
        type_changer_boosted: EffectId::NONE,
        ruined_stats: [MonId::NONE; 4],
        base_power: m.base_power as f64,
        accuracy: MoveAccuracy::Percent(100.0),
        hit_data: [HitData::default(); 12],
        self_boosts: OrderedBoosts::default(),
        effects: MoveEffectsScratch {
            base: &m.effects,
            boosts: OrderedBoosts::default(),
            status: m.effects.status,
            volatile_status: m.effects.volatile_status,
            side_condition: m.effects.side_condition,
            slot_condition: m.effects.slot_condition,
            weather: m.effects.weather,
            terrain: m.effects.terrain,
            pseudo_weather: m.effects.pseudo_weather,
            chance: None,
            heal: m.effects.heal,
            force_switch: m.effects.force_switch,
            self_switch: m.effects.self_switch,
            suppressed_hooks: 0,
        },
        self_effect: None,
        secondaries: [None; 4],
        secondary_count: 0,
        secondaries_present: false,
        recoil: m.recoil,
        drain: m.drain,
        damage: m.damage,
        self_destruct: m.self_destruct,
        offensive_stat: m.offensive_stat,
        defensive_stat: m.defensive_stat,
        offensive_target: m.offensive_target,
        ignore_immunity_types: 0,
        move_type: m.move_type,
        category: m.category,
        target: m.target,
        priority: m.priority,
        crit_ratio: m.crit_ratio,
        hit: 0,
        multihit: m.multihit,
    });
    let e = EffectRef::ActiveMove(2);
    assert_eq!(effect(&b, e, false), "U-turn");
    assert_eq!(effect(&b, e, true), "move: U-turn");
    // `[from] ${sourceEffect}` on a switch line is the Move name; `[from] ` + fullname elsewhere.
    let got = add_split(
        &mut b,
        "switch",
        &[LogArg::Mon(mon(1, 0)), LogArg::FullDetails(mon(1, 0))],
        &[LogTag::Value("from", LogArg::Effect(e))],
        1,
        false,
    );
    assert_eq!(got[1], "|switch|p2a: Klefki|Klefki, L78, F|217/217|[from] U-turn");
    let got = add(&mut b, "-activate", &[LogArg::Mon(mon(0, 0))], &[LogTag::From(e)]);
    assert_eq!(got, ["|-activate|p1a: Wugtrio|[from] move: U-turn"]);
}

#[test]
fn tags_keep_call_site_order_and_forms() {
    let mut b = standard_active();
    let w = mon(0, 0);
    let k = mon(1, 0);
    // [from] then [of], and the reversed order some call sites use (`-enditem ... [silent]|[from] ...`).
    let got = add(
        &mut b,
        "-item",
        &[LogArg::Mon(w), LogArg::Text("Leftovers")],
        &[LogTag::Of(k), LogTag::From(fx(EffectKind::Ability, "Frisk")), LogTag::Bare("silent")],
    );
    assert_eq!(got, ["|-item|p1a: Wugtrio|Leftovers|[of] p2a: Klefki|[from] ability: Frisk|[silent]"]);
    // Duplicated tags are kept as given.
    let got = add(
        &mut b,
        "-x",
        &[],
        &[LogTag::From(fx(EffectKind::Move, "Protect")), LogTag::From(fx(EffectKind::Move, "Protect"))],
    );
    assert_eq!(got, ["|-x|[from] move: Protect|[from] move: Protect"]);
    // Value tags: text, empty (trailing space), mon, number.
    let got = add(
        &mut b,
        "-x",
        &[],
        &[
            LogTag::Value("wisher", LogArg::Text("Vaporeon")),
            LogTag::Value("spread", LogArg::Empty),
            LogTag::Value("of", LogArg::Mon(w)),
            LogTag::Value("n", LogArg::Number(2)),
            LogTag::Text("[raw] text"),
        ],
    );
    assert_eq!(got, ["|-x|[wisher] Vaporeon|[spread] |[of] p1a: Wugtrio|[n] 2|[raw] text"]);
    // `[of] ${null}` is written by call sites as the text "null".
    let got = add(&mut b, "-x", &[], &[LogTag::Value("of", LogArg::Text("null"))]);
    assert_eq!(got, ["|-x|[of] null"]);
}
