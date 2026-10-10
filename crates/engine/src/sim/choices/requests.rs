//! OWNER C. Cache request optionals and preserve JSON.stringify property order.
//! Ports battle.ts:1381-1469, side.ts:357,521,851-913, pokemon.ts:949-1200.
//! Requests and errors use side updates, never LogSink battle entries.

use core::fmt::Write as _;
use std::sync::OnceLock;

use super::{
    TargetKind, move_presence as mp, request_flags as rf, slot_flags as sf,
    support::{status_name, type_name},
};
use crate::{
    Battle,
    actions::MoveInput,
    dex::{self, EventId},
    event::{EffectRef, EventArg, Relay, SyntheticEffect},
    ids::*,
    log::LogSink,
    state::{
        MoveSlot, Trapped, mon_flags,
        choices::{ActiveRequest, RequestKind, RequestMove, SideRequest},
    },
};

/// Mon identity plus non-string switch-request payload (pokemon.ts:1153-1190).
/// Ident/details/condition/pokeball come from this mon and immutable input data
/// at boundary serialization, retaining source property order. PRNG: none.
#[derive(Clone, Copy, Debug)]
pub struct PokemonRequestData {
    pub pokemon: MonId,
    pub active: bool,
    pub stats: [u16; 5],
    pub moves: [EffectId; 4],
    pub move_count: u8,
    pub base_ability: EffectId,
    pub ability: EffectId,
    pub item: EffectId,
    pub commanding: bool,
    pub reviving: bool,
    pub tera_type: TypeId,
    pub terastallized: TypeId,
}

/// Current party order for side.name/id/pokemon (sim/side.ts:357-367).
/// Names remain in immutable Battle player metadata. PRNG: none.
#[derive(Clone, Copy, Debug)]
pub struct SideRequestData {
    pub side: SideId,
    pub pokemon: [MonId; 6],
    pub pokemon_count: u8,
}

/// Closed patches corresponding to source update callbacks (side.ts:741-764,
/// 851-913,987-998). Applying a patch reports whether data actually changed.
/// PRNG: none.
#[derive(Clone, Copy, Debug)]
pub enum RequestUpdate {
    RevealDisabled {
        pokemon: MonId,
        move_id: EffectId,
        disabled_source: EffectId,
    },
    RevealTrapped {
        pokemon: MonId,
    },
    RefreshDisabled {
        pokemon: MonId,
    },
}

/// `getLockedMove()` result: a dex move or the synthetic Recharge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LockedMove {
    Dex(EffectId),
    Recharge,
}

impl LockedMove {
    pub(crate) fn input(self) -> MoveInput {
        match self {
            Self::Dex(id) => MoveInput::Dex(id),
            Self::Recharge => MoveInput::Recharge,
        }
    }
}

/// Whether the generated dex has any SemiLockMove callback (it has none for this
/// format). With no listener anywhere the priorityEvent has no handlers, draws
/// nothing and has no observable effect, so the dispatch can be skipped exactly.
fn dex_has_semi_lock_hooks() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| dex::HOOKS.iter().any(|h| h.event == EventId::SemiLockMove))
}

impl<L: LogSink> Battle<L> {
    /// Battle.makeRequest (sim/battle.ts:1381-1412). Option None refreshes the
    /// existing global kind; explicit kind clears choices first. PRNG: no direct
    /// draws; scoped LockMove/SemiLockMove and Type request hooks draw none.
    pub fn make_request(&mut self, kind: Option<RequestKind>) {
        let kind = match kind {
            Some(k) => {
                self.state.request_state = k;
                for side in 0..2 {
                    self.clear_choice(SideId(side));
                }
                k
            }
            None => self.state.request_state,
        };
        for r in &mut self.state.requests {
            *r = SideRequest::default();
        }
        // Teampreview is outside this format.
        let requests = self.get_requests(kind);
        self.state.requests = requests;
        if self.is_choice_done(SideId(0)) && self.is_choice_done(SideId(1)) {
            panic!("Choices are done immediately after a request");
        }
    }

    /// Battle.clearRequest (sim/battle.ts:1414-1420). Clear both cached requests
    /// and choices after setting global requestState to none. PRNG: none.
    pub fn clear_request(&mut self) {
        self.state.request_state = RequestKind::None;
        for side in 0..2 {
            self.state.requests[side] = SideRequest::default();
            self.clear_choice(SideId(side as u8));
        }
    }

    /// Battle.getRequests (sim/battle.ts:1422-1469). Waiting sides still receive a
    /// request; add noCancel only to acting requests under the source conditions.
    /// PRNG: no direct draws; scoped request hooks draw none; mutates maybe flags.
    pub fn get_requests(&mut self, kind: RequestKind) -> [SideRequest; 2] {
        let mut requests = [SideRequest::default(); 2];
        match kind {
            RequestKind::Switch => {
                for i in 0..2 {
                    let side = &self.state.sides[i];
                    if side.pokemon_left == 0 {
                        continue;
                    }
                    let mut mask = 0u8;
                    for (k, &mon) in side.active.iter().enumerate() {
                        if self.ch_switch_flag(mon) {
                            mask |= 1 << k;
                        }
                    }
                    if mask != 0 {
                        requests[i].kind = RequestKind::Switch;
                        requests[i].force_switch_mask = mask;
                    }
                }
            }
            _ => {
                for i in 0..2 {
                    if self.state.sides[i].pokemon_left == 0 {
                        continue;
                    }
                    let mut active = [ActiveRequest::default(); 2];
                    for (k, slot) in active.iter_mut().enumerate() {
                        let mon = self.state.sides[i].active[k];
                        assert_ne!(mon, MonId::NONE, "move request for an empty active slot");
                        *slot = self.get_move_request_data(mon);
                    }
                    requests[i].kind = RequestKind::Move;
                    requests[i].active = active;
                }
            }
        }
        let multiple = requests.iter().filter(|r| r.kind != RequestKind::None).count() >= 2;
        for r in &mut requests {
            if r.kind != RequestKind::None {
                if !self.state.support_cancel || !multiple {
                    r.no_cancel = true;
                }
            } else {
                r.kind = RequestKind::Wait;
            }
        }
        requests
    }

    /// Side.requestState getter (sim/side.ts:295-300). A wait request yields None,
    /// independently of the battle-wide kind. PRNG: none.
    pub fn side_request_kind(&self, side: SideId) -> RequestKind {
        match self.state.requests[side.0 as usize].kind {
            RequestKind::Move => RequestKind::Move,
            RequestKind::Switch => RequestKind::Switch,
            RequestKind::None | RequestKind::Wait => RequestKind::None,
        }
    }

    /// Pokemon.getLockedMove (pokemon.ts:947-950). The LockMove priorityEvent is
    /// sorted by redirect order, so it never draws. A literal `true` (no handler)
    /// means not locked.
    pub(crate) fn ch_get_locked_move(&mut self, pokemon: MonId) -> Option<LockedMove> {
        #[cfg(test)]
        if let Some(l) = super::tests::lock_override(pokemon) {
            return Some(l);
        }
        let relay = self.priority_event(
            EventId::LockMove,
            EventArg::Holder(Holder::mon(pokemon)),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            false,
        );
        match relay {
            Relay::Move(id) | Relay::Effect(id) => Some(LockedMove::Dex(id)),
            Relay::PseudoMove(SyntheticEffect::Recharge) => Some(LockedMove::Recharge),
            Relay::Bool(_) | Relay::Undefined | Relay::Null => None,
            other => panic!("unexpected LockMove relay {other:?}"),
        }
    }

    /// Pokemon.getSemiLockedMove (pokemon.ts:958-962). The generated dex has no
    /// SemiLockMove callback in this format, so the event cannot find a handler.
    pub(crate) fn ch_get_semi_locked_move(
        &mut self,
        pokemon: MonId,
        restrict_data: bool,
    ) -> Option<LockedMove> {
        if restrict_data && self.ch_mon(pokemon).flags & mon_flags::MAYBE_LOCKED != 0 {
            return None;
        }
        if !dex_has_semi_lock_hooks() {
            return None;
        }
        let relay = self.priority_event(
            EventId::SemiLockMove,
            EventArg::Holder(Holder::mon(pokemon)),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            false,
        );
        match relay {
            Relay::Move(id) | Relay::Effect(id) => Some(LockedMove::Dex(id)),
            Relay::PseudoMove(SyntheticEffect::Recharge) => Some(LockedMove::Recharge),
            Relay::Bool(_) | Relay::Undefined | Relay::Null => None,
            other => panic!("unexpected SemiLockMove relay {other:?}"),
        }
    }

    /// Pokemon.getMoveRequestData (sim/pokemon.ts:1083-1151). Sets hard trapped
    /// state and clears/restricts maybe flags in source order, even for fainted
    /// actives. PRNG: no direct draws; scoped priority/Type request hooks draw none.
    pub fn get_move_request_data(&mut self, pokemon: MonId) -> ActiveRequest {
        let idx = pokemon.0 as usize;
        let mut locked = self.ch_get_locked_move(pokemon);
        let hard_locked = locked.is_some();
        if hard_locked {
            self.state.pokemon[idx].trapped = Trapped::Yes;
        } else if self.state.pokemon[idx].flags & mon_flags::MAYBE_LOCKED != 0 {
            locked = None;
        } else {
            locked = self.ch_get_semi_locked_move(pokemon, true);
        }

        // Information is restricted for the last active Pokemon.
        let is_last_active = self.is_last_active(pokemon);
        let can_switch_in = self.ch_can_switch_count(pokemon.side()) > 0;
        let mut data = self.get_request_moves(pokemon, locked.map(LockedMove::input), is_last_active);
        let mut any_locked = locked.is_some();
        if data.move_count == 0 {
            data = ActiveRequest::default();
            data.moves[0] = RequestMove {
                id: dex::MOVE_STRUGGLE,
                target: dex::MoveTarget::RandomNormal,
                presence: mp::STRUGGLE,
                ..RequestMove::default()
            };
            data.move_count = 1;
            any_locked = true;
        }

        let mon = &mut self.state.pokemon[idx];
        let mut flags = 0u16;
        if hard_locked || !is_last_active {
            mon.flags &= !(mon_flags::MAYBE_DISABLED | mon_flags::MAYBE_LOCKED | mon_flags::MAYBE_TRAPPED);
            if (hard_locked || can_switch_in) && mon.trapped != Trapped::No {
                flags |= rf::TRAPPED;
            }
        } else {
            if mon.flags & mon_flags::MAYBE_DISABLED != 0 {
                mon.flags |= mon_flags::MAYBE_LOCKED;
            }
            if mon.flags & mon_flags::MAYBE_DISABLED != 0 {
                flags |= rf::MAYBE_DISABLED;
            }
            if mon.flags & mon_flags::MAYBE_LOCKED != 0 {
                flags |= rf::MAYBE_LOCKED;
            }
            if can_switch_in {
                if mon.trapped == Trapped::Yes {
                    flags |= rf::TRAPPED;
                } else if mon.flags & mon_flags::MAYBE_TRAPPED != 0 {
                    flags |= rf::MAYBE_TRAPPED;
                }
            }
        }
        data.flags = flags;
        if !any_locked {
            // canMegaEvo/X/Y, canUltraBurst, canZMove and Dynamax are never set in gen 9.
            let tera = self.ch_can_tera(pokemon);
            if tera != TypeId::NONE {
                data.can_tera = tera;
                data.flags |= rf::CAN_TERASTALLIZE;
            }
        }
        data
    }

    /// Pokemon.getMoves (sim/pokemon.ts:964-1043). Preserve locked/Recharge/full
    /// shapes, hidden disabled values, PP exhaustion and Curse/Pollen Puff/Tera
    /// Starstorm targeting. None means not locked; Recharge uses the kind tag.
    /// PRNG: none directly; Curse's Type dispatch draws none in this scope.
    pub fn get_request_moves(
        &mut self,
        pokemon: MonId,
        locked: Option<MoveInput>,
        restrict_data: bool,
    ) -> ActiveRequest {
        let mut out = ActiveRequest::default();
        match locked {
            Some(MoveInput::Recharge) => {
                out.moves[0] = RequestMove {
                    presence: mp::RECHARGE,
                    ..RequestMove::default()
                };
                out.move_count = 1;
                return out;
            }
            Some(MoveInput::Dex(id)) => {
                out.moves[0] = RequestMove {
                    id,
                    presence: mp::LOCKED,
                    ..RequestMove::default()
                };
                out.move_count = 1;
                return out;
            }
            Some(MoveInput::Active(_)) => panic!("a locked move is never a live ActiveMove"),
            None => {}
        }
        let mut slots = [MoveSlot::default(); 4];
        let count = {
            let s = self.ch_mon(pokemon).move_slots();
            slots[..s.len()].copy_from_slice(s);
            s.len()
        };
        let mut has_valid_move = false;
        for (i, slot) in slots[..count].iter().enumerate() {
            let mut kind = TargetKind::from(slot.target);
            if slot.id == dex::MOVE_CURSE {
                let ghost = dex::TYPE_GHOST;
                if !self.has_type(pokemon, &[ghost]) {
                    kind = TargetKind::SelfTarget;
                }
            } else if slot.id == dex::MOVE_POLLENPUFF {
                // Heal Block only prevents Pollen Puff from targeting an ally.
                if self.ch_volatile(pokemon, dex::CONDITION_HEALBLOCK).is_some() {
                    kind = TargetKind::AdjacentFoe;
                }
            } else if slot.id == dex::MOVE_TERASTARSTORM
                && self.ch_mon(pokemon).species == dex::SPECIES_TERAPAGOSSTELLAR
            {
                kind = TargetKind::AllAdjacentFoes;
            }
            let mut disabled = slot.flags & sf::DISABLED_MASK;
            if slot.pp == 0 {
                disabled = sf::DISABLED_TRUE;
            }
            let disabled = if disabled == sf::DISABLED_HIDDEN {
                !restrict_data
            } else {
                disabled != 0
            };
            if !disabled {
                has_valid_move = true;
            }
            let mut presence = mp::FULL;
            let mut target = dex::MoveTarget::Normal;
            if kind == TargetKind::AdjacentFoe {
                presence |= mp::ADJACENT_FOE;
            } else {
                target = kind.move_target();
            }
            out.moves[i] = RequestMove {
                id: slot.id,
                disabled_source: EffectId::NONE,
                pp: slot.pp,
                max_pp: slot.max_pp,
                target,
                disabled: disabled as u8,
                presence,
            };
        }
        if has_valid_move {
            out.move_count = count as u8;
        } else {
            out = ActiveRequest::default();
        }
        out
    }

    /// Pokemon.isLastActive (sim/pokemon.ts:1192-1200). Check activity and every
    /// higher-position living ally, not only the two-slot position. PRNG: none.
    pub fn is_last_active(&self, pokemon: MonId) -> bool {
        let p = self.ch_mon(pokemon);
        if p.flags & mon_flags::ACTIVE == 0 {
            return false;
        }
        let ally_active = &self.state.sides[pokemon.side().0 as usize].active;
        for i in (p.position as usize + 1)..ally_active.len() {
            if ally_active[i] != MonId::NONE && !self.ch_fainted(ally_active[i]) {
                return false;
            }
        }
        true
    }

    /// Pokemon.getSwitchRequestData (sim/pokemon.ts:1153-1190). Uses current party
    /// position, baseStoredStats, real details and current move IDs. PRNG: none.
    pub fn get_switch_request_data(&self, pokemon: MonId) -> PokemonRequestData {
        let p = self.ch_mon(pokemon);
        let mut moves = [EffectId::NONE; 4];
        let slots = p.move_slots();
        for (m, s) in moves.iter_mut().zip(slots) {
            *m = s.id;
        }
        PokemonRequestData {
            pokemon,
            active: p.position < 2,
            stats: p.base_stored_stats,
            moves,
            move_count: slots.len() as u8,
            base_ability: p.base_ability,
            ability: p.ability,
            item: p.item,
            commanding: self.ch_commanding(pokemon) && p.flags & mon_flags::FAINTED == 0,
            reviving: p.flags & mon_flags::ACTIVE != 0
                && self.ch_has_revival_slot(pokemon.side(), p.position),
            tera_type: self.ch_tera_type(pokemon),
            terastallized: p.terastallized,
        }
    }

    /// Side.getRequestData (sim/side.ts:357-367). Preserve current party array
    /// order after switch swaps. PRNG: none.
    pub fn get_side_request_data(&self, side: SideId) -> SideRequestData {
        let s = &self.state.sides[side.0 as usize];
        SideRequestData {
            side,
            pokemon: s.party[..6].try_into().unwrap(),
            pokemon_count: s.pokemon_count,
        }
    }

    /// Side.updateDisabledRequest (sim/side.ts:851-903). Clear maybe hypotheses,
    /// reveal disabled slots, and remove Tera when all moves are disabled or
    /// Struggle; return whether anything changed. PRNG: none.
    pub fn update_disabled_request(&mut self, pokemon: MonId) -> bool {
        let s = pokemon.side().0 as usize;
        let position = self.ch_mon(pokemon).position as usize;
        let mut req = self.state.requests[s].active[position];
        let mut updated = false;
        let flags = self.state.pokemon[pokemon.0 as usize].flags;
        if flags & mon_flags::MAYBE_LOCKED != 0 {
            self.state.pokemon[pokemon.0 as usize].flags &= !mon_flags::MAYBE_LOCKED;
            req.flags &= !rf::MAYBE_LOCKED;
            updated = true;
        }
        // gameType is doubles, so the singles exclusion never applies; gen >= 4.
        if self.state.pokemon[pokemon.0 as usize].flags & mon_flags::MAYBE_DISABLED != 0 {
            self.state.pokemon[pokemon.0 as usize].flags &= !mon_flags::MAYBE_DISABLED;
            req.flags &= !rf::MAYBE_DISABLED;
            updated = true;
            for m in &mut req.moves[..req.move_count as usize] {
                if m.presence & mp::RECHARGE != 0 {
                    continue;
                }
                let disabled = self
                    .get_move_data(pokemon, m.id)
                    .is_some_and(|slot| slot.flags & sf::DISABLED_MASK != 0);
                if disabled {
                    m.disabled = 1;
                    m.presence |= mp::DISABLED;
                    updated = true;
                }
            }
        }
        let every_disabled = req.moves[..req.move_count as usize]
            .iter()
            .all(|m| (m.presence & mp::DISABLED != 0 && m.disabled != 0) || m.id == dex::MOVE_STRUGGLE);
        if every_disabled && req.can_tera != TypeId::NONE {
            req.can_tera = TypeId::NONE;
            req.flags &= !rf::CAN_TERASTALLIZE;
            updated = true;
        }
        self.state.requests[s].active[position] = req;
        updated
    }

    /// Side.updateRequestForPokemon (sim/side.ts:906-913) and source update closures
    /// (741-764,987-998). Panic on missing active entry; apply a typed cache patch.
    /// PRNG: none.
    pub fn update_request_for_pokemon(&mut self, update: RequestUpdate) -> bool {
        let pokemon = match update {
            RequestUpdate::RevealDisabled { pokemon, .. }
            | RequestUpdate::RevealTrapped { pokemon }
            | RequestUpdate::RefreshDisabled { pokemon } => pokemon,
        };
        let s = pokemon.side().0 as usize;
        if self.state.requests[s].kind != RequestKind::Move {
            panic!("Can't update a request without active Pokemon");
        }
        let position = self.ch_mon(pokemon).position as usize;
        assert!(position < 2, "Pokemon not found in request's active field");
        match update {
            RequestUpdate::RefreshDisabled { pokemon } => self.update_disabled_request(pokemon),
            RequestUpdate::RevealDisabled {
                pokemon,
                move_id,
                disabled_source,
            } => {
                let mut updated = self.update_disabled_request(pokemon);
                let req = &mut self.state.requests[s].active[position];
                for m in &mut req.moves[..req.move_count as usize] {
                    if m.id != move_id || m.presence & mp::RECHARGE != 0 {
                        continue;
                    }
                    if m.presence & mp::DISABLED == 0 || m.disabled == 0 {
                        m.presence |= mp::DISABLED;
                        m.disabled = 1;
                        updated = true;
                    }
                    if m.presence & mp::DISABLED_SOURCE == 0 || m.disabled_source != disabled_source {
                        m.presence |= mp::DISABLED_SOURCE;
                        m.disabled_source = disabled_source;
                        updated = true;
                    }
                    break;
                }
                updated
            }
            RequestUpdate::RevealTrapped { .. } => {
                let req = &mut self.state.requests[s].active[position];
                let mut updated = false;
                if req.flags & rf::MAYBE_TRAPPED != 0 {
                    req.flags &= !rf::MAYBE_TRAPPED;
                    updated = true;
                }
                if req.flags & rf::TRAPPED == 0 {
                    req.flags |= rf::TRAPPED | rf::TRAPPED_LATE;
                    updated = true;
                }
                updated
            }
        }
    }

    /// Side.emitRequest (sim/side.ts:521-525). Set update last when requested and
    /// serialize the cached request. Adapter consumes this return as a sideupdate;
    /// no battle log entry or PRNG draw.
    pub fn emit_request(&mut self, side: SideId, updated: bool) -> String {
        if updated {
            self.state.requests[side.0 as usize].update = true;
        }
        let mut out = String::with_capacity(2048);
        if !self.write_request_json(side, &mut out) {
            panic!("emitRequest without an active request");
        }
        out
    }

    /// JSON.stringify(side.activeRequest) (sim/side.ts:523), as required by
    /// DIFFTEST.md. Serialize cached fields; do not regenerate/mutate requests.
    /// None only after battle end; invalid side is an input error. PRNG: none.
    pub fn request_json(&self, side: usize) -> Option<String> {
        assert!(side < 2, "invalid side index {side}");
        let mut out = String::with_capacity(2048);
        self.write_request_json(SideId(side as u8), &mut out)
            .then_some(out)
    }

    /// Allocation-reusing writer for JSON.stringify request order (side.ts:523,
    /// battle.ts:1422-1469, pokemon.ts:1083-1190). Append one request to out; false
    /// means no pending request. PRNG: none; does not regenerate the request.
    pub fn write_request_json(&self, side: SideId, out: &mut String) -> bool {
        let s = side.0 as usize;
        let req = &self.state.requests[s];
        match req.kind {
            RequestKind::None => return false,
            RequestKind::Wait => {
                out.push_str("{\"wait\":true,\"side\":");
                self.write_side_json(side, out);
            }
            RequestKind::Move => {
                out.push_str("{\"active\":[");
                for (k, a) in req.active.iter().enumerate() {
                    if k > 0 {
                        out.push(',');
                    }
                    write_active_json(a, out);
                }
                out.push_str("],\"side\":");
                self.write_side_json(side, out);
            }
            RequestKind::Switch => {
                out.push_str("{\"forceSwitch\":[");
                for k in 0..2 {
                    if k > 0 {
                        out.push(',');
                    }
                    out.push_str(if req.force_switch_mask & (1 << k) != 0 {
                        "true"
                    } else {
                        "false"
                    });
                }
                out.push_str("],\"side\":");
                self.write_side_json(side, out);
            }
        }
        if req.no_cancel {
            out.push_str(",\"noCancel\":true");
        }
        if req.update {
            out.push_str(",\"update\":true");
        }
        out.push('}');
        true
    }

    /// `{"name":..,"id":..,"pokemon":[..]}` (Side.getRequestData).
    fn write_side_json(&self, side: SideId, out: &mut String) {
        let data = self.get_side_request_data(side);
        out.push_str("{\"name\":");
        write_json_string(out, self.player_name(side));
        out.push_str(",\"id\":\"p");
        let _ = write!(out, "{}", side.0 + 1);
        out.push_str("\",\"pokemon\":[");
        for k in 0..data.pokemon_count as usize {
            if k > 0 {
                out.push(',');
            }
            self.write_pokemon_json(&self.get_switch_request_data(data.pokemon[k]), out);
        }
        out.push_str("]}");
    }

    /// One `Pokemon.getSwitchRequestData` entry in source key order.
    fn write_pokemon_json(&self, d: &PokemonRequestData, out: &mut String) {
        let p = self.ch_mon(d.pokemon);
        out.push_str("{\"ident\":\"p");
        let _ = write!(out, "{}", d.pokemon.side().0 + 1);
        out.push_str(": ");
        // `ident` is `p1: Name`; the name is escaped as part of the JSON string.
        escape_json_into(out, self.ch_name(d.pokemon));
        out.push_str("\",\"details\":");
        write_json_string(out, &self.details(d.pokemon));
        out.push_str(",\"condition\":\"");
        if p.hp == 0 {
            out.push_str("0 fnt");
        } else {
            let _ = write!(out, "{}/{}", p.hp, p.max_hp);
            if p.status != crate::state::Status::None {
                out.push(' ');
                out.push_str(status_name(p.status));
            }
        }
        out.push_str("\",\"active\":");
        out.push_str(if d.active { "true" } else { "false" });
        let [atk, def, spa, spd, spe] = d.stats;
        let _ = write!(
            out,
            ",\"stats\":{{\"atk\":{atk},\"def\":{def},\"spa\":{spa},\"spd\":{spd},\"spe\":{spe}}},\"moves\":["
        );
        for k in 0..d.move_count as usize {
            if k > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(id_key(d.moves[k]));
            out.push('"');
        }
        out.push_str("],\"baseAbility\":\"");
        out.push_str(id_key(d.base_ability));
        out.push_str("\",\"item\":\"");
        out.push_str(id_key(d.item));
        out.push_str("\",\"pokeball\":");
        write_json_string(out, &self.ch_set(d.pokemon).pokeball);
        out.push_str(",\"ability\":\"");
        out.push_str(id_key(d.ability));
        out.push_str("\",\"commanding\":");
        out.push_str(if d.commanding { "true" } else { "false" });
        out.push_str(",\"reviving\":");
        out.push_str(if d.reviving { "true" } else { "false" });
        out.push_str(",\"teraType\":\"");
        out.push_str(type_name(d.tera_type));
        out.push_str("\",\"terastallized\":\"");
        out.push_str(type_name(d.terastallized));
        out.push_str("\"}");
    }
}

/// The `id` string of an effect, or "" for none.
fn id_key(id: EffectId) -> &'static str {
    if id == EffectId::NONE {
        ""
    } else {
        dex::effect(id).key
    }
}

/// One `PokemonMoveRequestData` in JSON.stringify order. A move entry is
/// move,id,pp,maxpp,target,disabled,disabledSource; the active object is moves,
/// maybeDisabled,maybeLocked,trapped|maybeTrapped,canTerastallize, then a trapped
/// key that an update appended.
fn write_active_json(a: &ActiveRequest, out: &mut String) {
    out.push_str("{\"moves\":[");
    for (k, m) in a.moves[..a.move_count as usize].iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        write_move_json(m, out);
    }
    out.push(']');
    if a.flags & rf::MAYBE_DISABLED != 0 {
        out.push_str(",\"maybeDisabled\":true");
    }
    if a.flags & rf::MAYBE_LOCKED != 0 {
        out.push_str(",\"maybeLocked\":true");
    }
    if a.flags & rf::TRAPPED != 0 && a.flags & rf::TRAPPED_LATE == 0 {
        out.push_str(",\"trapped\":true");
    }
    if a.flags & rf::MAYBE_TRAPPED != 0 {
        out.push_str(",\"maybeTrapped\":true");
    }
    if a.can_tera != TypeId::NONE {
        out.push_str(",\"canTerastallize\":\"");
        out.push_str(type_name(a.can_tera));
        out.push('"');
    }
    if a.flags & rf::TRAPPED != 0 && a.flags & rf::TRAPPED_LATE != 0 {
        out.push_str(",\"trapped\":true");
    }
    out.push('}');
}

fn write_move_json(m: &RequestMove, out: &mut String) {
    if m.presence & mp::RECHARGE != 0 {
        out.push_str("{\"move\":\"Recharge\",\"id\":\"recharge\"");
    } else {
        out.push_str("{\"move\":");
        write_json_string(out, dex::move_data(m.id).name);
        out.push_str(",\"id\":\"");
        out.push_str(dex::effect(m.id).key);
        out.push('"');
    }
    if m.presence & mp::PP != 0 {
        let _ = write!(out, ",\"pp\":{}", m.pp);
    }
    if m.presence & mp::MAX_PP != 0 {
        let _ = write!(out, ",\"maxpp\":{}", m.max_pp);
    }
    if m.presence & mp::TARGET != 0 {
        out.push_str(",\"target\":\"");
        out.push_str(request_target(m).name());
        out.push('"');
    }
    if m.presence & mp::DISABLED != 0 {
        out.push_str(if m.disabled != 0 {
            ",\"disabled\":true"
        } else {
            ",\"disabled\":false"
        });
    }
    if m.presence & mp::DISABLED_SOURCE != 0 {
        out.push_str(",\"disabledSource\":");
        if m.disabled_source == EffectId::NONE {
            out.push_str("\"\"");
        } else {
            write_json_string(out, dex::effect(m.disabled_source).name);
        }
    }
    out.push('}');
}

/// The target string of a cached move entry (`target`, or the adjacentFoe override).
pub(crate) fn request_target(m: &RequestMove) -> TargetKind {
    if m.presence & mp::ADJACENT_FOE != 0 {
        TargetKind::AdjacentFoe
    } else {
        TargetKind::from(m.target)
    }
}

/// JSON.stringify string escaping at sim/side.ts:523. Handle quotes, controls,
/// backslashes and ordinary Unicode without dependencies. PRNG: none.
pub fn write_json_string(out: &mut String, text: &str) {
    out.push('"');
    escape_json_into(out, text);
    out.push('"');
}

/// The body of a JSON string literal (no surrounding quotes).
fn escape_json_into(out: &mut String, text: &str) {
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let esc: &str = match c {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\u{8}' => "\\b",
            '\u{c}' => "\\f",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            c if (c as u32) < 0x20 => {
                out.push_str(&text[start..i]);
                let _ = write!(out, "\\u{:04x}", c as u32);
                start = i + c.len_utf8();
                continue;
            }
            _ => continue,
        };
        out.push_str(&text[start..i]);
        out.push_str(esc);
        start = i + c.len_utf8();
    }
    out.push_str(&text[start..]);
}
