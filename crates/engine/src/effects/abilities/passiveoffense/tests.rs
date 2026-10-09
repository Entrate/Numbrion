mod support;
use crate::{
    dex,
    effects::registry,
    event::*,
    ids::*,
    state::scratch::{SecondaryScratch, move_runtime},
};
use support::*;
#[test]
fn numeric_callbacks_match_pinned_oracle_vectors() {
    for line in include_str!("vectors.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line.split('\t').collect();
        let mut b = battle();
        let id = dex::lookup(EffectKind::Ability, c[0]).unwrap();
        let n = |i: usize| c[i].parse::<u32>().unwrap();
        b.state.pokemon[0].hp = n(4) as u16;
        b.state.pokemon[6].active_turns = n(5);
        b.scratch.frames[0].as_mut().unwrap().modifier = n(7);
        let mv = b.scratch.moves[0].as_mut().unwrap();
        mv.move_type = dex::type_id(c[2]).unwrap();
        mv.flags = if n(3) == 1 {
            dex::FLAG_PUNCH
                | dex::FLAG_PULSE
                | dex::FLAG_BITE
                | dex::FLAG_SLICING
                | dex::FLAG_CONTACT
        } else {
            0
        };
        mv.recoil = if n(8) == 1 { Some([1, 4]) } else { None };
        mv.traits = if n(9) == 1 {
            dex::MOVE_TRAIT_HASCRASHDAMAGE
        } else {
            0
        };
        if n(10) == 1 {
            mv.traits |= dex::MOVE_TRAIT_FORCESTAB;
        }
        let r = invoke(&mut b, id, c[1], numeric_args(n(6) as f64));
        let expected = if c[12] == "undefined" {
            Relay::Undefined
        } else {
            Relay::Number(c[12].parse().unwrap())
        };
        assert_eq!(r, expected, "{}", line);
        assert_eq!(b.scratch.frames[0].unwrap().modifier, n(11), "{}", line);
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
}
#[test]
fn move_changes_preserve_presence_and_nested_payloads() {
    let mut b = battle();
    let mut self_effect = effects(&dex::move_data(dex::MOVE_THUNDERBOLT).effects);
    self_effect.chance = Some(15);
    let mut secondary = SecondaryScratch {
        effects: self_effect,
        self_effect: None,
        chance: 30,
        chance_present: true,
    };
    let mv = b.scratch.moves[0].as_mut().unwrap();
    mv.secondaries_present = true;
    mv.secondary_count = 3;
    mv.secondaries[0] = Some(secondary);
    secondary.chance = 0;
    mv.secondaries[1] = Some(secondary);
    secondary.chance_present = false;
    mv.secondaries[2] = Some(secondary);
    mv.self_effect = Some(self_effect);
    invoke(
        &mut b,
        dex::ABILITY_SERENEGRACE,
        "onModifyMove",
        move_args(),
    );
    let mv = b.scratch.moves[0].unwrap();
    assert_eq!(mv.secondaries[0].unwrap().chance, 60);
    assert_eq!(mv.secondaries[1].unwrap().chance, 0);
    assert!(!mv.secondaries[2].unwrap().chance_present);
    assert_eq!(mv.self_effect.unwrap().chance, Some(30));
    invoke(&mut b, dex::ABILITY_SHEERFORCE, "onModifyMove", move_args());
    let mv = b.scratch.moves[0].unwrap();
    assert!(!mv.secondaries_present);
    assert_eq!(mv.secondary_count, 0);
    assert!(mv.self_effect.is_none());
    assert!(mv.runtime_flags & move_runtime::HAS_SHEER_FORCE != 0);
    invoke(
        &mut b,
        dex::ABILITY_INFILTRATOR,
        "onModifyMove",
        move_args(),
    );
    assert!(b.scratch.moves[0].unwrap().runtime_flags & move_runtime::INFILTRATES != 0);
    b.scratch.moves[0].as_mut().unwrap().flags =
        dex::FLAG_CONTACT | dex::FLAG_PROTECT | dex::FLAG_SOUND;
    invoke(&mut b, dex::ABILITY_UNSEENFIST, "onModifyMove", move_args());
    assert_eq!(
        b.scratch.moves[0].unwrap().flags,
        dex::FLAG_CONTACT | dex::FLAG_SOUND
    );
    assert_eq!(b.seed(), [1, 2, 3, 4]);
}
#[test]
fn sheer_force_boost_skips_deletion_and_empty_array_still_activates() {
    let mut b = battle();
    let mv = b.scratch.moves[0].as_mut().unwrap();
    mv.secondaries_present = true;
    mv.traits |= dex::MOVE_TRAIT_HASSHEERFORCEBOOST;
    invoke(&mut b, dex::ABILITY_SHEERFORCE, "onModifyMove", move_args());
    assert!(b.scratch.moves[0].unwrap().secondaries_present);
    b.scratch.moves[0].as_mut().unwrap().traits = 0;
    invoke(&mut b, dex::ABILITY_SHEERFORCE, "onModifyMove", move_args());
    assert!(b.scratch.moves[0].unwrap().runtime_flags & move_runtime::HAS_SHEER_FORCE != 0);
}
#[test]
fn whole_offensive_batch_has_no_pending_hooks() {
    for id in [
        dex::ABILITY_ADAPTABILITY,
        dex::ABILITY_BLAZE,
        dex::ABILITY_DRAGONSMAW,
        dex::ABILITY_GUTS,
        dex::ABILITY_HEAVYMETAL,
        dex::ABILITY_HUGEPOWER,
        dex::ABILITY_INFILTRATOR,
        dex::ABILITY_IRONFIST,
        dex::ABILITY_MEGALAUNCHER,
        dex::ABILITY_OVERGROW,
        dex::ABILITY_PUREPOWER,
        dex::ABILITY_RECKLESS,
        dex::ABILITY_ROCKYPAYLOAD,
        dex::ABILITY_SERENEGRACE,
        dex::ABILITY_SHARPNESS,
        dex::ABILITY_SHEERFORCE,
        dex::ABILITY_STAKEOUT,
        dex::ABILITY_STRONGJAW,
        dex::ABILITY_SWARM,
        dex::ABILITY_TECHNICIAN,
        dex::ABILITY_TORRENT,
        dex::ABILITY_TOUGHCLAWS,
        dex::ABILITY_TOXICBOOST,
        dex::ABILITY_TRANSISTOR,
        dex::ABILITY_UNSEENFIST,
    ] {
        let manifest = &dex::MANIFESTS[id.0 as usize];
        for (i, h) in manifest.hooks().iter().enumerate() {
            if matches!(h.value, dex::HookValue::Function) {
                assert_eq!(
                    registry::hook_coverage(dex::HookId(manifest.hooks_start + i as u16)),
                    crate::effects::HookCoverage::Implemented
                );
            }
        }
    }
}
