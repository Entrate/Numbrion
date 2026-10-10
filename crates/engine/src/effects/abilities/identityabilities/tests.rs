use crate::effects::registry::abilities_protosynthesis::tests::support::*;
use crate::{dex, event::*, ids::*, state::mon_flags};
#[test]
fn illusion_party_selection_matches_pinned_callbacks() {
    for row in include_str!("illusion.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = battle();
        let pos = c[1].parse::<usize>().unwrap();
        let user = b.state.sides[0].party[pos];
        b.state.pokemon[3].species = id(EffectKind::Species, c[2]);
        if c[3] == "1" {
            b.state.pokemon[3].flags |= mon_flags::FAINTED;
        }
        b.state.pokemon[user.0 as usize].terastallized = if c[0].is_empty() {
            TypeId::NONE
        } else {
            dex::type_id(c[0]).unwrap()
        };
        b.state.pokemon[user.0 as usize].illusion = MonId(6);
        assert_eq!(
            invoke(
                &mut b,
                dex::ABILITY_ILLUSION,
                "onBeforeSwitchIn",
                [
                    holder(user.0),
                    EventArg::Undefined,
                    EventArg::Undefined,
                    EventArg::Undefined
                ]
            ),
            Relay::Undefined
        );
        let expected = c[4].parse::<i8>().unwrap();
        assert_eq!(
            b.state.pokemon[user.0 as usize].illusion,
            if expected < 0 {
                MonId::NONE
            } else {
                MonId(expected as u8)
            },
            "{row}"
        );
        assert_eq!(b.seed(), [1, 2, 3, 4]);
        assert!(b.log.0.is_empty());
    }
}
#[test]
fn trace_filters_and_samples_even_a_single_candidate() {
    for row in include_str!("trace.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = battle();
        b.state.pokemon[0].ability = dex::ABILITY_TRACE;
        b.state.pokemon[6].ability = id(EffectKind::Ability, c[0]);
        b.state.pokemon[7].ability = id(EffectKind::Ability, c[1]);
        let r = state(&mut b, dex::ABILITY_TRACE);
        b.state.effects.cells[r.cell.0 as usize].payload.words[0] = u32::from(c[2] == "1");
        b.state.effects.cells[r.cell.0 as usize].present |= 1 << 8;
        assert_eq!(
            invoke(
                &mut b,
                dex::ABILITY_TRACE,
                "onUpdate",
                [
                    holder(0),
                    EventArg::Undefined,
                    EventArg::Undefined,
                    EventArg::Undefined
                ]
            ),
            Relay::Undefined
        );
        assert_eq!(
            b.state.pokemon[0].ability,
            id(EffectKind::Ability, c[3]),
            "{row}"
        );
        assert_eq!(b.seed(), numbers::<u16>(c[5]).as_slice(), "{row}");
        if c[4] == "-1" {
            assert!(b.log.0.is_empty());
        } else {
            assert_eq!(b.log.0.len(), 1);
            let source = 6 + c[4].parse::<u8>().unwrap();
            assert!(
                b.log.0[0].ends_with(&format!("[from] ability: Trace|[of] m{source}")),
                "{row}: {:?}",
                b.log.0
            );
        }
    }
}
#[test]
fn as_one_start_restart_end_and_berry_sentinels_match_oracle() {
    for row in include_str!("asone.tsv").lines() {
        let c: Vec<_> = row.split('\t').collect();
        let mut b = battle();
        let ability = id(EffectKind::Ability, c[0]);
        let r = state(&mut b, ability);
        let a = [
            holder(0),
            EventArg::Undefined,
            EventArg::Undefined,
            EventArg::Undefined,
        ];
        assert_eq!(
            invoke(&mut b, ability, "onFoeTryEatItem", a),
            Relay::Bool(c[1] == "1")
        );
        invoke(&mut b, ability, "onStart", a);
        invoke(&mut b, ability, "onStart", a);
        assert_eq!(
            invoke(&mut b, ability, "onFoeTryEatItem", a),
            Relay::Bool(c[2] == "1")
        );
        assert_eq!(b.state.effects.cells[r.cell.0 as usize].payload.words[0], 1);
        assert_eq!(b.log.0.join("\\n"), c[5]);
        invoke(
            &mut b,
            ability,
            "onSourceAfterFaint",
            [
                EventArg::Number(2.),
                holder(6),
                holder(0),
                EventArg::Effect(EffectRef::Dex(dex::MOVE_THUNDERBOLT)),
            ],
        );
        assert_eq!(
            b.state.pokemon[0].boosts[if c[0] == "asoneglastrier" { 0 } else { 2 }],
            2
        );
        invoke(&mut b, ability, "onEnd", a);
        assert_eq!(
            invoke(&mut b, ability, "onFoeTryEatItem", a),
            Relay::Bool(c[3] == "1")
        );
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}
