//! Accuracy and declarative effect draw order recorded from pinned Showdown.
use super::{super::support::*, fixture::*};
use crate::{
    actions::{HitTarget, MoveInput, Targets},
    event::Relay,
    prng::Prng,
    state::scratch::move_runtime as rt,
};

#[test]
fn secondary_sleep_defaults_to_the_anonymous_hit_object() {
    use crate::{
        actions::{HitEffect, HitOptions, TargetResults},
        dex,
        ids::MonId,
        log::TextLog,
    };
    let mon = "Mew|||earlybird|irondefense|Serious||N|||100|,,,,,Normal";
    let team = format!("{mon}]{mon}");
    let mut b =
        crate::Battle::from_players([1, 2, 3, 4], ("A", &team), ("B", &team), TextLog::default())
            .unwrap();
    b.start().unwrap();
    let mut log = Vec::new();
    b.drain_log(&mut log);
    log.clear();
    let h = b.get_active_move(MoveInput::Dex(dex::MOVE_DIRECLAW));
    let list = b.stash_secondaries(b.active_move(h).secondaries);
    b.state.prng = Prng::from_seed([0, 0, 0, 2]); // sample chooses sleep, then its duration
    b.run_move_effects(
        TargetResults {
            values: [Relay::Undefined; 4],
            len: 1,
        },
        Targets::single(HitTarget::Pokemon(MonId(6))),
        MonId(0),
        h,
        HitEffect::Secondary(secondary_code(list, 0)),
        HitOptions {
            secondary: true,
            self_hit: false,
        },
    );
    b.drain_log(&mut log);
    assert_eq!(log, ["|-status|p2a: Mew|slp"]);
    assert_eq!(b.seed(), [21985, 47036, 24629, 8292]);
    b.release_relay(Relay::Secondaries(list));
}

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
