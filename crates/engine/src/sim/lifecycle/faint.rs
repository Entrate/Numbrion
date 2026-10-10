//! Faint queue flushing, result and battle termination. OWNER L.

use crate::sim::Outcome;
use crate::{
    Battle,
    event::{Relay, RunEventOptions},
    ids::{EventId, MonId, SideId},
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{FaintEntry, Phase, ResultFlag, Status, choices::SideRequest, mon_flags},
};

use super::util::{mon_arg, opt_mon_arg};

impl<L: LogSink> Battle<L> {
    /// Mark fainted active slots for replacement; battle.ts:2528-2537.
    /// PRNG: none. Sets fnt status and true switchFlag, not faint callbacks.
    pub fn check_fainted(&mut self) {
        for side in 0..2 {
            for slot in 0..2 {
                let m = self.state.sides[side].active[slot];
                if m != MonId::NONE && self.lc_is_fainted(m) {
                    self.lc_mon_mut(m).status = Status::Fainted;
                    self.lc_set_switch_flag_true(m);
                }
            }
        }
    }

    /// `faintQueue.shift()`.
    fn lc_faint_shift(&mut self) -> FaintEntry {
        let len = usize::from(self.state.faint_queue_len);
        assert!(len > 0);
        let first = self.state.faint_queue[0];
        self.state.faint_queue.copy_within(1..len, 0);
        self.state.faint_queue_len -= 1;
        first
    }

    /// Flush the ordered faint queue; battle.ts:2539-2608.
    /// PRNG: BeforeFaint/Faint/AfterFaint sorting and callbacks, ability/item End.
    /// Already-ended returns Undefined; ordinary nonterminal returns false.
    pub fn faint_messages(
        &mut self,
        last_first: bool,
        force_check: bool,
        check_win: bool,
    ) -> Relay {
        let mut check_win = check_win;
        if self.state.ended {
            return Relay::Undefined;
        }
        let length = self.state.faint_queue_len;
        if length == 0 {
            if force_check && self.check_win(None) {
                return Relay::Bool(true);
            }
            return Relay::Bool(false);
        }
        if last_first {
            // unshift(last); pop(): the last entry moves to the front.
            self.state.faint_queue[..usize::from(length)].rotate_right(1);
        }
        let mut faint_data: Option<FaintEntry> = None;
        while self.state.faint_queue_len > 0 {
            let faint_queue_left = self.state.faint_queue_len;
            let data = self.lc_faint_shift();
            faint_data = Some(data);
            let pokemon = data.target;
            let source = opt_mon_arg(data.source);
            let effect = data.effect.resolve();
            if self.lc_is_fainted(pokemon) {
                continue;
            }
            let before_faint = self.run_event(
                EventId::BeforeFaint,
                mon_arg(pokemon),
                source,
                effect,
                Relay::Undefined,
                RunEventOptions::default(),
            );
            if !before_faint.truthy() {
                continue;
            }
            self.add(LogEntry::new("faint", &[LogArg::Mon(pokemon)], &[]));
            let side = &mut self.state.sides[usize::from(pokemon.side().0)];
            if side.pokemon_left != 0 {
                side.pokemon_left -= 1;
            }
            if side.total_fainted < 100 {
                side.total_fainted += 1;
            }
            self.run_event(
                EventId::Faint,
                mon_arg(pokemon),
                source,
                effect,
                Relay::Undefined,
                RunEventOptions::default(),
            );
            self.lc_end_ability(pokemon);
            self.lc_end_item(pokemon);
            let regression = self.lc_flag(pokemon, mon_flags::FORME_REGRESSION);
            if regression && !self.lc_flag(pokemon, mon_flags::TRANSFORMED) {
                // before clearing volatiles
                let set = &self.teams.sides[usize::from(pokemon.side().0)].sets
                    [usize::from(pokemon.0 % 6)];
                let (species, ability) = (set.species, set.ability);
                let p = self.lc_mon_mut(pokemon);
                p.base_species = species;
                p.base_ability = ability;
            }
            self.clear_volatile(pokemon, false);
            {
                let p = self.lc_mon_mut(pokemon);
                p.flags |= mon_flags::FAINTED;
                p.flags &= !mon_flags::ACTIVE;
                p.illusion = MonId::NONE;
                // delete pokemon.terastallized
                p.terastallized = crate::ids::TypeId::NONE;
            }
            if regression {
                // after clearing volatiles; details are derived from baseSpecies
                self.add(LogEntry::new(
                    "detailschange",
                    &[LogArg::Mon(pokemon), LogArg::Details(pokemon)],
                    &[LogTag::Bare("silent")],
                ));
                self.update_max_hp(pokemon);
                self.lc_set_flag(pokemon, mon_flags::FORME_REGRESSION, false);
            }
            self.state.sides[usize::from(pokemon.side().0)].fainted_this_turn = ResultFlag::True;
            if self.state.faint_queue_len >= faint_queue_left {
                check_win = true;
            }
        }

        // gen <= 3 queue clearing is outside this format.
        if check_win && self.check_win(faint_data) {
            return Relay::Bool(true);
        }

        if let Some(data) = faint_data {
            if length != 0 {
                self.run_event(
                    EventId::AfterFaint,
                    mon_arg(data.target),
                    opt_mon_arg(data.source),
                    data.effect.resolve(),
                    Relay::Number(f64::from(length)),
                    RunEventOptions::default(),
                );
            }
        }
        Relay::Bool(false)
    }

    /// Resolve defeat, including simultaneous faint winner; battle.ts:2610-2621.
    /// PRNG: none. For both sides empty in Gen9, last faint target's side wins.
    pub fn check_win(&mut self, faint_data: Option<FaintEntry>) -> bool {
        if self.state.sides.iter().all(|s| s.pokemon_left == 0) {
            self.win(faint_data.map(|d| d.target.side()));
            return true;
        }
        for side in 0..2usize {
            // side.foePokemonLeft() is the foe's pokemonLeft.
            if self.state.sides[side ^ 1].pokemon_left == 0 {
                self.win(Some(SideId(side as u8)));
                return true;
            }
        }
        false
    }

    /// End battle with winner or tie; battle.ts:1524-1547.
    /// PRNG: none. Emit blank line then win/tie, clear requests, preserve turn.
    pub fn win(&mut self, side: Option<SideId>) -> bool {
        if self.state.ended {
            return false;
        }
        self.state.winner = side.unwrap_or(SideId(255));
        self.add(LogEntry::new("", &[], &[]));
        match side {
            Some(s) => self.add(LogEntry::new("win", &[LogArg::PlayerName(s)], &[])),
            None => self.add(LogEntry::new("tie", &[], &[])),
        }
        self.state.ended = true;
        self.state.request_state = crate::state::choices::RequestKind::None;
        // for (const s of this.sides) s.activeRequest = null
        self.state.requests = [SideRequest::default(); 2];
        self.state.phase = Phase::Ended;
        true
    }

    /// End in a tie; battle.ts:1520-1522. PRNG: none.
    pub fn tie(&mut self) -> bool {
        self.win(None)
    }

    /// Forfeit a side in doubles; battle.ts:1549-1568. PRNG: none.
    pub fn lose(&mut self, side: SideId) -> bool {
        // gameType is never 'freeforall': the foe wins.
        self.win(Some(SideId(side.0 ^ 1)))
    }

    /// External timeout tiebreaker; battle.ts:1471-1518.
    /// PRNG: none; compare unfainted count, HP percentages then total HP exactly.
    pub fn tiebreak(&mut self) -> bool {
        if self.state.ended {
            return false;
        }
        self.add(LogEntry::new(
            "message",
            &[LogArg::Text("Time's up! Going to tiebreaker...")],
            &[],
        ));
        let mut not_fainted = [0i32; 2];
        for (s, n) in not_fainted.iter_mut().enumerate() {
            let side = &self.state.sides[s];
            *n = side.party[..usize::from(side.pokemon_count)]
                .iter()
                .filter(|&&m| !self.lc_is_fainted(m))
                .count() as i32;
        }
        self.lc_tiebreak_message(
            [SideId(0), SideId(1)],
            2,
            |i| not_fainted[i],
            " Pokemon left",
            "",
        );
        let max_not_fainted = not_fainted[0].max(not_fainted[1]);
        let mut tied = [SideId(0), SideId(1)];
        let mut tied_len = 0;
        for s in 0..2 {
            if not_fainted[s] == max_not_fainted {
                tied[tied_len] = SideId(s as u8);
                tied_len += 1;
            }
        }
        if tied_len <= 1 {
            return self.win(Some(tied[0]));
        }

        // hpPercentage: sum(hp / maxhp) * 100 / 6, accumulated left to right in f64.
        let mut hp_percentage = [0f64; 2];
        for i in 0..tied_len {
            let side = &self.state.sides[usize::from(tied[i].0)];
            let mut it = side.party[..usize::from(side.pokemon_count)]
                .iter()
                .map(|&m| {
                    let p = self.lc_mon(m);
                    f64::from(p.hp) / f64::from(p.max_hp)
                });
            let first = it.next().expect("empty side");
            hp_percentage[i] = it.fold(first, |a, b| a + b) * 100.0 / 6.0;
        }
        self.lc_tiebreak_message(
            [tied[0], tied[1]],
            tied_len,
            |i| (hp_percentage[i] + 0.5).floor() as i32,
            "% total HP left",
            "",
        );
        let max_percentage = hp_percentage[..tied_len]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let mut next = [tied[0]; 2];
        let mut next_len = 0;
        for i in 0..tied_len {
            if hp_percentage[i] == max_percentage {
                next[next_len] = tied[i];
                next_len += 1;
            }
        }
        tied = next;
        tied_len = next_len;
        if tied_len <= 1 {
            return self.win(Some(tied[0]));
        }

        let mut hp_total = [0i32; 2];
        for i in 0..tied_len {
            let side = &self.state.sides[usize::from(tied[i].0)];
            hp_total[i] = side.party[..usize::from(side.pokemon_count)]
                .iter()
                .map(|&m| i32::from(self.lc_mon(m).hp))
                .sum();
        }
        self.lc_tiebreak_message(
            [tied[0], tied[1]],
            tied_len,
            |i| hp_total[i],
            " total HP left",
            "",
        );
        let max_total = hp_total[..tied_len].iter().copied().max().unwrap();
        let mut next = [tied[0]; 2];
        let mut next_len = 0;
        for i in 0..tied_len {
            if hp_total[i] == max_total {
                next[next_len] = tied[i];
                next_len += 1;
            }
        }
        if next_len <= 1 {
            return self.win(Some(next[0]));
        }
        self.tie()
    }

    /// `add('-message', sides.map(side => `${side.name}: ${value}${suffix}`).join('; '))`.
    fn lc_tiebreak_message(
        &mut self,
        sides: [SideId; 2],
        count: usize,
        value: impl Fn(usize) -> i32,
        suffix: &'static str,
        _unused: &'static str,
    ) {
        let v0 = value(0);
        let v1 = if count > 1 { value(1) } else { 0 };
        if count > 1 {
            let parts = [
                LogArg::PlayerName(sides[0]),
                LogArg::Text(": "),
                LogArg::Number(v0),
                LogArg::Text(suffix),
                LogArg::Text("; "),
                LogArg::PlayerName(sides[1]),
                LogArg::Text(": "),
                LogArg::Number(v1),
                LogArg::Text(suffix),
            ];
            self.add(LogEntry::new("-message", &[LogArg::Parts(&parts)], &[]));
        } else {
            let parts = [
                LogArg::PlayerName(sides[0]),
                LogArg::Text(": "),
                LogArg::Number(v0),
                LogArg::Text(suffix),
            ];
            self.add(LogEntry::new("-message", &[LogArg::Parts(&parts)], &[]));
        }
    }

    /// Engine-side difftest/training outcome; battle.ts:1524-1547,2610-2621.
    /// PRNG: none; None until battle ended, winner 0/1 or None for tie.
    pub fn outcome(&self) -> Option<Outcome> {
        if !self.state.ended {
            return None;
        }
        let winner = (self.state.winner.0 < 2).then_some(usize::from(self.state.winner.0));
        Some(Outcome {
            winner,
            tie: winner.is_none(),
            turns: u32::from(self.state.turn),
            pokemon_left: [
                u32::from(self.state.sides[0].pokemon_left),
                u32::from(self.state.sides[1].pokemon_left),
            ],
        })
    }
}
