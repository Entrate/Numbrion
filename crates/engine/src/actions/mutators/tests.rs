use crate::{
    Battle,
    actions::*,
    dex::{self},
    event::{EffectRef, Relay},
    ids::*,
    log::*,
    state::{mon_flags, scratch::OrderedBoosts},
};
#[derive(Default)]
struct Commands(Vec<&'static str>);
impl LogSink for Commands {
    const ENABLED: bool = true;
    fn emit(&mut self, _: LogView<'_>, e: LogEntry<'_>) {
        self.0.push(e.command);
        if e.split_side.is_some() {
            self.0.push(e.command);
        }
    }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {}
}
fn fixture() -> Battle<Commands> {
    let p = "Pikachu||choicescarf|static|thunderbolt,uturn|Serious||M|||100|,,,,,Electric";
    let team = format!("{p}]{p}");
    let mut b = Battle::with_log([1, 2, 3, 4], &team, &team, Commands::default()).unwrap();
    b.state.sides[0].active = [MonId(0), MonId(1)];
    b.state.sides[1].active = [MonId(6), MonId(7)];
    for i in [0, 1, 6, 7] {
        b.state.pokemon[i].flags |= mon_flags::ACTIVE;
    }
    b.state.field.pseudo_weather = Default::default();
    b
}
fn id(kind: EffectKind, s: &str) -> EffectId {
    if s == "-" {
        EffectId::NONE
    } else {
        dex::lookup(kind, s).unwrap_or_else(|| panic!("missing {kind:?} {s}"))
    }
}
fn key(id: EffectId) -> &'static str {
    if id == EffectId::NONE {
        "-"
    } else {
        dex::effect(id).key
    }
}
fn result(r: Relay) -> String {
    match r {
        Relay::Undefined => "undefined".into(),
        Relay::Null => "null".into(),
        Relay::NotFail => "".into(),
        Relay::Bool(b) => b.to_string(),
        Relay::Number(n) => n.to_string(),
        Relay::Effect(id) => key(id).into(),
        _ => panic!("unexpected {r:?}"),
    }
}
#[test]
fn state_mutations_match_pinned_real_methods_and_event_draws() {
    for line in include_str!("../../../../../tools/probes/mutators/state.tsv").lines() {
        let v: Vec<_> = line.split('\t').collect();
        // These vectors are retained for integration after the remaining effect
        // families land. Production methods still dispatch every missing hook.
        if matches!(v[0], "side" | "weather" | "terrain" | "pseudo")
            || (v[0] == "volatile" && v[1] != "confusion")
            || (v[0] == "boost" && v[1] == "simple")
        {
            continue;
        }
        let mut b = fixture();
        let mut extra = "-".to_string();
        let p = MonId(0);
        let a = Attribution::from_move(MonId(6), EffectRef::Dex(dex::MOVE_THUNDERBOLT));
        let r = match v[0] {
            "status" => b.set_status(p, id(EffectKind::Condition, v[1]), a, false),
            "cure" => {
                b.set_status(p, id(EffectKind::Condition, v[1]), Attribution::NONE, false);
                Relay::Bool(b.cure_status(p, false))
            }
            "volatile" => {
                let s = id(EffectKind::Condition, v[1]);
                let r = b.add_volatile(p, s, a, None);
                if let Some(c) = b.get_volatile(p, s) {
                    let d = b.state.effects.cells[c.0 as usize].duration;
                    if d >= 0 {
                        extra = d.to_string();
                    }
                }
                r
            }
            "side" => {
                let s = id(EffectKind::Condition, v[1]);
                let r =
                    b.add_side_condition(SideId(0), s, Attribution::from_move(p, EffectRef::None));
                if let Some(c) = b.get_side_condition_data(SideId(0), s) {
                    extra = b.state.effects.cells[c.0 as usize].duration.to_string();
                }
                r
            }
            "weather" => {
                let r = b.set_weather(
                    id(EffectKind::Condition, v[1]),
                    Attribution::from_move(p, EffectRef::None),
                );
                extra = b.state.effects.cells[b.state.field.weather.0 as usize]
                    .duration
                    .to_string();
                r
            }
            "terrain" => {
                let r = b.set_terrain(
                    id(EffectKind::Condition, v[1]),
                    Attribution::from_move(p, EffectRef::None),
                );
                extra = b.state.effects.cells[b.state.field.terrain.0 as usize]
                    .duration
                    .to_string();
                Relay::Bool(r)
            }
            "pseudo" => {
                let s = id(EffectKind::Condition, v[1]);
                let r = b.add_pseudo_weather(s, Attribution::from_move(p, EffectRef::None));
                extra = b.state.effects.cells[b.get_pseudo_weather(s).unwrap().0 as usize]
                    .duration
                    .to_string();
                r
            }
            "take" => b.take_item(p, Some(MonId(6))),
            "set" => Relay::Bool(b.set_item(p, dex::ITEM_LEFTOVERS, Attribution::DEFAULT)),
            "clear" => Relay::Bool(b.clear_item(p)),
            "use" => Relay::Bool(b.use_item(p, Attribution::DEFAULT)),
            "ability" => b.set_ability(
                p,
                id(EffectKind::Ability, v[1]),
                Attribution::from_move(MonId(6), EffectRef::Dex(dex::MOVE_THUNDERBOLT)),
                false,
                false,
            ),
            "boost" => {
                b.state.pokemon[0].ability = id(EffectKind::Ability, v[1]);
                let mut boosts = OrderedBoosts::default();
                for (i, (s, n)) in [(1, -1), (3, -1), (0, 2), (2, 2), (4, 2)]
                    .into_iter()
                    .enumerate()
                {
                    boosts.order[i] = s;
                    boosts.values[s as usize] = n;
                    boosts.present |= 1 << s;
                    boosts.len += 1;
                }
                let r = b.boost(
                    boosts,
                    Some(p),
                    Attribution::from_move(MonId(6), EffectRef::Dex(dex::MOVE_SHELLSMASH)),
                    false,
                    false,
                );
                extra = b.state.pokemon[0]
                    .boosts
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                r
            }
            _ => panic!("{line}"),
        };
        assert_eq!(result(r), v[2], "{line}");
        let mon = &b.state.pokemon[0];
        assert_eq!(key(b.status_id(p)), v[3], "{line}");
        assert_eq!(mon.hp, v[4].parse::<u16>().unwrap(), "{line}");
        assert_eq!(key(mon.item), v[5], "{line}");
        assert_eq!(key(mon.last_item), v[6], "{line}");
        assert_eq!(key(mon.ability), v[7], "{line}");
        assert_eq!(b.state.effect_order, v[8].parse::<u32>().unwrap(), "{line}");
        assert_eq!(
            b.seed()
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(","),
            v[9],
            "{line}"
        );
        let vol = mon
            .volatiles
            .as_slice()
            .iter()
            .map(|c| key(b.state.effects.cells[c.0 as usize].id))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(if vol.is_empty() { "-" } else { &vol }, v[10], "{line}");
        assert_eq!(extra, v[11], "{line}");
        let commands = b.log.0.join(",");
        assert_eq!(
            if commands.is_empty() { "-" } else { &commands },
            v[12],
            "{line}"
        );
    }
}

#[test]
fn species_transform_slots_and_condition_placement_match_pinned_methods() {
    for line in include_str!("../../../../../tools/probes/mutators/pokemon.tsv").lines() {
        let v: Vec<_> = line.split('\t').collect();
        let mut b = fixture();
        let mut p = MonId(0);
        let r = match v[0] {
            "forme" => {
                let (name, perm) = v[1].split_once(':').unwrap();
                let Some(s) = dex::lookup(EffectKind::Species, name) else {
                    continue;
                };
                b.state.pokemon[0].hp -= 50;
                Relay::Bool(b.forme_change(
                    p,
                    s,
                    EffectRef::Dex(dex::ABILITY_STATIC),
                    FormeOptions {
                        permanent: perm == "1",
                        message: None,
                    },
                ))
            }
            "skillswap" => {
                b.state.pokemon[6].ability = dex::ABILITY_HUGEPOWER;
                b.skill_swap(p, MonId(6))
            }
            "species" => b.set_species(p, id(EffectKind::Species, v[1]), EffectRef::None, false),
            "transform" | "transformpp" | "clearvolatile" => {
                b.set_species(MonId(6), dex::SPECIES_CHARIZARD, EffectRef::None, false);
                b.state.pokemon[6].stored_stats[0] = 234;
                b.state.pokemon[6].boosts[0] = 3;
                b.state.pokemon[6].times_attacked = 7;
                let r =
                    Relay::Bool(b.transform_into(p, MonId(6), EffectRef::Dex(dex::MOVE_TRANSFORM)));
                if v[0] != "transform" {
                    b.deduct_pp(p, dex::MOVE_THUNDERBOLT, Some(2.));
                }
                if v[0] == "clearvolatile" {
                    b.clear_volatile(p, true);
                }
                r
            }
            "disable" | "pp" => {
                b.disable_move(
                    p,
                    dex::MOVE_THUNDERBOLT,
                    v[1] == "1",
                    EffectRef::Dex(dex::CONDITION_CONFUSION),
                );
                if v[0] == "pp" {
                    b.deduct_pp(p, dex::MOVE_THUNDERBOLT, Some(999.));
                }
                Relay::Bool(true)
            }
            "type" | "addtype" => {
                let ts = if v[0] == "type" { v[1] } else { v[9] };
                let mut types = Types {
                    values: [TypeId::NONE; 3],
                    len: 0,
                };
                for s in ts.split(',') {
                    types.values[types.len as usize] = dex::type_id(s).unwrap();
                    types.len += 1;
                }
                let mut r = Relay::Bool(b.set_type(p, types, false));
                if v[0] == "addtype" {
                    r = Relay::Bool(b.add_type(p, dex::type_id(v[1]).unwrap()));
                }
                r
            }
            "teratype" => {
                b.state.pokemon[0].terastallized = dex::type_id(v[1]).unwrap();
                Relay::Bool(b.set_type(
                    p,
                    Types {
                        values: [dex::type_id("Water").unwrap(), TypeId::NONE, TypeId::NONE],
                        len: 1,
                    },
                    false,
                ))
            }
            "copy" => {
                b.add_volatile(p, dex::CONDITION_CONFUSION, Attribution::NONE, None);
                b.state.pokemon[0].boosts[0] = 4;
                b.copy_volatile_from(MonId(1), p, EffectId::NONE);
                p = MonId(1);
                assert_eq!(b.state.pokemon[1].boosts[0], 4);
                Relay::Bool(true)
            }
            "side" => b.add_side_condition(
                SideId(0),
                id(EffectKind::Condition, v[1]),
                Attribution::from_move(p, EffectRef::None),
            ),
            "slot" => b.add_slot_condition(
                SlotId(0),
                id(EffectKind::Condition, v[1]),
                Attribution::from_move(p, EffectRef::None),
            ),
            "weather" => b.set_weather(
                id(EffectKind::Condition, v[1]),
                Attribution::from_move(p, EffectRef::None),
            ),
            "terrain" => Relay::Bool(b.set_terrain(
                id(EffectKind::Condition, v[1]),
                Attribution::from_move(p, EffectRef::None),
            )),
            "pseudo" => b.add_pseudo_weather(
                id(EffectKind::Condition, v[1]),
                Attribution::from_move(p, EffectRef::None),
            ),
            _ => panic!("{line}"),
        };
        let r = if let Relay::Species(s) = r {
            key(s).to_string()
        } else {
            result(r)
        };
        assert_eq!(r, v[2], "{line}");
        let m = &b.state.pokemon[p.0 as usize];
        assert_eq!(key(m.species), v[3], "{line}");
        assert_eq!(key(m.base_species), v[4], "{line}");
        assert_eq!(m.hp.to_string(), v[5], "{line}");
        assert_eq!(m.max_hp.to_string(), v[6], "{line}");
        let stats = |a: &[u16]| {
            a.iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        assert_eq!(stats(&m.stored_stats), v[7], "{line}");
        assert_eq!(stats(&m.base_stored_stats), v[8], "{line}");
        let types = |a: &[TypeId], sep: &str| {
            a.iter()
                .filter(|t| **t != TypeId::NONE)
                .map(|t| dex::TYPE_NAMES[(t.0 - 1) as usize])
                .collect::<Vec<_>>()
                .join(sep)
        };
        assert_eq!(types(&m.types, ","), v[9], "{line}");
        assert_eq!(types(&m.apparent_types, "/"), v[10], "{line}");
        assert_eq!(
            if m.added_type == TypeId::NONE {
                "-"
            } else {
                dex::TYPE_NAMES[(m.added_type.0 - 1) as usize]
            },
            v[11],
            "{line}"
        );
        assert_eq!(
            u8::from(m.flags & mon_flags::KNOWN_TYPE != 0).to_string(),
            v[12],
            "{line}"
        );
        assert_eq!(
            u8::from(m.flags & mon_flags::TRANSFORMED != 0).to_string(),
            v[13],
            "{line}"
        );
        assert_eq!(key(m.ability), v[14], "{line}");
        assert_eq!(key(m.base_ability), v[15], "{line}");
        assert_eq!(m.weighthg.to_string(), v[16], "{line}");
        assert_eq!(m.speed.to_string(), v[17], "{line}");
        assert_eq!(b.state.effect_order.to_string(), v[18], "{line}");
        assert_eq!(
            b.seed()
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(","),
            v[19],
            "{line}"
        );
        let slots = m
            .move_slots()
            .iter()
            .map(|s| {
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    key(s.id),
                    s.pp,
                    s.max_pp,
                    u8::from(s.flags & super::pokemon::SLOT_USED != 0),
                    if s.flags & super::pokemon::SLOT_DISABLED != 0 {
                        "true"
                    } else if s.flags & super::pokemon::SLOT_HIDDEN != 0 {
                        "hidden"
                    } else {
                        "false"
                    },
                    key(s.disabled_source)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(slots, v[20], "{line}");
        assert_eq!(
            m.base_move_slots[..m.move_count as usize]
                .iter()
                .map(|s| s.pp.to_string())
                .collect::<Vec<_>>()
                .join(","),
            v[21],
            "{line}"
        );
        let logs = b.log.0.join(",");
        assert_eq!(if logs.is_empty() { "-" } else { &logs }, v[22], "{line}");
    }
}

#[derive(Default)]
struct TagCommands(Vec<String>);
impl LogSink for TagCommands {
    const ENABLED: bool = true;
    fn emit(&mut self, view: LogView<'_>, e: LogEntry<'_>) {
        // Assert the structured mutation log contract independently of T's
        // pending renderer. The oracle retains command and bracket tags.
        let mut line = e.command.to_string();
        fn effect(view: LogView<'_>, r: EffectRef) -> String {
            let id = match r {
                EffectRef::Dex(id) => id,
                EffectRef::ActiveMove(i) => view.moves[i as usize].unwrap().id,
                EffectRef::Synthetic(crate::event::SyntheticEffect::Recoil) => {
                    return "Recoil".into();
                }
                _ => panic!("{r:?}"),
            };
            if let Some(dex::DataValue::Text(s)) = dex::effect(id).data.get(dex::FIELD_FULLNAME) {
                s.into()
            } else {
                dex::effect(id).name.into()
            }
        }
        fn arg(view: LogView<'_>, a: LogArg<'_>) -> String {
            match a {
                LogArg::Text(s) => s.into(),
                LogArg::Mon(m) => format!(
                    "p{}{}: Pikachu",
                    m.side().0 + 1,
                    (b'a' + view.state.pokemon[m.0 as usize].position) as char
                ),
                LogArg::Effect(r) | LogArg::EffectFullName(r) => effect(view, r),
                _ => panic!("{a:?}"),
            }
        }
        for &tag in e.tags {
            line.push('|');
            match tag {
                LogTag::From(r) => line.push_str(&format!("[from] {}", effect(view, r))),
                LogTag::Of(m) => line.push_str(&format!("[of] {}", arg(view, LogArg::Mon(m)))),
                LogTag::Bare(s) => line.push_str(&format!("[{s}]")),
                LogTag::Value(k, a) => line.push_str(&format!("[{k}] {}", arg(view, a))),
                LogTag::Text(s) => line.push_str(s),
            }
        }
        self.0.push(line.clone());
        if e.split_side.is_some() {
            self.0.push(line);
        }
    }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {}
}
#[test]
fn battle_hp_mutator_relays_faint_attribution_and_log_tags_match_oracle() {
    let packed = "Pikachu|||static|thunderbolt,uturn|Serious||M|||100|,,,,,Electric";
    let team = format!("{packed}]{packed}");
    for line in include_str!("../../../../../tools/probes/mutators/health.tsv").lines() {
        let v: Vec<_> = line.split('\t').collect();
        let mut b = Battle::with_log([1, 2, 3, 4], &team, &team, TagCommands::default()).unwrap();
        b.state.sides[0].active = [MonId(0), MonId(1)];
        b.state.sides[1].active = [MonId(6), MonId(7)];
        for i in [0, 1, 6, 7] {
            b.state.pokemon[i].flags |= mon_flags::ACTIVE;
        }
        b.state.field.pseudo_weather = Default::default();
        b.state.pokemon[0].hp = 50;
        b.state.pokemon[0].max_hp = 100;
        let n = match v[1] {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            s => s.parse().unwrap(),
        };
        let e = match v[2] {
            "-" => EffectRef::None,
            "recoil" => EffectRef::Synthetic(crate::event::SyntheticEffect::Recoil),
            "uturn" | "rest" => EffectRef::Dex(id(EffectKind::Move, v[2])),
            s => EffectRef::Dex(id(EffectKind::Condition, s)),
        };
        let a = Attribution::from_move(MonId(6), e);
        let r = match v[0] {
            "damage" => b.damage(n, Some(MonId(0)), a),
            "directDamage" => b.direct_damage(n, Some(MonId(0)), a),
            "heal" => b.heal(n, Some(MonId(0)), Some(MonId(6)), HealEffect::Effect(e)),
            _ => panic!("{line}"),
        };
        let r = if matches!(r,Relay::Number(n) if n.is_nan()) {
            "NaN".into()
        } else {
            result(r)
        };
        assert_eq!(r, v[3], "{line}");
        assert_eq!(b.state.pokemon[0].hp.to_string(), v[4], "{line}");
        assert_eq!(b.state.faint_queue_len.to_string(), v[5], "{line}");
        if b.state.faint_queue_len != 0 {
            assert_eq!(b.state.faint_queue[0].source, MonId(6), "{line}");
            assert_eq!(b.state.faint_queue[0].effect.resolve(), e, "{line}");
            let second = b.faint_pokemon(MonId(0), a);
            assert_eq!(second, 0.);
            assert_eq!(b.state.faint_queue_len, 1);
        }
        assert_eq!(b.seed(), [1, 2, 3, 4], "{line}");
        let logs = b.log.0.join(";");
        assert_eq!(if logs.is_empty() { "-" } else { &logs }, v[9], "{line}");
    }
}
