//! OWNER C. Parse text in source check order; keep partially filled choices.
//! Ports sim/side.ts:527-1390 and sim/battle.ts:2962-3068. PRNG: none before commit.
//!
//! Showdown reports a failure through `emitChoiceError`, which sends a side update
//! and returns false. Here each failing check returns `Err(ChoiceError)`; the only
//! place where the source keeps going after an error is the `default:` arm of
//! `Side.choose` (an unrecognized word), which is carried as a pending error.

use super::{
    ChoiceError, TargetKind, move_presence as mp,
    requests::{LockedMove, RequestUpdate, request_target},
    support::*,
};
use crate::{
    Battle, dex,
    ids::*,
    log::LogSink,
    state::{
        Trapped, mon_flags,
        choices::{ChoiceKind, ChosenMoveKind, RequestKind, RequestMove, SideChoice, SlotChoice},
    },
};

/// Source move selector; numeric slots are one-based (sim/side.ts:552-629).
/// Names are normalized at the input boundary, not in the battle loop. PRNG: none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveSelection<'a> {
    Auto,
    Index(u32),
    Name(&'a str),
}

/// All parsed suffixes, including unusable mechanics whose errors are observable
/// (sim/side.ts:1215-1255,638-829). Aliases normalize to these variants. PRNG: none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChoiceModifier {
    #[default]
    None,
    Mega,
    MegaX,
    MegaY,
    ZMove,
    Ultra,
    Dynamax,
    Terastallize,
}

/// Resolved move identity inside `chooseMove` (the `moveid` string of the source).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mv {
    Dex(EffectId),
    Recharge,
    TestFight,
}

impl Mv {
    /// `dex.moves.get(moveid).name`; nonexistent ids keep their own text.
    fn name(self) -> &'static str {
        match self {
            Self::Dex(id) => dex::move_data(id).name,
            Self::Recharge => "recharge",
            Self::TestFight => "testfight",
        }
    }

    fn of_request(m: &RequestMove) -> Self {
        if m.presence & mp::RECHARGE != 0 {
            Self::Recharge
        } else {
            Self::Dex(m.id)
        }
    }

    fn matches_id(self, text: &str) -> bool {
        match self {
            Self::Dex(id) => dex::effect(id).key == text,
            Self::Recharge => text == "recharge",
            Self::TestFight => false,
        }
    }
}

/// Typed-API expectation checked against the text-equivalent resolution.
#[derive(Clone, Copy, Debug)]
struct TypedMove {
    /// move_slot == 255: the forced (locked/Recharge/Struggle) form is expected.
    forced: bool,
    id: EffectId,
    kind: ChosenMoveKind,
}

/// CHOOSABLE_TARGETS target of the Max move for a dynamax request
/// (battle-actions.ts:9-29,1470-1479): None when no Max move exists.
fn max_move_of(mv: Mv) -> Option<(&'static str, TargetKind)> {
    let Mv::Dex(id) = mv else { return None };
    let m = dex::move_data(id);
    if m.name == "Struggle" {
        return Some(("Struggle", TargetKind::RandomNormal));
    }
    if m.category == dex::Category::Status {
        return Some(("Max Guard", TargetKind::SelfTarget));
    }
    let name = match type_name(m.move_type) {
        "Flying" => "Max Airstream",
        "Dark" => "Max Darkness",
        "Fire" => "Max Flare",
        "Bug" => "Max Flutterby",
        "Water" => "Max Geyser",
        "Ice" => "Max Hailstorm",
        "Fighting" => "Max Knuckle",
        "Electric" => "Max Lightning",
        "Psychic" => "Max Mindstorm",
        "Poison" => "Max Ooze",
        "Grass" => "Max Overgrowth",
        "Ghost" => "Max Phantasm",
        "Ground" => "Max Quake",
        "Rock" => "Max Rockfall",
        "Fairy" => "Max Starfall",
        "Steel" => "Max Steelspike",
        "Normal" => "Max Strike",
        "Dragon" => "Max Wyrmwind",
        _ => return None,
    };
    Some((name, TargetKind::AdjacentFoe))
}

const SUFFIXES: [(&str, ChoiceModifier); 10] = [
    (" mega", ChoiceModifier::Mega),
    (" megax", ChoiceModifier::MegaX),
    (" megay", ChoiceModifier::MegaY),
    (" zmove", ChoiceModifier::ZMove),
    (" ultra", ChoiceModifier::Ultra),
    (" dynamax", ChoiceModifier::Dynamax),
    (" gigantamax", ChoiceModifier::Dynamax),
    (" max", ChoiceModifier::Dynamax),
    (" terastal", ChoiceModifier::Terastallize),
    (" terastallize", ChoiceModifier::Terastallize),
];

/// `/\s(?:-|\+)?[1-3]$/`: returns the text before the whitespace and the signed
/// location that `parseInt(data.slice(-2))` yields.
fn strip_target_suffix(data: &str) -> Option<(&str, i8)> {
    let mut it = data.char_indices().rev();
    let (_, digit) = it.next()?;
    if !('1'..='3').contains(&digit) {
        return None;
    }
    let value = digit as i8 - b'0' as i8;
    let (mut ws_i, mut ws) = it.next()?;
    let mut loc = value;
    if ws == '-' || ws == '+' {
        if ws == '-' {
            loc = -value;
        }
        (ws_i, ws) = it.next()?;
    }
    if !is_js_space(ws) {
        return None;
    }
    Some((&data[..ws_i], loc))
}

impl<L: LogSink> Battle<L> {
    /// Battle.choose (sim/battle.ts:2962-2981): parse, require completeness, then
    /// synchronously call lifecycle.commit_choices if every side is done.
    /// PRNG: none on rejection/noncommit; committing lifecycle draws transitively.
    pub fn choose(&mut self, side: usize, input: &str) -> Result<(), ChoiceError> {
        assert!(side < 2, "invalid side index {side}");
        let sid = SideId(side as u8);
        self.choose_no_commit(sid, input)?;
        self.commit_if_ready()
    }

    /// Everything in Battle.choose before `allChoicesDone`: parse the text, then
    /// require the side's own choice to be complete. Never commits. PRNG: none.
    pub(crate) fn choose_no_commit(&mut self, side: SideId, input: &str) -> Result<(), ChoiceError> {
        self.choose_side(side, input)?;
        if !self.is_choice_done(side) {
            let msg = format!("Incomplete choice: {input} - missing other pokemon");
            return Err(self.emit_choice_error(side, &msg, None));
        }
        Ok(())
    }

    /// `if (this.allChoicesDone()) this.commitChoices()`.
    fn commit_if_ready(&mut self) -> Result<(), ChoiceError> {
        if self.all_choices_done() {
            self.commit_choices().map_err(|e| ChoiceError {
                text: format!("[Battle error] {}", e.0),
                resent_request_json: None,
            })?;
        }
        Ok(())
    }

    /// Side.choose (sim/side.ts:1185-1301). Check request/cantUndo before clearing;
    /// unknown tokens record an error and continue with later comma-separated
    /// tokens. Legacy suffixes remain observable error paths. PRNG: none.
    ///
    /// Folds in `Battle.choose`'s "Unknown error" fallback for a silent false
    /// (a `pass`/`switch` with no slot left), because that is the only place that
    /// has the original input text.
    pub fn choose_side(&mut self, side: SideId, input: &str) -> Result<(), ChoiceError> {
        let s = side.0 as usize;
        if self.side_request_kind(side) == RequestKind::None {
            let msg = if self.state.ended {
                "Can't do anything: The game is over"
            } else {
                "Can't do anything: It's not your turn"
            };
            return Err(self.emit_choice_error(side, msg, None));
        }
        if self.state.sides[s].choice.cant_undo {
            return Err(self.emit_choice_error(
                side,
                "Can't undo: A trapping/disabling effect would cause undo to leak information",
                None,
            ));
        }
        self.clear_choice(side);

        let team_input = input.starts_with("team ");
        let count = if team_input { 1 } else { input.split(',').count() };
        if count > 2 {
            let msg = format!(
                "Can't make choices: You sent choices for {count} Pokémon, but this is a doubles game!"
            );
            return Err(self.emit_choice_error(side, &msg, None));
        }

        let mut pending: Option<ChoiceError> = None;
        let tokens: Vec<&str> = if team_input {
            vec![input]
        } else {
            input.split(',').collect()
        };
        for choice_string in tokens {
            let trimmed = js_trim(choice_string);
            let (mut choice_type, rest) = split_first_space(trimmed);
            let mut data = js_trim(rest);
            if choice_type == "testfight" {
                choice_type = "move";
                data = "testfight";
            }
            match choice_type {
                "move" => {
                    let original = data;
                    let mut data = data;
                    let mut target_loc: Option<i8> = None;
                    let mut event = ChoiceModifier::None;
                    loop {
                        let mut stripped = None;
                        if let Some((rest, loc)) = strip_target_suffix(data) {
                            if to_id(data) != "conversion2" {
                                stripped = Some((rest, loc));
                            }
                        }
                        if let Some((rest, loc)) = stripped {
                            if target_loc.is_some() {
                                return Err(self.conflicting_move_args(side, original));
                            }
                            target_loc = Some(loc);
                            data = rest;
                            continue;
                        }
                        let mut matched = false;
                        for (suffix, modifier) in SUFFIXES {
                            if let Some(rest) = data.strip_suffix(suffix) {
                                if event != ChoiceModifier::None {
                                    return Err(self.conflicting_move_args(side, original));
                                }
                                event = modifier;
                                data = rest;
                                matched = true;
                                break;
                            }
                        }
                        if !matched {
                            break;
                        }
                    }
                    let selection = if data.is_empty() {
                        MoveSelection::Auto
                    } else {
                        MoveSelection::Name(data)
                    };
                    self.choose_move(side, selection, target_loc.unwrap_or(0), event)?;
                }
                "switch" => {
                    let text = if data.is_empty() { None } else { Some(data) };
                    if !self.choose_switch_impl(side, text.map_or(SwitchSel::Auto, SwitchSel::Text))? {
                        return Err(pending.unwrap_or_else(|| self.unknown_choice_error(side, input)));
                    }
                }
                "shift" => {
                    if !data.is_empty() {
                        let msg = format!("Unrecognized data after \"shift\": {data}");
                        return Err(self.emit_choice_error(side, &msg, None));
                    }
                    self.choose_shift(side)?;
                }
                "team" => self.choose_team(side, data)?,
                "pass" | "skip" => {
                    if !data.is_empty() {
                        let msg = format!("Unrecognized data after \"pass\": {data}");
                        return Err(self.emit_choice_error(side, &msg, None));
                    }
                    if !self.choose_pass(side)? {
                        return Err(pending.unwrap_or_else(|| self.unknown_choice_error(side, input)));
                    }
                }
                "auto" | "default" => self.auto_choose(side),
                _ => {
                    let msg = format!("Unrecognized choice: {choice_string}");
                    pending = Some(self.emit_choice_error(side, &msg, None));
                }
            }
        }
        pending.map_or(Ok(()), Err)
    }

    fn conflicting_move_args(&mut self, side: SideId, original: &str) -> ChoiceError {
        let msg = format!("Conflicting arguments for \"move\": {original}");
        self.emit_choice_error(side, &msg, None)
    }

    /// Battle.choose's fallback when Side.choose returned false without an error
    /// (sim/battle.ts:2966-2971).
    fn unknown_choice_error(&mut self, side: SideId, input: &str) -> ChoiceError {
        let msg = format!(
            "Unknown error for choice: {input}. If you're not using a custom client, please report this as a bug."
        );
        self.emit_choice_error(side, &msg, None)
    }

    /// Side.chooseMove (sim/side.ts:552-849). Preserve target checks before locked
    /// move, disabled and late modifier checks; freeze move ID and slot presence.
    /// PRNG: none; scoped priorityEvent LockMove/SemiLockMove has stable ordering.
    pub fn choose_move(
        &mut self,
        side: SideId,
        selection: MoveSelection<'_>,
        target_loc: i8,
        modifier: ChoiceModifier,
    ) -> Result<(), ChoiceError> {
        self.choose_move_core(side, selection, target_loc, modifier, None)
    }

    fn choose_move_core(
        &mut self,
        side: SideId,
        selection: MoveSelection<'_>,
        mut target_loc: i8,
        modifier: ChoiceModifier,
        typed: Option<TypedMove>,
    ) -> Result<(), ChoiceError> {
        let s = side.0 as usize;
        let kind = self.side_request_kind(side);
        if kind != RequestKind::Move {
            let msg = format!("Can't move: You need a {} response", request_state_name(kind));
            return Err(self.emit_choice_error(side, &msg, None));
        }
        let index = self.choice_index(side, false) as usize;
        if index >= 2 {
            return Err(self.emit_choice_error(
                side,
                "Can't move: You sent more choices than unfainted Pokémon.",
                None,
            ));
        }
        let auto_choose = match selection {
            MoveSelection::Auto | MoveSelection::Index(0) => true,
            MoveSelection::Name(t) => t.is_empty(),
            MoveSelection::Index(_) => false,
        };
        let pokemon = self.ch_active(side, index);
        assert_ne!(pokemon, MonId::NONE);

        // Parse moveText (name or index). A failure needs no further inspection.
        let request = self.get_move_request_data(pokemon);
        let rmoves = &request.moves[..request.move_count as usize];
        let mut move_slot: Option<usize> = None;
        let mut mv = Mv::TestFight; // overwritten below unless testfight is the text
        let mut target_type: Option<TargetKind> = None;
        let number: Option<f64> = if auto_choose {
            Some(1.0)
        } else {
            match selection {
                MoveSelection::Index(n) => Some(f64::from(n)),
                MoveSelection::Name(t) if is_ascii_digits(t) => Some(digits_to_number(t)),
                _ => None,
            }
        };
        if let Some(n) = number {
            // One-based move index.
            let i = n - 1.0;
            if i < 0.0 || i >= rmoves.len() as f64 {
                let msg = format!(
                    "Can't move: Your {} doesn't have a move {}",
                    self.ch_name(pokemon),
                    js_number_string(i + 1.0)
                );
                return Err(self.emit_choice_error(side, &msg, None));
            }
            let i = i as usize;
            move_slot = Some(i);
            mv = Mv::of_request(&rmoves[i]);
            target_type = (rmoves[i].presence & mp::TARGET != 0).then(|| request_target(&rmoves[i]));
        } else {
            let MoveSelection::Name(text) = selection else { unreachable!() };
            let mut moveid = to_id(text);
            if moveid.starts_with("hiddenpower") {
                moveid = "hiddenpower".into();
            }
            for (i, m) in rmoves.iter().enumerate() {
                let candidate = Mv::of_request(m);
                if !candidate.matches_id(&moveid) {
                    continue;
                }
                move_slot = Some(i);
                mv = candidate;
                target_type = Some(if m.presence & mp::TARGET != 0 {
                    request_target(m)
                } else {
                    TargetKind::Normal
                });
                break;
            }
            // maxMoves / canZMove name lookups: never present in a gen 9 request.
            if target_type.is_none() {
                if moveid != "testfight" {
                    let msg = format!(
                        "Can't move: Your {} doesn't have a move matching {moveid}",
                        self.ch_name(pokemon)
                    );
                    return Err(self.emit_choice_error(side, &msg, None));
                }
                mv = Mv::TestFight;
            }
        }

        if let Some(t) = typed {
            // Typed choices name the move they mean; check it is the one resolved.
            if !t.forced && (mv != Mv::Dex(t.id) || t.kind != ChosenMoveKind::Dex) {
                let msg = format!(
                    "Can't move: Your {} doesn't have the requested move in that slot",
                    self.ch_name(pokemon)
                );
                return Err(self.emit_choice_error(side, &msg, None));
            }
        }

        let moves = self.get_request_moves(pokemon, None, false);
        if auto_choose {
            for i in 0..request.move_count as usize {
                let m = &request.moves[i];
                if m.presence & mp::DISABLED != 0 && m.disabled != 0 {
                    continue;
                }
                if i < moves.move_count as usize
                    && m.id == moves.moves[i].id
                    && moves.moves[i].disabled != 0
                {
                    continue;
                }
                mv = Mv::of_request(m);
                move_slot = Some(i);
                target_type = (m.presence & mp::TARGET != 0).then(|| request_target(m));
                break;
            }
        }
        let move_name = mv.name();

        // Z-move: getZMove is undefined for every scoped item.
        if modifier == ChoiceModifier::ZMove {
            let msg = format!(
                "Can't move: {} can't use {move_name} as a Z-move",
                self.ch_name(pokemon)
            );
            return Err(self.emit_choice_error(side, &msg, None));
        }

        // Dynamax: no volatile exists, but the event can still name a Max move.
        let max_move = if modifier == ChoiceModifier::Dynamax {
            max_move_of(mv)
        } else {
            None
        };
        if modifier == ChoiceModifier::Dynamax && max_move.is_none() {
            let msg = format!(
                "Can't move: {} can't use {move_name} as a Max Move",
                self.ch_name(pokemon)
            );
            return Err(self.emit_choice_error(side, &msg, None));
        }
        if let Some((_, tt)) = max_move {
            target_type = Some(tt);
        }

        // Validate targeting.
        let lenient = typed.is_some_and(|t| t.forced);
        if auto_choose || mv == Mv::TestFight {
            target_loc = 0;
        } else if target_type.is_some_and(TargetKind::choosable) {
            if target_loc == 0 && !lenient {
                let msg = format!("Can't move: {move_name} needs a target");
                return Err(self.emit_choice_error(side, &msg, None));
            }
            if !target_type
                .unwrap()
                .valid_loc(i32::from(target_loc), self.ch_mon(pokemon).position)
            {
                let msg = format!("Can't move: Invalid target for {move_name}");
                return Err(self.emit_choice_error(side, &msg, None));
            }
        } else if target_loc != 0 {
            let msg = format!("Can't move: You can't choose a target for {move_name}");
            return Err(self.emit_choice_error(side, &msg, None));
        }

        let locked = match self.ch_get_locked_move(pokemon) {
            Some(l) => Some(l),
            None => self.ch_get_semi_locked_move(pokemon, false),
        };
        if let Some(locked) = locked {
            let mut loc = self.ch_mon(pokemon).last_move_target_loc;
            if let LockedMove::Dex(id) = locked {
                if let Some(volatile_loc) = self.ch_locked_target_loc(pokemon, id) {
                    loc = volatile_loc;
                }
            }
            if self.ch_mon(pokemon).flags & mon_flags::MAYBE_LOCKED != 0 {
                self.state.sides[s].choice.cant_undo = true;
            }
            let (move_id, move_kind) = match locked {
                LockedMove::Dex(id) => (id, ChosenMoveKind::Dex),
                LockedMove::Recharge => (EffectId::NONE, ChosenMoveKind::Recharge),
            };
            self.push_action(
                side,
                SlotChoice {
                    kind: ChoiceKind::Move,
                    move_slot: 255,
                    switch_to: MonId::NONE,
                    target_loc: loc,
                    tera: false,
                    move_id,
                    move_kind,
                },
            );
            return self.typed_forced_result(side, typed, move_id, move_kind);
        } else if moves.move_count == 0 {
            // Override the action and use Struggle: no enabled move has PP.
            if self.ch_mon(pokemon).flags & mon_flags::MAYBE_LOCKED != 0 {
                self.state.sides[s].choice.cant_undo = true;
            }
            self.push_action(
                side,
                SlotChoice {
                    kind: ChoiceKind::Move,
                    move_slot: 255,
                    switch_to: MonId::NONE,
                    target_loc: 0,
                    tera: false,
                    move_id: dex::MOVE_STRUGGLE,
                    move_kind: ChosenMoveKind::Dex,
                },
            );
            return self.typed_forced_result(side, typed, dex::MOVE_STRUGGLE, ChosenMoveKind::Dex);
        } else if mv == Mv::TestFight {
            // Client "Fight" button helper.
            if self.ch_mon(pokemon).flags & mon_flags::MAYBE_LOCKED == 0 {
                let msg = format!(
                    "Can't move: {}'s Fight button is known to be safe",
                    self.ch_name(pokemon)
                );
                return Err(self.emit_choice_error(side, &msg, None));
            }
            self.update_request_for_pokemon(RequestUpdate::RefreshDisabled { pokemon });
            let json = self.emit_request(side, true);
            // `choice.error` is set to a placeholder so nothing is sent to the client.
            return Err(ChoiceError {
                text: String::new(),
                resent_request_json: Some(json),
            });
        } else if max_move.is_some() {
            // Dynamaxed: only Taunt and Assault Vest disable Max Guard, but the
            // base move must have PP remaining.
            if self.ch_max_move_disabled(pokemon, mv) {
                let msg = format!(
                    "Can't move: {}'s {} is disabled",
                    self.ch_name(pokemon),
                    max_move.unwrap().0
                );
                return Err(self.emit_choice_error(side, &msg, None));
            }
        } else {
            // Check for disabled moves; disabledSource stays '' because getMoves()
            // entries never carry one.
            let Mv::Dex(move_id) = mv else {
                unreachable!("recharge is always a locked move")
            };
            let mut is_enabled = false;
            for m in &moves.moves[..moves.move_count as usize] {
                if m.id != move_id {
                    continue;
                }
                if m.disabled == 0 {
                    is_enabled = true;
                    break;
                }
            }
            if !is_enabled {
                if auto_choose {
                    panic!("autoChoose chose a disabled move");
                }
                let msg = format!("Can't move: {}'s {move_name} is disabled", self.ch_name(pokemon));
                return Err(self.emit_choice_error(
                    side,
                    &msg,
                    Some(RequestUpdate::RevealDisabled {
                        pokemon,
                        move_id,
                        disabled_source: EffectId::NONE,
                    }),
                ));
            }
        }

        // Mega evolution, Ultra Burst: canMegaEvo / canUltraBurst are never set.
        let name = self.ch_name(pokemon);
        let late_error = match modifier {
            ChoiceModifier::Mega => Some(format!("Can't move: {name} can't mega evolve")),
            ChoiceModifier::MegaX => Some(format!("Can't move: {name} can't mega evolve X")),
            ChoiceModifier::MegaY => Some(format!("Can't move: {name} can't mega evolve Y")),
            ChoiceModifier::Ultra => Some(format!("Can't move: {name} can't ultra burst")),
            // canDynamax is undefined and there is no dynamax volatile: gen !== 8.
            ChoiceModifier::Dynamax => Some("Can't move: Dynamaxing doesn't outside of Gen 8.".to_string()),
            _ => None,
        };
        if let Some(msg) = late_error {
            return Err(self.emit_choice_error(side, &msg, None));
        }
        let terastallize = modifier == ChoiceModifier::Terastallize;
        if terastallize && self.ch_can_tera(pokemon) == TypeId::NONE {
            let msg = format!("Can't move: {} can't Terastallize.", self.ch_name(pokemon));
            return Err(self.emit_choice_error(side, &msg, None));
        }
        if terastallize && self.state.sides[s].choice.tera {
            return Err(self.emit_choice_error(
                side,
                "Can't move: You can only Terastallize once per battle.",
                None,
            ));
        }
        let move_slot = move_slot.expect("moveSlot should have been set by this point");
        let Mv::Dex(move_id) = mv else {
            unreachable!("a plain move choice resolves to a dex move")
        };
        if lenient {
            // A forced form was expected but the Pokemon is free to choose.
            let msg = format!("Can't move: {} is not forced to use that move", self.ch_name(pokemon));
            return Err(self.emit_choice_error(side, &msg, None));
        }
        self.push_action(
            side,
            SlotChoice {
                kind: ChoiceKind::Move,
                move_slot: move_slot as u8,
                switch_to: MonId::NONE,
                target_loc,
                tera: terastallize,
                move_id,
                move_kind: ChosenMoveKind::Dex,
            },
        );
        // (cantUndo for maybeDisabled applies to singles only.)
        if terastallize {
            self.state.sides[s].choice.tera = true;
        }
        Ok(())
    }

    /// For typed forced choices: the forced form actually chosen must be the one named.
    fn typed_forced_result(
        &mut self,
        side: SideId,
        typed: Option<TypedMove>,
        id: EffectId,
        kind: ChosenMoveKind,
    ) -> Result<(), ChoiceError> {
        let Some(t) = typed else { return Ok(()) };
        if !t.forced || (t.id == id && t.kind == kind) {
            return Ok(());
        }
        // Undo the pushed action, then report.
        let c = &mut self.state.sides[side.0 as usize].choice;
        c.len -= 1;
        Err(self.emit_choice_error(side, "Can't move: That is not the forced move", None))
    }

    /// `pokemon.volatiles[lockedMoveID]?.targetLoc` when truthy (side.ts:677-679).
    /// Charge-move volatiles (cell id == move id) keep the stored location in payload
    /// word 0 as a two's-complement i32 under the first custom presence bit.
    fn ch_locked_target_loc(&self, pokemon: MonId, id: EffectId) -> Option<i8> {
        let cell = self.ch_volatile(pokemon, id)?;
        let c = &self.state.effects.cells[cell.0 as usize];
        if c.present & (1 << crate::state::present::CUSTOM_START) == 0 {
            return None;
        }
        let loc = c.payload.words[0] as i32;
        (loc != 0).then_some(loc as i8)
    }

    /// Pokemon.maxMoveDisabled (pokemon.ts:1046-1050).
    fn ch_max_move_disabled(&self, pokemon: MonId, mv: Mv) -> bool {
        let Mv::Dex(id) = mv else { return true };
        if self.get_move_data(pokemon, id).is_none_or(|s| s.pp == 0) {
            return true;
        }
        dex::move_data(id).category == dex::Category::Status
            && (self.has_item(pokemon, &[dex::ITEM_ASSAULTVEST])
                || self.ch_volatile(pokemon, dex::CONDITION_TAUNT).is_some())
    }

    /// Side.chooseSwitch (sim/side.ts:915-1023). Parse JS parseInt or case-insensitive
    /// name/species; validate range/active/duplicate before fainted/trapped checks.
    /// Handles Revival Blessing selections. PRNG: none.
    pub fn choose_switch(
        &mut self,
        side: SideId,
        slot_text: Option<&str>,
    ) -> Result<(), ChoiceError> {
        let sel = slot_text.filter(|t| !t.is_empty()).map_or(SwitchSel::Auto, SwitchSel::Text);
        if self.choose_switch_impl(side, sel)? {
            Ok(())
        } else {
            Err(self.unknown_choice_error(side, ""))
        }
    }

    /// `Ok(false)` is a silent false (only `choosePass` can fail without an error).
    fn choose_switch_impl(&mut self, side: SideId, sel: SwitchSel<'_>) -> Result<bool, ChoiceError> {
        let s = side.0 as usize;
        let kind = self.side_request_kind(side);
        if kind != RequestKind::Move && kind != RequestKind::Switch {
            let msg = format!("Can't switch: You need a {} response", request_state_name(kind));
            return Err(self.emit_choice_error(side, &msg, None));
        }
        let index = self.choice_index(side, false) as usize;
        if index >= 2 {
            let msg = if kind == RequestKind::Switch {
                "Can't switch: You sent more switches than Pokémon that need to switch"
            } else {
                "Can't switch: You sent more choices than unfainted Pokémon"
            };
            return Err(self.emit_choice_error(side, msg, None));
        }
        let pokemon = self.ch_active(side, index);
        assert_ne!(pokemon, MonId::NONE);
        let position = self.ch_mon(pokemon).position;
        let revival = self.ch_has_revival_slot(side, position);
        let party_count = self.state.sides[s].pokemon_count as usize;

        let mut slot: f64;
        match sel {
            SwitchSel::Auto => {
                if kind != RequestKind::Switch {
                    return Err(self.emit_choice_error(
                        side,
                        "Can't switch: You need to select a Pokémon to switch in",
                        None,
                    ));
                }
                if revival {
                    let mut i = 0;
                    while !self.ch_fainted(self.state.sides[s].party[i]) {
                        i += 1;
                        assert!(i < party_count, "no fainted Pokémon for Revival Blessing");
                    }
                    slot = i as f64;
                } else {
                    if self.state.sides[s].choice.forced_switches_left == 0 {
                        return self.choose_pass(side);
                    }
                    let mut i = 2;
                    while self.state.sides[s].choice.switch_ins & (1 << i) != 0
                        || self.ch_fainted(self.state.sides[s].party[i])
                    {
                        i += 1;
                        assert!(i < party_count, "no switch target for forced switch");
                    }
                    slot = i as f64;
                }
            }
            SwitchSel::Text(text) => slot = js_parse_int(text) - 1.0,
            SwitchSel::Index(i) => slot = i as f64,
        }
        if slot.is_nan() || slot < 0.0 {
            // Maybe it is a name or species id.
            let SwitchSel::Text(text) = sel else { unreachable!() };
            let lower = text.to_lowercase();
            let id = to_id(text);
            let mut found = None;
            for i in 0..party_count {
                let mon = self.state.sides[s].party[i];
                if lower == self.ch_name(mon).to_lowercase()
                    || id == dex::effect(self.ch_mon(mon).species).key
                {
                    found = Some(i);
                    break;
                }
            }
            match found {
                Some(i) => slot = i as f64,
                None => {
                    let msg = format!(
                        "Can't switch: You do not have a Pokémon named \"{text}\" to switch to"
                    );
                    return Err(self.emit_choice_error(side, &msg, None));
                }
            }
        }
        if slot >= party_count as f64 {
            let msg = format!(
                "Can't switch: You do not have a Pokémon in slot {} to switch to",
                js_number_string(slot + 1.0)
            );
            return Err(self.emit_choice_error(side, &msg, None));
        } else if slot < 2.0 && !revival {
            return Err(self.emit_choice_error(side, "Can't switch: You can't switch to an active Pokémon", None));
        } else if self.state.sides[s].choice.switch_ins & (1 << slot as u8) != 0 {
            let msg = format!(
                "Can't switch: The Pokémon in slot {} can only switch in once",
                js_number_string(slot + 1.0)
            );
            return Err(self.emit_choice_error(side, &msg, None));
        }
        let slot = slot as usize;
        let target = self.state.sides[s].party[slot];

        if revival {
            if !self.ch_fainted(target) {
                return Err(self.emit_choice_error(side, "Can't switch: You have to pass to a fainted Pokémon", None));
            }
            let c = &mut self.state.sides[s].choice;
            c.forced_switches_left = c.forced_switches_left.saturating_sub(1);
            self.state.pokemon[pokemon.0 as usize].switch_flag = EffectId::NONE;
            self.push_action(
                side,
                SlotChoice {
                    kind: ChoiceKind::Revival,
                    switch_to: target,
                    ..SlotChoice::default()
                },
            );
            return Ok(true);
        }

        if self.ch_fainted(target) {
            return Err(self.emit_choice_error(side, "Can't switch: You can't switch to a fainted Pokémon", None));
        }
        if kind == RequestKind::Move {
            if self.ch_mon(pokemon).trapped != Trapped::No {
                return Err(self.emit_choice_error(
                    side,
                    "Can't switch: The active Pokémon is trapped",
                    Some(RequestUpdate::RevealTrapped { pokemon }),
                ));
            } else if self.ch_mon(pokemon).flags & mon_flags::MAYBE_TRAPPED != 0 {
                self.state.sides[s].choice.cant_undo = true;
            }
        } else {
            let c = &mut self.state.sides[s].choice;
            assert!(
                c.forced_switches_left != 0,
                "Player somehow switched too many Pokemon"
            );
            c.forced_switches_left -= 1;
        }
        self.state.sides[s].choice.switch_ins |= 1 << slot;
        self.push_action(
            side,
            SlotChoice {
                kind: ChoiceKind::Switch,
                switch_to: target,
                ..SlotChoice::default()
            },
        );
        Ok(true)
    }

    /// Side.choosePass (sim/side.ts:1330-1357). False means no slot remains, not a
    /// choice error; this distinction selects Battle.choose's fallback error.
    /// PRNG: none.
    pub fn choose_pass(&mut self, side: SideId) -> Result<bool, ChoiceError> {
        let s = side.0 as usize;
        let index = self.choice_index(side, true) as usize;
        if index >= 2 {
            return Ok(false);
        }
        let pokemon = self.ch_active(side, index);
        assert_ne!(pokemon, MonId::NONE);
        match self.side_request_kind(side) {
            RequestKind::Switch => {
                // Always true when called by Battle.choose().
                if self.ch_switch_flag(pokemon) {
                    if self.state.sides[s].choice.forced_passes_left == 0 {
                        let msg = format!(
                            "Can't pass: You need to switch in a Pokémon to replace {}",
                            self.ch_name(pokemon)
                        );
                        return Err(self.emit_choice_error(side, &msg, None));
                    }
                    self.state.sides[s].choice.forced_passes_left -= 1;
                }
            }
            RequestKind::Move => {
                if !self.ch_fainted(pokemon) && !self.ch_commanding(pokemon) {
                    let msg = format!(
                        "Can't pass: Your {} must make a move (or switch)",
                        self.ch_name(pokemon)
                    );
                    return Err(self.emit_choice_error(side, &msg, None));
                }
            }
            _ => {
                return Err(self.emit_choice_error(side, "Can't pass: Not a move or switch request", None));
            }
        }
        self.push_action(side, SlotChoice::default());
        Ok(true)
    }

    /// Side.chooseTeam (sim/side.ts:1031-1035). This format rejects Team Preview
    /// choices with its exact source error. PRNG: none.
    pub fn choose_team(&mut self, side: SideId, _data: &str) -> Result<(), ChoiceError> {
        // requestState is never 'teampreview' in this format.
        Err(self.emit_choice_error(
            side,
            "Can't choose for Team Preview: You're not in a Team Preview phase",
            None,
        ))
    }

    /// Side.chooseShift (sim/side.ts:1097-1116). Preserve index/request/data checks
    /// before the doubles-format rejection. PRNG: none.
    pub fn choose_shift(&mut self, side: SideId) -> Result<(), ChoiceError> {
        let index = self.choice_index(side, false) as usize;
        let msg = if index >= 2 {
            format!("Can't shift: You do not have a Pokémon in slot {}", index + 1)
        } else if self.side_request_kind(side) != RequestKind::Move {
            "Can't shift: You can only shift during a move phase".to_string()
        } else {
            // gameType is never 'triples'.
            "Can't shift: You can only shift to the center in triples".to_string()
        };
        Err(self.emit_choice_error(side, &msg, None))
    }

    /// Side.getChoiceIndex (sim/side.ts:1303-1328). Append auto-passes for fainted,
    /// commanding or non-switching slots unless explicitly choosing pass.
    /// PRNG: none; mutates the partial SideChoice.
    pub fn choice_index(&mut self, side: SideId, is_pass: bool) -> u8 {
        let s = side.0 as usize;
        let mut index = self.state.sides[s].choice.len;
        if !is_pass {
            match self.side_request_kind(side) {
                RequestKind::Move => {
                    while index < 2 {
                        let mon = self.ch_active(side, index as usize);
                        if !(self.ch_fainted(mon) || self.ch_commanding(mon)) {
                            break;
                        }
                        // Cannot fail: fainted/commanding slots may always pass.
                        let _ = self.choose_pass(side);
                        index += 1;
                    }
                }
                RequestKind::Switch => {
                    while index < 2 {
                        let mon = self.ch_active(side, index as usize);
                        if self.ch_switch_flag(mon) {
                            break;
                        }
                        let _ = self.choose_pass(side);
                        index += 1;
                    }
                }
                _ => {}
            }
        }
        index
    }

    /// Side.isChoiceDone (sim/side.ts:539-550). Waiting sides are complete; invoking
    /// getChoiceIndex may append auto-passes. PRNG: none.
    pub fn is_choice_done(&mut self, side: SideId) -> bool {
        let s = side.0 as usize;
        if self.side_request_kind(side) == RequestKind::None {
            return true;
        }
        if self.state.sides[s].choice.forced_switches_left != 0 {
            return false;
        }
        self.choice_index(side, false);
        self.state.sides[s].choice.len >= 2
    }

    /// Battle.allChoicesDone (sim/battle.ts:3058-3068). Also sets cantUndo when
    /// supportCancel is false. PRNG: none.
    pub fn all_choices_done(&mut self) -> bool {
        let mut total = 0;
        for s in 0..2 {
            if self.is_choice_done(SideId(s)) {
                if !self.state.support_cancel {
                    self.state.sides[s as usize].choice.cant_undo = true;
                }
                total += 1;
            }
        }
        total >= 2
    }

    /// Side.clearChoice (sim/side.ts:1118-1141). Compute forced switches/passes from
    /// global requestState before replacing SideChoice. PRNG: none.
    pub fn clear_choice(&mut self, side: SideId) {
        let s = side.0 as usize;
        let (mut forced_switches, mut forced_passes) = (0u8, 0u8);
        if self.state.request_state == RequestKind::Switch {
            let sd = &self.state.sides[s];
            let can_switch_out = sd.active.iter().filter(|&&m| self.ch_switch_flag(m)).count() as u8;
            let can_switch_in = sd.party[2.min(sd.pokemon_count as usize)..sd.pokemon_count as usize]
                .iter()
                .filter(|&&m| !self.ch_fainted(m))
                .count() as u8;
            forced_switches = can_switch_out.min(can_switch_in);
            forced_passes = can_switch_out - forced_switches;
        }
        self.state.sides[s].choice = SideChoice {
            forced_switches_left: forced_switches,
            forced_passes_left: forced_passes,
            ..SideChoice::default()
        };
    }

    /// Side.autoChoose (sim/side.ts:1360-1384). Deterministically choose the first
    /// eligible switch/move, with the source's ten-iteration guard. PRNG: none.
    pub fn auto_choose(&mut self, side: SideId) {
        match self.side_request_kind(side) {
            RequestKind::Switch => {
                let mut i = 0;
                while !self.is_choice_done(side) {
                    match self.choose_switch_impl(side, SwitchSel::Auto) {
                        Ok(true) => {}
                        Ok(false) => panic!("autoChoose switch crashed: no error"),
                        Err(e) => panic!("autoChoose switch crashed: {}", e.text),
                    }
                    i += 1;
                    if i > 10 {
                        panic!("autoChoose failed: infinite looping");
                    }
                }
            }
            RequestKind::Move => {
                let mut i = 0;
                while !self.is_choice_done(side) {
                    if let Err(e) = self.choose_move(side, MoveSelection::Auto, 0, ChoiceModifier::None) {
                        panic!("autoChoose crashed: {}", e.text);
                    }
                    i += 1;
                    if i > 10 {
                        panic!("autoChoose failed: infinite looping");
                    }
                }
            }
            _ => {}
        }
    }

    /// Battle.undoChoice (sim/battle.ts:3030-3052). Revealed request updates survive
    /// choice clearing; return the optional resent request on success. PRNG: none.
    pub fn undo_choice(&mut self, side: SideId) -> Result<Option<String>, ChoiceError> {
        let s = side.0 as usize;
        let kind = self.side_request_kind(side);
        if kind == RequestKind::None {
            return Ok(None);
        }
        if self.state.sides[s].choice.cant_undo {
            return Err(self.emit_choice_error(
                side,
                "Can't undo: A trapping/disabling effect would cause undo to leak information",
                None,
            ));
        }
        let mut updated = false;
        if kind == RequestKind::Move {
            let choice = self.state.sides[s].choice;
            for i in 0..choice.len as usize {
                if choice.slots[i].kind != ChoiceKind::Move {
                    continue;
                }
                let pokemon = self.ch_active(side, i);
                if self.update_request_for_pokemon(RequestUpdate::RefreshDisabled { pokemon }) {
                    updated = true;
                }
            }
        }
        self.clear_choice(side);
        Ok(updated.then(|| self.emit_request(side, true)))
    }

    /// Side.emitChoiceError (sim/side.ts:527-537). Error tag is Unavailable only if
    /// the requested cache patch changes data. Never writes to battle log.
    /// PRNG: none; String construction is confined to boundary/error handling.
    pub fn emit_choice_error(
        &mut self,
        side: SideId,
        message: &str,
        update: Option<RequestUpdate>,
    ) -> ChoiceError {
        let updated = update.is_some_and(|u| self.update_request_for_pokemon(u));
        let mut text = String::with_capacity(message.len() + 20);
        text.push_str(if updated {
            "[Unavailable choice] "
        } else {
            "[Invalid choice] "
        });
        text.push_str(message);
        let resent_request_json = updated.then(|| self.emit_request(side, true));
        ChoiceError {
            text,
            resent_request_json,
        }
    }

    pub(super) fn push_action(&mut self, side: SideId, choice: SlotChoice) {
        let c = &mut self.state.sides[side.0 as usize].choice;
        assert!(c.len < 2, "more than one action per active slot");
        c.slots[c.len as usize] = choice;
        c.len += 1;
    }

    /// Structured counterpart of Side.choose/Battle.choose (side.ts:1185,
    /// battle.ts:2962). Run identical semantic checks, including joint constraints;
    /// legal_actions output is accepted. PRNG: none until a successful commit.
    ///
    /// `slots[i]` is the choice for active slot i. A slot that auto-passes (fainted,
    /// or not flagged in a switch request) must be a Pass; trailing auto-pass slots
    /// may be omitted. Moves are checked through the text path (index and target),
    /// so hidden-information rejections behave exactly as in `choose`.
    pub fn choose_typed(&mut self, side: SideId, slots: &[SlotChoice]) -> Result<(), ChoiceError> {
        self.choose_typed_no_commit(side, slots)?;
        self.commit_if_ready()
    }

    /// `choose_typed` without the final `allChoicesDone`/commit step.
    pub(crate) fn choose_typed_no_commit(
        &mut self,
        side: SideId,
        slots: &[SlotChoice],
    ) -> Result<(), ChoiceError> {
        let s = side.0 as usize;
        if self.side_request_kind(side) == RequestKind::None {
            let msg = if self.state.ended {
                "Can't do anything: The game is over"
            } else {
                "Can't do anything: It's not your turn"
            };
            return Err(self.emit_choice_error(side, msg, None));
        }
        if self.state.sides[s].choice.cant_undo {
            return Err(self.emit_choice_error(
                side,
                "Can't undo: A trapping/disabling effect would cause undo to leak information",
                None,
            ));
        }
        self.clear_choice(side);
        if slots.len() > 2 {
            let msg = format!(
                "Can't make choices: You sent choices for {} Pokémon, but this is a doubles game!",
                slots.len()
            );
            return Err(self.emit_choice_error(side, &msg, None));
        }
        for (i, sc) in slots.iter().enumerate() {
            let index = self.choice_index(side, false) as usize;
            if index > i {
                // Slot i passed automatically.
                if sc.kind != ChoiceKind::Pass {
                    let msg = format!("Can't choose: Slot {} cannot act", i + 1);
                    return Err(self.emit_choice_error(side, &msg, None));
                }
                continue;
            }
            match sc.kind {
                ChoiceKind::Pass => {
                    if !self.choose_pass(side)? {
                        return Err(self.unknown_choice_error(side, &self.choice_text_of(slots)));
                    }
                }
                ChoiceKind::Switch | ChoiceKind::Revival => {
                    let party = self.state.sides[s].party;
                    let count = self.state.sides[s].pokemon_count as usize;
                    let Some(pos) = party[..count].iter().position(|&m| m == sc.switch_to) else {
                        return Err(self.emit_choice_error(side, "Can't switch: That Pokémon is not on your team", None));
                    };
                    if !self.choose_switch_impl(side, SwitchSel::Index(pos))? {
                        return Err(self.unknown_choice_error(side, &self.choice_text_of(slots)));
                    }
                }
                ChoiceKind::Move => {
                    let forced = sc.move_slot == 255;
                    let selection = MoveSelection::Index(if forced { 1 } else { u32::from(sc.move_slot) + 1 });
                    let modifier = if sc.tera {
                        ChoiceModifier::Terastallize
                    } else {
                        ChoiceModifier::None
                    };
                    let typed = TypedMove {
                        forced,
                        id: sc.move_id,
                        kind: sc.move_kind,
                    };
                    self.choose_move_core(side, selection, sc.target_loc, modifier, Some(typed))?;
                }
            }
        }
        if !self.is_choice_done(side) {
            let msg = format!(
                "Incomplete choice: {} - missing other pokemon",
                self.choice_text_of(slots)
            );
            return Err(self.emit_choice_error(side, &msg, None));
        }
        Ok(())
    }

    /// Text rendering of typed input for messages only.
    fn choice_text_of(&self, slots: &[SlotChoice]) -> String {
        let mut parts = Vec::new();
        for sc in slots {
            parts.push(match sc.kind {
                ChoiceKind::Pass => "pass".to_string(),
                ChoiceKind::Move => format!("move {}", u32::from(sc.move_slot) + 1),
                ChoiceKind::Switch | ChoiceKind::Revival => "switch".to_string(),
            });
        }
        parts.join(", ")
    }

    /// Side.getChoice (sim/side.ts:323-351). Produce the normalized input-log choice
    /// only at a boundary; targets include their positive '+' prefix. PRNG: none.
    pub fn choice_text(&self, side: SideId) -> String {
        let c = &self.state.sides[side.0 as usize].choice;
        let mut out = String::new();
        for i in 0..c.len as usize {
            if i > 0 {
                out.push_str(", ");
            }
            let a = &c.slots[i];
            match a.kind {
                ChoiceKind::Move => {
                    out.push_str("move ");
                    match a.move_kind {
                        ChosenMoveKind::Recharge => out.push_str("recharge"),
                        ChosenMoveKind::Dex => out.push_str(dex::effect(a.move_id).key),
                    }
                    if a.target_loc != 0 {
                        out.push(' ');
                        if a.target_loc > 0 {
                            out.push('+');
                        }
                        out.push_str(&a.target_loc.to_string());
                    }
                    if a.tera {
                        out.push_str(" terastallize");
                    }
                }
                ChoiceKind::Switch | ChoiceKind::Revival => {
                    out.push_str("switch ");
                    let pos = self.ch_mon(a.switch_to).position;
                    out.push_str(&(pos as u32 + 1).to_string());
                }
                ChoiceKind::Pass => out.push_str("pass"),
            }
        }
        out
    }
}

/// How `chooseSwitch` receives its slot (a text token, the auto path, or an index).
#[derive(Clone, Copy)]
enum SwitchSel<'a> {
    Auto,
    Text(&'a str),
    Index(usize),
}

/// The Side.requestState string used inside "You need a ${requestState} response".
fn request_state_name(kind: RequestKind) -> &'static str {
    match kind {
        RequestKind::Move => "move",
        RequestKind::Switch => "switch",
        RequestKind::None | RequestKind::Wait => "",
    }
}
