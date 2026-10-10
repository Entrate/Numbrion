//! BattleQueue port. OWNER L. Priority comparison and tie sorting belong to event/.
//!
//! The queue is `state.queue` (a Copy `ActionQueue`), so a saved mid-turn queue survives
//! requests. A JS `ActiveMove` attached to a queued action becomes a [`QueuedMove`]
//! (`id`, kind, the queue-time `priority` write and the Prankster / ignoreAbility bits);
//! every time Showdown touches `action.move` we materialise a scratch move frame from
//! it, run the events, copy the edits back and release the frame.
//!
//! Scoped-data fact used below: no move in the 359-move closure has a
//! `beforeTurnCallback` or `priorityChargeCallback` (Counter, Mirror Coat, Focus Punch,
//! Beak Blast, Shell Trap, Chilly Reception, Pursuit are all outside `data/scope.json`),
//! so those two sub-actions are never produced by `resolve_action`. Their execution
//! wrappers still exist in `turn.rs` and delegate to owner M as specified.

use crate::{
    Battle,
    actions::{MoveHandle, MoveInput},
    dex,
    event::{EffectRef, EventArg, Relay, RunEventOptions, SortOrder, compare_priority, speed_sort},
    ids::{EffectId, EventId, MonId},
    log::LogSink,
    state::{
        choices::{
            Action, ActionKind, ChosenMoveKind, QUEUE_CAPACITY, QueuedMove, queued_move_flags,
        },
        scratch::move_runtime,
    },
};

use super::util::{mon_arg, relay_number};

/// Unresolved ActionChoice (`sim/battle-queue.ts:115-120,166-277`).
/// None target_loc preserves JS undefined; explicit zero means random target.
/// PRNG: constructing this value draws nothing; resolve_action may draw.
#[derive(Clone, Copy, Debug)]
pub struct ActionChoice {
    pub kind: ActionKind,
    pub pokemon: MonId,
    pub target: MonId,
    pub move_id: EffectId,
    pub move_kind: ChosenMoveKind,
    pub target_loc: Option<i8>,
    pub source_effect: EffectRef,
    pub terastallize: bool,
    pub event: Option<EventId>,
}

impl ActionChoice {
    /// A choice carrying only its kind (`{choice: 'residual'}` and friends).
    pub const fn new(kind: ActionKind) -> Self {
        Self {
            kind,
            pokemon: MonId::NONE,
            target: MonId::NONE,
            move_id: EffectId::NONE,
            move_kind: ChosenMoveKind::Dex,
            target_loc: None,
            source_effect: EffectRef::None,
            terastallize: false,
            event: None,
        }
    }

    /// `{choice, pokemon}` (runSwitch, terastallize, ...).
    pub const fn for_pokemon(kind: ActionKind, pokemon: MonId) -> Self {
        let mut c = Self::new(kind);
        c.pokemon = pokemon;
        c
    }

    /// `{choice: 'move', pokemon, moveid, targetLoc}` as built by Instruct, After You...
    pub const fn for_move(pokemon: MonId, move_id: EffectId, target_loc: Option<i8>) -> Self {
        let mut c = Self::new(ActionKind::Move);
        c.pokemon = pokemon;
        c.move_id = move_id;
        c.target_loc = target_loc;
        c
    }
}

/// Gen9 expansion: beforeTurnMove, Tera, priorityChargeMove and main move.
/// `sim/battle-queue.ts:197-244`; PRNG: only resolving entries draws.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedActions {
    pub entries: [Action; 4],
    pub len: u8,
}

impl Default for ResolvedActions {
    fn default() -> Self {
        Self {
            entries: [Action::default(); 4],
            len: 0,
        }
    }
}

impl ResolvedActions {
    /// Populated entries in queue order (`[priorityChargeMove, terastallize,
    /// beforeTurnMove, move]` for a full move choice).
    pub fn as_slice(&self) -> &[Action] {
        &self.entries[..usize::from(self.len)]
    }

    fn push(&mut self, action: Action) {
        self.entries[usize::from(self.len)] = action;
        self.len += 1;
    }
}

/// Ephemeral index for a retrieved action (`sim/battle-queue.ts:315-365`).
/// Invalid after any queue mutation; copy the Action before calling events.
/// PRNG: none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct QueueHandle(pub u8);

/// `ORDERS` of `resolveAction` (battle-queue.ts:174-195). `pass` never reaches here.
fn action_order(kind: ActionKind) -> u16 {
    match kind {
        ActionKind::Start => 2,
        ActionKind::InstaSwitch => 3,
        ActionKind::BeforeTurn => 4,
        ActionKind::BeforeTurnMove => 5,
        ActionKind::RevivalBlessing => 6,
        ActionKind::RunSwitch => 101,
        ActionKind::Switch => 103,
        ActionKind::Terastallize => 106,
        ActionKind::PriorityChargeMove => 107,
        // 'shift' is 200 as well; 'event' and every move choice fall to the default 200.
        ActionKind::Move | ActionKind::Event => 200,
        ActionKind::Residual => 300,
        ActionKind::Pass | ActionKind::None => panic!("Unexpected orderless action {kind:?}"),
    }
}

/// True for the kinds that carry `action.move` (move, beforeTurnMove, priorityChargeMove).
fn has_move(kind: ActionKind) -> bool {
    matches!(
        kind,
        ActionKind::Move | ActionKind::BeforeTurnMove | ActionKind::PriorityChargeMove
    )
}

/// `move.beforeTurnCallback` presence. No scoped move defines one (see module docs), so
/// this is a verified constant, not a stub.
fn move_has_before_turn_callback(_move_id: EffectId) -> bool {
    false
}

/// `move.priorityChargeCallback` presence. No scoped move defines one (see module docs).
fn move_has_priority_charge_callback(_move_id: EffectId) -> bool {
    false
}

fn move_input(id: EffectId, kind: ChosenMoveKind) -> MoveInput {
    match kind {
        ChosenMoveKind::Dex => MoveInput::Dex(id),
        ChosenMoveKind::Recharge => MoveInput::Recharge,
    }
}

/// `dex.moves.get(move.id).priority`; the nonexistent Recharge move has priority 0.
fn base_priority(id: EffectId, kind: ChosenMoveKind) -> i8 {
    match kind {
        ChosenMoveKind::Dex => dex::move_data(id).priority,
        ChosenMoveKind::Recharge => 0,
    }
}

impl<L: LogSink> Battle<L> {
    fn lc_effect_id(&self, effect: EffectRef) -> EffectId {
        match effect {
            EffectRef::Dex(id) => id,
            EffectRef::ActiveMove(i) => {
                self.scratch.moves[i as usize]
                    .expect("released move frame")
                    .id
            }
            _ => EffectId::NONE,
        }
    }

    /// Materialise the scratch ActiveMove for a queued move (`dex.getActiveMove(moveid)` plus
    /// the edits earlier ModifyPriority passes made to that same object).
    pub(super) fn lc_move_frame(&mut self, mv: QueuedMove) -> MoveHandle {
        let handle = self.get_active_move(move_input(mv.id, mv.kind));
        let m = self.active_move_mut(handle);
        m.priority = mv.priority;
        if mv.flags & queued_move_flags::PRANKSTER_BOOSTED != 0 {
            m.runtime_flags |= move_runtime::PRANKSTER_BOOSTED;
        }
        if mv.flags & queued_move_flags::IGNORE_ABILITY != 0 {
            m.runtime_flags |= move_runtime::IGNORE_ABILITY;
        }
        handle
    }

    /// Copy the queue-visible ActiveMove edits (priority, Prankster, ignoreAbility) back.
    fn lc_read_frame(&self, handle: MoveHandle, mv: &mut QueuedMove) {
        let m = self.active_move(handle);
        mv.priority = m.priority;
        let mut flags = 0;
        if m.runtime_flags & move_runtime::PRANKSTER_BOOSTED != 0 {
            flags |= queued_move_flags::PRANKSTER_BOOSTED;
        }
        if m.runtime_flags & move_runtime::IGNORE_ABILITY != 0 {
            flags |= queued_move_flags::IGNORE_ABILITY;
        }
        mv.flags = flags;
    }

    /// Fill priority/speed/targets and expand actions; battle-queue.ts:166-277.
    /// PRNG: recursive pre-actions, FractionalPriority, random targets, then speed events.
    pub fn resolve_action(&mut self, choice: ActionChoice, mid_turn: bool) -> ResolvedActions {
        let mut out = ResolvedActions::default();
        if choice.kind == ActionKind::Pass {
            return out;
        }
        // `action.order = orders[choice]`; unknown choices throw "Unexpected orderless action".
        let order = action_order(choice.kind);

        let mut action = Action {
            kind: choice.kind,
            pokemon: choice.pokemon,
            target: choice.target,
            source_effect: self.lc_effect_id(choice.source_effect),
            event: choice.event,
            order,
            target_loc: choice.target_loc.unwrap_or(0),
            target_loc_present: choice.target_loc.is_some(),
            ..Action::default()
        };
        if choice.terastallize {
            // `terastallize: P.teraType` on the move action that spawns the tera action.
            action.tera = self.lc_tera_type(choice.pokemon);
        }
        if has_move(choice.kind) {
            action.move_data = QueuedMove {
                id: choice.move_id,
                flags: 0,
                priority: base_priority(choice.move_id, choice.move_kind),
                kind: choice.move_kind,
            };
        }

        let mut before_turn_move = None;
        let mut terastallize = None;
        let mut priority_charge = None;
        let mut frame: Option<MoveHandle> = None;
        if !mid_turn {
            if choice.kind == ActionKind::Move {
                // battle-queue.ts:213-218 (no maxMove / zmove in this format).
                if move_has_before_turn_callback(choice.move_id) {
                    let mut c = ActionChoice::new(ActionKind::BeforeTurnMove);
                    c.pokemon = choice.pokemon;
                    c.move_id = choice.move_id;
                    c.move_kind = choice.move_kind;
                    c.target_loc = choice.target_loc;
                    before_turn_move = self.resolve_action(c, false).as_slice().first().copied();
                }
                // battle-queue.ts:229-233.
                if choice.terastallize
                    && self.lc_mon(choice.pokemon).terastallized == crate::ids::TypeId::NONE
                {
                    terastallize = self
                        .resolve_action(
                            ActionChoice::for_pokemon(ActionKind::Terastallize, choice.pokemon),
                            false,
                        )
                        .as_slice()
                        .first()
                        .copied();
                }
                // battle-queue.ts:241-246: NB no targetLoc is passed.
                if move_has_priority_charge_callback(choice.move_id) {
                    let mut c = ActionChoice::new(ActionKind::PriorityChargeMove);
                    c.pokemon = choice.pokemon;
                    c.move_id = choice.move_id;
                    c.move_kind = choice.move_kind;
                    priority_charge = self.resolve_action(c, false).as_slice().first().copied();
                }
                // battle-queue.ts:247: relayVar 0, not the move's priority.
                let handle = self.lc_move_frame(action.move_data);
                frame = Some(handle);
                let fractional = self.run_event(
                    EventId::FractionalPriority,
                    mon_arg(choice.pokemon),
                    EventArg::Null,
                    EffectRef::ActiveMove(handle.0),
                    Relay::Number(0.0),
                    RunEventOptions::default(),
                );
                action.fractional_priority = relay_number(fractional, "FractionalPriority");
            } else if matches!(choice.kind, ActionKind::Switch | ActionKind::InstaSwitch) {
                // battle-queue.ts:248-253: a String switchFlag (U-turn...) names the effect.
                if let Some(id) = self.lc_switch_flag_move(choice.pokemon) {
                    action.source_effect = id;
                }
                self.lc_clear_switch_flag(choice.pokemon);
            }
        } else if choice.kind == ActionKind::Move {
            // `action.fractionalPriority` stays undefined (gen-7 mega re-insert only); the
            // addition in getActionSpeed then yields NaN exactly as in JS.
            action.fractional_priority = f64::NAN;
        }

        if has_move(choice.kind) {
            // `action.move = getActiveMove(action.move)`: same object, so reuse the frame.
            let handle = match frame {
                Some(h) => h,
                None => self.lc_move_frame(action.move_data),
            };
            frame = Some(handle);
            if !(action.target_loc_present && action.target_loc != 0) {
                let input = MoveInput::Active(handle);
                if let Some(target) = self.get_random_target(choice.pokemon, input) {
                    action.target_loc = self.get_loc_of(choice.pokemon, target);
                    action.target_loc_present = true;
                }
            }
            let loc = if action.target_loc_present {
                action.target_loc
            } else {
                0
            };
            action.original_target = self.get_at_loc(choice.pokemon, loc).unwrap_or(MonId::NONE);
        }
        self.lc_action_speed(&mut action, frame);
        if let Some(h) = frame {
            self.release_active_move(h);
        }

        // `actions.unshift(...)` in the order beforeTurnMove, tera, priorityChargeMove.
        if let Some(a) = priority_charge {
            out.push(a);
        }
        if let Some(a) = terastallize {
            out.push(a);
        }
        if let Some(a) = before_turn_move {
            out.push(a);
        }
        out.push(action);
        out
    }

    /// Append resolved choices; battle-queue.ts:307-313.
    /// PRNG: resolve_action only; no independent sort or insertion roll.
    pub fn queue_add_choice(&mut self, choice: ActionChoice) {
        let resolved = self.resolve_action(choice, false);
        for a in resolved.as_slice() {
            self.queue_push(*a);
        }
    }

    /// Insert without resorting existing actions; battle-queue.ts:369-403.
    /// PRNG: updateSpeed/resolve_action; one random(first,last+1) for a tied interval.
    pub fn queue_insert_choice(&mut self, choice: ActionChoice, mid_turn: bool) {
        if choice.pokemon != MonId::NONE {
            self.update_pokemon_speed(choice.pokemon);
        }
        let resolved = self.resolve_action(choice, mid_turn);
        let len = usize::from(self.state.queue.len);
        let Some(first_action) = resolved.as_slice().first().copied() else {
            // `actions[0]` is undefined: JS throws inside comparePriority unless the list is
            // empty, in which case the empty spread pushes nothing.
            assert_eq!(len, 0, "insertChoice of an action that resolves to nothing");
            return;
        };
        let key = <Action as crate::event::SpeedSortable>::sort_key(&first_action);
        let mut first_index: Option<usize> = None;
        let mut last_index: Option<usize> = None;
        for i in 0..len {
            let compared = compare_priority(
                key,
                <Action as crate::event::SpeedSortable>::sort_key(&self.state.queue.entries[i]),
                SortOrder::Priority,
            );
            if compared <= 0.0 && first_index.is_none() {
                first_index = Some(i);
            }
            if compared < 0.0 {
                last_index = Some(i);
                break;
            }
        }
        match first_index {
            None => {
                for a in resolved.as_slice() {
                    self.queue_push(*a);
                }
            }
            Some(first) => {
                let last = last_index.unwrap_or(len);
                let index = if first == last {
                    first
                } else {
                    self.state.prng.random_range(first as u32, last as u32 + 1) as usize
                };
                self.lc_queue_splice(index, resolved.as_slice());
            }
        }
    }

    /// `list.splice(index, 0, ...actions)`.
    fn lc_queue_splice(&mut self, index: usize, actions: &[Action]) {
        let q = &mut self.state.queue;
        let len = usize::from(q.len);
        assert!(index <= len);
        assert!(
            len + actions.len() <= QUEUE_CAPACITY,
            "Action queue capacity exceeded"
        );
        q.entries.copy_within(index..len, index + actions.len());
        q.entries[index..index + actions.len()].copy_from_slice(actions);
        q.len = (len + actions.len()) as u16;
    }

    /// Selection-sort queue with exact tie shuffles; battle-queue.ts:418-422.
    /// PRNG: event::speed_sort draws per Fisher-Yates step on ties, including no-op swaps.
    pub fn queue_sort(&mut self) {
        let len = usize::from(self.state.queue.len);
        speed_sort(
            &mut self.state.prng,
            &mut self.state.queue.entries[..len],
            SortOrder::Priority,
        );
    }

    /// Prioritize existing or newly resolved action; battle-queue.ts:282-293.
    /// A handle removes that existing entry; None inserts a fresh copied action.
    /// PRNG: none. Assign sourceEffect, order=3 and unshift exactly once.
    pub fn queue_prioritize_action(
        &mut self,
        action: Action,
        existing: Option<QueueHandle>,
        source_effect: EffectRef,
    ) {
        if let Some(handle) = existing {
            self.lc_queue_remove(usize::from(handle.0));
        }
        let mut action = action;
        // `action.sourceEffect = sourceEffect` (undefined when omitted).
        action.source_effect = self.lc_effect_id(source_effect);
        action.order = 3;
        self.queue_unshift(action);
    }

    /// Cancel actor actions and insert replacement; battle-queue.ts:301-305.
    /// PRNG: queue_insert_choice and its nested events/insertion tie roll.
    pub fn queue_change_action(&mut self, pokemon: MonId, choice: ActionChoice) {
        self.queue_cancel_action(pokemon);
        let mut choice = choice;
        if choice.pokemon == MonId::NONE {
            choice.pokemon = pokemon;
        }
        self.queue_insert_choice(choice, false);
    }

    /// Earliest move/switch/instaswitch; battle-queue.ts:315-322.
    /// PRNG: none; returned index is invalid after a queue mutation.
    pub fn queue_will_act(&self) -> Option<QueueHandle> {
        let q = &self.state.queue;
        q.entries[..usize::from(q.len)]
            .iter()
            .position(|a| {
                matches!(
                    a.kind,
                    ActionKind::Move | ActionKind::Switch | ActionKind::InstaSwitch
                )
            })
            .map(|i| QueueHandle(i as u8))
    }

    /// Actor's next move, unless fainted; battle-queue.ts:324-332.
    /// PRNG: none; returned index is invalid after a queue mutation.
    pub fn queue_will_move(&self, pokemon: MonId) -> Option<QueueHandle> {
        if self.lc_is_fainted(pokemon) {
            return None;
        }
        let q = &self.state.queue;
        q.entries[..usize::from(q.len)]
            .iter()
            .position(|a| a.kind == ActionKind::Move && a.pokemon == pokemon)
            .map(|i| QueueHandle(i as u8))
    }

    /// Actor's switch/instaswitch; battle-queue.ts:355-365.
    /// PRNG: none; returned index is invalid after a queue mutation.
    pub fn queue_will_switch(&self, pokemon: MonId) -> Option<QueueHandle> {
        let q = &self.state.queue;
        q.entries[..usize::from(q.len)]
            .iter()
            .position(|a| {
                matches!(a.kind, ActionKind::Switch | ActionKind::InstaSwitch)
                    && a.pokemon == pokemon
            })
            .map(|i| QueueHandle(i as u8))
    }

    /// Remove all actor actions; battle-queue.ts:334-343. PRNG: none.
    pub fn queue_cancel_action(&mut self, pokemon: MonId) -> bool {
        let q = &mut self.state.queue;
        let old_len = usize::from(q.len);
        let mut write = 0;
        for read in 0..old_len {
            if q.entries[read].pokemon != pokemon {
                if write != read {
                    q.entries[write] = q.entries[read];
                }
                write += 1;
            }
        }
        q.len = write as u16;
        write != old_len
    }

    /// Remove first actor move only; battle-queue.ts:345-353. PRNG: none.
    pub fn queue_cancel_move(&mut self, pokemon: MonId) -> bool {
        let q = &self.state.queue;
        let found = q.entries[..usize::from(q.len)]
            .iter()
            .position(|a| a.kind == ActionKind::Move && a.pokemon == pokemon);
        match found {
            Some(i) => {
                self.lc_queue_remove(i);
                true
            }
            None => false,
        }
    }

    /// `list.splice(i, 1)`.
    fn lc_queue_remove(&mut self, index: usize) {
        let q = &mut self.state.queue;
        let len = usize::from(q.len);
        assert!(index < len, "stale queue handle");
        q.entries.copy_within(index + 1..len, index);
        q.len -= 1;
    }

    /// Copy a retrieved action; battle-queue.ts:154-157,315-365. PRNG: none.
    pub fn queue_action(&self, handle: QueueHandle) -> Action {
        let q = &self.state.queue;
        assert!(u16::from(handle.0) < q.len, "stale queue handle");
        q.entries[usize::from(handle.0)]
    }

    /// Remove first action, shifting remaining entries; battle-queue.ts:143-145.
    /// PRNG: none.
    pub fn queue_shift(&mut self) -> Option<Action> {
        if self.state.queue.len == 0 {
            return None;
        }
        let first = self.state.queue.entries[0];
        self.lc_queue_remove(0);
        Some(first)
    }

    /// Inspect first/last action; battle-queue.ts:146-148. PRNG: none.
    pub fn queue_peek(&self, end: bool) -> Option<Action> {
        let q = &self.state.queue;
        if q.len == 0 {
            None
        } else if end {
            Some(q.entries[usize::from(q.len) - 1])
        } else {
            Some(q.entries[0])
        }
    }

    /// Append a resolved action; battle-queue.ts:149-151. PRNG: none.
    pub fn queue_push(&mut self, action: Action) {
        let q = &mut self.state.queue;
        assert!(
            usize::from(q.len) < QUEUE_CAPACITY,
            "Action queue capacity exceeded"
        );
        q.entries[usize::from(q.len)] = action;
        q.len += 1;
    }

    /// Prepend a resolved action; battle-queue.ts:152-154. PRNG: none.
    pub fn queue_unshift(&mut self, action: Action) {
        self.lc_queue_splice(0, &[action]);
    }

    /// Empty queue; battle-queue.ts:405-407. PRNG: none.
    pub fn queue_clear(&mut self) {
        self.state.queue.len = 0;
    }

    /// Mutate priority and action speed; sim/battle.ts:2623-2667.
    /// PRNG: getTarget may sample; ModifyPriority/stat handlers may tie-shuffle/draw.
    pub fn get_action_speed(&mut self, action: &mut Action) {
        self.lc_action_speed(action, None);
    }

    /// getActionSpeed with an optional already-materialised move frame (resolveAction
    /// shares the one JS `action.move` object between FractionalPriority, targeting and
    /// the speed computation).
    fn lc_action_speed(&mut self, action: &mut Action, frame: Option<MoveHandle>) {
        if action.kind == ActionKind::Move {
            let (handle, owned) = match frame {
                Some(h) => (h, false),
                None => (self.lc_move_frame(action.move_data), true),
            };
            // Take priority from the base move, so abilities like Prankster only apply once.
            let base = f64::from(base_priority(action.move_data.id, action.move_data.kind));
            let target_loc = action.target_loc_present.then_some(action.target_loc);
            let target =
                self.get_target(action.pokemon, MoveInput::Active(handle), target_loc, None);
            let target_arg = target.map_or(EventArg::Null, mon_arg);
            let mv = EffectRef::ActiveMove(handle.0);
            let priority = self.single_event(
                EventId::ModifyPriority,
                mv,
                None,
                mon_arg(action.pokemon),
                target_arg,
                EffectRef::None,
                Relay::Number(base),
                None,
            );
            let priority = self.run_event(
                EventId::ModifyPriority,
                mon_arg(action.pokemon),
                target_arg,
                mv,
                priority,
                RunEventOptions::default(),
            );
            let priority = relay_number(priority, "ModifyPriority");
            action.priority = priority + action.fractional_priority;
            // `if (this.gen > 5) action.move.priority = priority`.
            assert!(
                priority == priority.trunc() && priority >= -128.0 && priority <= 127.0,
                "queued move priority {priority} is outside the i8 state encoding"
            );
            self.active_move_mut(handle).priority = priority as i8;
            self.lc_read_frame(handle, &mut action.move_data);
            if owned {
                self.release_active_move(handle);
            }
        }
        action.speed = if action.pokemon == MonId::NONE {
            1
        } else {
            self.pokemon_action_speed(action.pokemon)
        };
    }
}

impl<L: LogSink> Battle<L> {
    /// Mutable action retrieved by willMove/willSwitch (battle-queue.ts:324-365;
    /// data/abilities.ts:575-580). PRNG: none; drop this borrow before callbacks.
    pub fn queue_action_mut(&mut self, handle: QueueHandle) -> &mut Action {
        let q = &mut self.state.queue;
        assert!(u16::from(handle.0) < q.len, "stale queue handle");
        &mut q.entries[usize::from(handle.0)]
    }
}
