//! Full outside caller oracle parity on declarative moves and recursive Dancer.
use super::fixture::ACTIVES;
use crate::{
    Battle,
    actions::{MoveInput, RunMoveOptions},
    dex,
    event::EffectRef,
    ids::EffectId,
    log::TextLog,
    state::{ResultFlag, mon_flags},
};
fn effect_key(id: EffectId) -> &'static str {
    if id == EffectId::NONE {
        "-"
    } else {
        dex::effect(id).key
    }
}
#[test]
fn outside_execution_state_seed_and_logs_match_pinned_showdown() {
    let mut rows = 0;
    for line in include_str!("execution-vectors.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<_> = line.split('\t').collect();
        let seed: Vec<u16> = c[0].split(',').map(|s| s.parse().unwrap()).collect();
        let team = format!(
            "Pikachu|||dancer|{}|Serious||M|||100|,,,,,Electric]Raichu|||dancer|{}|Serious||M|||100|,,,,,Electric",
            c[1], c[1]
        );
        let mut b =
            Battle::with_log(seed.try_into().unwrap(), &team, &team, TextLog::default()).unwrap();
        for (i, &mon) in ACTIVES.iter().enumerate() {
            let p = &mut b.state.pokemon[mon.0 as usize];
            p.flags |= mon_flags::ACTIVE;
            p.flags &= !mon_flags::NEWLY_SWITCHED;
            p.position = (i % 2) as u8;
        }
        b.state.sides[0].active = [ACTIVES[0], ACTIVES[1]];
        b.state.sides[1].active = [ACTIVES[2], ACTIVES[3]];
        b.run_move(
            MoveInput::Dex(super::fixture::move_id(c[1])),
            ACTIVES[0],
            Some(1),
            EffectRef::None,
            RunMoveOptions::default(),
        );
        let states: Vec<_> = ACTIVES
            .iter()
            .map(|m| {
                let p = &b.state.pokemon[m.0 as usize];
                let boosts = p
                    .boosts
                    .iter()
                    .map(i8::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let result = match p.move_this_turn_result {
                    ResultFlag::Undefined => "undefined",
                    ResultFlag::Null => "null",
                    ResultFlag::False => "false",
                    ResultFlag::True => "true",
                };
                let status = match p.status {
                    crate::state::Status::None => "-",
                    crate::state::Status::Burn => "brn",
                    crate::state::Status::Paralysis => "par",
                    crate::state::Status::Sleep => "slp",
                    crate::state::Status::Freeze => "frz",
                    crate::state::Status::Poison => "psn",
                    crate::state::Status::Toxic => "tox",
                    crate::state::Status::Fainted => "fnt",
                };
                format!(
                    "{}/{}/{}/{}/{}/{}/{}/{}/{}",
                    p.hp,
                    boosts,
                    effect_key(p.last_move),
                    p.active_move_actions,
                    p.times_attacked,
                    p.move_slots()[0].pp,
                    status,
                    result,
                    effect_key(p.switch_flag)
                )
            })
            .collect();
        assert_eq!(states.join(";"), c[2], "{} state", c[1]);
        let words = b
            .seed()
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(words, c[3], "{} seed", c[1]);
        let mut logs = Vec::new();
        b.drain_log(&mut logs);
        let logs = logs
            .iter()
            .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(format!("[{logs}]"), c[4], "{} log", c[1]);
        b.clear_active_move(false);
        assert!(b.scratch.moves.iter().all(Option::is_none));
        rows += 1;
    }
    assert_eq!(rows, 42);
}
