#[allow(dead_code)]
#[path = "../../abilities/passiveoffense/support.rs"]
mod support;
use crate::{
    dex,
    effects::registry,
    event::*,
    ids::*,
    state::scratch::{OrderedBoosts, SecondaryScratch},
};
use support::*;
#[test]
fn modifiers_and_take_item_guards_match_oracle() {
    let mut seen = std::collections::BTreeSet::new();
    for line in include_str!("vectors.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line.split('\t').collect();
        let mut b = battle();
        let id = dex::lookup(EffectKind::Item, c[0]).unwrap();
        seen.insert((id.0, c[1]));
        b.state.pokemon[0].base_species = dex::lookup(EffectKind::Species, c[2]).unwrap();
        let source = if c[3] == "none" {
            EventArg::Undefined
        } else {
            b.state.pokemon[6].base_species = dex::lookup(EffectKind::Species, c[3]).unwrap();
            EventArg::Holder(Holder::mon(MonId(6)))
        };
        b.scratch.moves[0].as_mut().unwrap().move_type = dex::type_id(c[4]).unwrap();
        b.scratch.frames[0].as_mut().unwrap().modifier = c[5].parse().unwrap();
        let mut args = numeric_args(40.0);
        if c[6] == "0" {
            args[0] = EventArg::Bool(true);
        }
        if c[1] == "onTakeItem" {
            args[0] = EventArg::Effect(EffectRef::Dex(id));
            args[2] = source;
        }
        let r = invoke(&mut b, id, c[1], args);
        let expected = match c[8] {
            "undefined" => Relay::Undefined,
            "true" => Relay::Bool(true),
            "false" => Relay::Bool(false),
            _ => panic!("unexpected oracle value"),
        };
        assert_eq!(r, expected, "{line}");
        assert_eq!(
            b.scratch.frames[0].unwrap().modifier,
            c[7].parse::<u32>().unwrap(),
            "{line}"
        );
        assert_eq!(b.seed(), [1, 2, 3, 4]);
    }
    assert_eq!(seen.len(), 63);
}
#[test]
fn clear_amulet_preserves_order_presence_and_self_drops() {
    let mut b = battle();
    let object = OrderedBoosts {
        values: [-1, 2, 0, 0, 0, 0, -2],
        order: [6, 1, 0, 0, 0, 0, 0],
        len: 3,
        present: 0b1000011,
    };
    let i = b.stash_boosts(object);
    let args = [
        EventArg::Relay(Relay::Boosts(i)),
        EventArg::Holder(Holder::mon(MonId(0))),
        EventArg::Holder(Holder::mon(MonId(6))),
        EventArg::Effect(EffectRef::ActiveMove(0)),
    ];
    let before = b.scratch.unsent_lines;
    invoke(&mut b, dex::ITEM_CLEARAMULET, "onTryBoost", args);
    let changed = *b.scratch_boosts(i);
    assert_eq!(changed.present, 2);
    assert_eq!(changed.len, 1);
    assert_eq!(changed.order[0], 1);
    assert_eq!(changed.values[1], 2);
    assert_eq!(b.scratch.unsent_lines, before + 1);
    *b.scratch_boosts(i) = object;
    b.scratch.moves[0].as_mut().unwrap().secondaries_present = true;
    invoke(&mut b, dex::ITEM_CLEARAMULET, "onTryBoost", args);
    assert_eq!(b.scratch.unsent_lines, before + 1);
    *b.scratch_boosts(i) = object;
    let mut own = args;
    own[2] = own[1];
    invoke(&mut b, dex::ITEM_CLEARAMULET, "onTryBoost", own);
    assert_eq!(b.scratch_boosts(i).present, object.present);
}
#[test]
fn covert_cloak_filters_without_mutating_original_array() {
    let mut b = battle();
    let payload = effects(&dex::move_data(dex::MOVE_THUNDERBOLT).effects);
    let a = SecondaryScratch {
        effects: payload,
        self_effect: None,
        chance: 10,
        chance_present: true,
    };
    let mut own = a;
    own.self_effect = Some(payload);
    let i = b.stash_secondaries([Some(a), Some(own), Some(a), Some(own)]);
    let out = invoke(
        &mut b,
        dex::ITEM_COVERTCLOAK,
        "onModifySecondaries",
        [
            EventArg::Relay(Relay::Secondaries(i)),
            EventArg::Undefined,
            EventArg::Undefined,
            EventArg::Undefined,
        ],
    );
    let Relay::Secondaries(j) = out else {
        panic!("array required")
    };
    assert_ne!(i, j);
    assert!(b.scratch_secondaries(i)[0].unwrap().self_effect.is_none());
    let filtered = *b.scratch_secondaries(j);
    assert!(filtered[0].unwrap().self_effect.is_some());
    assert!(filtered[1].unwrap().self_effect.is_some());
    assert!(filtered[2].is_none());
    b.release_relay(out);
    assert!(b.scratch.secondaries_used & (1 << i) != 0);
}
#[test]
fn all_item_modifier_hooks_registered() {
    let mut count = 0;
    for name in [
        "adamantcrystal",
        "assaultvest",
        "choiceband",
        "choicescarf",
        "choicespecs",
        "clearamulet",
        "cornerstonemask",
        "covertcloak",
        "dracoplate",
        "dreadplate",
        "earthplate",
        "eviolite",
        "fistplate",
        "flameplate",
        "griseouscore",
        "hearthflamemask",
        "icicleplate",
        "insectplate",
        "ironplate",
        "lifeorb",
        "lightball",
        "lustrousglobe",
        "lustrousorb",
        "magnet",
        "meadowplate",
        "mindplate",
        "mysticwater",
        "pixieplate",
        "rustedshield",
        "rustedsword",
        "silkscarf",
        "skyplate",
        "souldew",
        "splashplate",
        "spookyplate",
        "stoneplate",
        "toxicplate",
        "wellspringmask",
        "widelens",
        "zapplate",
    ] {
        let id = dex::lookup(EffectKind::Item, name).unwrap();
        let manifest = &dex::MANIFESTS[id.0 as usize];
        for (i, h) in manifest.hooks().iter().enumerate() {
            if matches!(h.value, dex::HookValue::Function) {
                count += 1;
                assert_eq!(
                    registry::hook_coverage(dex::HookId(manifest.hooks_start + i as u16)),
                    crate::effects::HookCoverage::Implemented
                );
            }
        }
    }
    assert_eq!(count, 73);
}
