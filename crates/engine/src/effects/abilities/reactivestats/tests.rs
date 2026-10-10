//! Unit tests for the reactive_stats helpers (no core needed).
use super::*;

#[test]
fn type_constants_match_the_generated_type_table() {
    for (name, id) in [
        ("Bug", TYPE_BUG),
        ("Dark", TYPE_DARK),
        ("Fire", TYPE_FIRE),
        ("Ghost", TYPE_GHOST),
        ("Water", TYPE_WATER),
    ] {
        assert_eq!(Some(id), dex::type_id(name), "{name}");
    }
}

#[test]
fn boost_objects_keep_insertion_order_and_presence() {
    // Weak Armor: this.boost({ def: -1, spe: 2 }) applies def first, then spe.
    let b = boosts([(Stat::Def, -1), (Stat::Spe, 2)]);
    assert_eq!(b.len, 2);
    assert_eq!(&b.order[..2], &[1, 4]);
    assert_eq!(b.present, 0b1_0010);
    assert_eq!((b.values[1], b.values[4]), (-1, 2));
    // Same keys in the other order are a different object.
    let b = boosts([(Stat::Spe, 2), (Stat::Def, -1)]);
    assert_eq!(&b.order[..2], &[4, 1]);
    assert_eq!(boost_get(&b, Stat::Atk), None);
    assert_eq!(boost_get(&b, Stat::Spe), Some(2));
}

#[test]
fn lowered_detection_ignores_absent_keys() {
    assert!(any_lowered(&boosts([(Stat::Atk, -1)])));
    assert!(any_lowered(&boosts([(Stat::Atk, 1), (Stat::Def, -2)])));
    assert!(!any_lowered(&boosts([(Stat::Atk, 2)])));
    assert!(!any_lowered(&OrderedBoosts::default()));
}
