//! Data/shape tests for the hazards_screens batch that need no battle core.
use super::*;

#[test]
fn type_ids_match_dex() {
    for (id, name) in [
        (TYPE_FIGHTING, "Fighting"),
        (TYPE_POISON, "Poison"),
        (TYPE_STEEL, "Steel"),
        (TYPE_FIRE, "Fire"),
        (TYPE_WATER, "Water"),
    ] {
        assert_eq!(dex::TYPE_NAMES[id.0 as usize - 1], name);
    }
}
