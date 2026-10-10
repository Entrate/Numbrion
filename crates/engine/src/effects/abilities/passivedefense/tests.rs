#[path = "testsupport.rs"]
mod support;
use crate::{
    dex,
    event::*,
    ids::*,
    state::{CellRef, mon_flags},
};
use support::*;
fn own(b: &mut crate::Battle, id: EffectId) {
    let holder = Holder::mon(MonId(0));
    let cell = b.state.effects.alloc(holder, holder, id, 0);
    b.state.pokemon[0].ability_state = cell;
    b.scratch.current_state = Some(CellRef {
        cell,
        generation: b.state.effects.cells[cell.0 as usize].generation,
    });
}
#[test]
fn numerical_callbacks_match_pinned_oracle() {
    let mut count = 0;
    for line in include_str!("numeric.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line
            .split('\t')
            .map(|c| if c == "<empty>" { "" } else { c })
            .collect();
        let v: u32 = c[4].parse().unwrap();
        let family = if c[0] == "abilities" {
            EffectKind::Ability
        } else {
            EffectKind::Condition
        };
        let id = dex::lookup(family, c[1]).unwrap();
        let mut b = battle();
        own(&mut b, id);
        let cell = b.scratch.current_state.unwrap().cell;
        b.state.effects.cells[cell.0 as usize].present |= 1 << 8;
        b.state.effects.cells[cell.0 as usize].payload.words[0] = if v % 2 != 0 { 5 } else { 0 };
        for m in [0, 6] {
            b.state.pokemon[m].hp = if v % 3 != 0 { 120 } else { 119 };
            b.state.pokemon[m].max_hp = 120;
            b.state.pokemon[m].item = if v % 2 != 0 {
                dex::ITEM_LIGHTBALL
            } else {
                EffectId::NONE
            };
            b.state.pokemon[m].ability = if v % 3 != 2 {
                dex::ABILITY_FLASHFIRE
            } else {
                dex::ABILITY_STATIC
            };
            b.state.pokemon[m].flags |= mon_flags::ACTIVE;
        }
        b.scratch.frames[0].as_mut().unwrap().modifier = if v % 3 != 0 { 6144 } else { 4096 };
        let mv = b.scratch.moves[0].as_mut().unwrap();
        mv.move_type = dex::type_id(c[3]).unwrap();
        mv.category = if v % 2 != 0 {
            dex::Category::Special
        } else {
            dex::Category::Physical
        };
        mv.flags = if v % 3 != 0 {
            dex::FLAG_SOUND | dex::FLAG_CONTACT
        } else {
            0
        };
        mv.type_changer_boosted = if v % 3 == 1 { id } else { EffectId::NONE };
        mv.hit_data[6].type_mod = (v % 3) as i8 - 1;
        // Unburden calls ignoringAbility: inactive Pokemon provide the mocked suppression branch.
        if v % 3 == 2 {
            b.state.pokemon[0].flags &= !mon_flags::ACTIVE;
        }
        let mut args = numeric_args(101.0);
        if c[2] == "onSourceModifyAccuracy" && v >= 6 {
            args[0] = EventArg::Bool(true)
        }
        let r = invoke(&mut b, id, c[2], args);
        let want = if c[6] == "undefined" {
            Relay::Undefined
        } else {
            Relay::Number(c[6].parse().unwrap())
        };
        assert_eq!(r, want, "{line}");
        assert_eq!(
            b.scratch.frames[0].unwrap().modifier,
            c[5].parse().unwrap(),
            "{line}"
        );
        assert_eq!(b.seed(), [1, 2, 3, 4]);
        count += 1;
    }
    assert_eq!(count, 2784);
}
#[test]
fn absorption_core_calls_match_pinned_oracle() {
    use super::host::{Host, absorb};
    struct Recording {
        truthy: bool,
        trace: Vec<String>,
        accuracy: bool,
    }
    impl Host for Recording {
        fn perfect_accuracy(&mut self) {
            self.accuracy = true
        }
        fn volatile(&mut self, m: MonId, id: EffectId) -> bool {
            self.trace
                .push(format!("volatile:{}:{}", m.0, dex::effect(id).key));
            self.truthy
        }
        fn boost(&mut self, s: usize) -> bool {
            self.trace
                .push(format!("boost:{}:1", if s == 0 { "atk" } else { "spe" }));
            self.truthy
        }
        fn heal(&mut self, _: MonId) -> bool {
            self.trace.push("heal:30".into());
            self.truthy
        }
        fn immune(&mut self, m: MonId, id: EffectId) {
            self.trace.push(format!(
                "-immune:{}:[from] ability: {}",
                m.0,
                dex::effect(id).name
            ));
        }
    }
    let mut count = 0;
    for line in include_str!("absorption.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line
            .split('\t')
            .map(|c| if c == "<empty>" { "" } else { c })
            .collect();
        let id = dex::lookup(EffectKind::Ability, c[0]).unwrap();
        let mut h = Recording {
            truthy: matches!(c[1], "true" | "12"),
            trace: vec![],
            accuracy: false,
        };
        assert_eq!(absorb(&mut h, id, MonId(0)), Relay::Null);
        assert_eq!(h.trace.join("|"), c[2], "{line}");
        assert_eq!(h.accuracy, c[3] == "true");
        count += 1;
    }
    assert_eq!(count, 36);
}
#[derive(Default)]
struct TraceSink(Vec<String>);
impl crate::log::LogSink for TraceSink {
    const ENABLED: bool = true;
    fn emit(&mut self, _: crate::log::LogView<'_>, e: crate::log::LogEntry<'_>) {
        use crate::log::{LogArg, LogTag};
        fn arg(a: LogArg<'_>) -> String {
            match a {
                LogArg::Text(s) => s.into(),
                LogArg::Mon(m) => m.0.to_string(),
                LogArg::Effect(e) => name(e, false),
                LogArg::EffectFullName(e) => name(e, true),
                _ => panic!("unexpected trace arg {a:?}"),
            }
        }
        fn name(e: EffectRef, full: bool) -> String {
            let EffectRef::Dex(id) = e else {
                panic!("expected dex effect")
            };
            if full {
                format!("ability: {}", dex::effect(id).name)
            } else {
                dex::effect(id).name.into()
            }
        }
        let mut parts = vec![e.command.to_owned()];
        parts.extend(e.args.iter().copied().map(arg));
        parts.extend(e.tags.iter().map(|tag| match tag {
            LogTag::From(e) => format!("[from] {}", name(*e, true)),
            LogTag::Of(m) => format!("[of] {}", m.0),
            LogTag::Bare(s) => format!("[{s}]"),
            _ => panic!("unexpected trace tag {tag:?}"),
        }));
        self.0.push(parts.join(":"));
    }
    fn edit_move(&mut self, _: crate::log::LogView<'_>, _: crate::log::MoveLineEdit<'_>) {
        panic!("unexpected move edit")
    }
}
fn logged() -> crate::Battle<TraceSink> {
    let b = battle();
    crate::Battle {
        state: b.state,
        teams: b.teams,
        parsed: b.parsed,
        log: TraceSink::default(),
        names: b.names,
        scratch: b.scratch,
    }
}
fn own_logged(b: &mut crate::Battle<TraceSink>, id: EffectId) {
    let holder = Holder::mon(MonId(0));
    let cell = b.state.effects.alloc(holder, holder, id, 0);
    b.state.pokemon[0].ability_state = cell;
    b.scratch.current_state = Some(CellRef {
        cell,
        generation: b.state.effects.cells[cell.0 as usize].generation,
    });
}
#[test]
fn guards_and_sparse_boost_deletions_match_pinned_oracle() {
    use super::set;
    let mut count = 0;
    for line in include_str!("guards.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line
            .split('\t')
            .map(|c| if c == "<empty>" { "" } else { c })
            .collect();
        let id = dex::lookup(EffectKind::Ability, c[0]).unwrap();
        let v: u32 = c[2].parse().unwrap();
        let mut b = logged();
        own_logged(&mut b, id);
        b.state.pokemon[0].types = [
            dex::type_id(if v >= 9 { "Grass" } else { "Normal" }).unwrap(),
            TypeId(0),
        ];
        b.state.pokemon[0].terastallized = TypeId(0);
        b.state.pokemon[0].flags |= mon_flags::ACTIVE;
        let source = MonId(if v % 3 == 0 {
            0
        } else if v % 3 == 1 {
            1
        } else {
            6
        });
        let status = dex::lookup(
            EffectKind::Condition,
            ["flinch", "confusion", "yawn", "taunt", "disable", "encore"][(v % 6) as usize],
        )
        .unwrap();
        let mv = b.scratch.moves[0].as_mut().unwrap();
        mv.id = if v % 3 != 0 {
            dex::MOVE_BODYPRESS
        } else {
            dex::MOVE_TAUNT
        };
        mv.move_type = dex::type_id("Normal").unwrap();
        mv.category = if v % 2 != 0 {
            dex::Category::Status
        } else {
            dex::Category::Physical
        };
        mv.flags = if v % 2 != 0 {
            dex::FLAG_BULLET | dex::FLAG_SOUND | dex::FLAG_POWDER
        } else {
            0
        };
        mv.effects.status = if v % 2 != 0 {
            dex::CONDITION_BRN
        } else {
            EffectId::NONE
        };
        mv.secondaries_present = v % 4 == 0;
        let mut boosts = crate::state::scratch::OrderedBoosts::default();
        set(&mut boosts, 4, (v % 3) as i8 - 1);
        set(&mut boosts, 0, if v % 2 != 0 { -1 } else { 1 });
        set(&mut boosts, 5, if v % 4 == 0 { -1 } else { 0 });
        let bs = b.stash_boosts(boosts);
        let effect = if v % 5 == 0 {
            EventArg::Effect(EffectRef::Dex(dex::ABILITY_INTIMIDATE))
        } else {
            EventArg::Move(0)
        };
        let args = if c[1].ends_with("TryHit") {
            [
                EventArg::Holder(Holder::mon(MonId(0))),
                EventArg::Holder(Holder::mon(source)),
                EventArg::Move(0),
                EventArg::Undefined,
            ]
        } else if c[1].ends_with("SetStatus") {
            [
                EventArg::Effect(EffectRef::Dex(
                    [
                        dex::CONDITION_SLP,
                        dex::CONDITION_BRN,
                        dex::CONDITION_PSN,
                        dex::CONDITION_TOX,
                    ][(v % 4) as usize],
                )),
                EventArg::Holder(Holder::mon(MonId(0))),
                EventArg::Holder(Holder::mon(source)),
                effect,
            ]
        } else if c[1].ends_with("TryAddVolatile") {
            [
                EventArg::Effect(EffectRef::Dex(status)),
                EventArg::Holder(Holder::mon(MonId(0))),
                EventArg::Holder(Holder::mon(source)),
                EventArg::Move(0),
            ]
        } else {
            [
                EventArg::Relay(Relay::Boosts(bs)),
                EventArg::Holder(Holder::mon(MonId(0))),
                EventArg::Holder(Holder::mon(source)),
                effect,
            ]
        };
        let r = invoke(&mut b, id, c[1], args);
        let want = match c[3] {
            "undefined" => Relay::Undefined,
            "null" => Relay::Null,
            "false" => Relay::Bool(false),
            _ => panic!("guard output"),
        };
        assert_eq!(r, want, "{line}");
        assert_eq!(b.log.0.join("|"), c[4], "{line}");
        let boosts = *b.scratch_boosts(bs);
        let names = ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"];
        let got = boosts.order[..boosts.len as usize]
            .iter()
            .map(|s| format!("{}:{}", names[*s as usize], boosts.values[*s as usize]))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(got, c[5], "{line}");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
        count += 1;
    }
    assert_eq!(count, 558);
}
#[test]
fn slow_start_counter_presence_and_expiry_match_source() {
    let mut b = logged();
    own_logged(&mut b, dex::ABILITY_SLOWSTART);
    for line in include_str!("state.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line
            .split('\t')
            .map(|c| if c == "<empty>" { "" } else { c })
            .collect();
        b.state.pokemon[0].active_turns = c[1].parse().unwrap();
        if c[2] == "true" {
            b.state.pokemon[0].flags |= mon_flags::BEING_CALLED_BACK
        } else {
            b.state.pokemon[0].flags &= !mon_flags::BEING_CALLED_BACK
        }
        b.log.0.clear();
        let key = format!("on{}", c[0]);
        assert_eq!(
            invoke(
                &mut b,
                dex::ABILITY_SLOWSTART,
                &key,
                [
                    EventArg::Holder(Holder::mon(MonId(0))),
                    EventArg::Undefined,
                    EventArg::Undefined,
                    EventArg::Undefined
                ]
            ),
            Relay::Undefined
        );
        let cell = b.scratch.current_state.unwrap().cell;
        let s = &b.state.effects.cells[cell.0 as usize];
        assert_eq!(s.present & (1 << 8) != 0, c[4] == "true", "{line}");
        if c[3] != "undefined" {
            assert_eq!(s.payload.words[0], c[3].parse().unwrap(), "{line}")
        }
        assert_eq!(b.log.0.join("|"), c[5], "{line}");
    }
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}
#[test]
fn move_mutations_preserve_immunity_booleans_and_type_exceptions() {
    use crate::state::scratch::move_runtime as rt;
    for line in include_str!("moves.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line
            .split('\t')
            .map(|c| if c == "<empty>" { "" } else { c })
            .collect();
        let v: u32 = c[1].parse().unwrap();
        let id = dex::lookup(EffectKind::Ability, c[0]).unwrap();
        let mut b = battle();
        b.state.pokemon[0].terastallized = if v >= 6 {
            dex::type_id("Normal").unwrap()
        } else {
            TypeId(0)
        };
        let mv = b.scratch.moves[0].as_mut().unwrap();
        mv.id = [
            dex::MOVE_BODYPRESS,
            dex::MOVE_JUDGMENT,
            dex::MOVE_REVELATIONDANCE,
            dex::MOVE_TERABLAST,
        ][v as usize % 4];
        mv.move_type = dex::type_id(if v % 3 != 0 { "Normal" } else { "Fire" }).unwrap();
        mv.flags = if v % 2 != 0 { dex::FLAG_SOUND } else { 0 };
        mv.runtime_flags = if v % 3 == 0 {
            rt::IGNORE_IMMUNITY | rt::IGNORE_IMMUNITY_PRESENT
        } else if v % 3 == 2 {
            rt::IGNORE_IMMUNITY_PRESENT
        } else {
            0
        };
        mv.ignore_immunity_types = if v % 3 == 2 {
            1 << dex::type_id("Fire").unwrap().0
        } else {
            0
        };
        let key = if matches!(
            id,
            dex::ABILITY_GALVANIZE | dex::ABILITY_PIXILATE | dex::ABILITY_LIQUIDVOICE
        ) {
            "onModifyType"
        } else {
            "onModifyMove"
        };
        invoke(&mut b, id, key, move_args());
        let mv = b.scratch.moves[0].unwrap();
        assert_eq!(mv.move_type, dex::type_id(c[2]).unwrap(), "{line}");
        assert_eq!(
            mv.type_changer_boosted,
            if c[3] == "none" { EffectId::NONE } else { id },
            "{line}"
        );
        assert_eq!(
            mv.runtime_flags & rt::IGNORE_ABILITY != 0,
            c[4] == "true",
            "{line}"
        );
        assert_eq!(
            mv.runtime_flags & rt::IGNORE_EVASION != 0,
            c[5] == "true",
            "{line}"
        );
        if c[6] == "true" {
            assert!(mv.runtime_flags & rt::IGNORE_IMMUNITY != 0)
        } else {
            let want = c[6]
                .split(',')
                .filter(|t| !t.is_empty())
                .fold(0, |mask, t| mask | (1 << dex::type_id(t).unwrap().0));
            assert_eq!(mv.ignore_immunity_types, want, "{line}");
        }
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}
#[test]
fn mirror_armor_relay_deletion_reflection_and_live_hp_match_source() {
    use super::{
        host::{MirrorHost, mirror},
        set,
    };
    use crate::state::scratch::OrderedBoosts;
    struct Recording {
        v: u32,
        boost: OrderedBoosts,
        alive: bool,
        trace: Vec<String>,
    }
    impl MirrorHost for Recording {
        fn boosts(&mut self) -> &mut OrderedBoosts {
            &mut self.boost
        }
        fn stage(&mut self, s: usize) -> i8 {
            if s == 4 && self.v % 3 == 0 || s == 0 && self.v % 3 == 1 {
                -6
            } else {
                0
            }
        }
        fn source_alive(&mut self) -> bool {
            self.alive
        }
        fn announce(&mut self) {
            self.trace.push("-ability:0:Mirror Armor".into())
        }
        fn reflect(&mut self, s: usize, n: i8) {
            let stat = ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"][s];
            self.trace.push(format!("boost:{stat}:{n}:6:0:null:true"));
            if self.v % 4 == 1 {
                self.alive = false
            }
        }
    }
    for line in include_str!("mirror.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line
            .split('\t')
            .map(|c| if c == "<empty>" { "" } else { c })
            .collect();
        let v: u32 = c[0].parse().unwrap();
        let mut boost = OrderedBoosts::default();
        set(&mut boost, 4, -1);
        set(&mut boost, 0, -2);
        set(&mut boost, 5, if v % 2 != 0 { 1 } else { -1 });
        let mut h = Recording {
            v,
            boost,
            alive: v % 4 != 0,
            trace: vec![],
        };
        mirror(&mut h);
        assert_eq!(h.trace.join("|"), c[1], "{line}");
        let got = h.boost.order[..h.boost.len as usize]
            .iter()
            .map(|s| {
                format!(
                    "{}:{}",
                    ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"][*s as usize],
                    h.boost.values[*s as usize]
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(got, c[2], "{line}");
    }
}
#[test]
fn event_relays_unaware_presence_and_damage_guards_match_oracle() {
    use super::set;
    for line in include_str!("events.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line
            .split('\t')
            .map(|c| if c == "<empty>" { "" } else { c })
            .collect();
        let id = dex::lookup(EffectKind::Ability, c[0]).unwrap();
        let v: u32 = c[2].parse().unwrap();
        let mut b = logged();
        own_logged(&mut b, id);
        let source = MonId(if v % 3 == 0 {
            0
        } else if v % 3 == 1 {
            1
        } else {
            6
        });
        let target = MonId(if v % 4 == 0 { 0 } else { 6 });
        b.state.pokemon[target.0 as usize].hp = if v % 3 == 0 { 119 } else { 120 };
        b.state.pokemon[target.0 as usize].max_hp = 120;
        b.scratch.active_pokemon = if v % 3 == 0 { MonId(0) } else { source };
        b.scratch.active_target = if v % 2 != 0 { MonId(0) } else { target };
        b.scratch.active_move = crate::actions::MoveHandle(0);
        b.scratch.moves[0].as_mut().unwrap().id = if v % 2 != 0 {
            dex::MOVE_STRUGGLE
        } else {
            dex::MOVE_BODYPRESS
        };
        let mov = if v % 3 == 0 {
            EventArg::Undefined
        } else if v % 3 == 1 {
            EventArg::Null
        } else {
            EventArg::Move(0)
        };
        let mut boost = crate::state::scratch::OrderedBoosts::default();
        set(&mut boost, 6, 2);
        set(&mut boost, 4, 1);
        let s = b.stash_boosts(boost);
        let m = |m| EventArg::Holder(Holder::mon(m));
        let args = match c[1] {
            "onAnyAccuracy" => [
                if v % 2 != 0 {
                    EventArg::Bool(true)
                } else {
                    EventArg::Number(83.0)
                },
                m(target),
                m(source),
                mov,
            ],
            "onAnyInvulnerability" => [m(target), m(source), mov, EventArg::Undefined],
            "onDeductPP" => [
                m(target),
                m(source),
                EventArg::Undefined,
                EventArg::Undefined,
            ],
            "onAnyModifyBoost" => [
                EventArg::Relay(Relay::Boosts(s)),
                m(if v % 4 == 0 { MonId(0) } else { source }),
                EventArg::Undefined,
                EventArg::Undefined,
            ],
            "onImmunity" => [
                match v % 6 {
                    0 => EventArg::StaticText("attract"),
                    1 => EventArg::StatusImmunity(dex::ImmunityId::Sandstorm),
                    2 => EventArg::StatusImmunity(dex::ImmunityId::Hail),
                    3 => EventArg::StatusImmunity(dex::ImmunityId::Powder),
                    4 => EventArg::StatusImmunity(dex::ImmunityId::Brn),
                    _ => EventArg::StaticText("poison"),
                },
                m(target),
                EventArg::Undefined,
                EventArg::Undefined,
            ],
            "onDamage" => [
                EventArg::Number(if v % 2 != 0 { 120.0 } else { 40.0 }),
                m(target),
                m(source),
                EventArg::Effect(match v % 4 {
                    0 => EffectRef::Synthetic(crate::state::scratch::SyntheticEffect::Recoil),
                    1 => EffectRef::Dex(dex::MOVE_BODYPRESS),
                    2 => EffectRef::Dex(dex::ABILITY_ROUGHSKIN),
                    _ => EffectRef::Dex(dex::CONDITION_SANDSTORM),
                }),
            ],
            _ => panic!(),
        };
        let r = invoke(&mut b, id, c[1], args);
        let want = match c[3] {
            "undefined" => Relay::Undefined,
            "null" => Relay::Null,
            "true" => Relay::Bool(true),
            "false" => Relay::Bool(false),
            n => Relay::Number(n.parse().unwrap()),
        };
        assert_eq!(r, want, "{line}");
        assert_eq!(b.log.0.join("|"), c[4], "{line}");
        let boost = *b.scratch_boosts(s);
        let got = boost.order[..boost.len as usize]
            .iter()
            .map(|s| {
                format!(
                    "{}:{}",
                    ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"][*s as usize],
                    boost.values[*s as usize]
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(got, c[5], "{line}");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}
#[test]
fn shield_dust_returns_a_new_stable_filtered_array() {
    use crate::state::scratch::SecondaryScratch;
    let mut b = battle();
    let effect = effects(&dex::move_data(dex::MOVE_THUNDERBOLT).effects);
    let sec = |chance, self_effect| SecondaryScratch {
        effects: effect,
        self_effect,
        chance,
        chance_present: true,
    };
    let original = [
        Some(sec(10, None)),
        Some(sec(20, Some(effect))),
        Some(sec(30, None)),
        Some(sec(40, Some(effect))),
    ];
    let s = b.stash_secondaries(original);
    let r = invoke(
        &mut b,
        dex::ABILITY_SHIELDDUST,
        "onModifySecondaries",
        [
            EventArg::Relay(Relay::Secondaries(s)),
            EventArg::Undefined,
            EventArg::Undefined,
            EventArg::Undefined,
        ],
    );
    let Relay::Secondaries(kept) = r else {
        panic!()
    };
    assert_ne!(kept, s);
    assert_eq!(b.scratch_secondaries(s)[0].unwrap().chance, 10);
    let got = b.scratch_secondaries(kept);
    assert_eq!(got[0].unwrap().chance, 20);
    assert_eq!(got[1].unwrap().chance, 40);
    assert!(got[2].is_none());
    assert!(got[3].is_none());
    b.release_relay(r);
    b.release_relay(Relay::Secondaries(s));
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}

#[test]
fn complete_batch_manifest_coverage() {
    let mut functions = 0;
    for id in [
        dex::ABILITY_AROMAVEIL,
        dex::ABILITY_BULLETPROOF,
        dex::ABILITY_CLEARBODY,
        dex::ABILITY_COMATOSE,
        dex::ABILITY_COMPOUNDEYES,
        dex::ABILITY_CONTRARY,
        dex::ABILITY_EARTHEATER,
        dex::ABILITY_FILTER,
        dex::ABILITY_FLASHFIRE,
        dex::ABILITY_FLOWERVEIL,
        dex::ABILITY_FLUFFY,
        dex::ABILITY_FULLMETALBODY,
        dex::ABILITY_FURCOAT,
        dex::ABILITY_GALVANIZE,
        dex::ABILITY_GOODASGOLD,
        dex::ABILITY_HUSTLE,
        dex::ABILITY_ICESCALES,
        dex::ABILITY_INNERFOCUS,
        dex::ABILITY_INSOMNIA,
        dex::ABILITY_KEENEYE,
        dex::ABILITY_LIQUIDVOICE,
        dex::ABILITY_MAGICGUARD,
        dex::ABILITY_MINDSEYE,
        dex::ABILITY_MIRRORARMOR,
        dex::ABILITY_MOLDBREAKER,
        dex::ABILITY_MOTORDRIVE,
        dex::ABILITY_MULTISCALE,
        dex::ABILITY_NOGUARD,
        dex::ABILITY_OBLIVIOUS,
        dex::ABILITY_OVERCOAT,
        dex::ABILITY_OWNTEMPO,
        dex::ABILITY_PIXILATE,
        dex::ABILITY_PRESSURE,
        dex::ABILITY_PRISMARMOR,
        dex::ABILITY_PUNKROCK,
        dex::ABILITY_PURIFYINGSALT,
        dex::ABILITY_ROCKHEAD,
        dex::ABILITY_SAPSIPPER,
        dex::ABILITY_SCRAPPY,
        dex::ABILITY_SHADOWSHIELD,
        dex::ABILITY_SHIELDDUST,
        dex::ABILITY_SLOWSTART,
        dex::ABILITY_SOLIDROCK,
        dex::ABILITY_SOUNDPROOF,
        dex::ABILITY_STURDY,
        dex::ABILITY_TELEPATHY,
        dex::ABILITY_TERAVOLT,
        dex::ABILITY_THICKFAT,
        dex::ABILITY_TINTEDLENS,
        dex::ABILITY_TURBOBLAZE,
        dex::ABILITY_UNAWARE,
        dex::ABILITY_UNBURDEN,
        dex::ABILITY_VITALSPIRIT,
        dex::ABILITY_VOLTABSORB,
        dex::ABILITY_WATERABSORB,
        dex::ABILITY_WATERBUBBLE,
        dex::ABILITY_WATERVEIL,
        dex::CONDITION_FLASHFIRE,
        dex::CONDITION_UNBURDEN,
    ] {
        let manifest = &dex::MANIFESTS[id.0 as usize];
        for (i, h) in manifest.hooks().iter().enumerate() {
            if matches!(h.value, dex::HookValue::Function) {
                functions += 1;
                assert_eq!(
                    crate::effects::registry::hook_coverage(dex::HookId(
                        manifest.hooks_start + i as u16
                    )),
                    crate::effects::HookCoverage::Implemented
                );
            }
        }
    }
    assert_eq!(functions, 109);
}
#[test]
fn cure_and_volatile_update_call_order_matches_source() {
    use super::host::{UpdateHost, update};
    use crate::state::Status;
    struct Recording {
        v: u32,
        id: EffectId,
        trace: Vec<String>,
    }
    impl UpdateHost for Recording {
        fn status(&mut self) -> Status {
            [Status::Sleep, Status::Burn, Status::Poison, Status::None][self.v as usize % 4]
        }
        fn volatile(&mut self, key: &'static str) -> bool {
            match key {
                "attract" => self.v % 3 != 0,
                "taunt" => self.v % 2 != 0,
                "confusion" => self.v % 2 == 0,
                _ => panic!(),
            }
        }
        fn activate(&mut self) {
            self.trace.push(format!(
                "-activate:0:ability: {}",
                dex::effect(self.id).name
            ));
        }
        fn cure(&mut self) {
            self.trace.push("cure".into())
        }
        fn remove(&mut self, key: &'static str) {
            self.trace.push(format!("remove:{key}"))
        }
        fn attract_end(&mut self) {
            self.trace
                .push("-end:0:move: Attract:[from] ability: Oblivious".into())
        }
    }
    for line in include_str!("update.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line
            .split('\t')
            .map(|c| if c == "<empty>" { "" } else { c })
            .collect();
        let id = dex::lookup(EffectKind::Ability, c[0]).unwrap();
        let v = c[1].parse().unwrap();
        let mut h = Recording {
            v,
            id,
            trace: vec![],
        };
        update(&mut h, id);
        assert_eq!(h.trace.join("|"), c[2], "{line}");
    }
}
