pub(crate) mod support;
use crate::{actions::*, dex, event::*, ids::*, state::mon_flags};
use support::*;
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
fn batch_function_sites_are_all_covered() {
    for e in crate::effects::registry::IMPLEMENTATIONS {
        if e.file.contains("species/")
            || [
                dex::ABILITY_PROTOSYNTHESIS,
                dex::ABILITY_QUARKDRIVE,
                dex::ITEM_BOOSTERENERGY,
                dex::MOVE_TERABLAST,
                dex::MOVE_TERASTARSTORM,
            ]
            .contains(&e.id)
        {
            for &h in e.hooks {
                assert!(matches!(
                    crate::effects::registry::hook_coverage(h),
                    crate::effects::HookCoverage::Implemented
                ));
            }
        }
    }
}
