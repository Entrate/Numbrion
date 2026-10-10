//! Endpoint vectors exercise the pinned methods without request/move execution stubs.
use super::{
    flow_tests::flow_state,
    queue::ActionChoice,
    tests::{parse_seed, seed_words},
};
use crate::{
    Battle,
    actions::Attribution,
    event::Relay,
    ids::{MonId, SideId},
    log::TextLog,
    state::choices::ActionKind,
};

fn run(b: &mut Battle<TextLog>, op: &str) -> String {
    let parts: Vec<_> = op.split(' ').collect();
    let x = || parts[1].parse::<u8>().unwrap();
    let relay = |r: Relay| match r {
        Relay::Undefined => "-".into(),
        Relay::Bool(v) => v.to_string(),
        other => panic!("unexpected endpoint result {other:?}"),
    };
    match parts[0] {
        "wipe" => {
            let party = b.state.sides[x() as usize].party;
            let count = b.state.sides[x() as usize].pokemon_count as usize;
            for &m in &party[..count] {
                b.faint_pokemon(m, Attribution::NONE);
            }
        }
        "faint" => {
            b.faint_pokemon(MonId(x()), Attribution::NONE);
        }
        "messages" => return relay(b.faint_messages(x() != 0, parts[2] == "1", parts[3] == "1")),
        "checkfainted" => b.check_fainted(),
        "tera" => b.terastallize(MonId(x())),
        "cap" => b.state.sides[x() as usize].total_fainted = 100,
        "left" => b.state.sides[x() as usize].pokemon_left = 0,
        "tie" => return b.tie().to_string(),
        "lose" => return b.lose(SideId(x())).to_string(),
        "fraction" => {
            let fraction: f64 = parts[2].parse().unwrap();
            let party = b.state.sides[x() as usize].party;
            let count = b.state.sides[x() as usize].pokemon_count as usize;
            for &m in &party[..count] {
                let p = &mut b.state.pokemon[m.0 as usize];
                p.hp = (f64::from(p.max_hp) * fraction).floor() as u16;
            }
        }
        "tiebreak" => return b.tiebreak().to_string(),
        "limit" => {
            b.state.turn = parts[1].parse().unwrap();
            return b.check_turn_limit().to_string();
        }
        other => panic!("unknown endpoint op {other}"),
    }
    "-".into()
}

#[test]
fn termination_tera_and_turn_limit_match_pinned_showdown() {
    let mut rows = 0;
    for line in include_str!("vectors/boundary.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        let [
            case,
            seed,
            p1,
            p2,
            ops,
            result,
            seed_after,
            state,
            log,
            outcome,
        ] = columns[..]
        else {
            panic!("malformed vector")
        };
        let mut b =
            Battle::from_players(parse_seed(seed), ("A", p1), ("B", p2), TextLog::default())
                .unwrap();
        b.state.started = true;
        b.queue_add_choice(ActionChoice::new(ActionKind::Start));
        b.state.mid_turn = true;
        while let Some(action) = b.queue_shift() {
            b.run_action(action);
        }
        let mut discarded = vec![];
        b.drain_log(&mut discarded);
        let got = ops
            .split(';')
            .map(|op| run(&mut b, op))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(got, result, "case {case} ({ops}): results");
        assert_eq!(seed_words(&b), seed_after, "case {case}: seed");
        assert_eq!(flow_state(&b), state, "case {case}: state");
        let mut entries = vec![];
        b.drain_log(&mut entries);
        assert_eq!(entries.join("~"), log, "case {case}: exact protocol");
        let actual_outcome = b.outcome().map_or("-".into(), |o| {
            format!(
                "{}:{}:{},{}",
                o.winner.map_or("-".into(), |w| w.to_string()),
                o.turns,
                o.pokemon_left[0],
                o.pokemon_left[1]
            )
        });
        assert_eq!(actual_outcome, outcome, "case {case}: outcome");
        rows += 1;
    }
    assert_eq!(rows, 290);
}

/// Full startup uses real rule dispatch, endTurn and C request caches. Unlike the
/// flow/queue suites this test has no request recorder or interrupted turn loop.
#[test]
#[ignore = "integration gate: six rule Begin hooks must be registered"]
fn full_startup_matches_pinned_showdown() {
    let mut rows = 0;
    for line in include_str!("vectors/startup.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        let [
            case,
            seed,
            p1,
            p2,
            seed_after,
            state,
            log,
            request1,
            request2,
        ] = columns[..]
        else {
            panic!("malformed startup vector")
        };
        let mut b =
            Battle::from_players(parse_seed(seed), ("A", p1), ("B", p2), TextLog::default())
                .unwrap();
        b.start().unwrap();
        assert_eq!(seed_words(&b), seed_after, "case {case}: startup seed");
        assert_eq!(flow_state(&b), state, "case {case}: startup state");
        let mut entries = vec![];
        b.drain_log(&mut entries);
        assert_eq!(entries.join("~"), log, "case {case}: startup protocol");
        assert_eq!(
            b.request_json(0).as_deref(),
            Some(request1),
            "case {case}: p1 request bytes"
        );
        assert_eq!(
            b.request_json(1).as_deref(),
            Some(request2),
            "case {case}: p2 request bytes"
        );
        assert!(b.outcome().is_none());
        rows += 1;
    }
    assert_eq!(rows, 50);
}
