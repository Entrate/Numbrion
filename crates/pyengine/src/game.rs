//! `Game`: one engine battle plus everything the training interface needs around it (packed teams for
//! cloning, hidden-information-safe per-player log views). Pure Rust, no Python.

use crate::mask::SideActions;
use engine::{
    Battle,
    ids::{MonId, SideId},
    log::{LogSink, NoLog, TextLog},
    prng::Prng,
    sim::{ChoiceError, Outcome},
    state::{BattleState, choices::SlotChoice},
};
use std::sync::Arc;

pub enum Sink {
    No(Battle<NoLog>),
    Text(Battle<TextLog>),
}

/// Log views of a `Game` built on a `TextLog`. Lines are pulled out of the engine's raw log after every
/// engine call (which is what a Showdown `BattleStream` does with `sendUpdates`) and resolved once into
/// the omniscient stream and one stream per player.
#[derive(Default)]
struct LogViews {
    cursor: usize,
    keep_omni: bool,
    omni: Vec<String>,
    players: [Vec<String>; 2],
}

impl LogViews {
    /// `extractChannelMessages` (sim/battle.ts:35): `|split|pN` is followed by a secret and a shared line;
    /// player N sees the secret one, everybody else the shared one, and empty lines are dropped.
    fn absorb(&mut self, entries: &[String]) {
        let new = &entries[self.cursor.min(entries.len())..];
        let mut i = 0;
        while i < new.len() {
            let line = &new[i];
            if self.keep_omni {
                self.omni.push(line.clone());
            }
            if let Some(n) = line.strip_prefix("|split|p")
                && let Ok(owner) = n.parse::<usize>()
                && (1..=4).contains(&owner)
                && i + 2 < new.len()
            {
                let (secret, shared) = (&new[i + 1], &new[i + 2]);
                if self.keep_omni {
                    self.omni.push(secret.clone());
                    self.omni.push(shared.clone());
                }
                for p in 0..2 {
                    let l = if p + 1 == owner { secret } else { shared };
                    if !l.is_empty() {
                        self.players[p].push(l.clone());
                    }
                }
                i += 3;
                continue;
            }
            if !line.is_empty() {
                for p in 0..2 {
                    self.players[p].push(line.clone());
                }
            }
            i += 1;
        }
        self.cursor = entries.len();
    }
}

pub struct Game {
    pub sink: Sink,
    packed: [Arc<str>; 2],
    names: [String; 2],
    seed0: [u16; 4],
    views: Option<LogViews>,
}

macro_rules! with {
    ($g:expr, $b:ident => $e:expr) => {
        match &$g.sink {
            Sink::No($b) => $e,
            Sink::Text($b) => $e,
        }
    };
}
macro_rules! with_mut {
    ($g:expr, $b:ident => $e:expr) => {
        match &mut $g.sink {
            Sink::No($b) => $e,
            Sink::Text($b) => $e,
        }
    };
}

fn build<L: LogSink + Default>(
    seed: [u16; 4],
    packed: &[Arc<str>; 2],
    names: &[String; 2],
) -> Result<Battle<L>, String> {
    Battle::from_players(seed, (&names[0], &packed[0]), (&names[1], &packed[1]), L::default())
        .map_err(|e| e.0)
}

impl Game {
    /// A battle before `start()`. `log` selects `TextLog` (player/omniscient views) over `NoLog`;
    /// `keep_omni` additionally keeps the omniscient stream (debugging).
    pub fn new(
        seed: [u16; 4],
        p1: Arc<str>,
        p2: Arc<str>,
        names: [String; 2],
        log: bool,
        keep_omni: bool,
    ) -> Result<Game, String> {
        let packed = [p1, p2];
        let sink = if log {
            Sink::Text(build::<TextLog>(seed, &packed, &names)?)
        } else {
            Sink::No(build::<NoLog>(seed, &packed, &names)?)
        };
        let views = log.then(|| LogViews { keep_omni, ..LogViews::default() });
        Ok(Game { sink, packed, names, seed0: seed, views })
    }

    pub fn start(&mut self) -> Result<(), String> {
        let r = with_mut!(self, b => b.start()).map_err(|e| e.0);
        self.flush_log();
        r
    }

    /// Pull new raw log entries into the views. Called after every engine call that can log.
    fn flush_log(&mut self) {
        if let (Sink::Text(b), Some(v)) = (&self.sink, &mut self.views) {
            v.absorb(&b.log.entries);
        }
    }

    pub fn state(&self) -> &BattleState {
        with!(self, b => &b.state)
    }

    pub fn is_text(&self) -> bool {
        matches!(self.sink, Sink::Text(_))
    }

    pub fn names(&self) -> &[String; 2] {
        &self.names
    }

    pub fn packed_teams(&self) -> &[Arc<str>; 2] {
        &self.packed
    }

    /// The seed the battle was constructed with (not the current PRNG state, see `prng_seed`).
    pub fn initial_seed(&self) -> [u16; 4] {
        self.seed0
    }

    /// The current PRNG state, in the same four-word form as the constructor seed.
    pub fn prng_seed(&self) -> [u16; 4] {
        self.state().prng.seed()
    }

    pub fn turn(&self) -> u32 {
        with!(self, b => b.turn())
    }

    pub fn ended(&self) -> bool {
        self.state().ended
    }

    pub fn started(&self) -> bool {
        self.state().started
    }

    pub fn outcome(&self) -> Option<Outcome> {
        with!(self, b => b.outcome())
    }

    pub fn request_json(&self, side: usize) -> Option<String> {
        with!(self, b => b.request_json(side))
    }

    /// Showdown's normalized text of the choice stored for `side` (`Side.getChoice`).
    pub fn choice_text(&self, side: usize) -> String {
        with!(self, b => b.choice_text(SideId(side as u8)))
    }

    /// Everything observable at a decision boundary, for equality checks between copies of a battle.
    pub fn signature(&self) -> (u32, [u16; 4], bool, [Option<String>; 2]) {
        (self.turn(), self.prng_seed(), self.ended(), [self.request_json(0), self.request_json(1)])
    }

    /// The engine's own joint-legality predicate (`Battle::is_legal_joint_choice`), for cross-checks.
    pub fn engine_joint_legal(&self, side: usize, slots: &[SlotChoice]) -> bool {
        with!(self, b => b.is_legal_joint_choice(SideId(side as u8), slots))
    }

    /// Best-effort engine choice for any code, see `mask::unchecked_slot_choice`.
    pub fn unchecked_choice(&self, side: usize, slot: usize, code: usize) -> SlotChoice {
        with!(self, b => crate::mask::unchecked_slot_choice(b, SideId(side as u8), slot, code))
    }

    /// Canonical codes of the choice currently stored for `side`, see `mask::encode_side_choice`.
    pub fn encode_choice(&self, side: usize) -> Option<[usize; 2]> {
        with!(self, b => crate::mask::encode_side_choice(b, SideId(side as u8)))
    }

    pub fn party(&self, side: usize) -> [MonId; 6] {
        self.state().sides[side].party
    }

    pub fn side_actions(&self, side: usize) -> SideActions {
        with!(self, b => SideActions::from_battle(b, SideId(side as u8)))
    }

    /// Text `Battle.choose` (sim/battle.ts:2962). Rejections are returned with Showdown's error text.
    pub fn choose(&mut self, side: usize, text: &str) -> Result<(), ChoiceError> {
        let r = with_mut!(self, b => b.choose(side, text));
        self.flush_log();
        r
    }

    /// Typed `Battle.choose`; `slots[i]` is the choice of active slot i.
    pub fn choose_typed(&mut self, side: usize, slots: &[SlotChoice]) -> Result<(), ChoiceError> {
        let r = with_mut!(self, b => b.choose_typed(SideId(side as u8), slots));
        self.flush_log();
        r
    }

    /// An independent copy of the battle at its current decision boundary: same state, same PRNG.
    ///
    /// Rebuilds the battle from the packed teams (immutable team definitions are not part of the
    /// copyable snapshot) and overwrites its `BattleState` with a bitwise copy of this one. The log views
    /// of the copy start empty.
    pub fn duplicate(&self) -> Game {
        let mut g = Game::new(
            self.seed0,
            self.packed[0].clone(),
            self.packed[1].clone(),
            self.names.clone(),
            self.is_text(),
            self.views.as_ref().is_some_and(|v| v.keep_omni),
        )
        .expect("teams that built the original battle build again");
        g.copy_state_from(self);
        g
    }

    /// Overwrite this battle's state with `other`'s (same teams and seed required: `same_origin`).
    pub fn copy_state_from(&mut self, other: &Game) {
        debug_assert!(self.same_origin(other));
        let state = *other.state();
        with_mut!(self, b => b.state = state);
        if let (Sink::Text(b), Some(v)) = (&self.sink, &mut self.views) {
            // The copy's own log restarts at the copied position.
            v.cursor = b.log.entries.len();
        }
    }

    /// Both games were built from the same packed teams, names and seed (so team definitions agree).
    pub fn same_origin(&self, other: &Game) -> bool {
        self.seed0 == other.seed0 && self.packed == other.packed && self.names == other.names
    }

    /// Replace the battle's PRNG state. The next draw is the first draw of the new stream.
    pub fn reseed(&mut self, seed: [u16; 4]) {
        with_mut!(self, b => b.state.prng = Prng::from_seed(seed));
    }

    /// Omniscient raw battle log since the last call (engine `battle.log` entries, split triples included).
    pub fn drain_omni(&mut self) -> Vec<String> {
        self.views.as_mut().map(|v| std::mem::take(&mut v.omni)).unwrap_or_default()
    }

    /// What `side` has been shown since the last call.
    pub fn drain_player(&mut self, side: usize) -> Vec<String> {
        self.views.as_mut().map(|v| std::mem::take(&mut v.players[side])).unwrap_or_default()
    }

    pub fn has_log(&self) -> bool {
        self.views.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_triples_resolve_per_player() {
        let mut v = LogViews { keep_omni: true, ..LogViews::default() };
        let raw: Vec<String> = [
            "|turn|1",
            "|split|p1",
            "|switch|p1a: A|A, L50|50/50",
            "|switch|p1a: A|A, L50|100/100",
            "|split|p2",
            "|-hint|secret",
            "",
            "",
            "|move|p1a: A|Tackle|p2a: B",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        v.absorb(&raw[..4]);
        v.absorb(&raw);
        assert_eq!(
            v.players[0],
            ["|turn|1", "|switch|p1a: A|A, L50|50/50", "|move|p1a: A|Tackle|p2a: B"]
        );
        assert_eq!(
            v.players[1],
            ["|turn|1", "|switch|p1a: A|A, L50|100/100", "|-hint|secret", "|move|p1a: A|Tackle|p2a: B"]
        );
        assert_eq!(v.omni, raw);
        assert!(v.players[0].iter().all(|l| !l.is_empty()));
    }
}
