//! Accuracy and declarative effect draw order recorded from pinned Showdown.
use super::{super::support::*, fixture::*};
use crate::{
    actions::{HitTarget, MoveInput, Targets},
    event::Relay,
    prng::Prng,
    state::scratch::move_runtime as rt,
};

fn boosts_json(b: &crate::Battle, mon: crate::ids::MonId) -> String {
    let v = b.state.pokemon[mon.0 as usize].boosts;
    format!(
        "{{\"atk\":{},\"def\":{},\"spa\":{},\"spd\":{},\"spe\":{},\"accuracy\":{},\"evasion\":{}}}",
        v[0], v[1], v[2], v[3], v[4], v[5], v[6]
    )
}

#[test]
fn hit_stage_results_and_seeds_match_pinned_showdown() {
    let mut count = 0;
    for line in include_str!("hit-stage-vectors.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line.split('\t').collect();
        let mut b = battle_with([0; 4]);
        let seed: Vec<u16> = c[1].split(',').map(|s| s.parse().unwrap()).collect();
        b.state.prng = Prng::from_seed(seed.try_into().unwrap());
        let h = b.get_active_move(MoveInput::Dex(move_id(c[2])));
        let result = match c[0] {
            "A" => {
                b.state.pokemon[0].boosts[5] = c[3].parse().unwrap();
                for mon in [6, 7] {
                    b.state.pokemon[mon].boosts[6] = c[4].parse().unwrap();
                }
                let mut targets = Targets::default();
                for index in c[5].split(',') {
                    targets
                        .push_target(HitTarget::Pokemon(ACTIVES[index.parse::<usize>().unwrap()]));
                }
                let results = b.hit_step_accuracy(targets, ACTIVES[0], h).unwrap();
                let values: Vec<_> = (0..results.count())
                    .map(|i| match results.at(i) {
                        Relay::Bool(v) => u8::from(v).to_string(),
                        r => panic!("unexpected accuracy {r:?}"),
                    })
                    .collect();
                format!(
                    "{}/{}",
                    values.join(","),
                    u8::from(b.input_smart_target(MoveInput::Active(h)))
                )
            }
            "E" => {
                let target = match c[5] {
                    "pokemon" => HitTarget::Pokemon(ACTIVES[2]),
                    "null" => HitTarget::Null,
                    "false" => HitTarget::False,
                    _ => unreachable!(),
                };
                b.set_active_move(Some(h), Some(ACTIVES[0]), Some(ACTIVES[2]));
                b.self_drops(
                    Targets::single(target),
                    ACTIVES[0],
                    h,
                    crate::actions::HitEffect::Primary,
                    false,
                );
                b.secondaries(
                    Targets::single(target),
                    ACTIVES[0],
                    h,
                    crate::actions::HitEffect::Primary,
                    false,
                );
                format!(
                    "[{},{},{}]",
                    boosts_json(&b, ACTIVES[0]),
                    boosts_json(&b, ACTIVES[2]),
                    b.active_move(h).runtime_flags & rt::SELF_DROPPED != 0
                )
            }
            _ => unreachable!(),
        };
        assert_eq!(result, c[6], "{line}");
        assert_eq!(seed_string(&b), c[7], "{line}");
        count += 1;
    }
    assert_eq!(count, 654);
}
