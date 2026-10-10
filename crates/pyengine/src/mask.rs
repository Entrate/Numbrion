//! Legal-action masks for one side at one decision boundary, built from the engine's typed
//! `legal_actions` and exact with respect to the joint constraints (docs/design/TRAINING-API.md).
//!
//! * `slot_set[k]`: the codes slot `k` may take on its own.
//! * `mask0()`: codes of slot 0 that extend to at least one complete legal joint choice.
//! * `mask1(a0)`: codes of slot 1 that complete a legal joint choice given slot 0 chose `a0`.
//!
//! Joint constraints (see `engine::sim::choices::legal`): the two incoming Pokemon must differ, at most
//! one Tera per side (Move requests); the forced switch / forced pass budget of a replacement request,
//! which every flagged slot must consume exactly (Switch requests).

use crate::action::*;
use engine::{
    Battle,
    ids::{MonId, SideId},
    log::LogSink,
    sim::choices::{LegalActions, LegalMove, move_presence as mp},
    state::choices::{ChoiceKind, ChosenMoveKind, RequestKind, SlotChoice},
};

/// What the engine is asking a side for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Req {
    /// Waiting for the other side, or the battle is over (or the side already answered).
    #[default]
    None,
    Move,
    Switch,
}

impl Req {
    /// 0 none/wait, 1 move, 2 switch (the value exposed to Python as `request_kind`).
    pub const fn code(self) -> u8 {
        match self {
            Req::None => 0,
            Req::Move => 1,
            Req::Switch => 2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SideActions {
    pub req: Req,
    pub legal: LegalActions,
    pub slot_set: [u64; 2],
    /// Slot is not flagged / fainted: its only action is `pass` and it takes no part in the budgets.
    pub auto: [bool; 2],
    /// Revival Blessing slot of a Switch request (revives instead of switching).
    pub revival: [bool; 2],
}

impl Default for SideActions {
    fn default() -> Self {
        Self::none()
    }
}

impl SideActions {
    pub fn none() -> Self {
        SideActions {
            req: Req::None,
            legal: LegalActions::default(),
            slot_set: [0; 2],
            auto: [false; 2],
            revival: [false; 2],
        }
    }

    pub fn from_battle<L: LogSink>(b: &Battle<L>, side: SideId) -> Self {
        let legal = b.legal_actions(side);
        if legal.slot_count == 0 {
            return Self::none();
        }
        let req = match b.side_request_kind(side) {
            RequestKind::Move => Req::Move,
            RequestKind::Switch => Req::Switch,
            RequestKind::None | RequestKind::Wait => return Self::none(),
        };
        let mut out = SideActions {
            req,
            legal,
            slot_set: [0; 2],
            auto: [false; 2],
            revival: [false; 2],
        };
        for k in 0..2 {
            let ls = &legal.slots[k];
            out.auto[k] = ls.automatic_pass;
            out.revival[k] = ls.revival_mask != 0;
            let mut set = 0u64;
            if ls.automatic_pass {
                set |= 1 << PASS;
            } else {
                for m in &ls.moves[..ls.move_count as usize] {
                    if m.move_slot == 255 {
                        // Locked, Recharge or Struggle: one canonical code.
                        set |= 1 << FORCED_MOVE;
                        continue;
                    }
                    for t in 0..N_TARGETS as u8 {
                        if m.targets.contains(target_loc(t)) {
                            set |= 1 << move_action(m.move_slot, t, false);
                            if m.can_terastallize {
                                set |= 1 << move_action(m.move_slot, t, true);
                            }
                        }
                    }
                }
                let sw = ls.switch_mask | ls.revival_mask;
                for p in 0..N_SWITCH as u8 {
                    if sw & (1 << p) != 0 {
                        set |= 1 << switch_action(p);
                    }
                }
                if ls.can_pass {
                    set |= 1 << PASS;
                }
            }
            out.slot_set[k] = set;
        }
        out
    }

    /// The side has to submit a choice now.
    #[inline]
    pub fn needs_action(&self) -> bool {
        self.req != Req::None
    }

    /// Joint legality of two codes that are individually legal for their slots.
    fn joint_ok(&self, a0: usize, a1: usize) -> bool {
        let c = [classify(a0), classify(a1)];
        match self.req {
            Req::None => false,
            Req::Move => {
                if matches!(c[0], Class::Move { tera: true }) && matches!(c[1], Class::Move { tera: true }) {
                    return false;
                }
                !matches!((c[0], c[1]), (Class::Switch(x), Class::Switch(y)) if x == y)
            }
            Req::Switch => {
                let k = self.legal.constraints;
                let mut forced_left = k.forced_switches;
                let mut passes_left = k.forced_passes;
                let mut incoming = 0u8;
                for slot in 0..2 {
                    if self.auto[slot] {
                        continue;
                    }
                    match c[slot] {
                        Class::Move { .. } => return false,
                        Class::Pass => {
                            if passes_left == 0 {
                                return false;
                            }
                            passes_left -= 1;
                        }
                        Class::Switch(p) => {
                            if self.revival[slot] {
                                forced_left = forced_left.saturating_sub(1);
                            } else {
                                if incoming & (1 << p) != 0 || forced_left == 0 {
                                    return false;
                                }
                                forced_left -= 1;
                                incoming |= 1 << p;
                            }
                        }
                    }
                }
                forced_left == 0
            }
        }
    }

    /// Codes of slot 1 that complete a legal joint choice after slot 0 chose `a0`.
    pub fn mask1(&self, a0: usize) -> u64 {
        if a0 >= N_ACTIONS || self.slot_set[0] & (1 << a0) == 0 {
            return 0;
        }
        let mut out = 0;
        for cls in CLASS_MASKS {
            let members = self.slot_set[1] & cls;
            if members != 0 && self.joint_ok(a0, members.trailing_zeros() as usize) {
                out |= members;
            }
        }
        out
    }

    /// Codes of slot 0 that extend to at least one legal joint choice.
    pub fn mask0(&self) -> u64 {
        let mut out = 0;
        for cls in CLASS_MASKS {
            let members = self.slot_set[0] & cls;
            if members != 0 && self.mask1(members.trailing_zeros() as usize) != 0 {
                out |= members;
            }
        }
        out
    }

    pub fn is_legal(&self, a0: usize, a1: usize) -> bool {
        a1 < N_ACTIONS && self.mask1(a0) & (1 << a1) != 0
    }

    /// Row-per-`a0` joint mask: `joint[a0 * N_ACTIONS + a1]`, all false for codes outside `mask0()`.
    pub fn write_joint(&self, out: &mut [u8]) {
        debug_assert_eq!(out.len(), N_ACTIONS * N_ACTIONS);
        out.fill(0);
        let mut by_class = [None::<u64>; N_CLASSES];
        for a0 in bits(self.slot_set[0]) {
            let cls = class_of(a0);
            let row = *by_class[cls].get_or_insert_with(|| self.mask1(a0));
            for a1 in bits(row) {
                out[a0 * N_ACTIONS + a1] = 1;
            }
        }
    }

    /// Slot `k` holds a forced move (locked, charging, Recharge, Struggle): its only move code is `FORCED_MOVE`.
    pub fn forced(&self, k: usize) -> bool {
        self.legal_move(k, 255).is_some()
    }

    /// Map codes the engine treats as aliases of a forced move onto the canonical forced code.
    pub fn canonical(&self, a: [usize; 2]) -> [usize; 2] {
        let mut out = a;
        for k in 0..2 {
            if self.forced(k) && matches!(decode(a[k]), Some(Act::Move { .. })) {
                out[k] = FORCED_MOVE;
            }
        }
        out
    }

    fn legal_move(&self, k: usize, slot: u8) -> Option<&LegalMove> {
        let ls = &self.legal.slots[k];
        ls.moves[..ls.move_count as usize].iter().find(|m| m.move_slot == slot)
    }

    /// The engine choice for code `a` of slot `k`. `None` if `a` is not in `slot_set[k]`.
    pub fn slot_choice(&self, k: usize, a: usize, party: &[MonId; 6]) -> Option<SlotChoice> {
        if a >= N_ACTIONS || self.slot_set[k] & (1 << a) == 0 {
            return None;
        }
        Some(match decode(a)? {
            Act::Pass => SlotChoice::default(),
            Act::Switch(p) => SlotChoice {
                kind: ChoiceKind::Switch,
                switch_to: party[p as usize],
                ..SlotChoice::default()
            },
            Act::Move { slot, target, tera } => {
                let m = if a == FORCED_MOVE {
                    // The forced entry of the slot (move_slot 255) or, rarely, a plain move 0.
                    self.legal_move(k, 255).or_else(|| self.legal_move(k, 0))?
                } else {
                    self.legal_move(k, slot)?
                };
                SlotChoice {
                    kind: ChoiceKind::Move,
                    move_slot: m.move_slot,
                    switch_to: MonId::NONE,
                    target_loc: if m.move_slot == 255 { 0 } else { target_loc(target) },
                    tera: tera && m.move_slot != 255,
                    move_id: m.move_id,
                    move_kind: m.move_kind,
                }
            }
        })
    }

    /// The pair of engine choices for a joint-legal `(a0, a1)`; `None` if the pair is not legal.
    pub fn choices(&self, a0: usize, a1: usize, party: &[MonId; 6]) -> Option<[SlotChoice; 2]> {
        if !self.is_legal(a0, a1) {
            return None;
        }
        Some([self.slot_choice(0, a0, party)?, self.slot_choice(1, a1, party)?])
    }

    /// Showdown choice text of a joint-legal pair (`move 1 +2 terastallize, switch 3`), accepted by
    /// `Battle::choose` and identical in effect to the typed choice.
    pub fn text(&self, a0: usize, a1: usize) -> Option<String> {
        if !self.is_legal(a0, a1) {
            return None;
        }
        Some(format!("{}, {}", self.slot_text(0, a0)?, self.slot_text(1, a1)?))
    }

    fn slot_text(&self, k: usize, a: usize) -> Option<String> {
        Some(match decode(a)? {
            Act::Pass => "pass".to_string(),
            Act::Switch(p) => format!("switch {}", p + 1),
            Act::Move { slot, target, tera } => {
                if a == FORCED_MOVE && self.legal_move(k, 255).is_some() {
                    return Some("move 1".to_string());
                }
                let loc = target_loc(target);
                let mut s = format!("move {}", slot + 1);
                if loc != 0 {
                    s.push_str(&format!(" {loc:+}"));
                }
                if tera {
                    s.push_str(" terastallize");
                }
                s
            }
        })
    }
}

/// Best-effort engine choice for any code, legal or not, built from the cached request alone. The engine
/// remains the sole judge of whether it is accepted. Used to ask the engine what it says about codes the
/// mask forbids (error messages, exhaustive tests).
pub fn unchecked_slot_choice<L: LogSink>(b: &Battle<L>, side: SideId, k: usize, a: usize) -> SlotChoice {
    let sd = &b.state.sides[side.0 as usize];
    match decode(a) {
        None | Some(Act::Pass) => SlotChoice::default(),
        Some(Act::Switch(p)) => SlotChoice {
            kind: ChoiceKind::Switch,
            switch_to: sd.party[p as usize % 6],
            ..SlotChoice::default()
        },
        Some(Act::Move { slot, target, tera }) => {
            let req = &b.state.requests[side.0 as usize].active[k];
            let n = req.move_count as usize;
            let forced = n == 1 && req.moves[0].presence & mp::PP == 0;
            let (move_slot, rm) = if forced && slot == 0 {
                (255, &req.moves[0])
            } else {
                (slot, &req.moves[slot as usize])
            };
            let (move_id, move_kind) = if rm.presence & mp::RECHARGE != 0 {
                (engine::ids::EffectId::NONE, ChosenMoveKind::Recharge)
            } else if slot as usize >= n && move_slot != 255 {
                (engine::ids::EffectId::NONE, ChosenMoveKind::Dex)
            } else {
                (rm.id, ChosenMoveKind::Dex)
            };
            SlotChoice {
                kind: ChoiceKind::Move,
                move_slot,
                switch_to: MonId::NONE,
                target_loc: target_loc(target),
                tera,
                move_id,
                move_kind,
            }
        }
    }
}

/// Re-encode the side's stored choice (after an accepted `choose`) into canonical codes.
pub fn encode_side_choice<L: LogSink>(b: &Battle<L>, side: SideId) -> Option<[usize; 2]> {
    let sd = &b.state.sides[side.0 as usize];
    if sd.choice.len < 2 {
        return None;
    }
    let mut out = [0usize; 2];
    for k in 0..2 {
        let sc = &sd.choice.slots[k];
        out[k] = match sc.kind {
            ChoiceKind::Pass => PASS,
            ChoiceKind::Switch | ChoiceKind::Revival => {
                let pos = sd.party[..sd.pokemon_count as usize].iter().position(|&m| m == sc.switch_to)?;
                switch_action(pos as u8)
            }
            ChoiceKind::Move => {
                if sc.move_slot == 255 {
                    FORCED_MOVE
                } else {
                    move_action(sc.move_slot, (sc.target_loc + 2) as u8, sc.tera)
                }
            }
        };
    }
    Some(out)
}
