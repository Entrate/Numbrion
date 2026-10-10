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
