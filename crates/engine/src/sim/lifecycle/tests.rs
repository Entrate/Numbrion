//! Lifecycle tests. Expected values come from the pinned Showdown through the probes under
//! tools/probes/lifecycle/ (vectors/*.tsv); nothing here is derived from this port.

use super::queue::{ActionChoice, QueueHandle};
use crate::{
    Battle,
    event::EffectRef,
    ids::MonId,
    log::NoLog,
    state::choices::{Action, ActionKind},
};

fn kind_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::None => "none",
        ActionKind::Start => "start",
        ActionKind::BeforeTurn => "beforeTurn",
        ActionKind::BeforeTurnMove => "beforeTurnMove",
        ActionKind::PriorityChargeMove => "priorityChargeMove",
        ActionKind::Move => "move",
        ActionKind::Switch => "switch",
        ActionKind::InstaSwitch => "instaswitch",
        ActionKind::RevivalBlessing => "revivalblessing",
        ActionKind::RunSwitch => "runSwitch",
        ActionKind::Terastallize => "terastallize",
        ActionKind::Residual => "residual",
        ActionKind::Pass => "pass",
        ActionKind::Event => "event",
    }
}

fn mon_index(m: MonId) -> String {
    if m == MonId::NONE {
        "-".into()
    } else {
        m.0.to_string()
    }
}

fn num(n: f64) -> String {
    if n == n.trunc() {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// `choice:pokemon:order:priority:speed` joined by '/', matching tools/probes/lifecycle/common.mjs.
fn action_summary(a: &Action) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        kind_name(a.kind),
        mon_index(a.pokemon),
        a.order,
        num(a.priority),
        a.speed
    )
}

fn queue_summary(b: &Battle<NoLog>) -> String {
    let q = &b.state.queue;
    if q.len == 0 {
        return "-".into();
    }
    q.entries[..q.len as usize]
        .iter()
        .map(action_summary)
        .collect::<Vec<_>>()
        .join("/")
}

fn seed_words(b: &Battle<NoLog>) -> String {
    b.seed()
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_seed(text: &str) -> [u16; 4] {
    let v: Vec<u16> = text.split(',').map(|s| s.parse().unwrap()).collect();
    [v[0], v[1], v[2], v[3]]
}

/// The state columns of start.tsv / queue.tsv (everything after the action summary).
struct StateSummary {
    active: String,
    party: String,
    speeds: String,
    orders: String,
    speed_order: String,
}

fn state_summary(b: &Battle<NoLog>) -> StateSummary {
    let mut active = vec![];
    let mut party = vec![];
    let mut speeds = vec![];
    let mut orders = vec![];
    for side in &b.state.sides {
        for m in side.active {
            active.push(mon_index(m));
        }
        let mons = &side.party[..side.pokemon_count as usize];
        party.push(
            mons.iter()
                .map(|m| mon_index(*m))
                .collect::<Vec<_>>()
                .join(","),
        );
        for &m in mons {
            let p = &b.state.pokemon[m.0 as usize];
            speeds.push(p.speed.to_string());
            let a = b.state.effects.cells[p.ability_state.0 as usize].effect_order;
            let i = b.state.effects.cells[p.item_state.0 as usize].effect_order;
            orders.push(format!("{a}.{i}"));
        }
    }
    StateSummary {
        active: active.join(","),
        party: party.join(";"),
        speeds: speeds.join(","),
        orders: orders.join(","),
        speed_order: b
            .state
            .speed_order
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(","),
    }
}

/// Showdown's `start()` up to the point where the queue holds the `start` action. Rule
/// Begin hooks, the opening log and the flush need other owners' code and are covered by
/// the ignored `start_*` integration tests.
fn battle_with_start_queued(seed: [u16; 4], p1: &str, p2: &str) -> Battle<NoLog> {
    let mut b = Battle::new(seed, p1, p2).expect("teams");
    b.state.started = true;
    b.queue_add_choice(super::queue::ActionChoice::new(ActionKind::Start));
    b.state.mid_turn = true;
    b
}

#[test]
fn start_action_and_initial_switch_ins_match_pinned_showdown() {
    let mut rows = 0;
    let mut current_case = String::new();
    let mut b: Option<Battle<NoLog>> = None;
    for line in include_str!("vectors/start.tsv")
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
            choice,
            seed_after,
            effect_order,
            queue,
            active,
            party,
            speeds,
            orders,
            speed_order,
        ] = c[..]
        else {
            panic!("malformed row {line}");
        };
        if case != current_case {
            current_case = case.to_string();
            b = Some(battle_with_start_queued(parse_seed(seed), p1, p2));
        }
        let b = b.as_mut().unwrap();
        // turn_loop's body without endTurn (Showdown's endTurn is intercepted by the probe).
        let action = b.queue_shift().expect("queued action");
        assert_eq!(kind_name(action.kind), choice, "case {case} step {step}");
        b.run_action(action);
        let ctx = format!("case {case} step {step} ({choice})");
        assert_eq!(seed_words(b), seed_after, "{ctx}: PRNG state");
        assert_eq!(
            b.state.effect_order.to_string(),
            effect_order,
            "{ctx}: effectOrder"
        );
        assert_eq!(queue_summary(b), queue, "{ctx}: queue");
        let s = state_summary(b);
        assert_eq!(s.active, active, "{ctx}: active");
        assert_eq!(s.party, party, "{ctx}: party order");
        assert_eq!(s.speeds, speeds, "{ctx}: speeds");
        assert_eq!(s.orders, orders, "{ctx}: ability/item effectOrder");
        assert_eq!(s.speed_order, speed_order, "{ctx}: speedOrder");
        rows += 1;
    }
    assert_eq!(rows, 600);
}

/// Run every queued action to completion without endTurn (what the probes observe).
fn drain_queue(b: &mut Battle<NoLog>) {
    while let Some(action) = b.queue_shift() {
        b.run_action(action);
    }
}

fn will_summary(b: &Battle<NoLog>) -> String {
    let act = b.queue_will_act().map_or(-1, |h| i32::from(h.0));
    let sw: Vec<String> = [0u8, 1, 6, 7]
        .iter()
        .map(|&a| {
            b.queue_will_switch(MonId(a))
                .map_or(-1, |h| i32::from(h.0))
                .to_string()
        })
        .collect();
    format!("act={act} sw={}", sw.join(","))
}

fn run_queue_op(b: &mut Battle<NoLog>, op: &str) -> String {
    let parts: Vec<&str> = op.split(' ').collect();
    let mon = |i: usize| MonId(parts[i].parse::<u8>().unwrap());
    match parts[0] {
        "ibt" => {
            b.queue_insert_choice(ActionChoice::new(ActionKind::BeforeTurn), false);
            String::new()
        }
        "ares" => {
            b.queue_add_choice(ActionChoice::new(ActionKind::Residual));
            String::new()
        }
        "irs" => {
            b.queue_insert_choice(
                ActionChoice::for_pokemon(ActionKind::RunSwitch, mon(1)),
                false,
            );
            String::new()
        }
        "itera" => {
            b.queue_insert_choice(
                ActionChoice::for_pokemon(ActionKind::Terastallize, mon(1)),
                false,
            );
            String::new()
        }
        "atera" => {
            b.queue_add_choice(ActionChoice::for_pokemon(ActionKind::Terastallize, mon(1)));
            String::new()
        }
        "iis" | "isw" => {
            let mut c = ActionChoice::for_pokemon(
                if parts[0] == "iis" {
                    ActionKind::InstaSwitch
                } else {
                    ActionKind::Switch
                },
                mon(1),
            );
            c.target = mon(2);
            b.queue_insert_choice(c, false);
            String::new()
        }
        "cancel" => b.queue_cancel_action(mon(1)).to_string(),
        "sort" => {
            b.queue_sort();
            String::new()
        }
        "prio" => {
            let k = parts[1].parse::<u16>().unwrap();
            if k >= b.state.queue.len {
                return "skip".into();
            }
            let handle = QueueHandle(k as u8);
            let action = b.queue_action(handle);
            b.queue_prioritize_action(action, Some(handle), EffectRef::None);
            String::new()
        }
        "shift" => b
            .queue_shift()
            .map_or("none".to_string(), |a| kind_name(a.kind).to_string()),
        "chg" => {
            b.queue_change_action(mon(1), ActionChoice::new(ActionKind::RunSwitch));
            String::new()
        }
        "boost" => {
            b.state.pokemon[mon(1).0 as usize].boosts[4] = parts[2].parse().unwrap();
            String::new()
        }
        "stat" => {
            b.state.pokemon[mon(1).0 as usize].stored_stats[4] = parts[2].parse().unwrap();
            String::new()
        }
        other => panic!("unknown op {other}"),
    }
}

#[test]
fn queue_operations_match_pinned_showdown() {
    let mut rows = 0;
    let mut current_case = String::new();
    let mut b: Option<Battle<NoLog>> = None;
    for line in include_str!("vectors/queue.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let c: Vec<&str> = line.split('\t').collect();
        let [case, seed, p1, p2, step, op, result, seed_after, queue] = c[..] else {
            panic!("malformed row {line}");
        };
        if case != current_case {
            current_case = case.to_string();
            let mut nb = battle_with_start_queued(parse_seed(seed), p1, p2);
            drain_queue(&mut nb);
            b = Some(nb);
        }
        let b = b.as_mut().unwrap();
        let ctx = format!("case {case} step {step} ({op})");
        let mut got = run_queue_op(b, op);
        if !got.is_empty() {
            got.push(' ');
        }
        got.push_str(&will_summary(b));
        assert_eq!(got, result, "{ctx}: result");
        assert_eq!(seed_words(b), seed_after, "{ctx}: PRNG state");
        assert_eq!(queue_summary(b), queue, "{ctx}: queue");
        rows += 1;
    }
    assert_eq!(rows, 3697);
}
