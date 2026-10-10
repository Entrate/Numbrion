pub(crate) mod support;
use crate::{actions::*, dex, event::*, ids::*, state::mon_flags};
use support::*;
#[test]
fn teraform_zero_is_silent_on_an_empty_field() {
    let ally = "Mew|||earlybird|irondefense|Serious||N|||100|,,,,,Normal";
    let team = format!("Terapagos|||terashift|irondefense|Serious||M|||73|,,,,,Stellar]{ally}");
    let foe = format!("{ally}]{ally}");
    let mut b = crate::Battle::from_players(
        [15999, 37686, 31317, 49466],
        ("A", &team),
        ("B", &foe),
        crate::log::TextLog::default(),
    )
    .unwrap();
    b.start().unwrap();
    let mut log = Vec::new();
    b.drain_log(&mut log);
    log.clear();
    b.terastallize(MonId(0));
    b.drain_log(&mut log);
    assert_eq!(b.state.pokemon[0].ability, dex::ABILITY_TERAFORMZERO);
    assert!(log.iter().any(|l| l.starts_with("|-terastallize|")));
    assert!(!log.iter().any(|l| l.contains("Teraform Zero")), "{log:?}");
}

#[test]
fn move_overlays_match_pinned_callbacks() {
    for row in include_str!("moves.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        assert_eq!(c.len(), 15);
        let mut b = battle();
        let move_id = id(EffectKind::Move, c[0]);
        let species = id(EffectKind::Species, c[1]);
        let tera = if c[2].is_empty() {
            TypeId::NONE
        } else {
            dex::type_id(c[2]).unwrap()
        };
        b.teams.sides[0].sets[0].tera_type = tera;
        let p = &mut b.state.pokemon[0];
        p.species = species;
        p.base_species = species;
        p.terastallized = tera;
        p.stored_stats = [c[3].parse().unwrap(), 100, c[4].parse().unwrap(), 100, 100];
        p.types = [TypeId::NONE; 2];
        for (i, t) in c[5].split(',').enumerate() {
            p.types[i] = dex::type_id(t).unwrap();
        }
        p.item = id(EffectKind::Item, c[6]);
        if c[7] == "1" {
            p.flags &= !mon_flags::ACTIVE;
        }
        let mut mv = active_move(move_id);
        mv.base_power = c[8].parse().unwrap();
        mv.move_type = dex::type_id(c[9]).unwrap();
        b.scratch.moves[0] = Some(mv);
        let args = [EventArg::Move(0), holder(0), holder(6), EventArg::Undefined];
        assert_eq!(
            invoke(&mut b, move_id, "onModifyType", args),
            Relay::Undefined,
            "{row}"
        );
        assert_eq!(
            invoke(&mut b, move_id, "onModifyMove", args),
            Relay::Undefined,
            "{row}"
        );
        let bp = invoke(
            &mut b,
            move_id,
            "basePowerCallback",
            [holder(0), holder(6), EventArg::Move(0), EventArg::Undefined],
        );
        let mv = b.scratch.moves[0].unwrap();
        assert_eq!(mv.move_type, dex::type_id(c[10]).unwrap(), "{row}");
        assert_eq!(format!("{:?}", mv.category), c[11], "{row}");
        assert_eq!(
            mv.target,
            if c[12] == "allAdjacentFoes" {
                dex::MoveTarget::AllAdjacentFoes
            } else {
                dex::MoveTarget::Normal
            },
            "{row}"
        );
        assert_eq!(
            if let Relay::Number(n) = bp {
                n
            } else {
                mv.base_power
            },
            c[13].parse::<f64>().unwrap(),
            "{row}"
        );
        if c[14].is_empty() {
            assert!(mv.self_effect.is_none(), "{row}");
        } else {
            let s = mv.self_effect.unwrap();
            assert_eq!(&s.boosts.order[..s.boosts.len as usize], &[0, 2]);
            assert_eq!(s.boosts.values[0], -1);
            assert_eq!(s.boosts.values[2], -1);
            assert!(s.base.hooks.is_empty());
            assert!(s.chance.is_none());
        }
        assert_eq!(b.seed(), [1, 2, 3, 4], "{row}");
    }
}
#[test]
fn paradox_activation_stats_suppression_and_end_match_oracle() {
    for row in include_str!("paradox.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = battle();
        let ability = id(EffectKind::Ability, c[0]);
        let cond = id(EffectKind::Condition, c[0]);
        b.state.pokemon[0].ability = ability;
        b.state.pokemon[0].stored_stats = numbers::<u16>(c[1]).try_into().unwrap();
        let r = state(&mut b, cond);
        let effect = if c[2] == "1" {
            dex::ITEM_BOOSTERENERGY
        } else {
            ability
        };
        assert_eq!(
            invoke(
                &mut b,
                cond,
                "onStart",
                [
                    holder(0),
                    EventArg::Null,
                    EventArg::Effect(EffectRef::Dex(effect)),
                    EventArg::Undefined
                ]
            ),
            Relay::Undefined
        );
        if c[3] == "1" {
            b.state.pokemon[0].flags |= mon_flags::TRANSFORMED;
        }
        let expected = numbers::<u32>(c[5]);
        for (i, key) in [
            "onModifyAtk",
            "onModifyDef",
            "onModifySpA",
            "onModifySpD",
            "onModifySpe",
        ]
        .iter()
        .enumerate()
        {
            b.scratch.frames[0].as_mut().unwrap().modifier = 4096;
            assert_eq!(
                invoke(
                    &mut b,
                    cond,
                    key,
                    [
                        EventArg::Number(100.),
                        holder(0),
                        EventArg::Undefined,
                        EventArg::Undefined
                    ]
                ),
                Relay::Undefined
            );
            assert_eq!(b.scratch.frames[0].unwrap().modifier, expected[i], "{row}");
        }
        let payload = b.state.effects.cells[r.cell.0 as usize];
        assert_eq!(payload.payload.words[0], u32::from(c[2] == "1"));
        let best = ["atk", "def", "spa", "spd", "spe"]
            .iter()
            .position(|s| *s == c[4])
            .unwrap();
        assert_eq!(payload.payload.words[1], best as u32, "{row}");
        assert_eq!(
            invoke(
                &mut b,
                cond,
                "onEnd",
                [
                    holder(0),
                    EventArg::Undefined,
                    EventArg::Undefined,
                    EventArg::Undefined
                ]
            ),
            Relay::Undefined
        );
        assert_eq!(b.log.0.join("\\n"), c[6], "{row}");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}

#[test]
fn battle_scenario_teams_are_in_scope() {
    check_scenario_teams(include_str!("scenarios.tsv"));
}
macro_rules! scenario {
    ($($name:ident),*) => {$ (
        #[test]
        #[ignore = "needs core"]
        fn $name() { replay_scenario(include_str!("scenarios.tsv"), stringify!($name)); }
    )*};
}
scenario!(
    sun_to_booster,
    electric_to_booster_teraform,
    disguise_bust,
    ice_face_break_restore,
    morpeko_toggle,
    palafin_switch,
    relic_song_toggle,
    ogerpon_spe,
    ogerpon_spd,
    ogerpon_atk,
    ogerpon_def,
    stellar_tera_blast,
    shields_down_crossing
);

scenario!(
    crowned_zacian,
    crowned_zacian_crowned,
    crowned_zamazenta,
    crowned_zamazenta_crowned
);

#[test]
fn shields_keep_zero_false_and_undefined_distinct() {
    for row in include_str!("guards.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = battle();
        let ability = id(EffectKind::Ability, c[0]);
        b.state.pokemon[0].species = id(EffectKind::Species, c[2]);
        b.state.pokemon[0].types = [
            dex::type_id(if c[5] == "1" { "Normal" } else { "Ghost" }).unwrap(),
            TypeId::NONE,
        ];
        let r = state(&mut b, ability);
        if c[4] != "plain" {
            let cell = b.state.effects.alloc(
                Holder::mon(MonId(0)),
                Holder::mon(MonId(0)),
                dex::CONDITION_SUBSTITUTE,
                0,
            );
            b.state.pokemon[0].volatiles.push(cell);
        }
        let mut mv = active_move(dex::MOVE_BODYSLAM);
        mv.category = match c[3] {
            "Physical" => dex::Category::Physical,
            "Special" => dex::Category::Special,
            _ => dex::Category::Status,
        };
        mv.flags = if c[4] == "bypass" {
            dex::FLAG_BYPASSSUB
        } else {
            0
        };
        mv.runtime_flags = if c[4] == "infiltrates" {
            crate::state::scratch::move_runtime::INFILTRATES
        } else {
            0
        };
        b.scratch.moves[0] = Some(mv);
        let effect = if c[3] == "Confused" {
            EffectRef::Synthetic(SyntheticEffect::Confused)
        } else {
            EffectRef::ActiveMove(0)
        };
        let args = match c[1] {
            "onDamage" => [
                EventArg::Number(37.),
                holder(0),
                holder(6),
                EventArg::Effect(effect),
            ],
            "onCriticalHit" => [holder(0), holder(6), EventArg::Move(0), EventArg::Undefined],
            _ => [
                EventArg::Number(1.),
                holder(0),
                EventArg::Type(dex::type_id("Normal").unwrap()),
                EventArg::Move(0),
            ],
        };
        let expected = match c[6] {
            "undefined" => Relay::Undefined,
            "false" => Relay::Bool(false),
            "0" => Relay::Number(0.),
            other => panic!("{other}"),
        };
        assert_eq!(invoke(&mut b, ability, c[1], args), expected, "{row}");
        let cell = b.state.effects.cells[r.cell.0 as usize];
        assert_eq!(cell.present & (1 << 8) != 0, c[7] != "absent", "{row}");
        assert_eq!(
            cell.payload.words[0],
            if c[7] == "absent" {
                0
            } else {
                c[7].parse().unwrap()
            },
            "{row}"
        );
        assert_eq!(b.log.0.join("\\n"), c[8], "{row}");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}

#[test]
fn inherited_arceus_views_use_raw_identity_and_item() {
    for row in include_str!("arceus.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = battle();
        let species = id(EffectKind::Species, c[0]);
        let p = &mut b.state.pokemon[0];
        p.species = species;
        p.ability = id(EffectKind::Ability, c[1]);
        p.item = id(EffectKind::Item, c[2]);
        p.flags &= !mon_flags::ACTIVE; // item suppression must not affect Multitype's raw item read.
        if c[3] == "1" {
            p.flags |= mon_flags::TRANSFORMED;
        }
        let original = b.stash_types(Types {
            values: [
                dex::type_id(c[4].split(',').next().unwrap()).unwrap(),
                dex::type_id("Flying").unwrap(),
                TypeId::NONE,
            ],
            len: 2,
        });
        let result = invoke(
            &mut b,
            species,
            "onType",
            [
                EventArg::Relay(Relay::Types(original)),
                holder(0),
                EventArg::Undefined,
                EventArg::Undefined,
            ],
        );
        let Relay::Types(handle) = result else {
            panic!("{row}")
        };
        let actual = *b.scratch_types(handle);
        let expected: Vec<_> = c[5].split(',').map(|t| dex::type_id(t).unwrap()).collect();
        assert_eq!(
            &actual.values[..actual.len as usize],
            expected.as_slice(),
            "{row}"
        );
        if c[3] == "1" || c[1] != "multitype" {
            assert_eq!(handle, original, "{row}");
        }
        b.release_relay(result);
        if handle != original {
            b.release_relay(Relay::Types(original));
        }
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}

#[test]
fn every_batch_function_site_is_registered() {
    const IDS: &[EffectId] = &[
        dex::SPECIES_ARCEUS,
        dex::SPECIES_ARCEUSBUG,
        dex::SPECIES_ARCEUSDARK,
        dex::SPECIES_ARCEUSDRAGON,
        dex::SPECIES_ARCEUSELECTRIC,
        dex::SPECIES_ARCEUSFAIRY,
        dex::SPECIES_ARCEUSFIGHTING,
        dex::SPECIES_ARCEUSFIRE,
        dex::SPECIES_ARCEUSFLYING,
        dex::SPECIES_ARCEUSGHOST,
        dex::SPECIES_ARCEUSGRASS,
        dex::SPECIES_ARCEUSGROUND,
        dex::SPECIES_ARCEUSICE,
        dex::SPECIES_ARCEUSPOISON,
        dex::SPECIES_ARCEUSPSYCHIC,
        dex::SPECIES_ARCEUSROCK,
        dex::SPECIES_ARCEUSSTEEL,
        dex::SPECIES_ARCEUSWATER,
        dex::SPECIES_ZACIAN,
        dex::SPECIES_ZACIANCROWNED,
        dex::SPECIES_ZAMAZENTA,
        dex::SPECIES_ZAMAZENTACROWNED,
        dex::MOVE_AURAWHEEL,
        dex::MOVE_IVYCUDGEL,
        dex::MOVE_JUDGMENT,
        dex::MOVE_RELICSONG,
        dex::MOVE_REVELATIONDANCE,
        dex::MOVE_TERABLAST,
        dex::MOVE_TERASTARSTORM,
        dex::ABILITY_DISGUISE,
        dex::ABILITY_EMBODYASPECTCORNERSTONE,
        dex::ABILITY_EMBODYASPECTHEARTHFLAME,
        dex::ABILITY_EMBODYASPECTTEAL,
        dex::ABILITY_EMBODYASPECTWELLSPRING,
        dex::ABILITY_GULPMISSILE,
        dex::ABILITY_HUNGERSWITCH,
        dex::ABILITY_ICEFACE,
        dex::ABILITY_LIBERO,
        dex::ABILITY_PROTEAN,
        dex::ABILITY_PROTOSYNTHESIS,
        dex::ABILITY_QUARKDRIVE,
        dex::ABILITY_SHIELDSDOWN,
        dex::ABILITY_TERAFORMZERO,
        dex::ABILITY_TERASHIFT,
        dex::ABILITY_ZEROTOHERO,
        dex::ITEM_BOOSTERENERGY,
        dex::CONDITION_PROTOSYNTHESIS,
        dex::CONDITION_QUARKDRIVE,
    ];
    let mut count = 0;
    for (i, h) in dex::HOOKS.iter().enumerate() {
        if IDS.contains(&h.effect) && matches!(h.value, dex::HookValue::Function) {
            assert!(
                matches!(
                    crate::effects::hook_coverage(dex::HookId(i as u16)),
                    crate::effects::HookCoverage::Implemented
                ),
                "{} {} {}",
                dex::effect(h.effect).key,
                h.key,
                h.site
            );
            count += 1;
        }
    }
    assert_eq!(count, 85);
}

#[test]
fn booster_energy_take_item_uses_base_species_tags() {
    for row in include_str!("takeitem.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = battle();
        b.state.pokemon[0].base_species = id(EffectKind::Species, c[0]);
        b.state.pokemon[0].species = dex::SPECIES_PIKACHU;
        b.state.pokemon[0].flags |= mon_flags::TRANSFORMED;
        assert_eq!(
            invoke(
                &mut b,
                dex::ITEM_BOOSTERENERGY,
                "onTakeItem",
                [
                    EventArg::Effect(EffectRef::Dex(dex::ITEM_BOOSTERENERGY)),
                    holder(0),
                    holder(6),
                    EventArg::Undefined
                ]
            ),
            Relay::Bool(c[1] == "true"),
            "{row}"
        );
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}
#[test]
fn shields_down_status_and_yawn_keep_different_failure_sentinels() {
    for row in include_str!("status.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = battle();
        b.state.pokemon[0].species = id(EffectKind::Species, c[0]);
        if c[1] == "1" {
            b.state.pokemon[0].flags |= mon_flags::TRANSFORMED;
        }
        let key = if c[2] == "yawn" || c[2] == "substitute" {
            "onTryAddVolatile"
        } else {
            "onSetStatus"
        };
        let status = if key == "onSetStatus" {
            dex::CONDITION_BRN
        } else {
            id(EffectKind::Condition, c[2])
        };
        let effect = if c[2] == "statusmove" {
            dex::MOVE_WILLOWISP
        } else {
            dex::ABILITY_STATIC
        };
        let expected = match c[3] {
            "false" => Relay::Bool(false),
            "null" => Relay::Null,
            "undefined" => Relay::Undefined,
            _ => panic!("{row}"),
        };
        assert_eq!(
            invoke(
                &mut b,
                dex::ABILITY_SHIELDSDOWN,
                key,
                [
                    EventArg::Relay(Relay::Effect(status)),
                    holder(0),
                    holder(6),
                    EventArg::Effect(EffectRef::Dex(effect))
                ]
            ),
            expected,
            "{row}"
        );
        assert_eq!(b.log.0.join("\\n"), c[4], "{row}");
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}

scenario!(gulping_missile, gorging_missile);
