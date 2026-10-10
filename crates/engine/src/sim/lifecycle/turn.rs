//! Start, commit, action dispatch and turn boundaries. OWNER L.

use crate::actions::{HitTarget, MoveInput, RunMoveOptions};
use crate::event::{CallArgs, EffectRef, EventArg, Relay, RunEventOptions};
use crate::sim::BattleError;
use crate::state::{
    Phase, ResultFlag, Trapped,
    choices::{Action, ActionKind, ChoiceKind, ChosenMoveKind, RequestKind},
    mon_flags,
};
use crate::{
    Battle, dex,
    ids::{EffectId, EventId, MonId, SideId, SlotId, TypeId},
    log::{LogArg, LogEntry, LogSink, LogTag},
};

use super::queue::ActionChoice;
use super::switching::PURSUIT_FAINT;
use super::util::{SLOT_DISABLED, SLOT_HIDDEN, USED_ITEM_THIS_TURN, dex_effect, mon_arg};

/// HP captures used after the residual action; battle.ts:2671,2819,2860-2863.
/// PRNG: capture draws nothing; EmergencyExit events may draw afterwards.
#[derive(Clone, Copy, Debug)]
pub struct ResidualSnapshot {
    pub entries: [(MonId, u16); 4],
    pub len: u8,
}

impl Default for ResidualSnapshot {
    fn default() -> Self {
        Self {
            entries: [(MonId::NONE, 0); 4],
            len: 0,
        }
    }
}

/// The six `onBegin` hooks of the rule table, in rule-table iteration order
/// (`potd, obtainable, obtainablemoves, obtainableabilities, obtainableformes, evlimit,
/// obtainablemisc, -unreleased, -tag:unobtainable, -nonexistent, speciesclause,
/// hppercentagemod, cancelmod, illusionlevelmod, sleepclausemod`); the Obtainable family
/// and EV Limit have no `onBegin`. sim/battle.ts:1948-1953; docs/showdown/01 section 9.
const RULE_BEGIN_HOOKS: [dex::HookId; 6] = [
    dex::HOOK_RULE_POTD_ONBEGIN,
    dex::HOOK_RULE_SPECIESCLAUSE_ONBEGIN,
    dex::HOOK_RULE_HPPERCENTAGEMOD_ONBEGIN,
    dex::HOOK_RULE_CANCELMOD_ONBEGIN,
    dex::HOOK_RULE_ILLUSIONLEVELMOD_ONBEGIN,
    dex::HOOK_RULE_SLEEPCLAUSEMOD_ONBEGIN,
];

impl<L: LogSink> Battle<L> {
    /// Production request dispatch, with the exact oracle recorder available only to
    /// isolated lifecycle vector tests. The scripts intentionally omit choice objects.
    fn lc_make_request(&mut self, kind: RequestKind) {
        #[cfg(test)]
        if super::flow_tests::records_requests() {
            self.state.request_state = kind;
            return;
        }
        self.make_request(Some(kind));
    }

    /// Start synchronously through the first request; sim/battle.ts:1909-1973.
    /// PRNG: Begin/BattleStart callbacks, initial switch/runSwitch/event ordering.
    pub fn start(&mut self) -> Result<(), BattleError> {
        if self.state.started {
            return Err(BattleError("Battle already started".into()));
        }
        // The constructor's buffered lines (|t:|, |gametype|, both |player|) are realized
        // exactly once here; the second setPlayer in Showdown is what triggers start().
        self.emit_opening_log();
        self.state.started = true;
        self.state.phase = Phase::Start;
        // sides[0].foe / sides[1].foe are implicit (side ^ 1).

        self.add(LogEntry::new("gen", &[LogArg::Number(9)], &[]));
        self.add(LogEntry::new(
            "tier",
            &[LogArg::Text("[Gen 9] Random Doubles Battle")],
            &[],
        ));
        // format.onBegin is undefined; each rule's onBegin is a direct `.call` (no event frame).
        for hook in RULE_BEGIN_HOOKS {
            self.call_hook(
                hook,
                CallArgs {
                    values: [EventArg::Undefined; 4],
                    len: 0,
                },
            );
        }
        // `sides.some(side => !side.pokemon[0])` cannot happen: teams are non-empty.

        self.run_pick_team();
        self.queue_add_choice(ActionChoice::new(ActionKind::Start));
        self.state.mid_turn = true;
        if self.state.request_state == RequestKind::None {
            self.turn_loop();
        }
        self.lc_sync_phase();
        // BattleStream semantics: sendUpdates after every input (flushes the logical count).
        self.send_updates();
        Ok(())
    }

    /// Phase bookkeeping after the turn loop yields (request pending or ended).
    fn lc_sync_phase(&mut self) {
        self.state.phase = if self.state.ended {
            Phase::Ended
        } else if self.state.request_state != RequestKind::None {
            Phase::Choices
        } else {
            Phase::Running
        };
    }

    /// Execute the start action; sim/battle.ts:2674-2707.
    /// PRNG: BattleStart events, switchIn/runSwitch ties and switch-in callbacks.
    pub fn run_start_action(&mut self) {
        for side in 0..2usize {
            let count = self.state.sides[side].pokemon_count;
            if self.state.sides[side].pokemon_left != 0 {
                self.state.sides[side].pokemon_left = count;
            }
            self.add(LogEntry::new(
                "teamsize",
                &[
                    LogArg::SideId(SideId(side as u8)),
                    LogArg::Number(i32::from(count)),
                ],
                &[],
            ));
        }

        self.add(LogEntry::new("start", &[], &[]));

        // getAllPokemon(): p1 team order then p2. Only the zacian/zamazenta *conditions*
        // define onBattleStart; the lookup is by the Pokemon's current species id.
        let all = self.get_all_pokemon();
        for &m in all.as_slice() {
            let (species, cell) = {
                let p = self.lc_mon(m);
                (p.species, p.species_state)
            };
            let state = self.lc_capture(cell);
            self.single_event(
                EventId::BattleStart,
                EffectRef::SpeciesCondition(species),
                Some(state),
                mon_arg(m),
                EventArg::Undefined,
                EffectRef::None,
                Relay::Undefined,
                None,
            );
        }

        // format.onBattleStart and the rules' onBattleStart do not exist in this format.

        for side in 0..2usize {
            for i in 0..2usize {
                let m = self.state.sides[side].party[i];
                if self.state.sides[side].pokemon_left == 0 {
                    // forfeited before starting
                    self.state.sides[side].active[i] = m;
                    self.lc_set_flag(m, mon_flags::FAINTED, true);
                    self.lc_mon_mut(m).hp = 0;
                } else {
                    self.switch_in(m, i as u8, EffectRef::None, false);
                }
            }
        }
        self.state.mid_turn = true;
    }

    /// Pick-team callbacks; sim/battle.ts:1981-2011.
    /// This format has no team preview; preserve direct callback context.
    /// PRNG: callbacks only; scoped format/rules currently draw none here.
    pub fn run_pick_team(&mut self) {
        // format.onTeamPreview is undefined and none of the seven rules defines
        // onTeamPreview; requestState is not 'teampreview' and ruleTable.pickedTeamSize is
        // null, so neither branch of runPickTeam executes (docs/showdown/02 section 1).
    }

    /// Convert one chosen slot into the pending ActionChoice that `side.commitChoices()`
    /// hands to `queue.addChoice` (sim/side.ts:1142-1183; the choice objects of
    /// side.ts:552-1023). Passes resolve to nothing and are skipped.
    fn lc_slot_action_choice(
        &self,
        side: usize,
        i: usize,
        request: RequestKind,
    ) -> Option<ActionChoice> {
        let s = &self.state.sides[side];
        let slot = s.choice.slots[i];
        let pokemon = s.active[i];
        match slot.kind {
            ChoiceKind::Pass => None,
            ChoiceKind::Move => {
                let mut c = ActionChoice::new(ActionKind::Move);
                c.pokemon = pokemon;
                c.move_id = slot.move_id;
                c.move_kind = slot.move_kind;
                // Struggle is pushed without a targetLoc (side.ts:799); everything else carries one.
                c.target_loc = if slot.move_kind == ChosenMoveKind::Dex
                    && slot.move_id == dex::MOVE_STRUGGLE
                {
                    None
                } else {
                    Some(slot.target_loc)
                };
                c.terastallize = slot.tera;
                Some(c)
            }
            ChoiceKind::Switch => {
                let mut c = ActionChoice::new(if request == RequestKind::Switch {
                    ActionKind::InstaSwitch
                } else {
                    ActionKind::Switch
                });
                c.pokemon = pokemon;
                c.target = slot.switch_to;
                Some(c)
            }
            ChoiceKind::Revival => {
                let mut c = ActionChoice::new(ActionKind::RevivalBlessing);
                c.pokemon = pokemon;
                c.target = slot.switch_to;
                Some(c)
            }
        }
    }

    /// Resolve choices before saved midturn queue; sim/battle.ts:2996-3028.
    /// PRNG: updateSpeed, p1 then p2 resolution, sort only new queue, resumed loop.
    pub fn commit_choices(&mut self) -> Result<(), BattleError> {
        self.update_speed();

        // Sometimes you need to make switch choices mid-turn (e.g. U-turn, fainting). The
        // rest of the turn is saved (and not re-sorted); the new choices are sorted and
        // placed before it.
        let old_queue = self.state.queue;
        self.queue_clear();
        if !self.all_choices_done() {
            return Err(BattleError("Not all choices done".into()));
        }

        // inputLog is a harness concern.
        let request = self.state.request_state;
        for side in 0..2usize {
            // Side.commitChoices(): queue.addChoice(this.choice.actions), p1 then p2.
            let n = usize::from(self.state.sides[side].choice.len);
            for i in 0..n {
                if let Some(choice) = self.lc_slot_action_choice(side, i, request) {
                    self.queue_add_choice(choice);
                }
            }
        }
        self.clear_request();

        self.queue_sort();
        let new_len = usize::from(self.state.queue.len);
        let old_len = usize::from(old_queue.len);
        assert!(
            new_len + old_len <= self.state.queue.entries.len(),
            "Action queue capacity exceeded"
        );
        self.state.queue.entries[new_len..new_len + old_len]
            .copy_from_slice(&old_queue.entries[..old_len]);
        self.state.queue.len = (new_len + old_len) as u16;

        self.state.request_state = RequestKind::None;
        // for (const side of this.sides) side.activeRequest = null is part of clear_request.

        self.turn_loop();
        self.lc_sync_phase();
        // The harness flushes after every input (BattleStream); this subsumes the
        // `log.length - sentLogPos > 500` workaround at the end of commitChoices.
        self.send_updates();
        Ok(())
    }

    /// Run until request/end, otherwise endTurn; sim/battle.ts:2936-2955.
    /// PRNG: beforeTurn insertion, action callbacks, residuals and endTurn events.
    pub fn turn_loop(&mut self) {
        self.state.phase = Phase::Running;
        self.add(LogEntry::new("", &[], &[]));
        // add('t:', Math.floor(Date.now() / 1000)): the timestamp is normalized away.
        self.add(LogEntry::new("t:", &[LogArg::Empty], &[]));
        if self.state.request_state != RequestKind::None {
            self.state.request_state = RequestKind::None;
        }

        if !self.state.mid_turn {
            self.queue_insert_choice(ActionChoice::new(ActionKind::BeforeTurn), false);
            self.queue_add_choice(ActionChoice::new(ActionKind::Residual));
            self.state.mid_turn = true;
        }

        while let Some(action) = self.queue_shift() {
            self.run_action(action);
            if self.state.request_state != RequestKind::None || self.state.ended {
                return;
            }
        }

        self.end_turn();
        self.state.mid_turn = false;
        self.queue_clear();
    }

    /// Execute each scoped action and shared postprocessing; battle.ts:2669-2933.
    /// PRNG: delegated action/events, faint processing, forced switches, dynamic queue sort.
    /// True means the action ended in a request or battle end.
    pub fn run_action(&mut self, action: Action) -> bool {
        let original_hp = (action.pokemon != MonId::NONE).then(|| self.lc_mon(action.pokemon).hp);
        let mut residual = ResidualSnapshot::default();
        match action.kind {
            ActionKind::Start => self.run_start_action(),
            ActionKind::Move => {
                if !self.lc_is_active(action.pokemon) {
                    return false;
                }
                if self.lc_is_fainted(action.pokemon) {
                    return false;
                }
                self.lc_run_move_action(&action);
            }
            ActionKind::Terastallize => self.terastallize(action.pokemon),
            ActionKind::BeforeTurnMove => {
                if self.run_before_turn_move_action(action) == Relay::FAIL {
                    return false;
                }
            }
            ActionKind::PriorityChargeMove => {
                if !self.lc_is_active(action.pokemon) || self.lc_is_fainted(action.pokemon) {
                    return false;
                }
                self.run_priority_charge_move_action(action);
            }
            ActionKind::Event => {
                let event = action.event.expect("event action without an event");
                self.run_event(
                    event,
                    mon_arg(action.pokemon),
                    EventArg::Undefined,
                    EffectRef::None,
                    Relay::Undefined,
                    RunEventOptions::default(),
                );
            }
            ActionKind::Pass | ActionKind::None => return false,
            ActionKind::InstaSwitch | ActionKind::Switch => {
                if action.kind == ActionKind::Switch
                    && self.lc_mon(action.pokemon).status != crate::state::Status::None
                {
                    self.single_event(
                        EventId::CheckShow,
                        EffectRef::Dex(dex::ABILITY_NATURALCURE),
                        None,
                        mon_arg(action.pokemon),
                        EventArg::Undefined,
                        EffectRef::None,
                        Relay::Undefined,
                        None,
                    );
                }
                let position = self.lc_mon(action.pokemon).position;
                let result = self.switch_in(
                    action.target,
                    position,
                    dex_effect(action.source_effect),
                    false,
                );
                if result == PURSUIT_FAINT {
                    // gen 5+: the switch is cancelled
                    self.hint(
                        LogArg::Text(
                            "A Pokemon can't switch between when it runs out of HP and when it faints",
                        ),
                        false,
                        None,
                    );
                }
            }
            ActionKind::RevivalBlessing => {
                self.run_revival_blessing(action.pokemon, action.target);
            }
            ActionKind::RunSwitch => self.run_switch(action.pokemon),
            ActionKind::BeforeTurn => {
                self.each_event(EventId::BeforeTurn, EffectRef::None, Relay::Undefined);
            }
            ActionKind::Residual => {
                residual = self.run_residual();
            }
        }
        self.process_action_boundary(action, original_hp, residual)
    }

    /// `actions.runMove(action.move, ...)` for a queued move: materialise the use's scratch
    /// frame from the queue entry (priority / Prankster / ignoreAbility edits made while
    /// the action sat in the queue), run it, and release the frame afterwards.
    fn lc_run_move_action(&mut self, action: &Action) {
        let handle = self.lc_move_frame(action.move_data);
        let target_loc = action.target_loc_present.then_some(action.target_loc);
        let original_target =
            (action.original_target != MonId::NONE).then_some(action.original_target);
        self.run_move(
            MoveInput::Active(handle),
            action.pokemon,
            target_loc,
            dex_effect(action.source_effect),
            RunMoveOptions {
                external: false,
                original_target,
            },
        );
        self.release_active_move(handle);
    }

    /// Residual action's clearActiveMove/updateSpeed/fieldEvent sequence;
    /// sim/battle.ts:2815-2822. PRNG: event ordering/tie shuffles and callbacks.
    /// Capture original HP per active before residuals for EmergencyExit.
    pub fn run_residual(&mut self) -> ResidualSnapshot {
        self.add(LogEntry::new("", &[], &[]));
        self.clear_active_move(true);
        self.update_speed();
        let mut snapshot = ResidualSnapshot::default();
        let actives = self.get_all_active(false);
        for &m in actives.as_slice() {
            snapshot.entries[usize::from(snapshot.len)] = (m, self.lc_mon(m).hp);
            snapshot.len += 1;
        }
        self.field_event(EventId::Residual, None);
        if !self.state.ended {
            self.add(LogEntry::new("upkeep", &[], &[]));
        }
        snapshot
    }

    /// Advance turn and rebuild trapping/disabling/requests; battle.ts:1627-1805.
    /// PRNG: DisableMove, TrapPokemon/MaybeTrapPokemon and hypothetical ability events.
    /// Gen2/3 Quick Claw draws are absent from this Gen9 path.
    pub fn end_turn(&mut self) {
        self.state.turn = self
            .state
            .turn
            .checked_add(1)
            .expect("turn counter overflow");
        self.state.last_successful_move = EffectId::NONE;

        // Dynamax ending, gen-1 partial trapping and staleness bookkeeping (only consumed by
        // the Endless Battle Clause, which this format lacks) have no effect here.
        for side in 0..2usize {
            for slot in 0..2usize {
                let pokemon = self.state.sides[side].active[slot];
                if pokemon == MonId::NONE {
                    continue;
                }
                self.lc_end_turn_pokemon(pokemon);
            }
            let s = &mut self.state.sides[side];
            s.fainted_last_turn = s.fainted_this_turn;
            s.fainted_this_turn = ResultFlag::Null;
        }

        if self.check_turn_limit() {
            return;
        }

        // Triples centering and multi-battle Dynamax announcements do not apply.
        self.add(LogEntry::new(
            "turn",
            &[LogArg::Number(i32::from(self.state.turn))],
            &[],
        ));
        // gen 2 / gen 3 quickClawRoll: absent.
        self.lc_make_request(RequestKind::Move);
    }

    /// `this.dex.getImmunity('trapped', pokemon)`: true unless a Type of the Pokemon is
    /// immune to trapping (Ghost). Calls `pokemon.getTypes()`, i.e. the Type event.
    fn lc_not_immune_to_trapped(&mut self, pokemon: MonId) -> bool {
        let types = self.get_types(pokemon, false, false);
        let col = dex::IMMUNITY_NAMES
            .iter()
            .position(|&n| n == "trapped")
            .expect("trapped immunity column");
        types.values[..usize::from(types.len)]
            .iter()
            .all(|t| dex::IMMUNITY_CHART[usize::from(t.0) - 1][col] != 3)
    }

    /// The per-Pokemon body of endTurn's side loop (battle.ts:1658-1759).
    fn lc_end_turn_pokemon(&mut self, pokemon: MonId) {
        let turn = self.state.turn;
        {
            let p = self.lc_mon_mut(pokemon);
            p.flags &= !mon_flags::NEWLY_SWITCHED;
            p.move_last_turn_result = p.move_this_turn_result;
            p.move_this_turn_result = ResultFlag::Undefined;
            if turn != 1 {
                p.flags &=
                    !(USED_ITEM_THIS_TURN | mon_flags::STATS_RAISED | mon_flags::STATS_LOWERED);
            }
            p.flags &= !(mon_flags::MAYBE_DISABLED | mon_flags::MAYBE_LOCKED);
        }
        for i in 0..self.lc_slot_count(pokemon) {
            let ms = self.lc_slot_mut(pokemon, i);
            ms.flags &= !(SLOT_DISABLED | SLOT_HIDDEN);
            ms.disabled_source = EffectId::NONE;
        }
        self.run_event(
            EventId::DisableMove,
            mon_arg(pokemon),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            RunEventOptions::default(),
        );
        for i in 0..self.lc_slot_count(pokemon) {
            let id = self.lc_mon(pokemon).move_slots()[i].id;
            // dex.getActiveMove(id) is only used for its static callback and flags here.
            self.single_event(
                EventId::DisableMove,
                EffectRef::Dex(id),
                None,
                mon_arg(pokemon),
                EventArg::Undefined,
                EffectRef::None,
                Relay::Undefined,
                None,
            );
            let last_move = self.lc_mon(pokemon).last_move;
            if dex::move_data(id).flags & dex::FLAG_CANTUSETWICE != 0 && last_move == id {
                self.disable_move(pokemon, last_move, false, EffectRef::None);
            }
        }

        // If it was an illusion, it's not any more (gen >= 7).
        if self.state.last_attacked_by(pokemon).is_some() {
            self.lc_set_flag(pokemon, mon_flags::KNOWN_TYPE, true);
        }

        self.state.prune_attacks(pokemon);
        self.state.seq_at_last_end_turn = self.state.attack_seq;

        if self.lc_mon(pokemon).terastallized == TypeId::NONE {
            // In Gen 7+, the real type of every Pokemon is visible to all players.
            let illusion = self.lc_mon(pokemon).illusion;
            let seen = if illusion != MonId::NONE {
                illusion
            } else {
                pokemon
            };
            let real = self.get_types(seen, true, false);
            let apparent = self.lc_mon(seen).apparent_types;
            let apparent_len = apparent.iter().take_while(|t| **t != TypeId::NONE).count();
            let same = usize::from(real.len) == apparent_len
                && real.values[..apparent_len] == apparent[..apparent_len];
            if !same {
                let ty = &real.values;
                let parts2 = [LogArg::Type(ty[0]), LogArg::Text("/"), LogArg::Type(ty[1])];
                let parts3 = [
                    LogArg::Type(ty[0]),
                    LogArg::Text("/"),
                    LogArg::Type(ty[1]),
                    LogArg::Text("/"),
                    LogArg::Type(ty[2]),
                ];
                let one = [LogArg::Type(ty[0])];
                let joined = match real.len {
                    1 => LogArg::Parts(&one),
                    2 => LogArg::Parts(&parts2),
                    3 => LogArg::Parts(&parts3),
                    n => panic!("getTypes returned {n} types"),
                };
                self.add(LogEntry::new(
                    "-start",
                    &[LogArg::Mon(pokemon), LogArg::Text("typechange"), joined],
                    &[LogTag::Bare("silent")],
                ));
                assert!(real.len <= 2, "apparentType capacity exceeded");
                self.lc_mon_mut(seen).apparent_types = [
                    real.values[0],
                    if real.len > 1 {
                        real.values[1]
                    } else {
                        TypeId::NONE
                    },
                ];
                let added = self.lc_mon(pokemon).added_type;
                if added != TypeId::NONE {
                    // The typechange message removes the added type, so put it back.
                    self.add(LogEntry::new(
                        "-start",
                        &[
                            LogArg::Mon(pokemon),
                            LogArg::Text("typeadd"),
                            LogArg::Type(added),
                        ],
                        &[LogTag::Bare("silent")],
                    ));
                }
            }
        }

        {
            let p = self.lc_mon_mut(pokemon);
            p.trapped = Trapped::No;
            p.flags &= !mon_flags::MAYBE_TRAPPED;
        }
        self.run_event(
            EventId::TrapPokemon,
            mon_arg(pokemon),
            EventArg::Undefined,
            EffectRef::None,
            Relay::Undefined,
            RunEventOptions::default(),
        );
        if !self.lc_flag(pokemon, mon_flags::KNOWN_TYPE) || self.lc_not_immune_to_trapped(pokemon) {
            self.run_event(
                EventId::MaybeTrapPokemon,
                mon_arg(pokemon),
                EventArg::Undefined,
                EffectRef::None,
                Relay::Undefined,
                RunEventOptions::default(),
            );
        }
        // Canceling switches would leak information if a foe might have a trapping ability.
        let foes = self.foes(pokemon, false);
        for hit in &foes.entries[..usize::from(foes.len)] {
            let HitTarget::Pokemon(source) = *hit else {
                continue;
            };
            let illusion = self.lc_mon(source).illusion;
            let seen = if illusion != MonId::NONE {
                illusion
            } else {
                source
            };
            let species = dex::species(self.lc_mon(seen).species);
            for slot in &species.abilities {
                if slot.key.is_empty() {
                    continue; // no such ability slot
                }
                // `abilityName === source.ability` compares a display name with an id.
                let source_ability = self.lc_mon(source).ability;
                if source_ability != EffectId::NONE && slot.name == dex::effect(source_ability).key
                {
                    continue;
                }
                // 'obtainableabilities' is present and the format has a team generator, and no
                // gen-9 species has unreleasedHidden or a '-ability:' ban, so nothing else skips.
                if self.lc_flag(pokemon, mon_flags::KNOWN_TYPE)
                    && !self.lc_not_immune_to_trapped(pokemon)
                {
                    continue;
                }
                if slot.id != EffectId::NONE {
                    self.single_event(
                        EventId::FoeMaybeTrapPokemon,
                        EffectRef::Dex(slot.id),
                        None,
                        mon_arg(pokemon),
                        mon_arg(source),
                        EffectRef::None,
                        Relay::Undefined,
                        None,
                    );
                }
                // Abilities outside the executable closure have no FoeMaybeTrapPokemon
                // callback (verified by codegen), so their singleEvent is a pure no-op.
            }
        }

        if self.lc_is_fainted(pokemon) {
            return;
        }
        self.lc_mon_mut(pokemon).active_turns += 1;
    }

    /// Enforce warnings and auto-tie after turn 1000; battle.ts:1807-1907.
    /// PRNG: none; Endless Battle Clause itself is not enabled in this format.
    pub fn check_turn_limit(&mut self) -> bool {
        let turn = i32::from(self.state.turn);
        if turn <= 100 {
            return false;
        }
        // the turn limit is not a part of Endless Battle Clause
        if turn > 1000 {
            self.add(LogEntry::new(
                "message",
                &[LogArg::Text(
                    "It is turn 1000. You have hit the turn limit!",
                )],
                &[],
            ));
            self.tie();
            return true;
        }
        if (turn >= 500 && turn % 100 == 0) || (turn >= 900 && turn % 10 == 0) || turn >= 990 {
            let turns_left = 1000 - turn;
            let count = [LogArg::Number(turns_left), LogArg::Text(" turns")];
            let one = [LogArg::Text("1 turn")];
            let text = if turns_left == 1 {
                &one[..]
            } else {
                &count[..]
            };
            let parts = [
                LogArg::Text("You will auto-tie if the battle doesn't end in "),
                LogArg::Parts(text),
                LogArg::Text(" (on turn 1000)."),
            ];
            self.add(LogEntry::new("bigerror", &[LogArg::Parts(&parts)], &[]));
        }
        // ruleTable.has('endlessbattleclause') is false in this format.
        false
    }

    /// Update every non-fainted active's cached speed; battle.ts:391-395.
    /// PRNG: getStat/getActionSpeed event sorting and handlers only.
    pub fn update_speed(&mut self) {
        let actives = self.get_all_active(false);
        for &m in actives.as_slice() {
            self.update_pokemon_speed(m);
        }
    }

    /// Post-action forced switches and request boundary; battle.ts:2825-2933.
    /// PRNG: dragIn sampling, Update/EmergencyExit/BeforeSwitchOut events and queue sort.
    pub fn process_action_boundary(
        &mut self,
        action: Action,
        original_hp: Option<u16>,
        residual: ResidualSnapshot,
    ) -> bool {
        // phazing (Roar, etc): iterate the live active arrays by index like a JS for..of
        for side in 0..2usize {
            for slot in 0..2usize {
                let pokemon = self.state.sides[side].active[slot];
                if pokemon == MonId::NONE {
                    continue;
                }
                if self.lc_flag(pokemon, mon_flags::FORCE_SWITCH) {
                    if self.lc_mon(pokemon).hp != 0 {
                        let position = self.lc_mon(pokemon).position;
                        self.drag_in(SideId(side as u8), position);
                    }
                    self.lc_set_flag(pokemon, mon_flags::FORCE_SWITCH, false);
                }
            }
        }

        self.clear_active_move(false);

        // fainting
        self.faint_messages(false, false, true);
        if self.state.ended {
            return true;
        }

        // switching (fainted pokemon, U-turn, Baton Pass, etc)
        match self.queue_peek(false) {
            None => self.check_fainted(),
            // (the gen-7 mega branch is outside this format)
            Some(next) if next.kind == ActionKind::InstaSwitch => return false,
            Some(_) => {}
        }

        if action.kind != ActionKind::Start {
            self.each_event(EventId::Update, EffectRef::None, Relay::Undefined);
            for i in 0..usize::from(residual.len) {
                let (pokemon, original) = residual.entries[i];
                self.run_event(
                    EventId::EmergencyExit,
                    mon_arg(pokemon),
                    EventArg::Undefined,
                    EffectRef::None,
                    Relay::Number(f64::from(original)),
                    RunEventOptions::default(),
                );
            }
        }

        if action.kind == ActionKind::RunSwitch {
            self.run_event(
                EventId::EmergencyExit,
                mon_arg(action.pokemon),
                EventArg::Undefined,
                EffectRef::None,
                Relay::Number(f64::from(
                    original_hp.expect("runSwitch without an acting Pokemon"),
                )),
                RunEventOptions::default(),
            );
        }

        let mut switches = [false; 2];
        for (side, flag) in switches.iter_mut().enumerate() {
            *flag = self.state.sides[side]
                .active
                .iter()
                .any(|&m| m != MonId::NONE && self.lc_switch_flag(m));
        }

        for i in 0..2usize {
            // Used to ignore the fake switch for Revival Blessing
            let mut revive_switch = false;
            if switches[i] && self.can_switch(SideId(i as u8)) == 0 {
                for slot in 0..2usize {
                    let pokemon = self.state.sides[i].active[slot];
                    if pokemon == MonId::NONE {
                        continue;
                    }
                    let position = self.lc_mon(pokemon).position;
                    if self.lc_has_revival_slot(SideId(i as u8), position) {
                        revive_switch = true;
                        continue;
                    }
                    self.lc_clear_switch_flag(pokemon);
                }
                if !revive_switch {
                    switches[i] = false;
                }
            } else if switches[i] {
                for slot in 0..2usize {
                    let pokemon = self.state.sides[i].active[slot];
                    if pokemon == MonId::NONE {
                        continue;
                    }
                    if self.lc_mon(pokemon).hp != 0
                        && self.lc_switch_flag(pokemon)
                        && !self.lc_switch_flag_is_revival(pokemon)
                        && !self.lc_flag(pokemon, mon_flags::SKIP_BEFORE_SWITCH_OUT)
                    {
                        self.run_event(
                            EventId::BeforeSwitchOut,
                            mon_arg(pokemon),
                            EventArg::Undefined,
                            EffectRef::None,
                            Relay::Undefined,
                            RunEventOptions::default(),
                        );
                        self.lc_set_flag(pokemon, mon_flags::SKIP_BEFORE_SWITCH_OUT, true);
                        // Pokemon may have fainted in BeforeSwitchOut
                        self.faint_messages(false, false, true);
                        if self.state.ended {
                            return true;
                        }
                        if self.lc_is_fainted(pokemon) {
                            switches[i] = self.state.sides[i]
                                .active
                                .iter()
                                .any(|&m| m != MonId::NONE && self.lc_switch_flag(m));
                        }
                    }
                }
            }
        }

        for player_switch in switches {
            if player_switch {
                self.lc_make_request(RequestKind::Switch);
                return true;
            }
        }

        // gen < 5 Update is outside this format. Gen 8+: speed is updated dynamically, so
        // update the queue's speed properties and sort it.
        if self
            .queue_peek(false)
            .is_some_and(|a| a.kind == ActionKind::Move)
        {
            self.update_speed();
            let mut i = 0;
            while i < usize::from(self.state.queue.len) {
                let mut queue_action = self.state.queue.entries[i];
                if queue_action.pokemon != MonId::NONE {
                    self.get_action_speed(&mut queue_action);
                    // getActionSpeed mutates the queued object in place.
                    self.state.queue.entries[i] = queue_action;
                }
                i += 1;
            }
            self.queue_sort();
        }
        false
    }

    /// Revival action revives at half HP and can queue instaswitch;
    /// battle.ts:2785-2802. PRNG: queued instaswitch resolution/events; no direct draw.
    pub fn run_revival_blessing(&mut self, user: MonId, target: MonId) {
        let side = user.side();
        self.state.sides[usize::from(side.0)].pokemon_left += 1;
        if self.lc_mon(target).position < 2 {
            let mut c = ActionChoice::new(ActionKind::InstaSwitch);
            c.pokemon = target;
            c.target = target;
            self.queue_add_choice(c);
        }
        {
            let t = self.lc_mon_mut(target);
            t.flags &= !(mon_flags::FAINTED | mon_flags::FAINT_QUEUED);
            t.status = crate::state::Status::None;
            t.hp = 1; // Needed so hp functions works
        }
        let half = f64::from(self.lc_mon(target).max_hp) / 2.0;
        self.set_hp(target, half);
        self.add(LogEntry::split(
            "-heal",
            &[LogArg::Mon(target), LogArg::Health(target)],
            &[LogTag::From(EffectRef::Dex(dex::MOVE_REVIVALBLESSING))],
            side,
            false,
        ));
        let position = self.lc_mon(user).position;
        self.remove_slot_condition(SlotId::new(side, position), dex::CONDITION_REVIVALBLESSING);
    }

    /// BeforeTurnMove action wrapper; battle.ts:2734-2742.
    /// Delegate active/fainted checks and direct callback to M's run_before_turn_move.
    /// PRNG: getTarget may sample, then direct callback; preserve parent event/effect frame.
    ///
    /// Returns `Relay::FAIL` for each of the source's three early `return false` paths
    /// (inactive, fainted, no target). Owner M's `run_before_turn_move` resolves the target
    /// from `target_loc` and must signal "no target" with `Relay::FAIL`; any other relay
    /// (the callback's own undefined) means the callback ran.
    pub fn run_before_turn_move_action(&mut self, action: Action) -> Relay {
        if !self.lc_is_active(action.pokemon) {
            return Relay::FAIL;
        }
        if self.lc_is_fainted(action.pokemon) {
            return Relay::FAIL;
        }
        let input = match action.move_data.kind {
            ChosenMoveKind::Dex => MoveInput::Dex(action.move_data.id),
            ChosenMoveKind::Recharge => MoveInput::Recharge,
        };
        let target_loc = action.target_loc_present.then_some(action.target_loc);
        self.run_before_turn_move(input, action.pokemon, target_loc)
    }

    /// PriorityChargeMove action wrapper; battle.ts:2743-2749.
    /// Delegate callback execution to M's run_priority_charge_move.
    /// PRNG: direct callback only; preserve the current event/effect frame.
    pub fn run_priority_charge_move_action(&mut self, action: Action) -> Relay {
        let input = match action.move_data.kind {
            ChosenMoveKind::Dex => MoveInput::Dex(action.move_data.id),
            ChosenMoveKind::Recharge => MoveInput::Recharge,
        };
        self.run_priority_charge_move(input, action.pokemon)
    }
}
