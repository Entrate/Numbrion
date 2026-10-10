//! These use real registry dispatch and the live engine adapter, not the recorder.
mod support;
use super::{HOOKS, ID, VECTORS};
use crate::{dex, event::*, ids::*, state::mon_flags};
use support::*;
fn captured(b: &mut crate::Battle<crate::log::NoLog>, owner: MonId) {
    let cell = b
        .state
        .effects
        .alloc(Holder::mon(owner), Holder::mon(owner), ID, 0);
    b.scratch.current_state = Some(b.state.effects.pin(cell));
}
#[test]
fn ruin_numeric_registry_and_holder_overlay_match_oracle() {
    let name = dex::effect(ID).key;
    let slot = match name {
        "beadsofruin" => 3,
        "swordofruin" => 1,
        "tabletsofruin" => 0,
        "vesselofruin" => 2,
        _ => return,
    };
    let mut count = 0;
    for line in VECTORS.lines().filter(|l| {
        l.split('\t')
            .nth(1)
            .is_some_and(|h| h.starts_with("onAnyModify"))
    }) {
        let c: Vec<_> = line.split('\t').collect();
        let n = |i: usize| c[3 + i].parse::<i32>().unwrap();
        for initial in [4096, 6144, 1] {
            let mut b = battle();
            for p in &mut b.state.pokemon {
                p.flags = mon_flags::ACTIVE;
                p.ability = dex::ABILITY_STATIC;
            }
            let old = n(17);
            b.state.pokemon[0].ability = if n(16) != 0 { ID } else { dex::ABILITY_STATIC };
            if n(18) != 0 {
                for p in &mut b.state.pokemon[1..] {
                    p.ability = ID;
                }
            }
            let owner = MonId(n(1) as u8);
            captured(&mut b, owner);
            b.scratch.frames[0].as_mut().unwrap().modifier = initial;
            b.scratch.moves[0].as_mut().unwrap().ruined_stats[slot] = if old < 0 {
                MonId::NONE
            } else {
                MonId(old as u8)
            };
            let expected = c[37].split('|').any(|t| t == "chain:0.75");
            let result = invoke(&mut b, ID, c[1], numeric_args(100.0));
            assert_eq!(result, Relay::Undefined, "{line}");
            let modifier = if expected {
                ((initial as f64 * 0.75) + 0.5).floor() as u32
            } else {
                initial
            };
            assert_eq!(b.scratch.frames[0].unwrap().modifier, modifier, "{line}");
            assert_eq!(
                b.scratch.moves[0].unwrap().ruined_stats[slot].0,
                if c[43] == "-1" {
                    255
                } else {
                    c[43].parse().unwrap()
                },
                "{line}"
            );
            assert_eq!(b.seed(), [1, 2, 3, 4]);
            count += 1;
        }
    }
    assert_eq!(count, 1131 * 3);
    assert!(
        HOOKS
            .iter()
            .all(|h| crate::effects::registry::hook_coverage(*h)
                == crate::effects::HookCoverage::Implemented)
    );
}
