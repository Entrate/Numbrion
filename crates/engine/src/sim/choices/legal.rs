//! OWNER C. Fixed-size accepted-action options for training, without formatting.
//! Derived from side.ts:552-849,915-1023,1330-1357 and battle.ts:2403-2438.
//! This is authoritative legality: hidden information may remove an option that
//! the public cached request still advertises. Enumeration never reveals/patches
//! requests, executes hooks, emits logs or advances the PRNG.

use super::{
    TargetKind, move_presence as mp, requests::request_target, slot_flags as sf,
};
use crate::{
    Battle, dex,
    dex::MoveTarget,
    ids::*,
    log::LogSink,
    state::{
        Trapped,
        choices::{ChoiceKind, ChosenMoveKind, RequestKind, SlotChoice},
    },
};

/// Choices of signed Showdown targetLoc (battle.ts:2403-2438). Bit order is
/// -2,-1,0,+1,+2; zero means automatic targeting, not a missing choice. PRNG: none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetMask(pub u8);

impl TargetMask {
    /// Mask with only the automatic (0) location.
    pub const AUTO: Self = Self(1 << 2);

    /// Bit of a signed location in -2..=2.
    #[inline]
    pub const fn bit(loc: i8) -> u8 {
        1 << (loc + 2) as u8
    }

    #[inline]
    pub const fn contains(self, loc: i8) -> bool {
        loc >= -2 && loc <= 2 && self.0 & Self::bit(loc) != 0
    }

    /// The legal locations of a user at active `position` for `kind`.
    pub const fn of_kind(kind: TargetKind, position: u8) -> Self {
        if !kind.choosable() {
            return Self::AUTO;
        }
        let mut mask = 0u8;
        let mut loc = -2i8;
        while loc <= 2 {
            if loc != 0 && kind.valid_loc(loc as i32, position) {
                mask |= Self::bit(loc);
            }
            loc += 1;
        }
        Self(mask)
    }
}

/// One legal move selection (side.ts:552-849), including locked/Recharge/Struggle
/// forms. `move_slot=255` means absent; target_type is the post-restriction target.
/// PRNG: none during enumeration; automatic target selection draws at commit.
#[derive(Clone, Copy, Debug, Default)]
pub struct LegalMove {
    pub move_id: EffectId,
    pub move_kind: ChosenMoveKind,
    pub move_slot: u8,
    pub target_type: MoveTarget,
    pub targets: TargetMask,
    pub can_terastallize: bool,
}

/// Structured choices for one active position (side.ts:552,915,1330). Switch and
/// revival masks use current party indices 0..5. `automatic_pass` is inserted by
/// getChoiceIndex; it remains explicit in structured joint choices. PRNG: none.
#[derive(Clone, Copy, Debug)]
pub struct LegalSlot {
    pub pokemon: MonId,
    pub moves: [LegalMove; 4],
    pub move_count: u8,
    pub switch_mask: u8,
    pub revival_mask: u8,
    pub can_pass: bool,
    pub automatic_pass: bool,
}

impl Default for LegalSlot {
    fn default() -> Self {
        Self {
            pokemon: MonId::NONE,
            moves: [LegalMove::default(); 4],
            move_count: 0,
            switch_mask: 0,
            revival_mask: 0,
            can_pass: false,
            automatic_pass: false,
        }
    }
}

/// Constraints checked jointly across two slots (side.ts:978,824,1337-1341).
/// Incoming Pokémon must be distinct and at most one move may request Tera.
/// forced switch/pass counts describe the source's replacement limits. PRNG: none.
///
/// For a switch request, scan the slots in order: a flagged slot that is not
/// reviving may switch only while forced switches remain (each switch consumes one)
/// and may pass only while forced passes remain; a reviving slot may revive any
/// fainted Pokémon (consuming a forced switch when any remain) or pass while forced
/// passes remain. Every forced switch must be consumed.
#[derive(Clone, Copy, Debug, Default)]
pub struct JointConstraints {
    pub unique_switch_ins: bool,
    pub terastallize_budget: u8,
    pub forced_switches: u8,
    pub forced_passes: u8,
}

/// Fixed-size legal-action space (side.ts:552-849,915-1023,1330-1357).
/// A waiting side has slot_count zero; legal joint selections must satisfy the
/// per-slot options and every JointConstraints member. PRNG: none.
#[derive(Clone, Copy, Debug, Default)]
pub struct LegalActions {
    pub side: SideId,
    pub slots: [LegalSlot; 2],
    pub slot_count: u8,
    pub constraints: JointConstraints,
}

impl<L: LogSink> Battle<L> {
    /// Authoritative options consistent with choose_move/switch/pass acceptance
    /// (side.ts:552,915,1330). Read the frozen request plus actual disabling and
    /// trapping state; never run mutating hooks or expose request updates.
    /// PRNG: none; consumes no draws for automatically targeted moves.
    pub fn legal_actions(&self, side: SideId) -> LegalActions {
        let mut out = LegalActions {
            side,
            ..LegalActions::default()
        };
        // Side.choose and the typed boundary both reject before parsing when a
        // prior choice revealed information. There are no accepted resubmissions.
        if self.state.sides[side.0 as usize].choice.cant_undo {
            return out;
        }
        match self.side_request_kind(side) {
            RequestKind::Move => self.legal_move_slots(side, &mut out),
            RequestKind::Switch => self.legal_switch_slots(side, &mut out),
            RequestKind::None | RequestKind::Wait => {}
        }
        out
    }

    /// Bitmask of living bench members (party index >= 2).
    fn legal_bench_mask(&self, side: SideId) -> u8 {
        let sd = &self.state.sides[side.0 as usize];
        let mut mask = 0;
        if sd.pokemon_left == 0 {
            return 0;
        }
        for i in 2..sd.pokemon_count as usize {
            if !self.ch_fainted(sd.party[i]) {
                mask |= 1 << i;
            }
        }
        mask
    }

    fn legal_move_slots(&self, side: SideId, out: &mut LegalActions) {
        let s = side.0 as usize;
        out.slot_count = 2;
        out.constraints = JointConstraints {
            unique_switch_ins: true,
            terastallize_budget: 1,
            forced_switches: 0,
            forced_passes: 0,
        };
        let bench = self.legal_bench_mask(side);
        for k in 0..2 {
            let mon = self.ch_active(side, k);
            let mut slot = LegalSlot {
                pokemon: mon,
                ..LegalSlot::default()
            };
            if self.ch_fainted(mon) || self.ch_commanding(mon) {
                slot.can_pass = true;
                slot.automatic_pass = true;
                out.slots[k] = slot;
                continue;
            }
            let req = &self.state.requests[s].active[k];
            let n = req.move_count as usize;
            let position = self.ch_mon(mon).position;
            if n == 1 && req.moves[0].presence & mp::PP == 0 {
                // Locked, Recharge or Struggle: chosen as plain `move 1`; Tera is ignored.
                slot.moves[0] = self.legal_forced_move(&req.moves[0]);
                slot.move_count = 1;
            } else {
                let can_tera = self.ch_can_tera(mon) != TypeId::NONE;
                let slots = self.ch_mon(mon).move_slots();
                let mut any_enabled = false;
                for sl in slots.iter().take(n) {
                    // chooseMove checks getMoves() unrestricted: PP, 'hidden' and true all disable.
                    any_enabled |= sl.pp != 0 && sl.flags & sf::DISABLED_MASK == 0;
                }
                if !any_enabled {
                    // getMoves() is empty, so every selection becomes Struggle.
                    slot.moves[0] = LegalMove {
                        move_id: dex::MOVE_STRUGGLE,
                        move_kind: ChosenMoveKind::Dex,
                        move_slot: 255,
                        target_type: MoveTarget::RandomNormal,
                        targets: TargetMask::AUTO,
                        can_terastallize: false,
                    };
                    slot.move_count = 1;
                } else {
                    for (j, sl) in slots.iter().enumerate().take(n) {
                        if sl.pp == 0 || sl.flags & sf::DISABLED_MASK != 0 {
                            continue;
                        }
                        let kind = request_target(&req.moves[j]);
                        slot.moves[slot.move_count as usize] = LegalMove {
                            move_id: sl.id,
                            move_kind: ChosenMoveKind::Dex,
                            move_slot: j as u8,
                            target_type: kind.move_target(),
                            targets: TargetMask::of_kind(kind, position),
                            can_terastallize: can_tera,
                        };
                        slot.move_count += 1;
                    }
                }
            }
            // Hidden trapping removes switching even when the request does not show it.
            if self.ch_mon(mon).trapped == Trapped::No {
                slot.switch_mask = bench;
            }
            out.slots[k] = slot;
        }
    }

    fn legal_forced_move(&self, m: &crate::state::choices::RequestMove) -> LegalMove {
        let (move_id, move_kind, target_type) = if m.presence & mp::RECHARGE != 0 {
            (EffectId::NONE, ChosenMoveKind::Recharge, MoveTarget::SelfTarget)
        } else if m.id == dex::MOVE_STRUGGLE {
            (dex::MOVE_STRUGGLE, ChosenMoveKind::Dex, MoveTarget::RandomNormal)
        } else {
            (m.id, ChosenMoveKind::Dex, dex::move_data(m.id).target)
        };
        LegalMove {
            move_id,
            move_kind,
            move_slot: 255,
            target_type,
            targets: TargetMask::AUTO,
            can_terastallize: false,
        }
    }

    fn legal_switch_slots(&self, side: SideId, out: &mut LegalActions) {
        let sd = &self.state.sides[side.0 as usize];
        out.slot_count = 2;
        let flagged = (0..2).filter(|&k| self.ch_switch_flag(sd.active[k])).count() as u8;
        let bench = self.legal_bench_mask(side);
        let forced_switches = flagged.min(bench.count_ones() as u8);
        out.constraints = JointConstraints {
            unique_switch_ins: true,
            terastallize_budget: 0,
            forced_switches,
            forced_passes: flagged - forced_switches,
        };
        let mut fainted_mask = 0u8;
        for i in 0..sd.pokemon_count as usize {
            if self.ch_fainted(sd.party[i]) {
                fainted_mask |= 1 << i;
            }
        }
        for k in 0..2 {
            let mon = sd.active[k];
            let mut slot = LegalSlot {
                pokemon: mon,
                ..LegalSlot::default()
            };
            if !self.ch_switch_flag(mon) {
                slot.can_pass = true;
                slot.automatic_pass = true;
            } else if self.ch_has_revival_slot(side, self.ch_mon(mon).position) {
                slot.revival_mask = fainted_mask;
                slot.can_pass = out.constraints.forced_passes > 0;
            } else {
                slot.switch_mask = bench;
                slot.can_pass = out.constraints.forced_passes > 0;
            }
            out.slots[k] = slot;
        }
    }

    /// Shared acceptance check for typed joint choices and the text validator
    /// (side.ts:552-849,915-1023,1330-1357). No partial choice mutation; hidden
    /// disclosure errors are handled by choose_typed/choose_side. PRNG: none.
    ///
    /// `slots[k]` is the choice for active slot k. Automatically passing slots must
    /// be a Pass (or be omitted when trailing).
    pub fn is_legal_joint_choice(&self, side: SideId, slots: &[SlotChoice]) -> bool {
        let la = self.legal_actions(side);
        if la.slot_count == 0 || slots.len() > 2 {
            return false;
        }
        let s = side.0 as usize;
        let party_index = |mon: MonId| -> Option<usize> {
            let sd = &self.state.sides[s];
            sd.party[..sd.pokemon_count as usize]
                .iter()
                .position(|&m| m == mon)
        };
        match self.side_request_kind(side) {
            RequestKind::Move => {
                let mut tera_used = false;
                let mut switch_ins = 0u8;
                for k in 0..2 {
                    let ls = &la.slots[k];
                    let Some(sc) = slots.get(k) else {
                        if ls.automatic_pass {
                            continue;
                        }
                        return false;
                    };
                    if ls.automatic_pass {
                        if sc.kind != ChoiceKind::Pass {
                            return false;
                        }
                        continue;
                    }
                    match sc.kind {
                        ChoiceKind::Pass | ChoiceKind::Revival => return false,
                        ChoiceKind::Switch => {
                            let Some(i) = party_index(sc.switch_to) else { return false };
                            if ls.switch_mask & (1 << i) == 0 || switch_ins & (1 << i) != 0 {
                                return false;
                            }
                            switch_ins |= 1 << i;
                        }
                        ChoiceKind::Move => {
                            let Some(m) = ls.moves[..ls.move_count as usize].iter().find(|m| {
                                m.move_slot == sc.move_slot
                                    && m.move_id == sc.move_id
                                    && m.move_kind == sc.move_kind
                            }) else {
                                return false;
                            };
                            if !m.targets.contains(sc.target_loc) {
                                return false;
                            }
                            if m.move_slot != 255 && sc.tera {
                                // Tera on a forced move is ignored, never counted.
                                if !m.can_terastallize || tera_used {
                                    return false;
                                }
                                tera_used = true;
                            }
                        }
                    }
                }
                true
            }
            RequestKind::Switch => {
                let mut forced_left = la.constraints.forced_switches;
                let mut passes_left = la.constraints.forced_passes;
                let mut switch_ins = 0u8;
                for k in 0..2 {
                    let ls = &la.slots[k];
                    let Some(sc) = slots.get(k) else {
                        if ls.automatic_pass {
                            continue;
                        }
                        return false;
                    };
                    if ls.automatic_pass {
                        if sc.kind != ChoiceKind::Pass {
                            return false;
                        }
                        continue;
                    }
                    match sc.kind {
                        ChoiceKind::Move => return false,
                        ChoiceKind::Pass => {
                            if passes_left == 0 {
                                return false;
                            }
                            passes_left -= 1;
                        }
                        ChoiceKind::Switch | ChoiceKind::Revival => {
                            let Some(i) = party_index(sc.switch_to) else { return false };
                            if self.ch_has_revival_slot(side, self.ch_mon(ls.pokemon).position) {
                                if ls.revival_mask & (1 << i) == 0 {
                                    return false;
                                }
                                forced_left = forced_left.saturating_sub(1);
                            } else {
                                if ls.switch_mask & (1 << i) == 0
                                    || switch_ins & (1 << i) != 0
                                    || forced_left == 0
                                {
                                    return false;
                                }
                                forced_left -= 1;
                                switch_ins |= 1 << i;
                            }
                        }
                    }
                }
                forced_left == 0
            }
            RequestKind::None | RequestKind::Wait => false,
        }
    }
}
