//! Move actions in the queue (tools/probes/lifecycle/move-vectors.mjs): resolveAction's
//! expansion (terastallize + move, Struggle, Recharge), FractionalPriority, random targets,
//! getActionSpeed, the commit sort, the gen-8 dynamic re-sort after every action and
//! mid-turn faints. Move *execution* (owner M's run_move) is replaced by "nothing happens
//! (optionally someone faints during the move)" on both sides, so only queue behaviour is
//! compared. Needs owner M (get_active_move, get_target, get_random_target, ...) and owner C
//! (make_request / clear_request / all_choices_done).

use super::flow_tests::{faint_mon, flow_state};
use super::queue::ActionChoice;
use super::tests::{
    battle_with_start_queued, drain_queue, kind_name, parse_seed, queue_summary, seed_words,
};
use super::turn::ResidualSnapshot;
use crate::{
    Battle, dex,
    ids::{EffectId, EffectKind, MonId},
    log::NoLog,
    state::{
        choices::{ActionKind, ActionQueue, ChosenMoveKind, RequestKind},
        mon_flags,
    },
};

/// `runAction` with BattleActions.runMove replaced by a no-op that may faint `faint`.
fn step(b: &mut Battle<NoLog>, faint: Option<MonId>) -> String {
    let Some(action) = b.queue_shift() else {
        return "none".into();
    };
    let who = if action.pokemon == MonId::NONE {
        String::new()
    } else {
        format!(":{}", action.pokemon.0)
    };
    if action.kind == ActionKind::Move {
        // runAction: early returns for inactive / fainted users skip all post-processing.
        if b.lc_mon(action.pokemon).flags & mon_flags::ACTIVE != 0
            && b.lc_mon(action.pokemon).flags & mon_flags::FAINTED == 0
        {
            if let Some(f) = faint {
                faint_mon(b, f);
            }
            let hp = b.lc_mon(action.pokemon).hp;
            b.process_action_boundary(action, Some(hp), ResidualSnapshot::default());
        }
    } else {
        b.run_action(action);
    }
    format!("{}{who}", kind_name(action.kind))
}

fn run_move_op(b: &mut Battle<NoLog>, held: &mut ActionQueue, op: &str) -> String {
    let parts: Vec<&str> = op.split(' ').collect();
    let mon = |i: usize| MonId(parts[i].parse::<u8>().unwrap());
    let choice = |kind: ActionKind, pokemon: MonId, target: MonId| {
        let mut c = ActionChoice::for_pokemon(kind, pokemon);
        c.target = target;
        c
    };
    match parts[0] {
        "faint" => faint_mon(b, mon(1)),
        "speed" => b.state.pokemon[mon(1).0 as usize].stored_stats[4] = parts[2].parse().unwrap(),
        "updatespeed" => b.update_speed(),
        "stash" => {
            *held = b.state.queue;
            b.queue_clear();
        }
        "commitsort" => {
            b.queue_sort();
            let new_len = b.state.queue.len as usize;
            let old_len = held.len as usize;
            b.state.queue.entries[new_len..new_len + old_len]
                .copy_from_slice(&held.entries[..old_len]);
            b.state.queue.len = (new_len + old_len) as u16;
            held.len = 0;
        }
        "clearrequest" => b.state.request_state = RequestKind::None,
        "prelude" => {
            if !b.state.mid_turn {
                b.queue_insert_choice(ActionChoice::new(ActionKind::BeforeTurn), false);
                b.queue_add_choice(ActionChoice::new(ActionKind::Residual));
                b.state.mid_turn = true;
            }
        }
        "addmove" => {
            let id = dex::lookup(EffectKind::Move, parts[2]).expect("move");
            let mut c = ActionChoice::for_move(mon(1), id, Some(parts[3].parse().unwrap()));
            c.terastallize = parts.get(4) == Some(&"tera");
            b.queue_add_choice(c);
        }
        "addstruggle" => {
            let c = ActionChoice::for_move(mon(1), dex::MOVE_STRUGGLE, None);
            b.queue_add_choice(c);
        }
        "addrecharge" => {
            let mut c =
                ActionChoice::for_move(mon(1), EffectId::NONE, Some(parts[2].parse().unwrap()));
            c.move_kind = ChosenMoveKind::Recharge;
            b.queue_add_choice(c);
        }
        "addsw" => b.queue_add_choice(choice(ActionKind::Switch, mon(1), mon(2))),
        "addinsw" => b.queue_add_choice(choice(ActionKind::InstaSwitch, mon(1), mon(2))),
        "step" => return step(b, None),
        "stepfaint" => return step(b, Some(mon(1))),
        "endturn" => {
            b.end_turn();
            b.state.mid_turn = false;
            b.queue_clear();
        }
        other => panic!("unknown op {other}"),
    }
    "-".into()
}

#[test]
#[ignore = "needs M (get_active_move/get_target/get_random_target/get_loc_of/get_at_loc) and C (make_request/clear_request/all_choices_done)"]
fn move_queue_matches_pinned_showdown() {
    let mut rows = 0;
    let mut current_case = String::new();
    let mut b: Option<Battle<NoLog>> = None;
    let mut held = ActionQueue::default();
    for line in include_str!("vectors/move.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<&str> = line.split('\t').collect();
        let [
            case,
            seed,
            p1,
            p2,
            step_no,
            op,
            result,
            seed_after,
            queue,
            effect_order,
            state,
        ] = c[..]
        else {
            panic!("malformed row {line}");
        };
        if case != current_case {
            current_case = case.to_string();
            let mut nb = battle_with_start_queued(parse_seed(seed), p1, p2);
            drain_queue(&mut nb);
            held.len = 0;
            b = Some(nb);
        }
        let b = b.as_mut().unwrap();
        let ctx = format!("case {case} step {step_no} ({op})");
        let got = run_move_op(b, &mut held, op);
        assert_eq!(got, result, "{ctx}: result");
        assert_eq!(seed_words(b), seed_after, "{ctx}: PRNG state");
        assert_eq!(queue_summary(b), queue, "{ctx}: queue");
        assert_eq!(
            b.state.effect_order.to_string(),
            effect_order,
            "{ctx}: effectOrder"
        );
        assert_eq!(flow_state(b), state, "{ctx}: state");
        rows += 1;
    }
    assert_eq!(rows, 11710);
}
