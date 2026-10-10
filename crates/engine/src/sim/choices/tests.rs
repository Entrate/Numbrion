//! Vector-driven tests for the choice module. The expected values come from the
//! pinned Showdown (tools/probes/choices/gen-vectors.mjs): cached requests as
//! `JSON.stringify` text, probe `choose` results with exact error strings and
//! re-sent requests, partial choices, hidden-information state, and a brute-force
//! acceptance bit-vector with a hash of the accepted normalized choices.
//!
//! Lifecycle's `can_terastallize` is another owner's work; tests use a state-derived
//! model (`LOCAL_TERA`) and a twin `#[ignore = "needs L"]` test runs the real one.

use std::cell::Cell;
use std::collections::BTreeSet;

use super::{
    requests::{LockedMove, write_json_string},
    *,
};
use crate::{
    Battle, dex,
    ids::*,
    log::{LogEntry, LogSink, LogView, MoveLineEdit},
    state::{
        Status, Trapped, mon_flags,
        choices::{ChoiceKind, ChosenMoveKind, RequestKind, SlotChoice},
    },
};

/// Choices and requests are side updates. Any battle-log write or edit in a
/// noncommitting probe is a contract violation, even if its bytes are unchanged.
struct ForbiddenLog;
impl LogSink for ForbiddenLog {
    const ENABLED: bool = true;
    fn emit(&mut self, _: LogView<'_>, _: LogEntry<'_>) {
        panic!("choice/request emitted a battle-log entry");
    }
    fn edit_move(&mut self, _: LogView<'_>, _: MoveLineEdit<'_>) {
        panic!("choice/request edited a battle-log entry");
    }
}

thread_local! {
    /// When set, tera eligibility comes from side.tera_used / TERA_BLOCKED instead of lifecycle.
    pub(crate) static LOCAL_TERA: Cell<bool> = const { Cell::new(false) };
    /// Locked moves of the lockedmove/twoturnmove volatiles, whose LockMove callbacks
    /// belong to the effect owners and are not implemented yet. Indexed by MonId.
    static LOCK_OVERRIDE: std::cell::RefCell<[Option<LockedMove>; 12]> =
        const { std::cell::RefCell::new([None; 12]) };
}

/// Test seam used by `Battle::ch_get_locked_move` (see requests.rs).
pub(crate) fn lock_override(mon: MonId) -> Option<LockedMove> {
    LOCK_OVERRIDE.with(|o| o.borrow()[mon.0 as usize])
}

// ---------------------------------------------------------------------------------
// Minimal JSON reader (tests only; the engine has no dependencies).
// ---------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

static NULL: J = J::Null;

impl J {
    fn get(&self, key: &str) -> &J {
        match self {
            J::Obj(v) => v.iter().find(|(k, _)| k == key).map_or(&NULL, |(_, v)| v),
            _ => &NULL,
        }
    }
    fn at(&self, i: usize) -> &J {
        match self {
            J::Arr(v) => v.get(i).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
    fn arr(&self) -> &[J] {
        match self {
            J::Arr(v) => v,
            _ => &[],
        }
    }
    fn str(&self) -> &str {
        match self {
            J::Str(s) => s,
            _ => "",
        }
    }
    fn opt_str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }
    fn num(&self) -> f64 {
        match self {
            J::Num(n) => *n,
            _ => 0.0,
        }
    }
    fn int(&self) -> i64 {
        self.num() as i64
    }
    fn bool(&self) -> bool {
        matches!(self, J::Bool(true))
    }
    fn is_null(&self) -> bool {
        matches!(self, J::Null)
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> J {
        self.ws();
        match self.b[self.i] {
            b'n' => {
                self.i += 4;
                J::Null
            }
            b't' => {
                self.i += 4;
                J::Bool(true)
            }
            b'f' => {
                self.i += 5;
                J::Bool(false)
            }
            b'"' => J::Str(self.string()),
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.b[self.i] == b']' {
                        self.i += 1;
                        return J::Arr(v);
                    }
                    v.push(self.value());
                    self.ws();
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.b[self.i] == b'}' {
                        self.i += 1;
                        return J::Obj(v);
                    }
                    let k = self.string();
                    self.ws();
                    assert_eq!(self.b[self.i], b':');
                    self.i += 1;
                    let val = self.value();
                    v.push((k, val));
                    self.ws();
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    }
                }
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len()
                    && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    self.i += 1;
                }
                J::Num(std::str::from_utf8(&self.b[start..self.i]).unwrap().parse().unwrap())
            }
        }
    }
    fn string(&mut self) -> String {
        assert_eq!(self.b[self.i], b'"');
        self.i += 1;
        let mut out: Vec<u16> = Vec::new();
        let mut buf = String::new();
        let flush = |buf: &mut String, out: &mut Vec<u16>| {
            out.extend(buf.encode_utf16());
            buf.clear();
        };
        loop {
            let c = self.b[self.i];
            match c {
                b'"' => {
                    self.i += 1;
                    flush(&mut buf, &mut out);
                    return String::from_utf16(&out).unwrap();
                }
                b'\\' => {
                    self.i += 1;
                    let e = self.b[self.i];
                    self.i += 1;
                    match e {
                        b'n' => buf.push('\n'),
                        b't' => buf.push('\t'),
                        b'r' => buf.push('\r'),
                        b'b' => buf.push('\u{8}'),
                        b'f' => buf.push('\u{c}'),
                        b'u' => {
                            flush(&mut buf, &mut out);
                            let h = std::str::from_utf8(&self.b[self.i..self.i + 4]).unwrap();
                            out.push(u16::from_str_radix(h, 16).unwrap());
                            self.i += 4;
                        }
                        other => buf.push(other as char),
                    }
                }
                _ => {
                    // Copy one UTF-8 scalar.
                    let len = match c {
                        0..=0x7f => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    buf.push_str(std::str::from_utf8(&self.b[self.i..self.i + len]).unwrap());
                    self.i += len;
                }
            }
        }
    }
}

fn parse_json(text: &str) -> J {
    Parser { b: text.as_bytes(), i: 0 }.value()
}

// ---------------------------------------------------------------------------------
// Vector loading
// ---------------------------------------------------------------------------------

static EMBEDDED: &str = include_str!("vectors/cases.jsonl");

fn load_cases() -> Vec<J> {
    let text = match std::env::var("CHOICES_VECTORS") {
        Ok(path) => std::fs::read_to_string(path).expect("CHOICES_VECTORS file"),
        Err(_) => EMBEDDED.to_string(),
    };
    text.lines().filter(|l| !l.is_empty()).map(parse_json).collect()
}

// ---------------------------------------------------------------------------------
// State reconstruction
// ---------------------------------------------------------------------------------

fn effect(kind: EffectKind, id: &str) -> EffectId {
    if id.is_empty() {
        return EffectId::NONE;
    }
    dex::lookup(kind, id).unwrap_or_else(|| panic!("unknown {kind:?} {id}"))
}

fn status_of(s: &str) -> Status {
    match s {
        "" => Status::None,
        "brn" => Status::Burn,
        "par" => Status::Paralysis,
        "slp" => Status::Sleep,
        "frz" => Status::Freeze,
        "psn" => Status::Poison,
        "tox" => Status::Toxic,
        "fnt" => Status::Fainted,
        other => panic!("status {other}"),
    }
}

/// Volatiles that influence request/choice behaviour in the scoped dex.
const RELEVANT_VOLATILES: [&str; 5] = ["healblock", "mustrecharge", "taunt", "lockedmove", "twoturnmove"];

/// None when the case needs effect callbacks that are not implemented yet.
fn build_battle(case: &J) -> Option<Battle<ForbiddenLog>> {
    let seed: Vec<u16> = case.get("seed").arr().iter().map(|n| n.int() as u16).collect();
    let teams = case.get("teams").arr();
    let mut b = Battle::with_log(
        [seed[0], seed[1], seed[2], seed[3]],
        teams[0].str(),
        teams[1].str(),
        ForbiddenLog,
    )
    .expect("teams");
    LOCK_OVERRIDE.with(|o| *o.borrow_mut() = [None; 12]);
    let names = case.get("names").arr();
    b.set_names(names[0].str(), names[1].str());
    let state = case.get("state");
    for (s, sj) in state.get("sides").arr().iter().enumerate() {
        let mut any_terad = false;
        let party = sj.get("party").arr();
        for (i, pj) in party.iter().enumerate() {
            let ti = pj.get("ti").int() as usize;
            let mon = MonId((s * 6 + ti) as u8);
            b.state.sides[s].party[i] = mon;
            let p = &mut b.state.pokemon[mon.0 as usize];
            p.position = i as u8;
            p.hp = pj.get("hp").int() as u16;
            p.max_hp = pj.get("maxhp").int() as u16;
            p.status = status_of(pj.get("status").str());
            p.species = effect(EffectKind::Species, pj.get("species").str());
            p.base_species = effect(EffectKind::Species, pj.get("baseSpecies").str());
            p.ability = effect(EffectKind::Ability, pj.get("ability").str());
            p.base_ability = effect(EffectKind::Ability, pj.get("baseAbility").str());
            p.item = effect(EffectKind::Item, pj.get("item").str());
            p.terastallized = match pj.get("terastallized").str() {
                "" => TypeId::NONE,
                t => dex::type_id(t).expect("type"),
            };
            any_terad |= p.terastallized != TypeId::NONE;
            let types = pj.get("types").arr();
            p.types = [TypeId::NONE; 2];
            for (k, t) in types.iter().take(2).enumerate() {
                p.types[k] = dex::type_id(t.str()).expect("type");
            }
            let stats = pj.get("stats").arr();
            for k in 0..5 {
                p.base_stored_stats[k] = stats[k].int() as u16;
            }
            p.flags &= !(mon_flags::ACTIVE
                | mon_flags::FAINTED
                | mon_flags::MAYBE_TRAPPED
                | mon_flags::MAYBE_DISABLED
                | mon_flags::MAYBE_LOCKED);
            if pj.get("isActive").bool() {
                p.flags |= mon_flags::ACTIVE;
            }
            if pj.get("fainted").bool() {
                p.flags |= mon_flags::FAINTED;
            }
            if pj.get("maybeTrapped").bool() {
                p.flags |= mon_flags::MAYBE_TRAPPED;
            }
            if pj.get("maybeDisabled").bool() {
                p.flags |= mon_flags::MAYBE_DISABLED;
            }
            if pj.get("maybeLocked").bool() {
                p.flags |= mon_flags::MAYBE_LOCKED;
            }
            if pj.get("canTerastallize").opt_str().is_none() && p.terastallized == TypeId::NONE {
                p.flags |= mon_flags::TERA_BLOCKED;
            }
            p.trapped = match pj.get("trapped") {
                J::Bool(true) => Trapped::Yes,
                J::Str(_) => Trapped::Hidden,
                _ => Trapped::No,
            };
            p.switch_flag = match pj.get("switchFlag") {
                J::Bool(false) | J::Null => EffectId::NONE,
                J::Str(m) => effect(EffectKind::Move, m),
                _ => dex::MOVE_STRUGGLE,
            };
            p.last_move_target_loc = pj.get("lastMoveTargetLoc").int() as i8;
            let slots = pj.get("moveSlots").arr();
            assert!(!pj.get("transformed").bool());
            p.move_count = slots.len() as u8;
            for (k, mj) in slots.iter().enumerate() {
                let id = effect(EffectKind::Move, mj.at(0).str());
                p.base_move_slots[k] = crate::state::MoveSlot {
                    id,
                    disabled_source: EffectId::NONE,
                    pp: mj.at(1).int() as u8,
                    max_pp: mj.at(2).int() as u8,
                    target: dex::move_data(id).target,
                    flags: mj.at(4).int() as u8,
                };
            }
            // Volatiles.
            if let J::Obj(vols) = pj.get("volatiles") {
                for (vid, info) in vols {
                    if !RELEVANT_VOLATILES.contains(&vid.as_str()) && info.get("targetLoc").is_null() {
                        continue;
                    }
                    if vid == "lockedmove" || vid == "twoturnmove" {
                        let m = effect(EffectKind::Move, info.get("move").str());
                        LOCK_OVERRIDE.with(|o| o.borrow_mut()[mon.0 as usize] = Some(LockedMove::Dex(m)));
                        continue;
                    }
                    let id = dex::lookup(EffectKind::Condition, vid)
                        .or_else(|| dex::lookup(EffectKind::Move, vid));
                    let Some(id) = id else { continue };
                    let cell = b.state.effects.alloc(Holder::mon(mon), Holder::mon(mon), id, 0);
                    if let J::Num(loc) = info.get("targetLoc") {
                        let c = &mut b.state.effects.cells[cell.0 as usize];
                        c.payload.words[0] = *loc as i32 as u32;
                        c.present |= 1 << crate::state::present::CUSTOM_START;
                    }
                    b.state.pokemon[mon.0 as usize].volatiles.push(cell);
                }
            }
        }
        let sd = &mut b.state.sides[s];
        sd.pokemon_count = party.len() as u8;
        sd.pokemon_left = sj.get("pokemonLeft").int() as u8;
        sd.tera_used = any_terad;
        for (k, a) in sj.get("active").arr().iter().enumerate() {
            sd.active[k] = match a {
                J::Num(i) => sd.party[*i as usize],
                _ => MonId::NONE,
            };
        }
        for (pos, conds) in sj.get("slotConditions").arr().iter().enumerate() {
            for c in conds.arr() {
                let id = effect(EffectKind::Condition, c.str());
                let cell = b.state.effects.alloc(Holder::side(SideId(s as u8)), Holder::NONE, id, 0);
                b.state.sides[s].slot_conditions[pos].push(cell);
            }
        }
    }
    b.state.turn = state.get("turn").int() as u16;
    b.state.support_cancel = state.get("supportCancel").bool();
    let kind = match state.get("requestState").str() {
        "move" => RequestKind::Move,
        "switch" => RequestKind::Switch,
        other => panic!("request state {other:?}"),
    };
    b.state.request_state = kind;
    for s in 0..2 {
        b.clear_choice(SideId(s));
    }
    if !case.get("boundarySeed").is_null() {
        let words = case.get("boundarySeed").arr();
        b.state.prng = crate::prng::Prng::from_seed(core::array::from_fn(|i| words[i].int() as u16));
    }
    let seed_before = b.seed();
    let requests = b.get_requests(kind);
    assert_eq!(b.seed(), seed_before, "request rebuilding advanced PRNG");
    b.state.requests = requests;
    Some(b)
}

fn flag_string(b: &Battle<ForbiddenLog>, side: usize) -> String {
    let sd = &b.state.sides[side];
    (0..sd.pokemon_count as usize)
        .map(|i| {
            let p = &b.state.pokemon[sd.party[i].0 as usize];
            let trapped = match p.trapped {
                Trapped::No => 0,
                Trapped::Yes => 1,
                Trapped::Hidden => 2,
            };
            format!(
                "{}{}{}{}{}",
                trapped,
                u8::from(p.flags & mon_flags::MAYBE_TRAPPED != 0),
                u8::from(p.flags & mon_flags::MAYBE_DISABLED != 0),
                u8::from(p.flags & mon_flags::MAYBE_LOCKED != 0),
                u8::from(p.switch_flag != EffectId::NONE),
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------------
// Shared with gen-vectors.mjs
// ---------------------------------------------------------------------------------

fn candidate_tokens() -> Vec<String> {
    let mut tokens = vec!["pass".to_string()];
    for m in 1..=5 {
        for loc in ["", " -2", " -1", " 1", " 2", " 3"] {
            for tera in ["", " terastallize"] {
                tokens.push(format!("move {m}{loc}{tera}"));
            }
        }
    }
    for s in 1..=7 {
        tokens.push(format!("switch {s}"));
    }
    tokens
}

fn candidates() -> Vec<String> {
    let tokens = candidate_tokens();
    let mut out = tokens.clone();
    for a in &tokens {
        for b in &tokens {
            out.push(format!("{a}, {b}"));
        }
    }
    out
}

fn fnv64(text: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn set_hash(set: &BTreeSet<String>) -> String {
    let joined = set.iter().cloned().collect::<Vec<_>>().join("\n");
    format!("{:x}", fnv64(&joined))
}

fn base64_bits(s: &str) -> Vec<bool> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 255,
    };
    let mut bytes = Vec::new();
    let mut acc = 0u32;
    let mut nbits = 0;
    for &c in s.as_bytes() {
        let v = val(c);
        if v == 255 {
            continue;
        }
        acc = (acc << 6) | u32::from(v);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            bytes.push(((acc >> nbits) & 0xff) as u8);
        }
    }
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for b in bytes {
        for k in 0..8 {
            bits.push(b & (1 << k) != 0);
        }
    }
    bits
}

// ---------------------------------------------------------------------------------
// Typed enumeration (joint choices from LegalActions)
// ---------------------------------------------------------------------------------

fn slot_options(b: &Battle<ForbiddenLog>, la: &LegalActions, k: usize, forced_left: u8, passes_left: u8, tera_used: bool, switch_ins: u8, kind: RequestKind) -> Vec<SlotChoice> {
    let ls = &la.slots[k];
    let mut out = Vec::new();
    let pass = SlotChoice::default();
    if ls.automatic_pass {
        return vec![pass];
    }
    let side = &b.state.sides[la.side.0 as usize];
    match kind {
        RequestKind::Move => {
            for m in &ls.moves[..ls.move_count as usize] {
                for loc in -2i8..=2 {
                    if !m.targets.contains(loc) {
                        continue;
                    }
                    for tera in [false, true] {
                        if tera && (!m.can_terastallize || tera_used) {
                            continue;
                        }
                        out.push(SlotChoice {
                            kind: ChoiceKind::Move,
                            move_slot: m.move_slot,
                            switch_to: MonId::NONE,
                            target_loc: loc,
                            tera,
                            move_id: m.move_id,
                            move_kind: m.move_kind,
                        });
                    }
                }
            }
            for i in 0..6 {
                if ls.switch_mask & (1 << i) != 0 && switch_ins & (1 << i) == 0 {
                    out.push(SlotChoice {
                        kind: ChoiceKind::Switch,
                        switch_to: side.party[i],
                        ..SlotChoice::default()
                    });
                }
            }
        }
        _ => {
            for i in 0..6 {
                if ls.switch_mask & (1 << i) != 0 && switch_ins & (1 << i) == 0 && forced_left > 0 {
                    out.push(SlotChoice {
                        kind: ChoiceKind::Switch,
                        switch_to: side.party[i],
                        ..SlotChoice::default()
                    });
                }
                if ls.revival_mask & (1 << i) != 0 {
                    out.push(SlotChoice {
                        kind: ChoiceKind::Revival,
                        switch_to: side.party[i],
                        ..SlotChoice::default()
                    });
                }
            }
            if passes_left > 0 {
                out.push(pass);
            }
        }
    }
    out
}

/// Every joint choice implied by the documented LegalActions semantics.
fn enumerate_typed(b: &Battle<ForbiddenLog>, la: &LegalActions) -> Vec<[SlotChoice; 2]> {
    let kind = b.side_request_kind(la.side);
    let mut out = Vec::new();
    let c = la.constraints;
    for first in slot_options(b, la, 0, c.forced_switches, c.forced_passes, false, 0, kind) {
        let (mut forced, mut passes, mut tera, mut ins) = (c.forced_switches, c.forced_passes, false, 0u8);
        apply(b, la, 0, &first, &mut forced, &mut passes, &mut tera, &mut ins);
        for second in slot_options(b, la, 1, forced, passes, tera, ins, kind) {
            let (mut f2, mut p2, mut t2, mut i2) = (forced, passes, tera, ins);
            apply(b, la, 1, &second, &mut f2, &mut p2, &mut t2, &mut i2);
            if kind == RequestKind::Switch && f2 != 0 {
                continue;
            }
            out.push([first, second]);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn apply(
    b: &Battle<ForbiddenLog>,
    la: &LegalActions,
    k: usize,
    sc: &SlotChoice,
    forced: &mut u8,
    passes: &mut u8,
    tera: &mut bool,
    ins: &mut u8,
) {
    if la.slots[k].automatic_pass {
        return;
    }
    match sc.kind {
        ChoiceKind::Pass => *passes = passes.saturating_sub(1),
        ChoiceKind::Revival => *forced = forced.saturating_sub(1),
        ChoiceKind::Switch => {
            let pos = b.state.pokemon[sc.switch_to.0 as usize].position;
            *ins |= 1 << pos;
            if b.side_request_kind(la.side) == RequestKind::Switch {
                *forced -= 1;
            }
        }
        ChoiceKind::Move => {
            if sc.tera {
                *tera = true;
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// The main differential test
// ---------------------------------------------------------------------------------

fn check_case(case: &J, skipped: &mut usize) {
    let id = case.get("id").str().to_string();
    let Some(mut b) = build_battle(case) else {
        *skipped += 1;
        return;
    };
    let ctx = |what: &str| format!("case {id}: {what}");

    let frozen_seed = b.seed();
    let frozen_log_count = b.scratch.unsent_lines;

    // 1. Cached requests are byte-identical, and rebuilding them did not disturb state.
    for s in 0..2 {
        let expected = case.get("requests").at(s).str();
        let actual = b.request_json(s).unwrap_or_default();
        assert_eq!(actual, expected, "{}", ctx(&format!("request p{}", s + 1)));
        assert_eq!(
            flag_string(&b, s),
            case.get("initialFlags").at(s).str(),
            "{}",
            ctx("hypothesis flags after request rebuild")
        );
    }

    // 2. Probe choices in order, with partial-choice state.
    for (n, op) in case.get("ops").arr().iter().enumerate() {
        let side = SideId(op.get("side").int() as u8);
        let input = op.get("input").str();
        let res = b.choose_no_commit(side, input);
        assert_eq!(b.seed(), frozen_seed, "case {id} op {n}: noncommit advanced PRNG");
        assert_eq!(b.scratch.unsent_lines, frozen_log_count, "case {id} op {n}: noncommit changed log count");
        let what = |t: &str| format!("case {id} op {n} ({} {:?}): {t}", side.0 + 1, input);
        match (op.get("ok").bool(), &res) {
            (true, Ok(())) => {}
            (true, Err(e)) => panic!("{}", what(&format!("Showdown accepted, engine rejected: {}", e.text))),
            (false, Ok(())) => panic!("{}", what(&format!("Showdown rejected ({:?}), engine accepted", op.get("error").opt_str()))),
            (false, Err(e)) => {
                let expect_text = op.get("error").opt_str().unwrap_or("");
                assert_eq!(e.text, expect_text, "{}", what("error text"));
                assert_eq!(
                    e.resent_request_json.as_deref(),
                    op.get("resent").opt_str(),
                    "{}",
                    what("re-sent request")
                );
            }
        }
        assert_eq!(b.choice_text(side), op.get("choice").str(), "{}", what("partial choice"));
        let c = &b.state.sides[side.0 as usize].choice;
        assert_eq!(c.cant_undo, op.get("cantUndo").bool(), "{}", what("cantUndo"));
        assert_eq!(i64::from(c.forced_switches_left), op.get("fsl").int(), "{}", what("forcedSwitchesLeft"));
        assert_eq!(i64::from(c.forced_passes_left), op.get("fpl").int(), "{}", what("forcedPassesLeft"));
    }
    for s in 0..2 {
        let after = case.get("afterOps").at(s);
        let expected_req = after.get("request").opt_str().unwrap_or(case.get("requests").at(s).str());
        assert_eq!(b.request_json(s).unwrap_or_default(), expected_req, "{}", ctx("request after ops"));
        assert_eq!(flag_string(&b, s), after.get("flags").str(), "{}", ctx("flags after ops"));
    }
    for s in 0..2 {
        b.clear_choice(SideId(s));
    }

    // 3. Brute-force acceptance against the Showdown bit-vector, then typed choices.
    if case.get("brute").is_null() {
        return;
    }
    let list = candidates();
    for s in 0..2usize {
        let side = SideId(s as u8);
        let expected = case.get("brute").at(s);
        if expected.is_null() {
            continue;
        }
        let request_before = b.request_json(s);
        let flags_before = flag_string(&b, s);
        let typed = b.legal_actions(side);
        assert_eq!(b.request_json(s), request_before, "case {id}: legal enumeration patched request");
        assert_eq!(flag_string(&b, s), flags_before, "case {id}: legal enumeration changed hypotheses");
        assert_eq!(b.seed(), frozen_seed, "case {id}: legal enumeration advanced PRNG");
        let typed_choices = enumerate_typed(&b, &typed);
        let bits = base64_bits(expected.get("bits").str());
        let mut accepted = BTreeSet::new();
        for (i, input) in list.iter().enumerate() {
            b.clear_choice(side);
            let ok = b.choose_no_commit(side, input).is_ok();
            assert_eq!(b.seed(), frozen_seed, "case {id} candidate {i}: PRNG");
            assert_eq!(b.scratch.unsent_lines, frozen_log_count, "case {id} candidate {i}: log count");
            if ok {
                accepted.insert(b.choice_text(side));
            }
            let want = bits.get(i).copied().unwrap_or(false);
            assert_eq!(
                ok, want,
                "{}",
                ctx(&format!("p{} candidate #{i} {input:?}: engine {ok}, Showdown {want}", s + 1))
            );
        }
        b.clear_choice(side);
        assert_eq!(accepted.len() as i64, expected.get("count").int(), "{}", ctx("accepted count"));
        assert_eq!(set_hash(&accepted), expected.get("hash").str(), "{}", ctx("accepted set hash"));

        // Typed: every enumerated joint choice is accepted, and the normalized set equals Showdown's.
        let mut typed_set = BTreeSet::new();
        for joint in &typed_choices {
            b.clear_choice(side);
            if let Err(e) = b.choose_typed_no_commit(side, joint) {
                panic!("{}", ctx(&format!("p{} typed {joint:?} rejected: {}", s + 1, e.text)));
            }
            assert!(
                b.is_legal_joint_choice(side, joint),
                "{}",
                ctx(&format!("p{} is_legal_joint_choice false for accepted {joint:?}", s + 1))
            );
            assert_eq!(b.seed(), frozen_seed, "case {id}: typed noncommit advanced PRNG");
            assert_eq!(b.scratch.unsent_lines, frozen_log_count, "case {id}: typed noncommit changed log count");
            typed_set.insert(b.choice_text(side));
        }
        b.clear_choice(side);
        if set_hash(&typed_set) != expected.get("hash").str() {
            let missing: Vec<_> = accepted.difference(&typed_set).take(5).collect();
            let extra: Vec<_> = typed_set.difference(&accepted).take(5).collect();
            panic!(
                "{}",
                ctx(&format!(
                    "p{} typed set differs: showdown {} vs typed {}; missing {missing:?}; extra {extra:?}",
                    s + 1,
                    accepted.len(),
                    typed_set.len()
                ))
            );
        }
    }
    for s in 0..2 {
        let after = case.get("afterBrute").at(s);
        let before = case.get("afterOps").at(s);
        let expected_req = after
            .get("request")
            .opt_str()
            .or(before.get("request").opt_str())
            .unwrap_or(case.get("requests").at(s).str());
        assert_eq!(b.request_json(s).unwrap_or_default(), expected_req, "{}", ctx("request after brute force"));
        assert_eq!(flag_string(&b, s), after.get("flags").str(), "{}", ctx("flags after brute force"));
    }
}

fn run_vectors() {
    let cases = load_cases();
    assert!(!cases.is_empty(), "no vectors");
    let mut skipped = 0;
    for case in &cases {
        check_case(case, &mut skipped);
    }
    eprintln!("choices vectors: {} cases, {} skipped (need effect callbacks)", cases.len(), skipped);
}

#[test]
fn vectors_match_showdown_with_local_tera_model() {
    LOCAL_TERA.with(|c| c.set(true));
    run_vectors();
}

#[test]
#[ignore = "needs L (Battle::can_terastallize)"]
fn vectors_match_showdown_with_lifecycle_tera() {
    LOCAL_TERA.with(|c| c.set(false));
    run_vectors();
}

// ---------------------------------------------------------------------------------
// Focused unit tests
// ---------------------------------------------------------------------------------

#[test]
fn target_suffix_trims_before_resolving_legacy_modifier() {
    LOCAL_TERA.with(|c| c.set(true));
    let case = parse_json(EMBEDDED.lines().next().unwrap());
    let mut b = build_battle(&case).unwrap();
    let seed = b.seed();
    // Pinned side.ts:1233 applies trim after stripping the target, exposing mega.
    let error = b.choose_no_commit(SideId(0), "move 1 mega  1").unwrap_err();
    assert_eq!(error.text, "[Invalid choice] Can't move: Rampardos can't mega evolve");
    assert_eq!(b.seed(), seed);
}

#[test]
fn locked_undo_excludes_every_legal_resubmission() {
    LOCAL_TERA.with(|c| c.set(true));
    let case = parse_json(EMBEDDED.lines().next().unwrap());
    let mut b = build_battle(&case).unwrap();
    let mon = b.state.sides[0].active[0];
    b.state.pokemon[mon.0 as usize].flags |= mon_flags::MAYBE_TRAPPED;
    // side.ts:1000 accepts this switch but forbids a later undo/resubmission.
    b.choose_no_commit(SideId(0), "switch 3, move 1 1").unwrap();
    assert!(b.state.sides[0].choice.cant_undo);
    let slots = b.state.sides[0].choice.slots;
    assert_eq!(b.legal_actions(SideId(0)).slot_count, 0);
    assert!(!b.is_legal_joint_choice(SideId(0), &slots));
    let error = b.choose_typed_no_commit(SideId(0), &slots).unwrap_err();
    assert_eq!(error.text, "[Invalid choice] Can't undo: A trapping/disabling effect would cause undo to leak information");
}

#[test]
fn json_string_escaping_matches_json_stringify() {
    let mut out = String::new();
    write_json_string(&mut out, "a\"b\\c\n\t\u{1}\u{1f}é😀\u{7f}\u{2028}");
    assert_eq!(out, "\"a\\\"b\\\\c\\n\\t\\u0001\\u001fé😀\u{7f}\u{2028}\"");
}

#[test]
fn target_validity_matches_source_table() {
    use TargetKind::*;
    // (kind, position, valid locations)
    let cases: [(TargetKind, u8, &[i32]); 6] = [
        (Normal, 0, &[-2, 1, 2]),
        (Normal, 1, &[-1, 1, 2]),
        (Any, 0, &[-2, 1, 2]),
        (AdjacentAlly, 0, &[-2]),
        (AdjacentAllyOrSelf, 1, &[-2, -1]),
        (AdjacentFoe, 1, &[1, 2]),
    ];
    for (kind, pos, valid) in cases {
        let got: Vec<i32> = (-3..=3).filter(|&l| l != 0 && kind.valid_loc(l, pos)).collect();
        assert_eq!(got, valid, "{kind:?} at {pos}");
    }
    assert!(!SelfTarget.valid_loc(1, 0));
    assert!(Normal.valid_loc(0, 0));
}

#[allow(dead_code)]
fn _kinds(_: ChosenMoveKind) {}
