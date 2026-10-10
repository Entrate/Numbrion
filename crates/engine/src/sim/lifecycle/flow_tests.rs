//! Flow vectors (tools/probes/lifecycle/flow-vectors.mjs). They exercise switch/instaswitch,
//! terastallize, forced replacement requests, drags, Revival Blessing, residual, endTurn and
//! the win checks against the pinned Showdown, so they need owner C (make_request /
//! clear_request / all_choices_done) to be present; until then the test is ignored.

use super::queue::ActionChoice;
use super::tests::{
    battle_with_start_queued, drain_queue, parse_seed, queue_summary, seed_words, state_summary,
};
use crate::{
    Battle,
    actions::Attribution,
    dex,
    ids::{MonId, TypeId},
    log::NoLog,
    state::{
        Status,
        choices::{ActionKind, ActionQueue, RequestKind},
        mon_flags,
    },
};

/// `pokemon.faint()` with the default null source and effect (pokemon.ts:1581-1593).
pub(super) fn faint_mon(b: &mut Battle<NoLog>, m: MonId) {
    b.faint_pokemon(m, Attribution::NONE);
}

/// `battle.turnLoop()` minus the final endTurn.
fn loop_no_end(b: &mut Battle<NoLog>) -> &'static str {
    if b.state.request_state != RequestKind::None {
        b.state.request_state = RequestKind::None;
    }
    if !b.state.mid_turn {
        b.queue_insert_choice(ActionChoice::new(ActionKind::BeforeTurn), false);
        b.queue_add_choice(ActionChoice::new(ActionKind::Residual));
        b.state.mid_turn = true;
    }
    while let Some(action) = b.queue_shift() {
        b.run_action(action);
        if b.state.request_state != RequestKind::None || b.state.ended {
            return "stop";
        }
    }
    "empty"
}

pub(super) fn flow_state(b: &Battle<NoLog>) -> String {
    let s = state_summary(b);
    let rs = match b.state.request_state {
        RequestKind::None => "-",
        RequestKind::Wait => "wait",
        RequestKind::Move => "move",
        RequestKind::Switch => "switch",
    };
    let winner = match b.state.winner.0 {
        0 => "A",
        1 => "B",
        _ => "-",
    };
    let pair = |f: &dyn Fn(usize) -> String| format!("{},{}", f(0), f(1));
    let mut mons = vec![];
    for side in &b.state.sides {
        let mut ids: Vec<MonId> = side.party[..side.pokemon_count as usize].to_vec();
        ids.sort_by_key(|m| m.0);
        for m in ids {
            let p = &b.state.pokemon[m.0 as usize];
            let tera = if p.terastallized == TypeId::NONE {
                "0".to_string()
            } else {
                dex::TYPE_NAMES[p.terastallized.0 as usize - 1].to_string()
            };
            let flag = |f: u32| u8::from(p.flags & f != 0);
            mons.push(format!(
                "{}.{}.{}.{}.{}.{}.{}.{}.{}.{}.{}.{}",
                m.0,
                p.hp,
                flag(mon_flags::FAINTED),
                flag(mon_flags::FAINT_QUEUED),
                flag(mon_flags::ACTIVE),
                u8::from(b.lc_switch_flag(m)),
                flag(mon_flags::FORCE_SWITCH),
                u8::from(p.status == Status::Fainted),
                p.position,
                tera,
                u8::from(b.can_terastallize(m) != TypeId::NONE),
                flag(mon_flags::BEING_CALLED_BACK),
            ));
        }
    }
    format!(
        "rs={rs} ended={} winner={winner} turn={} mid={} left={} fl={} active={} party={} speeds={} orders={} so={} mons={}",
        u8::from(b.state.ended),
        b.state.turn,
        u8::from(b.state.mid_turn),
        pair(&|i| b.state.sides[i].pokemon_left.to_string()),
        pair(&|i| b.state.sides[i].total_fainted.to_string()),
        s.active,
        s.party,
        s.speeds,
        s.orders,
        s.speed_order,
        mons.join(" "),
    )
}

fn run_flow_op(b: &mut Battle<NoLog>, held: &mut ActionQueue, op: &str) -> String {
    let parts: Vec<&str> = op.split(' ').collect();
    let mon = |i: usize| MonId(parts[i].parse::<u8>().unwrap());
    let choice = |kind: ActionKind, pokemon: MonId, target: MonId| {
        let mut c = ActionChoice::for_pokemon(kind, pokemon);
        c.target = target;
        c
    };
    match parts[0] {
        "faint" => faint_mon(b, mon(1)),
        "force" => b.lc_set_flag(mon(1), mon_flags::FORCE_SWITCH, true),
        "swflag" => b.lc_set_switch_flag_true(mon(1)),
        "addsw" => b.queue_add_choice(choice(ActionKind::Switch, mon(1), mon(2))),
        "addinsw" => b.queue_add_choice(choice(ActionKind::InstaSwitch, mon(1), mon(2))),
        "addtera" => b.queue_add_choice(choice(ActionKind::Terastallize, mon(1), MonId::NONE)),
        "addrev" => b.queue_add_choice(choice(ActionKind::RevivalBlessing, mon(1), mon(2))),
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
        "speed" => b.state.pokemon[mon(1).0 as usize].stored_stats[4] = parts[2].parse().unwrap(),
        "loop" => return loop_no_end(b).to_string(),
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
#[ignore = "needs C (make_request/clear_request/all_choices_done)"]
fn lifecycle_flows_match_pinned_showdown() {
    let mut rows = 0;
    let mut current_case = String::new();
    let mut b: Option<Battle<NoLog>> = None;
    let mut held = ActionQueue::default();
    for line in include_str!("vectors/flow.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<&str> = line.split('\t').collect();
        let [
            case,
            seed,
            p1,
            p2,
            step,
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
            // Showdown's start() leaves midTurn = true; the probe records from the first endTurn.
            drain_queue(&mut nb);
            held.len = 0;
            b = Some(nb);
        }
        let b = b.as_mut().unwrap();
        let ctx = format!("case {case} step {step} ({op})");
        let got = run_flow_op(b, &mut held, op);
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
    assert_eq!(rows, 8936);
}
