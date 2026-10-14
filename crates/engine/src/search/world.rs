//! Fair-information determinization (docs/design/SEARCH-PREREQUISITES.md item 1). A `WorldBuilder`
//! turns what one player has observed into battles that search may simulate:
//!
//! * inputs: the viewer's own team, its own choices and its player-view protocol lines, in order
//!   (`push_lines`, `push_choice`). Never a live battle, so the same path serves self-play and a
//!   ladder client.
//! * a *world* (particle) is a battle constructed from the viewer's team and an opponent team
//!   sampled from a `SetPool`, consistent with the `RevealTracker` facts, then started and replayed
//!   through the observed history. The viewer's choices are replayed as given; the opponent's
//!   are read off the protocol (move, target, Tera, switch target) and sampled where the protocol
//!   is silent.
//! * a world is kept only if its replayed player view equals the observed one, line by line
//!   (`protocol::is_compared` lines). Random outcomes are searched, not guessed: on a mismatch a
//!   window of the draws that can decide the offending line is resampled through
//!   `Prng::set_resample` (every other draw kept), from the replay that reproduced the most lines
//!   so far, within a per-segment budget. Wrong hidden hypotheses (an item, stat spread or exact HP
//!   that cannot reproduce the observed order or damage) therefore die instead of being reported;
//!   lines no draw of the segment can change die early (`Dead::Settled`, `Dead::ExactHp`).
//! * every world carries a weight: a segment its hypothesis reproduced after `n` attempts
//!   multiplies it by `1/n`. This is a search-effort heuristic, not a likelihood estimate (the
//!   guided search keeps matched prefixes, so effort undercounts joint improbability); it only
//!   makes hypotheses that needed far more luck count less. Exact set repairs also multiply the
//!   weight by the replacement record's conditional pool share. `worlds` draws worlds in proportion.
//! * worlds are maintained incrementally: each new segment extends the surviving ones; an
//!   opponent Pokemon seen for the first time is swapped into a never-revealed bench slot with
//!   `replace_hidden_set` (exact by its contract); dead worlds are rebuilt from scratch within a
//!   budget, the rest refilled by copying survivors. Collapsed rebuilds visit distinct revealed
//!   pool candidates before repeating them, with proposal weights corrected to the pool prior.
//! * every world keeps snapshots of itself at earlier boundaries (log-spaced, plus the battle start
//!   and the boundary before each opponent introduction) and a record of every segment's randomness
//!   (`Replay`). When every world died, recovery brings one back, cheapest first: `repair_sets`
//!   swaps a revealed opponent set for a pool alternative at the last snapshot where it was still
//!   hidden and replays the segments since with their recorded randomness (exact, no search);
//!   `backtrack` restarts from snapshots with new random streams and new sets for Pokemon revealed
//!   since; rebuilding from scratch is the fallback.
//! * Illusion entries are pinned to their attributed owner. Unbroken entries may instead belong
//!   to a per-world hidden Illusion user: observations are folded by stint, and at most 6! party
//!   orders are checked against switches, faints and swaps before replay. Fresh worlds carry a
//!   pool-frequency prior on a separate RNG. Guided rebuilds require off-pool facts, opponent death
//!   lines or known Illusion stints; evidence and unresolved leads precede rotating older entries.
//!   Preparation is capped separately, and particles share constraint views for each update.
//! * `worlds` returns copies at the current decision boundary, reseeded from the caller's seed (the
//!   PRNG state is hidden information too).
//!
//! Everything depends only on the inputs, the pool, the builder seed and when `update`/`worlds` are
//! called (worlds are maintained incrementally): two observers with the same public information
//! and the same call sequence get the same worlds (tests/world.rs, the leak test).
use super::{
    SplitMix64,
    protocol::{self, Line, parse_ident},
    reveal::{IllusionSource, MonInfo, PartyEvent, RevealTracker, Stint},
    sets::{SetConstraints, SetPool},
};
use crate::{
    Battle, dex,
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogView, MoveLineEdit, TextLog},
    prng::{RESAMPLE_CAPACITY, RESUME},
    sim::choices::LegalActions,
    state::choices::{ChoiceKind, RequestKind, SlotChoice},
    teams::{Gender, SetDef, TeamDef},
};
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Tuning of a `WorldBuilder`. The defaults favour robustness over speed.
#[derive(Clone, Copy, Debug)]
pub struct WorldConfig {
    /// Worlds maintained.
    pub particles: usize,
    /// Replay attempts per world and segment before the world is dropped.
    pub segment_budget: u32,
    /// Worlds built from scratch per `update` when none survived, at most.
    pub rebuild_budget: usize,
    /// Worlds built from scratch per `update` otherwise (diversity; copies of survivors fill up).
    pub fresh_per_update: usize,
    /// Probability of choosing the target shown in a `|move|` line (redirection and retargeting
    /// make the shown target differ from the chosen one).
    pub shown_target: f64,
    /// Snapshot restores per `update` when no world survived and no set repair brought one back,
    /// before rebuilding from scratch (`backtrack`; 0 disables it). Repairs and restores share
    /// `BACKTRACK_SEGMENTS` segment budgets of attempts.
    pub backtrack_budget: usize,
    /// Pool sets tried per `update` by exact set repair when no world survived (`repair_sets`; 0
    /// disables it), within `REPAIR_SEGMENTS` segment budgets of attempts.
    pub repair_budget: usize,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            particles: 8,
            segment_budget: 1000,
            rebuild_budget: 16,
            fresh_per_update: 1,
            shown_target: 0.85,
            backtrack_budget: 16,
            repair_budget: 16,
        }
    }
}

/// Log-weight a world pays per set patched by `repair` (a factor of about 1/1000): an off-pool set
/// is possible (the live server's sets drift) but far less likely than any explanation the pool
/// supports, such as a hidden Illusion user. When every world needed patching, weights are relative,
/// so nothing changes.
const REPAIR_LOG_PENALTY: f64 = 6.9;

/// Why a world was dropped, or why no world could be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldError {
    /// No world reproduced the observations within the budgets.
    NoConsistentWorld,
    /// A revealed opponent species is missing from the set pool.
    UnknownSpecies(String),
    /// The viewer pushed a choice whose resulting lines have not been pushed yet.
    PendingChoice,
    /// The observed stream contradicts itself (for example a choice where the viewer could not act).
    Desync(&'static str),
    /// Team construction failed.
    Team(String),
    /// The sampled identities cannot produce the observed disguises in any party order.
    IllusionOrder,
}

impl core::fmt::Display for WorldError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoConsistentWorld => f.write_str("no world reproduced the observed history"),
            Self::UnknownSpecies(s) => write!(f, "revealed species {s} is not in the set pool"),
            Self::PendingChoice => f.write_str("a choice was pushed without the lines it produced"),
            Self::Desync(what) => write!(f, "inconsistent observation stream: {what}"),
            Self::Team(e) => write!(f, "invalid team: {e}"),
            Self::IllusionOrder => f.write_str("no sampled party order reproduces the observed Illusion entries"),
        }
    }
}

impl std::error::Error for WorldError {}

/// Cumulative counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    /// Segment replays run (each one turn or less of simulation).
    pub attempts: u64,
    /// Segments a world reproduced.
    pub segments: u64,
    /// Worlds built from scratch (including the first ones).
    pub builds: u64,
    /// Worlds dropped.
    pub deaths: u64,
    /// Worlds refilled by copying a survivor.
    pub copies: u64,
    /// `replace_hidden_set` swaps for opponent Pokemon seen for the first time.
    pub swaps: u64,
    /// Updates in which every world died (or none was left from a failed update).
    pub collapses: u64,
    /// Worlds restored from a snapshot after a collapse (`backtrack`).
    pub restores: u64,
    /// Restored worlds that reached the end of the observed stream.
    pub backtracks: u64,
    /// Pool sets tried by `repair_sets` (each one a replay of the history since its Pokemon entered).
    pub set_repair_attempts: u64,
    /// Dead worlds brought back by a set repair (they reached the end of the observed stream).
    pub set_repairs: u64,
    /// Recorded segments replayed while checking set repairs (also counted in `attempts`).
    pub repair_replays: u64,
    /// Set repairs rejected because the new set changed the number of random draws.
    pub repair_draw_shifts: u64,
}

/// `TextLog` that also records `Prng::draws` at the emission of every raw entry.
#[derive(Debug, Default)]
pub struct DrawLog {
    pub text: TextLog,
    pub draws: Vec<u32>,
    /// Replay-world entry identities, for forced drags and other engine-selected replacements.
    /// Disabled when the pool cannot contain Illusion; no per-line allocation on either path.
    track_entries: bool,
    entry_mons: Vec<(usize, MonId)>,
}

impl LogSink for DrawLog {
    const ENABLED: bool = true;

    fn reset(&mut self) {
        self.text.reset();
        self.draws.clear();
        self.entry_mons.clear();
    }

    fn emit(&mut self, view: LogView<'_>, entry: LogEntry<'_>) {
        let draws = view.state.prng.draws();
        self.text.emit(view, entry);
        self.draws.resize(self.text.entries.len(), draws);
        if self.track_entries
            && matches!(entry.command, "switch" | "drag")
            && let Some(LogArg::Mon(mon)) = entry.args.first()
        {
            // These entries are split: the last emitted line is the opponent's public view.
            self.entry_mons.push((self.text.entries.len() - 1, *mon));
        }
    }

    fn edit_move(&mut self, view: LogView<'_>, edit: MoveLineEdit<'_>) {
        let (len, line) = (self.text.entries.len(), self.text.last_move_line);
        self.text.edit_move(view, edit);
        if self.text.entries.len() < len
            && let Some(i) = line
        {
            self.draws.remove(i);
            for (at, _) in &mut self.entry_mons {
                if *at > i {
                    *at -= 1;
                }
            }
        }
    }
}

/// One world: a battle at a decision boundary of the observed history.
#[derive(Clone)]
struct Particle {
    battle: Battle<DrawLog>,
    /// Observed (compared) lines reproduced so far.
    cursor: usize,
    /// Viewer choices replayed so far.
    choice: usize,
    rng: SplitMix64,
    /// Log weight (search effort `-ln attempts` over reproduced segments, plus set contributions).
    log_w: f64,
    /// Set-dependent part of `log_w` per opponent slot (pool share, patches and Tera).
    set_log_w: [f64; 6],
    /// Why the world was dropped (diagnostics).
    why: String,
    hypothesis: Option<Arc<IllusionHypothesis>>,
    /// Some opponent set was patched by `repair`.
    repaired: bool,
    /// Observed lines when the world was built: its revealed sets already fit them.
    built_at: usize,
    /// Opponent slots drawn with their revealed Tera known since the original build.
    tera_known: u16,
    /// Segments reproduced so far.
    seg: u32,
    /// The world at earlier boundaries, oldest first (`remember`), for `backtrack` and `repair_sets`.
    /// Copies share them.
    history: Vec<Arc<Snapshot>>,
    /// The randomness of every segment reproduced, newest first; copies share it.
    trail: Option<Arc<Replay>>,
    /// Why the world was dropped, and the pool sets `repair_sets` tried on it since, for
    /// `repair_sets`.
    death: Option<Dead>,
    tried: Vec<(MonId, u32)>,
}

/// A world at a boundary it reached (before the next segment introduced anyone): what `backtrack`
/// restarts from and `repair_sets` replays from. About 2 * log2(segments) + 5 per world, each one
/// `Battle` clone (no log).
struct Snapshot {
    battle: Battle<DrawLog>,
    cursor: usize,
    choice: usize,
    log_w: f64,
    set_log_w: [f64; 6],
    hypothesis: Option<Arc<IllusionHypothesis>>,
    repaired: bool,
    built_at: usize,
    tera_known: u16,
    seg: u32,
    /// The next segment revealed an opponent Pokemon: kept at any age, so a set
    /// that a later reveal contradicts can be drawn again just before its Pokemon entered.
    introduces: AtomicBool,
}

/// One reproduced segment, enough to replay it exactly from the boundary before it: the opponent
/// team it ran with (introductions, Tera types and redraws since the previous segment show as
/// differences), the PRNG it started from (copies and restores reseed at boundaries), the resample
/// schedule and both choices that matched, and what it produced.
struct Replay {
    previous: Option<Arc<Replay>>,
    /// The segment's index (`Particle::seg` before it).
    seg: u32,
    cursor: usize,
    end: usize,
    choice: usize,
    /// The viewer chose `choice` at its start.
    viewer: bool,
    team: Arc<TeamDef>,
    prng: crate::prng::Prng,
    key: u64,
    points: Vec<(u32, u32)>,
    opponent: Option<[SlotChoice; 2]>,
    end_draws: u32,
    /// At its end: the viewer acts, the opponent acts, the battle ended.
    boundary: [bool; 3],
}

impl Replay {
    /// The trail before segment `seg`.
    fn before(trail: &Option<Arc<Replay>>, seg: u32) -> Option<Arc<Replay>> {
        let mut t = trail.as_ref();
        while let Some(r) = t.filter(|r| r.seg >= seg) {
            t = r.previous.as_ref();
        }
        t.cloned()
    }
}

impl Snapshot {
    fn of(p: &Particle) -> Self {
        Self {
            battle: p.battle.clone(),
            cursor: p.cursor,
            choice: p.choice,
            log_w: p.log_w,
            set_log_w: p.set_log_w,
            hypothesis: p.hypothesis.clone(),
            repaired: p.repaired,
            built_at: p.built_at,
            tera_known: p.tera_known,
            seg: p.seg,
            introduces: AtomicBool::new(false),
        }
    }
}

#[derive(Clone, Debug)]
struct IllusionHypothesis {
    user: String,
    species: String,
    stints: Vec<u32>,
    view: Vec<MonInfo>,
    revision: usize,
    exact: bool,
}

/// What the protocol says one opponent slot chose at a boundary.
#[derive(Clone, Debug, PartialEq)]
enum SlotSpec {
    Pass,
    Move { id: EffectId, loc: Option<i8>, tera: bool },
    /// The slot acted invisibly (flinch, sleep, fainted first) or not at all: any legal move,
    /// terastallizing when the protocol showed its Tera.
    AnyMove { tera: bool },
    Switch(String, usize),
    Revive(String),
}

/// A dropped world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dead {
    /// Observed line `line` was not reproduced within the segment's budget.
    Budget { line: usize },
    /// Recovery stopped before its normal segment budget was exhausted; it can be retried.
    Interrupted { line: usize },
    /// No draw of the segment, and no choice of the opponent, can change observed line `line`: the
    /// world's hypothesis or something an earlier segment decided (a sleep or confusion length,
    /// a hidden charge target) is wrong.
    Settled { line: usize },
    /// Observed line `line` shows the HP of an opponent Pokemon that this world gave an exact HP
    /// which the line contradicts, possibly fixed in an earlier segment: its HP was shown as
    /// percentages since observed line `since` (each covering several exact HPs), so replaying
    /// from a boundary at or before `since` with other outcomes may reproduce it.
    ExactHp { line: usize, since: usize },
    Desync,
    Choice,
    Swap,
}

impl Dead {
    /// The observed line the world failed to reproduce, when one did.
    fn line(self) -> Option<usize> {
        match self {
            Self::Budget { line } | Self::Interrupted { line } | Self::Settled { line } | Self::ExactHp { line, .. } => Some(line),
            _ => None,
        }
    }
}

/// Candidate coverage during one collapsed rebuild. This changes proposals, not the pool's
/// constraints: a failed set may work with different partners or random outcomes on the next pass.
#[derive(Default)]
struct RebuildSamples {
    used: RefCell<Vec<Vec<u32>>>,
}

/// Borrowed inputs of a replay.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    viewer: SideId,
    lines: &'a [String],
    choices: &'a [(usize, String)],
    requests: &'a [Option<String>],
    move_orders: &'a [(String, u32, Vec<EffectId>)],
    tracker: &'a RevealTracker,
    pool: &'a SetPool,
    config: &'a WorldConfig,
    seen: &'a [MonInfo],
    hypothesis: Option<&'a IllusionHypothesis>,
    repair: bool,
    /// Sets patched by `repair` so far (a world pays `REPAIR_LOG_PENALTY` for each).
    repairs: &'a Cell<u32>,
    /// Patches of facts other than Tera, which keep fresh Illusion proposals alive.
    patches: &'a Cell<u32>,
    rebuild_samples: Option<&'a RebuildSamples>,
}

impl Ctx<'_> {
    fn opp(&self) -> SideId {
        SideId(1 - self.viewer.0)
    }

    fn owner<'a>(&'a self, entry: &'a Stint) -> &'a str {
        match self.hypothesis {
            Some(h) if h.stints.contains(&entry.id) => &h.user,
            _ => &entry.owner,
        }
    }
}

pub struct WorldBuilder {
    viewer: SideId,
    own: Arc<TeamDef>,
    names: [String; 2],
    pool: Arc<SetPool>,
    config: WorldConfig,
    rng: SplitMix64,
    proposal_rng: SplitMix64,
    /// Restores after a collapse draw from their own stream: when they find nothing, the rebuild
    /// that follows draws exactly what it would have drawn without them.
    backtrack_rng: SplitMix64,
    evidence: Vec<u32>,
    rotation: usize,
    tracker: RevealTracker,
    /// Observed compared lines.
    lines: Vec<String>,
    /// (index into `lines` at which the choice was made, Showdown choice text).
    choices: Vec<(usize, String)>,
    /// Viewer request JSON by the index of the choice that answered it (`None`: not pushed).
    requests: Vec<Option<String>>,
    /// Viewer Pokemon name -> the opponent Pokemon it is transformed into (`-transform` lines).
    transformed: Vec<(String, String, u32)>,
    /// Opponent Pokemon name -> its exact moves in slot order, read off the viewer's request while a
    /// viewer Pokemon was transformed into it (Transform copies the move slots in order).
    move_orders: Vec<(String, u32, Vec<EffectId>)>,
    particles: Vec<Particle>,
    /// The dead worlds of a failed update, for the next update to backtrack from.
    graveyard: Vec<Particle>,
    /// Why the most recently dropped world was dropped.
    last_failure: String,
    /// `particles` are at the end of the observed stream.
    synced: bool,
    pub stats: WorldStats,
}

impl WorldBuilder {
    /// `own` is the viewer's team with every gender resolved (as its requests show it): worlds are
    /// built without constructor draws. `names` are the player names as the protocol prints them
    /// (`|player|` lines, when pushed, override them).
    pub fn new(
        viewer: SideId,
        own: Arc<TeamDef>,
        names: [String; 2],
        pool: Arc<SetPool>,
        config: WorldConfig,
        seed: u64,
    ) -> Result<Self, WorldError> {
        assert!(viewer.0 < 2, "viewer must be side 0 or 1");
        own.validate().map_err(|e| WorldError::Team(e.0))?;
        // An unresolved random gender would be drawn at construction, where no resampling reaches it.
        if let Some(set) = own.sets[..own.len as usize].iter().find(|s| {
            s.gender == crate::teams::Gender::Unspecified
                && !matches!(dex::species(s.species).gender, "M" | "F" | "N")
        }) {
            return Err(WorldError::Team(format!(
                "{}: the viewer's genders must be resolved (as its requests show them)",
                set.name
            )));
        }
        Ok(Self {
            viewer,
            own,
            names,
            pool,
            config,
            rng: SplitMix64(seed),
            proposal_rng: SplitMix64(seed ^ 0x494c4c5553494f4e),
            backtrack_rng: SplitMix64(seed ^ 0x4241434b54524143),
            evidence: Vec::new(),
            rotation: 0,
            tracker: RevealTracker::new(),
            lines: Vec::new(),
            choices: Vec::new(),
            requests: Vec::new(),
            transformed: Vec::new(),
            move_orders: Vec::new(),
            particles: Vec::new(),
            graveyard: Vec::new(),
            last_failure: String::new(),
            synced: false,
            stats: WorldStats::default(),
        })
    }

    pub fn viewer(&self) -> SideId {
        self.viewer
    }

    /// Why the most recently dropped world was dropped (empty if none was): the reason, the
    /// observed line index and, for an exhausted budget, the expected and the replayed line.
    pub fn last_failure(&self) -> &str {
        &self.last_failure
    }

    pub fn tracker(&self) -> &RevealTracker {
        &self.tracker
    }

    /// Worlds currently alive (after the last `update`).
    pub fn alive(&self) -> usize {
        self.particles.len()
    }

    /// Player-view lines of the viewer, in order, from the start of the battle.
    pub fn push_lines<'a>(&mut self, lines: impl IntoIterator<Item = &'a str>) {
        for line in lines {
            self.tracker.feed(line);
            if let Some(l) = Line::parse(line)
                && l.cmd() == "player"
                && let Some(side) = protocol::parse_side(l.arg(0))
                && !l.arg(1).is_empty()
            {
                self.names[side.0 as usize] = l.arg(1).to_owned();
            }
            if let Some(l) = Line::parse(line)
                && l.cmd() == "-transform"
                && let (Some(user), Some(target)) = (parse_ident(l.arg(0)), parse_ident(l.arg(1)))
                && user.side == self.viewer
                && target.side != self.viewer
            {
                let stint = target.position.and_then(|p| self.tracker.active_stint(target.side, p)).unwrap_or(0);
                self.transformed.retain(|(u, _, _)| u != user.name);
                self.transformed.push((user.name.to_owned(), target.name.to_owned(), stint));
            }
            if protocol::is_compared(line) {
                self.lines.push(line.to_owned());
                self.synced = false;
            }
        }
    }

    /// Optional: the viewer's request JSON at the boundary it is about to answer (push it after the
    /// lines that led there and before the choice; a Showdown server sends the request first, so a
    /// ladder client should hold it until the turn's lines arrived). Worlds must then reproduce it
    /// exactly, which checks what only the request shows (own exact HP and PP, disabled moves,
    /// trapping); whitespace and Showdown's `rqid` field are ignored.
    pub fn push_request(&mut self, json: &str) {
        // A transformed Pokemon's request lists the copied opponent's move slots in order.
        let json = normalize_request(json);
        for (k, ids) in active_move_ids(&json).into_iter().enumerate().take(2) {
            let Some(i) = self.tracker.active(self.viewer, k as u8) else { continue };
            let m = &self.tracker.mons(self.viewer)[i];
            if !m.transformed {
                continue;
            }
            if let Some((_, target, stint)) = self.transformed.iter().find(|(u, _, _)| *u == m.name)
                && !ids.is_empty()
                && ids.len() <= 4
            {
                let target = target.clone();
                self.move_orders.retain(|(t, st, _)| *t != target || st != stint);
                self.move_orders.push((target, *stint, ids));
            }
        }
        let at = self.choices.len();
        self.requests.resize(at + 1, None);
        self.requests[at] = Some(json);
        self.synced = false;
    }

    /// The viewer's choice (Showdown choice text, e.g. `move 1 +2, switch 3`) at a boundary where it
    /// had to act. Push it before the lines it produced.
    pub fn push_choice(&mut self, text: &str) {
        self.choices.push((self.lines.len(), text.to_owned()));
        self.synced = false;
    }

    /// Bring every world to the end of the observed stream: extend survivors, rebuild dead worlds
    /// within `rebuild_budget`, refill the rest by copying survivors.
    pub fn update(&mut self) -> Result<(), WorldError> {
        if self.synced && !self.particles.is_empty() {
            return Ok(());
        }
        if self.choices.last().is_some_and(|c| c.0 == self.lines.len()) {
            return Err(WorldError::PendingChoice);
        }
        let repairs = Cell::new(0);
        let patches = Cell::new(0);
        let cx = Ctx {
            viewer: self.viewer,
            lines: &self.lines,
            choices: &self.choices,
            requests: &self.requests,
            move_orders: &self.move_orders,
            tracker: &self.tracker,
            pool: &self.pool,
            config: &self.config,
            seen: self.tracker.mons(SideId(1 - self.viewer.0)),
            hypothesis: None,
            repair: true,
            repairs: &repairs,
            patches: &patches,
            rebuild_samples: None,
        };
        let mut alive = Vec::with_capacity(self.config.particles);
        let mut dead = Vec::new();
        let mut source = None;
        let mut cached: Vec<Arc<IllusionHypothesis>> = Vec::new();
        for mut p in std::mem::take(&mut self.particles) {
            refresh_hypothesis(&cx, &mut p.hypothesis, &mut source, &mut cached);
            match advance(&cx, &mut p, &mut self.stats) {
                Ok(()) => alive.push(p),
                Err(d) => {
                    record_evidence(&cx, d, &mut self.evidence);
                    self.stats.deaths += 1;
                    self.last_failure = format!("{d:?}: {}", p.why);
                    p.death = Some(d);
                    p.tried.clear();
                    dead.push(p);
                }
            }
        }
        // Every world died, now or in a failed update before: bring one back from the dead worlds,
        // cheapest first, before rebuilding from turn 1. A set repair replays its world's history
        // with the recorded randomness; a restore from an earlier boundary searches the segments
        // since anew. Both share `BACKTRACK_SEGMENTS` segment budgets of attempts.
        if alive.is_empty() && dead.is_empty() {
            dead = std::mem::take(&mut self.graveyard);
        }
        if alive.is_empty() && !dead.is_empty() {
            self.stats.collapses += 1;
            let cap = self.stats.attempts + u64::from(self.config.segment_budget) * BACKTRACK_SEGMENTS;
            alive = repair_sets(&cx, &mut dead, cap, &mut self.stats, &mut source, &mut cached);
            if alive.is_empty() {
                alive = backtrack(&cx, &dead, 1, cap, &mut self.backtrack_rng, &mut self.stats, &mut self.last_failure,
                    &mut source, &mut cached);
            }
        }
        // Fresh worlds: as many as it takes (within `rebuild_budget`) when none survived, otherwise a
        // few per update (`fresh_per_update`) to keep hypotheses diverse; copies fill the rest. When
        // a few worlds carry nearly all the weight, fresh worlds replace the lightest ones.
        let ess = |alive: &[Particle]| 1.0 / weights(alive).iter().map(|w| w * w).sum::<f64>();
        let (target, budget) = if alive.is_empty() {
            (self.config.particles.div_ceil(2), self.config.rebuild_budget)
        } else {
            let collapsed = alive.len() >= 2 && ess(&alive) < self.config.particles as f64 / 4.0;
            // Worlds that all needed a patched set never die on their own; keep looking for an
            // exact explanation (an Illusion user), which then outweighs them.
            let patched = !cx.pool.illusion_species().is_empty() && alive.iter().all(|p| p.repaired);
            let want = if collapsed || patched || alive.len() < self.config.particles {
                self.config.fresh_per_update
            } else {
                0
            };
            (alive.len() + want, want)
        };
        // A surviving replay already explains its compared lines. Death-only evidence is unresolved
        // only when all particles died; do not keep spending fresh diversity builds on settled lines.
        if !alive.is_empty() { self.evidence.clear(); }
        let mut builds = 0;
        let mut error = None;
        let possible = !cx.pool.illusion_species().is_empty()
            && cx.tracker.stints(cx.opp()).iter().any(|e| can_explain_entry(&cx, e));
        let unsupported: Vec<_> = if possible && budget > 0 {
            (0..cx.seen.len()).filter(|&i| !cx.pool.consistent(&constraints(&cx, i)))
                .map(|i| cx.seen[i].name.as_str()).collect()
        } else { Vec::new() };
        let known = cx.seen.iter().any(|m| m.illusion);
        let contradiction = possible && cx.tracker.stints(cx.opp()).iter().any(|e| {
            e.faint_line.is_some_and(|line| cx.tracker.stints(cx.opp()).iter()
                .any(|later| later.line > line && later.owner == e.owner))
        });
        // Strong evidence (an off-pool fact, a known Illusion stint, a fainted Pokemon back) puts
        // the hypotheses first. Weak evidence (worlds died while a possible disguise was active,
        // as happens in any battle) only gets every other build up to a quarter of the budget, so
        // ordinary rebuilds keep most of their plain attempts. Deaths during this loop count from
        // the next update on.
        let strong = possible && (!unsupported.is_empty() || known || contradiction);
        let weak = possible && !strong && !self.evidence.is_empty();
        let evidence = if strong || weak { self.evidence.clone() } else { Vec::new() };
        let allowance = if strong { (budget * 3 / 4).max(1) } else { (budget / 4).max(1) };
        let mut proposals = Vec::new();
        let mut prepared = false;
        let mut guided_builds = 0;
        let mut tried_illusion = false;
        let rebuild_samples = RebuildSamples::default();
        while alive.len() < target.max(1) && builds < budget {
            let guided = strong || weak && builds % 2 == 1 && guided_builds < allowance;
            // The prior has its own stream and is drawn before any preparation. Without evidence,
            // every rejected prior leaves the plain builds, repair policy and RNG exactly as before.
            let prior = possible && !guided && self.proposal_rng.chance(illusion_prior(&cx));
            if (guided || prior) && !prepared {
                let src = source.get_or_insert_with(|| cx.tracker.illusion_source(cx.opp()));
                proposals = prepare_hypotheses(&cx, src, &unsupported, &evidence, self.rotation,
                    if guided { budget.saturating_mul(2).clamp(1, 32) } else { 2 });
                self.rotation = self.rotation.wrapping_add(1);
                prepared = true;
            }
            let use_guided = guided && guided_builds < allowance && !proposals.is_empty();
            let h = if use_guided {
                let h = proposals[guided_builds % proposals.len()].clone();
                guided_builds += 1;
                Some(h)
            } else if prior && !proposals.is_empty() {
                Some(prior_hypothesis(&cx, &proposals, &mut self.proposal_rng).clone())
            } else { None };
            // Exact-support proposals lead. The second half also tries the retained repair-needed
            // proposals, rather than reusing a list from which those explanations were discarded.
            let repairing = !use_guided || guided_builds > allowance.div_ceil(2);
            let h = if use_guided && repairing {
                let exact = proposals.partition_point(|h| h.exact);
                if exact == proposals.len() { h } else {
                    Some(proposals[exact + (guided_builds - allowance.div_ceil(2) - 1)
                        % (proposals.len() - exact)].clone())
                }
            } else { h };
            let build_cx = Ctx {
                repair: repairing,
                // Ordinary fresh worlds retain the pool prior. After a collapse, avoid spending
                // the small rebuild budget on the same revealed sets over and over.
                rebuild_samples: (alive.is_empty() && self.stats.deaths > 0).then_some(&rebuild_samples),
                ..cx
            };
            tried_illusion |= h.is_some();
            builds += 1;
            match build(&build_cx, h, self.own.clone(), &self.names, &mut self.rng, &mut self.stats) {
                Ok(mut p) => match advance(&cx, &mut p, &mut self.stats) {
                    Ok(()) => {
                        if alive.len() >= self.config.particles {
                            let lightest = (0..alive.len())
                                .min_by(|&a, &b| alive[a].log_w.total_cmp(&alive[b].log_w))
                                .unwrap();
                            alive.swap_remove(lightest);
                        }
                        alive.push(p);
                    }
                    Err(d) => {
                        record_evidence(&cx, d, &mut self.evidence);
                        self.stats.deaths += 1;
                        self.last_failure = format!("{d:?}: {}", p.why);
                    }
                },
                Err(e @ (WorldError::Team(_) | WorldError::UnknownSpecies(_))) => {
                    error = Some(e);
                    break;
                }
                Err(e) => {
                    self.last_failure = e.to_string();
                    error = Some(e);
                }
            }
        }
        if alive.is_empty() {
            if tried_illusion {
                self.last_failure = format!("Illusion hypotheses tried and failed; {}", self.last_failure);
            }
            // Their snapshots are still prefixes of the observed history: the next update backtracks
            // from them again.
            self.graveyard = dead;
            return Err(error.unwrap_or(WorldError::NoConsistentWorld));
        }
        // Refill by copying survivors in proportion to their weights; copies split the weight.
        let missing = self.config.particles.saturating_sub(alive.len());
        let picks = systematic(&weights(&alive), missing, self.rng.next_u64());
        let mut counts = vec![0usize; alive.len()];
        for &i in &picks {
            counts[i] += 1;
        }
        for (i, &c) in counts.iter().enumerate() {
            if c > 0 {
                alive[i].log_w -= ((c + 1) as f64).ln();
            }
        }
        for i in picks {
            let mut copy = alive[i].clone();
            copy.rng = self.rng.fork();
            // Copies share a hypothesis; a fresh PRNG (hidden anyway) at least lets them diverge.
            copy.battle.state.prng = crate::prng::Prng::from_seed(copy.rng.seed());
            alive.push(copy);
            self.stats.copies += 1;
        }
        self.particles = alive;
        self.graveyard.clear();
        self.synced = true;
        Ok(())
    }

    /// `k` worlds at the current boundary, drawn from the surviving worlds in proportion to their
    /// weights (systematic resampling driven by `seed`), each reseeded from `seed` (one
    /// PRNG stream per world). Each is an independent battle with a fresh sink.
    pub fn worlds<M: LogSink + Default>(
        &mut self,
        k: usize,
        seed: u64,
    ) -> Result<Vec<Battle<M>>, WorldError> {
        self.update()?;
        let mut seeds = SplitMix64(seed);
        let picks = systematic(&weights(&self.particles), k, seeds.next_u64());
        Ok(picks
            .into_iter()
            .map(|i| {
                let mut b = self.particles[i].battle.clone_with_log(M::default());
                b.state.prng = crate::prng::Prng::from_seed(seeds.seed());
                b
            })
            .collect())
    }

    /// Normalized weights of the current worlds and their effective sample size
    /// (`1 / sum w^2`).
    pub fn weights(&self) -> (Vec<f64>, f64) {
        let w = weights(&self.particles);
        let ess = 1.0 / w.iter().map(|x| x * x).sum::<f64>().max(f64::MIN_POSITIVE);
        (w, ess)
    }

    /// The opponent teams of the current worlds (resolved definitions), for diagnostics and tests.
    pub fn opponent_teams(&self) -> Vec<TeamDef> {
        let opp = 1 - self.viewer.0 as usize;
        self.particles.iter().map(|p| (*p.battle.teams().sides[opp]).clone()).collect()
    }
}

/// Request JSON in the engine's form: whitespace outside strings removed (clients may re-serialize),
/// and a Showdown server's changing `"rqid":N` field dropped.
fn normalize_request(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let (mut in_string, mut escaped) = (false, false);
    for c in json.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if !c.is_whitespace() {
            out.push(c);
        }
    }
    strip_rqid(&out)
}

fn strip_rqid(json: &str) -> String {
    match json.find("\"rqid\":") {
        None => json.to_owned(),
        Some(i) => {
            let rest = &json[i + 7..];
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            let mut out = String::with_capacity(json.len());
            out.push_str(&json[..i]);
            let tail = &rest[end..];
            out.push_str(tail.strip_prefix(',').unwrap_or(tail));
            if out.ends_with(",}") {
                out.replace_range(out.len() - 2.., "}");
            }
            out
        }
    }
}

/// Normalized weights from log weights.
fn weights(particles: &[Particle]) -> Vec<f64> {
    let max = particles.iter().map(|p| p.log_w).fold(f64::NEG_INFINITY, f64::max);
    let raw: Vec<f64> = particles.iter().map(|p| (p.log_w - max).exp()).collect();
    let total: f64 = raw.iter().sum();
    raw.into_iter().map(|w| w / total).collect()
}

/// `k` indices drawn by systematic resampling from normalized `weights` (deterministic in `u`).
fn systematic(weights: &[f64], k: usize, u: u64) -> Vec<usize> {
    let mut out = Vec::with_capacity(k);
    if weights.is_empty() || k == 0 {
        return out;
    }
    let start = (u >> 11) as f64 / (1u64 << 53) as f64 / k as f64;
    let (mut i, mut acc) = (0, weights[0]);
    for j in 0..k {
        let target = start + j as f64 / k as f64;
        while target >= acc && i + 1 < weights.len() {
            i += 1;
            acc += weights[i];
        }
        out.push(i);
    }
    out
}

/// Bring a world's Illusion hypothesis up to the current observations: dropped once every stint it
/// claims is publicly attributed to its user, otherwise its view is rebuilt (once per hypothesis and
/// update, shared through `cached`).
fn refresh_hypothesis(
    cx: &Ctx<'_>,
    hypothesis: &mut Option<Arc<IllusionHypothesis>>,
    source: &mut Option<IllusionSource>,
    cached: &mut Vec<Arc<IllusionHypothesis>>,
) {
    let Some(h) = hypothesis.as_ref() else { return };
    if h.stints.iter().all(|st| stint(cx, *st).is_some_and(|e| e.owner == h.user)) {
        *hypothesis = None;
    } else if h.revision != cx.lines.len() {
        let view = cached.iter().find(|v| v.user == h.user && v.species == h.species && v.stints == h.stints);
        *hypothesis = Some(match view {
            Some(v) => v.clone(),
            None => {
                let src = source.get_or_insert_with(|| cx.tracker.illusion_source(cx.opp()));
                let next = IllusionHypothesis { user: h.user.clone(), species: h.species.clone(),
                    stints: h.stints.clone(), view: src.view(&h.stints, &h.user, &h.species),
                    revision: cx.lines.len(), exact: h.exact };
                let next = Arc::new(next);
                cached.push(next.clone());
                next
            }
        });
    }
}

/// Attempts allowed to the recovery of one update (`repair_sets`, then `backtrack`), in segment
/// budgets (a restore that dies spends at most one segment budget on the segment it dies in;
/// reproduced segments cost far less).
const BACKTRACK_SEGMENTS: u64 = 8;

/// Recovery after a collapse: restart from snapshots of the dead worlds and re-advance only the
/// segments since, instead of replaying the whole history from a fresh team.
///
/// * Order: each lineage's snapshots from newest to oldest, one rank at a time across lineages
///   (not the boundary it died from, unless that segment introduced a Pokemon to draw anew);
///   snapshots of copies (same boundary, same opponent team and hypothesis) only after all distinct
///   ones, as a copy differs only in random outcomes that a restore redraws anyway. A lineage
///   whose set contradicts what is now known of its Pokemon (`contradicted`: a move, item, ability
///   or level revealed since) skips every snapshot in which that Pokemon is already in play.
/// * Diversity: each restore takes a new random stream (resample keys and the opponent's open
///   choices) and draws anew the hidden sets of Pokemon revealed since its snapshot, under
///   everything revealed now (`redraw_hidden`), covering distinct pool records first
///   (`RebuildSamples`, as collapsed rebuilds do). Snapshots carry their Illusion hypothesis.
/// * Weights: a restored world starts from its snapshot's `log_w` and pays for the segments it
///   re-advances like any world (plus the pool-coverage correction of its new sets), so it compares
///   with survivors and fresh builds on the usual terms. Worlds restored from one snapshot split
///   its weight like copies.
/// * Budget: `backtrack_budget` restores, cycling through the candidates, until `stats.attempts`
///   reaches `cap` (what set repairs left of `BACKTRACK_SEGMENTS`); it stops once `target` worlds
///   came back. Restores draw from their own stream and record no Illusion evidence, so a
///   fruitless backtrack leaves the rebuild that follows unchanged.
#[allow(clippy::too_many_arguments)]
fn backtrack(
    cx: &Ctx<'_>,
    dead: &[Particle],
    target: usize,
    cap: u64,
    rng: &mut SplitMix64,
    stats: &mut WorldStats,
    last_failure: &mut String,
    source: &mut Option<IllusionSource>,
    cached: &mut Vec<Arc<IllusionHypothesis>>,
) -> Vec<Particle> {
    let opp = cx.opp().0 as usize;
    // (copies before it, rank in its lineage, lineage, index in the lineage's history)
    let mut candidates: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (d, p) in dead.iter().enumerate() {
        let mut hypothesis = p.hypothesis.clone();
        refresh_hypothesis(cx, &mut hypothesis, source, cached);
        let hcx = Ctx { seen: hypothesis.as_ref().map_or(cx.seen, |h| &h.view), hypothesis: hypothesis.as_deref(), ..*cx };
        let fixed = contradicted(&hcx, &p.battle);
        // An exact HP that a later line contradicts became one of several at `since`: restores at
        // or before it draw that HP anew, later ones keep part of it, so they come after.
        let since = match p.death {
            Some(Dead::ExactHp { since, .. }) => since,
            _ => usize::MAX,
        };
        let first = candidates.len();
        for (k, snap) in p.history.iter().enumerate().rev() {
            // The world just spent a segment budget from its last boundary; only a set drawn anew
            // there makes another try differ from that. Restarting before the battle started is
            // what a rebuild does.
            if !snap.battle.state.started
                || snap.seg == p.seg && !snap.introduces.load(Ordering::Relaxed)
                || fixed.iter().any(|name| in_play(cx, &snap.battle, name))
                || candidates.iter().any(|&(_, _, e, j)| Arc::ptr_eq(&dead[e].history[j], snap))
            {
                continue;
            }
            candidates.push((0, 0, d, k));
        }
        candidates[first..].sort_by_key(|c| dead[d].history[c.3].cursor > since);
        for (rank, c) in candidates[first..].iter_mut().enumerate() {
            c.1 = rank;
        }
    }
    let same = |a: &Snapshot, b: &Snapshot| {
        a.cursor == b.cursor && same_world(opp, &a.battle, &a.hypothesis, &b.battle, &b.hypothesis)
    };
    for i in 0..candidates.len() {
        let (_, _, d, k) = candidates[i];
        let s = &dead[d].history[k];
        candidates[i].0 = candidates[..i].iter().filter(|&&(_, _, e, j)| same(&dead[e].history[j], s)).count();
    }
    candidates.sort_by_key(|&(copies, rank, d, _)| (copies, rank, d));
    let samples = RebuildSamples::default();
    let bcx = Ctx { rebuild_samples: Some(&samples), ..*cx };
    let (mut out, mut from) = (Vec::new(), Vec::new());
    let mut tries = 0;
    while !candidates.is_empty() && out.len() < target && tries < cx.config.backtrack_budget && stats.attempts < cap {
        let c = tries % candidates.len();
        tries += 1;
        let (_, _, d, k) = candidates[c];
        let history = &dead[d].history;
        let snap = &history[k];
        stats.restores += 1;
        let mut p = Particle {
            battle: snap.battle.clone(),
            cursor: snap.cursor,
            choice: snap.choice,
            rng: rng.fork(),
            log_w: snap.log_w,
            set_log_w: snap.set_log_w,
            why: String::new(),
            hypothesis: snap.hypothesis.clone(),
            repaired: snap.repaired,
            built_at: snap.built_at,
            tera_known: snap.tera_known,
            seg: snap.seg,
            history: history[..k].to_vec(),
            trail: Replay::before(&dead[d].trail, snap.seg),
            death: None,
            tried: Vec::new(),
        };
        p.battle.state.prng = crate::prng::Prng::from_seed(p.rng.seed());
        refresh_hypothesis(cx, &mut p.hypothesis, source, cached);
        let result = redraw_hidden(&bcx, &mut p).and_then(|()| {
            advance_capped(&bcx, &mut p, cap, stats)
        });
        match result {
            Ok(()) => {
                stats.backtracks += 1;
                out.push(p);
                from.push(c);
            }
            Err(dd) => {
                *last_failure = format!("{dd:?} after backtracking to line {}: {}", snap.cursor, p.why);
            }
        }
    }
    for i in 0..out.len() {
        let n = from.iter().filter(|&&c| c == from[i]).count();
        if n > 1 {
            out[i].log_w -= (n as f64).ln();
        }
    }
    out
}

/// Worlds that differ at most in random outcomes (copies): the same opponent team and hypothesis.
fn same_world(
    opp: usize,
    a: &Battle<DrawLog>,
    a_hypothesis: &Option<Arc<IllusionHypothesis>>,
    b: &Battle<DrawLog>,
    b_hypothesis: &Option<Arc<IllusionHypothesis>>,
) -> bool {
    a.teams().sides[opp] == b.teams().sides[opp]
        && match (a_hypothesis, b_hypothesis) {
            (None, None) => true,
            (Some(x), Some(y)) => x.user == y.user && x.stints == y.stints,
            _ => false,
        }
}

/// Opponent Pokemon whose set in `b` contradicts what is now known of them: a level, gender, item,
/// ability or move revealed after the set was drawn (an Illusion user shows its own details only
/// when the disguise breaks; Tera types are fixed up by `retera`).
fn contradicted<'a>(cx: &Ctx<'a>, b: &Battle<DrawLog>) -> Vec<&'a str> {
    let team = &b.teams().sides[cx.opp().0 as usize];
    (0..cx.seen.len())
        .filter(|&i| {
            let c = constraints(cx, i);
            team.sets[..team.len as usize].iter().find(|s| s.name == c.name).is_some_and(|set| {
                let moves = &set.moves[..set.move_count as usize];
                !(c.level.is_none_or(|l| l == set.level)
                    && (!matches!(c.gender, Gender::Male | Gender::Female) || c.gender == set.gender)
                    && c.item.is_none_or(|i| i == set.item)
                    && c.ability.is_none_or(|a| a == set.ability)
                    && c.moves.iter().all(|m| moves.contains(m)))
            })
        })
        .map(|i| cx.seen[i].name.as_str())
        .collect()
}

/// The opponent Pokemon `name` has entered play in `b` (its set can no longer be swapped).
fn in_play(cx: &Ctx<'_>, b: &Battle<DrawLog>, name: &str) -> bool {
    let side = &b.state.sides[cx.opp().0 as usize];
    let hidden = b.hidden_mons(cx.viewer);
    side.party[..side.pokemon_count as usize].iter().any(|&m| set_name(b, m) == name && hidden & (1 << m.0) == 0)
}

/// Draw anew the sets of hidden opponent Pokemon that were revealed after this boundary, under
/// everything revealed now (`replace_hidden_set` is exact for them).
fn redraw_hidden(cx: &Ctx<'_>, p: &mut Particle) -> Result<(), Dead> {
    let hypothesis = p.hypothesis.clone();
    let cx = &Ctx { seen: hypothesis.as_ref().map_or(cx.seen, |h| &h.view), hypothesis: hypothesis.as_deref(), ..*cx };
    let opp = cx.opp();
    let hidden = p.battle.hidden_mons(cx.viewer);
    let side = &p.battle.state.sides[opp.0 as usize];
    for mon in side.party[..side.pokemon_count as usize].to_vec() {
        let Some(index) = cx.seen.iter().position(|m| m.name == set_name(&p.battle, mon)) else { continue };
        if hidden & (1 << mon.0) == 0 {
            continue;
        }
        let before_patches = cx.patches.get();
        let (set, log_w) = sample_revealed(cx, index, &mut p.rng).map_err(|e| {
            p.why = format!("redrawing {}: {e}", cx.seen[index].name);
            Dead::Swap
        })?;
        p.battle.replace_hidden_set(cx.viewer, mon, &set).map_err(|_| Dead::Swap)?;
        let slot = mon.0 as usize % 6;
        p.log_w += log_w - p.set_log_w[slot];
        p.set_log_w[slot] = log_w;
        p.repaired |= cx.patches.get() > before_patches;
        p.tera_known &= !(1 << mon.0);
        if cx.seen[index].tera_type.is_some() { p.tera_known |= 1 << mon.0; }
    }
    Ok(())
}

/// Set repairs allowed to one update, in segment budgets (a set tried replays the recorded segments
/// since its Pokemon entered once each, then searches the segment its world died in).
const REPAIR_SEGMENTS: u64 = 4;

/// Pool sets tried per dead world, across its opponent Pokemon.
const REPAIR_CANDIDATES: usize = 8;

/// Recovery by exact set repair, when every world died: a revealed opponent Pokemon of a dead world
/// gets another pool set at the world's last snapshot where it was still hidden
/// (`replace_hidden_set` is exact there), the segments since are replayed with their recorded
/// randomness and choices (`replay_set`: no search, so it is cheap), and the world searches on from
/// the boundary it died at. Unlike `backtrack` this also reaches the leads (the snapshot before the
/// start) and keeps every outcome the world already reproduced. A Pokemon that could not enter at
/// all (a species with a battle-start effect fits only a slot that was hidden from the start) is
/// given such a slot from the start the same way.
///
/// * Order: distinct dead worlds before copies (same boundary, opponent team and hypothesis).
///   Within a world: Pokemon whose set contradicts what is known now (`contradicted`), then the
///   actor of the line the world died on, then the active ones (a set that fits every tracked fact
///   can still miss a speed order or a damage roll), each with the pool's exact-support records
///   most frequent first (`SetPool::repair_candidates`).
/// * Weights: replace the Pokemon's old set-dependent weight with the new record's share of all
///   records consistent with what is revealed (those beyond the bound included) and its Tera share
///   or penalty. The world keeps its replay effort and pays for new searches like any world.
/// * Budget: `repair_budget` sets in total, `REPAIR_CANDIDATES` per world and `REPAIR_SEGMENTS`
///   segment budgets of attempts (replays included), up to `cap`; it stops at the first world that
///   comes back (copies of it fill the rest). Tried pool records are kept by Pokemon, so later
///   updates skip only completed rejections even when new reveals change the candidate order.
///   A cap-interrupted candidate stays eligible.
///   It draws only from the dead worlds' own streams, so the rebuild after a fruitless repair
///   draws what it would have drawn without it.
fn repair_sets(
    cx: &Ctx<'_>,
    dead: &mut [Particle],
    cap: u64,
    stats: &mut WorldStats,
    source: &mut Option<IllusionSource>,
    cached: &mut Vec<Arc<IllusionHypothesis>>,
) -> Vec<Particle> {
    let opp = cx.opp().0 as usize;
    let mut order: Vec<(usize, usize)> = (0..dead.len())
        .map(|d| {
            let (p, earlier) = (&dead[d], &dead[..d]);
            let copies = earlier
                .iter()
                .filter(|e| e.cursor == p.cursor && same_world(opp, &e.battle, &e.hypothesis, &p.battle, &p.hypothesis))
                .count();
            (copies, d)
        })
        .collect();
    order.sort_unstable();
    let cap = cap.min(stats.attempts + u64::from(cx.config.segment_budget) * REPAIR_SEGMENTS);
    let mut budget = cx.config.repair_budget;
    for (_, d) in order {
        if budget == 0 || stats.attempts >= cap {
            break;
        }
        if let Some(p) = repair_world(cx, &mut dead[d], &mut budget, cap, stats, source, cached) {
            stats.set_repairs += 1;
            return vec![p];
        }
    }
    Vec::new()
}

/// `repair_sets` for one dead world: the first set that replays and then reaches the end of the
/// observed stream.
fn repair_world(
    cx: &Ctx<'_>,
    p: &mut Particle,
    budget: &mut usize,
    cap: u64,
    stats: &mut WorldStats,
    source: &mut Option<IllusionSource>,
    cached: &mut Vec<Arc<IllusionHypothesis>>,
) -> Option<Particle> {
    let Some(death) = p.death else { return None };
    let at = p.history.last().filter(|s| s.seg == p.seg)?;
    let mut hypothesis = p.hypothesis.clone();
    refresh_hypothesis(cx, &mut hypothesis, source, cached);
    let cx = &Ctx { seen: hypothesis.as_ref().map_or(cx.seen, |h| &h.view), hypothesis: hypothesis.as_deref(), ..*cx };
    let opp = cx.opp();
    let team = p.battle.teams().sides[opp.0 as usize].clone();
    let actor = death
        .line()
        .and_then(|line| cx.lines.get(line))
        .and_then(|l| Line::parse(l))
        .and_then(|l| parse_ident(l.arg(0)).filter(|id| id.side == opp).map(|id| id.name.to_owned()));
    let fixed = contradicted(cx, &p.battle);
    let active = at.battle.state.sides[opp.0 as usize].active;
    let hidden = at.battle.hidden_mons(cx.viewer);
    let start = p.history.first().filter(|s| !s.battle.state.started).map(|_| 0);
    let mut free = (0..team.len as usize)
        .filter(|&k| hidden & (1 << (opp.0 * 6 + k as u8)) != 0 && cx.tracker.find(opp, &team.sets[k].name).is_none());
    // (seen index, Pokemon, its last snapshot while hidden)
    let mut mons: Vec<(usize, MonId, usize)> = (0..cx.seen.len())
        .filter_map(|i| match team.sets[..team.len as usize].iter().position(|s| s.name == cx.seen[i].name) {
            Some(slot) if death != Dead::Swap => {
                let mon = MonId(opp.0 * 6 + slot as u8);
                let anchor = p.history.iter().rposition(|s| s.battle.hidden_mons(cx.viewer) & (1 << mon.0) != 0)?;
                Some((i, mon, anchor))
            }
            // A Pokemon that could not enter: a species with a battle-start effect takes a hidden slot
            // only before the start.
            None if death == Dead::Swap => Some((i, MonId(opp.0 * 6 + free.next()? as u8), start?)),
            _ => None,
        })
        .collect();
    mons.sort_by_key(|&(i, mon, _)| {
        let name = cx.seen[i].name.as_str();
        (!fixed.contains(&name), actor.as_deref() != Some(name), !active.contains(&mon), i)
    });
    let mut rng = p.rng;
    let mut left = REPAIR_CANDIDATES.min(*budget);
    let repaired = 'search: {
        for (i, mon, anchor) in mons {
            let old = &team.sets[mon.0 as usize % 6];
            let c = constraints(cx, i);
            for (record, mut set, share) in repair_candidates(cx, p, mon, &c, old, left, &mut rng) {
                let mut set_log_w = share.ln();
                if let Some(tera) = c.tera {
                    set_log_w += cx.pool.tera_share(&set, tera).filter(|&s| s > 0.0)
                        .map_or(-REPAIR_LOG_PENALTY, f64::ln);
                    set.tera_type = tera;
                }
                repair_move_order(&mut set, old, move_order(cx, &old.name));
                if set == *old || p.tried.contains(&(mon, record)) {
                    continue;
                }
                if left == 0 || stats.attempts >= cap {
                    break 'search None;
                }
                left -= 1;
                *budget -= 1;
                stats.set_repair_attempts += 1;
                let result = replay_set_capped(cx, p, anchor, mon, &set, set_log_w, cap, stats).and_then(|mut q| {
                    q.rng = rng.fork();
                    q.hypothesis = hypothesis.clone();
                    match advance_capped(cx, &mut q, cap, stats) {
                        Ok(()) => Ok(q),
                        Err(Dead::Interrupted { .. }) => Err(ReplaySetError::Interrupted),
                        Err(_) => Err(ReplaySetError::Rejected),
                    }
                });
                match result {
                    Ok(q) => break 'search Some(q),
                    Err(ReplaySetError::Interrupted) => break 'search None,
                    Err(ReplaySetError::Rejected) => p.tried.push((mon, record)),
                }
            }
        }
        None
    };
    repaired
}

/// Inspect a larger prefix only when tried records or copies of the old set leave too few new
/// candidates. Reuse the draw stream, so enlarging the prefix does not shuffle earlier proposals.
fn repair_candidates(
    cx: &Ctx<'_>,
    p: &Particle,
    mon: MonId,
    c: &SetConstraints<'_>,
    old: &SetDef,
    left: usize,
    rng: &mut SplitMix64,
) -> Vec<(u32, SetDef, f64)> {
    let mut limit = p.tried.len() + left + 1;
    loop {
        let mut draw = *rng;
        let candidates = cx.pool.repair_candidates(c, limit, &mut draw);
        let new = candidates.iter().filter(|(record, set, _)| {
            if p.tried.contains(&(mon, *record)) { return false; }
            let mut set = set.clone();
            if let Some(tera) = c.tera { set.tera_type = tera; }
            repair_move_order(&mut set, old, move_order(cx, &old.name));
            set != *old
        }).take(left).count();
        if new == left || candidates.len() < limit {
            *rng = draw;
            return candidates;
        }
        limit = limit.saturating_mul(2);
    }
}

/// Keep the moves a new set shares with the old one in their old slots (recorded choices name the
/// slot), or use the slot order a transformed viewer's request showed.
fn repair_move_order(set: &mut SetDef, old: &SetDef, order: Option<&[EffectId]>) {
    if let Some(order) = order {
        set.moves = [EffectId::NONE; 4];
        set.moves[..order.len()].copy_from_slice(order);
        set.move_count = order.len() as u8;
        return;
    }
    let n = set.move_count as usize;
    let have = set.moves;
    set.moves = [EffectId::NONE; 4];
    for i in 0..n.min(old.move_count as usize) {
        if have[..n].contains(&old.moves[i]) {
            set.moves[i] = old.moves[i];
        }
    }
    for &m in &have[..n] {
        if !set.moves[..n].contains(&m) {
            let free = set.moves[..n].iter().position(|&x| x == EffectId::NONE).unwrap();
            set.moves[free] = m;
        }
    }
}

/// The viewer's request at a boundary, if pushed, is what `b` asks.
fn replay_request(cx: &Ctx<'_>, b: &Battle<DrawLog>, choice: usize) -> bool {
    !needs(b, cx.viewer)
        || cx.requests.get(choice).and_then(|r| r.as_deref()).is_none_or(|want| {
            b.request_json(cx.viewer.0 as usize).as_deref() == Some(want)
        })
}

/// The dead world `p` with `set` for `mon` from its snapshot `anchor` on (`mon` is hidden there,
/// so `replace_hidden_set` is exact), replayed to the boundary it died at with every segment's
/// recorded team changes, PRNG, resample schedule and choices. `None` when any segment differs from
/// what the world reproduced before: a compared line or entry owner, the viewer's request, who
/// acts next, or the number of random draws (a new set can shift later draws, e.g. by a new speed
/// tie). Snapshots from the anchor on are taken anew at the same boundaries; they and the world
/// replace the Pokemon's set-dependent contribution with `set_log_w`.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn replay_set(
    cx: &Ctx<'_>,
    p: &Particle,
    anchor: usize,
    mon: MonId,
    set: &SetDef,
    set_log_w: f64,
    stats: &mut WorldStats,
) -> Option<Particle> {
    replay_set_capped(cx, p, anchor, mon, set, set_log_w, u64::MAX, stats).ok()
}

#[derive(Debug)]
enum ReplaySetError {
    Rejected,
    Interrupted,
}

#[allow(clippy::too_many_arguments)]
fn replay_set_capped(
    cx: &Ctx<'_>,
    p: &Particle,
    anchor: usize,
    mon: MonId,
    set: &SetDef,
    set_log_w: f64,
    cap: u64,
    stats: &mut WorldStats,
) -> Result<Particle, ReplaySetError> {
    let (viewer, opp) = (cx.viewer, cx.opp());
    let from = &p.history[anchor];
    let mut battle = from.battle.clone();
    battle.replace_hidden_set(viewer, mon, set).map_err(|_| ReplaySetError::Rejected)?;
    let last = p.history.last().ok_or(ReplaySetError::Rejected)?;
    let mut records = Vec::new();
    let mut t = p.trail.as_ref();
    while let Some(r) = t.filter(|r| r.seg >= from.seg) {
        records.push(r);
        t = r.previous.as_ref();
    }
    if last.seg != p.seg || records.len() != (p.seg - from.seg) as usize {
        return Err(ReplaySetError::Rejected);
    }
    let mut trail = t.cloned();
    let slot = mon.0 as usize % 6;
    let replaced_weight = |mut weights: [f64; 6]| { weights[slot] = set_log_w; weights };
    let tera_known = if cx.seen.iter().any(|m| m.name == set.name && m.tera_type.is_some()) { 1 << mon.0 } else { 0 };
    let mut history = p.history[..anchor].to_vec();
    history.push(Arc::new(Snapshot {
        battle: battle.clone(),
        log_w: from.log_w - from.set_log_w[slot] + set_log_w,
        set_log_w: replaced_weight(from.set_log_w),
        hypothesis: from.hypothesis.clone(),
        tera_known: (from.tera_known & !(1 << mon.0)) | tera_known,
        introduces: AtomicBool::new(from.introduces.load(Ordering::Relaxed)),
        ..**from
    }));
    let mut later = p.history[anchor + 1..].iter().peekable();
    let mut produced = Vec::new();
    for r in records.into_iter().rev() {
        if stats.attempts >= cap {
            return Err(ReplaySetError::Interrupted);
        }
        let now = battle.teams().sides[opp.0 as usize].clone();
        if !Arc::ptr_eq(&now, &r.team) {
            for (i, (a, b)) in now.sets.iter().zip(&r.team.sets).enumerate().take(r.team.len as usize) {
                let m = MonId(opp.0 * 6 + i as u8);
                if m == mon || a == b {
                    continue;
                }
                let mut retyped = a.clone();
                retyped.tera_type = b.tera_type;
                if retyped == *b {
                    battle.set_hidden_tera_type(viewer, m, b.tera_type).map_err(|_| ReplaySetError::Rejected)?;
                } else {
                    battle.replace_hidden_set(viewer, m, b).map_err(|_| ReplaySetError::Rejected)?;
                }
            }
        }
        let team = battle.teams().sides[opp.0 as usize].clone();
        battle.log.track_entries = team.sets.iter().any(|s| s.ability == dex::ABILITY_ILLUSION);
        battle.log.reset();
        battle.state.prng = r.prng;
        battle.state.prng.set_resample(r.key, &r.points);
        stats.attempts += 1;
        stats.repair_replays += 1;
        if !battle.state.started {
            battle.start().map_err(|_| ReplaySetError::Rejected)?;
        } else {
            if r.viewer {
                battle.choose(viewer.0 as usize, &cx.choices[r.choice].1).map_err(|_| ReplaySetError::Rejected)?;
            }
            if let Some(choice) = &r.opponent {
                battle.choose_typed(opp, choice).map_err(|_| ReplaySetError::Rejected)?;
            }
        }
        if battle.state.prng.draws() != r.end_draws {
            stats.repair_draw_shifts += 1;
            return Err(ReplaySetError::Rejected);
        }
        produced.clear();
        view(&battle.log, viewer, &mut produced);
        let region = &cx.lines[r.cursor..r.end];
        let entries = &battle.log.text.entries;
        if produced.len() != region.len()
            || produced.iter().zip(region).enumerate().any(|(offset, (&(raw, _), want))| {
                entries[raw] != *want || !entry_matches(cx, &battle, raw, r.cursor + offset)
            })
            || [needs(&battle, viewer), needs(&battle, opp), battle.state.ended] != r.boundary
            || !replay_request(cx, &battle, r.choice + usize::from(r.viewer))
        {
            return Err(ReplaySetError::Rejected);
        }
        trail = Some(Arc::new(Replay {
            previous: trail,
            seg: r.seg,
            cursor: r.cursor,
            end: r.end,
            choice: r.choice,
            viewer: r.viewer,
            team,
            prng: r.prng,
            key: r.key,
            points: r.points.clone(),
            opponent: r.opponent,
            end_draws: r.end_draws,
            boundary: r.boundary,
        }));
        if let Some(s) = later.next_if(|s| s.seg == r.seg + 1) {
            history.push(Arc::new(Snapshot {
                battle: battle.clone(),
                log_w: s.log_w - s.set_log_w[slot] + set_log_w,
                set_log_w: replaced_weight(s.set_log_w),
                tera_known: (s.tera_known & !(1 << mon.0)) | tera_known,
                hypothesis: s.hypothesis.clone(),
                introduces: AtomicBool::new(s.introduces.load(Ordering::Relaxed)),
                ..**s
            }));
        }
    }
    Ok(Particle {
        battle,
        cursor: last.cursor,
        choice: last.choice,
        rng: p.rng,
        log_w: last.log_w - last.set_log_w[slot] + set_log_w,
        set_log_w: replaced_weight(last.set_log_w),
        why: String::new(),
        hypothesis: p.hypothesis.clone(),
        repaired: last.repaired,
        built_at: last.built_at,
        tera_known: (last.tera_known & !(1 << mon.0)) | tera_known,
        seg: last.seg,
        history,
        trail,
        death: None,
        tried: Vec::new(),
    })
}

/// A fresh world at the start of the battle: the viewer's team and a sampled opponent team.
fn build(
    cx: &Ctx<'_>,
    hypothesis: Option<Arc<IllusionHypothesis>>,
    own: Arc<TeamDef>,
    names: &[String; 2],
    rng: &mut SplitMix64,
    stats: &mut WorldStats,
) -> Result<Particle, WorldError> {
    let mut prng = rng.fork();
    let cx = &Ctx { seen: hypothesis.as_ref().map_or(cx.seen, |h| &h.view), hypothesis: hypothesis.as_deref(), ..*cx };
    let before_patches = cx.patches.get();
    let (opp, set_log_w) = sample_team(cx, &mut prng)?;
    let opp = Arc::new(opp);
    let log_w = set_log_w.iter().sum();
    let (p1, p2) = if cx.viewer.0 == 0 { (own, opp) } else { (opp, own) };
    let log = DrawLog { track_entries: has_illusion(&p1, &p2, cx.opp()), ..DrawLog::default() };
    let battle = Battle::from_team_defs(prng.seed(), p1, p2, names.clone(), log).map_err(|e| WorldError::Team(e.0))?;
    stats.builds += 1;
    let repaired = cx.patches.get() > before_patches;
    let built_at = cx.lines.len();
    Ok(Particle {
        battle,
        cursor: 0,
        choice: 0,
        rng: prng,
        log_w,
        set_log_w,
        why: String::new(),
        hypothesis,
        repaired,
        built_at,
        tera_known: 0,
        seg: 0,
        history: Vec::new(),
        trail: None,
        death: None,
        tried: Vec::new(),
    })
}

/// Constraints for an opponent Pokemon from what the tracker saw.
fn constraints<'a>(cx: &'a Ctx<'_>, index: usize) -> SetConstraints<'a> {
    let mut c = mon_constraints(&cx.seen[index]);
    if let Some(order) = move_order(cx, c.name) {
        c.moves = order;
    }
    c
}

fn move_order<'a>(cx: &'a Ctx<'_>, name: &str) -> Option<&'a [EffectId]> {
    cx.move_orders
        .iter()
        .rev()
        .find(|(n, st, _)| {
            let owner = stint(cx, *st).map_or(n.as_str(), |e| cx.owner(e));
            owner == name
        })
        .map(|(_, _, order)| order.as_slice())
}

fn mon_constraints(m: &MonInfo) -> SetConstraints<'_> {
    SetConstraints {
        name: &m.name,
        shown_species: &m.details_species,
        level: (m.level != 0).then_some(m.level),
        gender: m.gender,
        shiny: m.shiny,
        moves: &m.moves,
        item: m.starting_item,
        ability: m.starting_ability,
        tera: m.tera_type,
    }
}

fn sample_revealed(cx: &Ctx<'_>, index: usize, rng: &mut SplitMix64) -> Result<(SetDef, f64), WorldError> {
    let c = constraints(cx, index);
    let mut log_w = 0.0;
    let sampled = if let Some(samples) = cx.rebuild_samples.filter(|_| cx.hypothesis.is_none()) {
        let mut used = samples.used.borrow_mut();
        used.resize_with(cx.seen.len(), Vec::new);
        cx.pool
            .sample_distinct(&c, rng, &mut used[index])
            .map(|(sampled, weight)| {
                log_w = weight.ln();
                sampled
            })
    } else {
        cx.pool.sample(&c, rng)
    }
    .ok_or_else(|| WorldError::UnknownSpecies(c.shown_species.to_owned()))?;
    let mut set = sampled.set;
    if !sampled.consistent {
        if !cx.repair {
            return Err(WorldError::NoConsistentWorld);
        }
        patch_set(cx, &mut set, &c);
        log_w -= REPAIR_LOG_PENALTY;
    }
    if let Some(order) = move_order(cx, c.name) {
        set.moves = [EffectId::NONE; 4];
        set.moves[..order.len()].copy_from_slice(order);
        set.move_count = order.len() as u8;
    }
    Ok((set, log_w))
}

fn patch_set(cx: &Ctx<'_>, set: &mut SetDef, c: &SetConstraints<'_>) {
    let original = SetDef { tera_type: c.tera.unwrap_or(set.tera_type), ..set.clone() };
    repair(set, c);
    cx.repairs.set(cx.repairs.get() + 1);
    if *set != original {
        cx.patches.set(cx.patches.get() + 1);
    }
}

/// Makes the best pool set fit what was revealed when no pool set does (a live server whose sets
/// changed since the pool was generated: a new move, item, ability or level). Without this, one
/// off-pool reveal kills every world for the rest of the battle. Revealed moves replace the set's
/// unrevealed ones from the end; an item that would change the shown species is left alone.
fn repair(set: &mut SetDef, c: &SetConstraints<'_>) {
    let mut revealed: Vec<EffectId> = Vec::with_capacity(4);
    for &m in c.moves {
        if !revealed.contains(&m) && revealed.len() < 4 {
            revealed.push(m);
        }
    }
    let have = &set.moves[..set.move_count as usize];
    if revealed.iter().any(|m| !have.contains(m)) {
        let mut moves: Vec<EffectId> = have.to_vec();
        for &m in &revealed {
            if moves.contains(&m) {
                continue;
            }
            if moves.len() < 4 {
                moves.push(m);
            } else if let Some(slot) = moves.iter().rposition(|x| !revealed.contains(x)) {
                moves[slot] = m;
            }
        }
        set.moves = [EffectId::NONE; 4];
        set.moves[..moves.len()].copy_from_slice(&moves);
        set.move_count = moves.len() as u8;
    }
    if let Some(level) = c.level {
        set.level = level;
    }
    if let Some(ability) = c.ability {
        set.ability = ability;
    }
    if let Some(tera) = c.tera {
        set.tera_type = tera;
    }
    if let Some(item) = c.item.filter(|&i| i != set.item) {
        let old = std::mem::replace(&mut set.item, item);
        if SetPool::shown_species(set) != c.shown_species {
            set.item = old;
        }
    }
}

/// Move ids of each `active` entry of a normalized request JSON, in slot order (a minimal scan: the
/// engine's and Showdown's requests list `"id":"..."` per move inside `"moves":[...]`); empty for an
/// entry that is not a full moveset.
fn active_move_ids(json: &str) -> Vec<Vec<EffectId>> {
    let Some(start) = json.find("\"active\":[") else { return Vec::new() };
    let rest = &json[start..];
    let end = rest.find("\"side\":").unwrap_or(rest.len());
    rest[..end]
        .split("\"moves\":[")
        .skip(1)
        .map(|entry| {
            let entry = &entry[..entry.find(']').unwrap_or(entry.len())];
            let ids: Vec<EffectId> = entry
                .split("\"id\":\"")
                .skip(1)
                .filter_map(|t| dex::lookup(EffectKind::Move, &t[..t.find('"').unwrap_or(t.len())]))
                .collect();
            // A locked or charging move's request lists only that move, without PP: not the moveset.
            if entry.matches("\"pp\":").count() == ids.len() { ids } else { Vec::new() }
        })
        .collect()
}

/// The opponent's team: revealed Pokemon get sets consistent with what was seen (leads at their
/// lead positions, the others at random bench positions), unrevealed slots sets from the pool's
/// distribution under the species clause.
fn sample_team(cx: &Ctx<'_>, rng: &mut SplitMix64) -> Result<(TeamDef, [f64; 6]), WorldError> {
    if cx.hypothesis.is_none()
        && !cx.seen.iter().any(|m| {
            m.illusion
                || m.starting_ability == Some(dex::ABILITY_ILLUSION)
                || m.starting_ability.is_none() && cx.pool.illusion_species().iter().any(|(s, _)| *s == m.species)
        })
    {
        return sample_plain_team(cx, rng);
    }
    let entries = cx.tracker.stints(cx.opp());
    let sampled: Vec<_> = (0..cx.seen.len()).map(|i| sample_revealed(cx, i, rng)).collect::<Result<_, _>>()?;
    let (mut sets, mut weights): (Vec<_>, Vec<_>) = sampled.into_iter().unzip();
    // A broken lead's disguise may have no remaining MonInfo, but the party still needs it.
    for e in entries {
        if !sets.iter().any(|s| s.name == e.shown.name) {
            let c = mon_constraints(&e.shown);
            let sampled =
                cx.pool.sample(&c, rng).ok_or_else(|| WorldError::UnknownSpecies(c.shown_species.to_owned()))?;
            let mut set = sampled.set;
            let mut log_w = 0.0;
            if !sampled.consistent {
                if !cx.repair {
                    return Err(WorldError::NoConsistentWorld);
                }
                patch_set(cx, &mut set, &c);
                log_w -= REPAIR_LOG_PENALTY;
            }
            sets.push(set);
            weights.push(log_w);
        }
    }
    let size = cx.tracker.team_size(cx.opp()).unwrap_or(6).clamp(1, 6) as usize;
    if sets.len() > size {
        return Err(WorldError::Desync("Illusion identities exceed opponent team size"));
    }
    let mut species: Vec<_> = sets.iter().map(|s| s.species).collect();
    while sets.len() < size {
        let set = cx.pool.sample_unrevealed(&species, rng).ok_or(WorldError::Desync("the set pool is empty"))?;
        species.push(set.species);
        sets.push(set);
        weights.push(0.0);
    }
    let order = party_order(cx, &sets, rng).ok_or(WorldError::IllusionOrder)?;
    let mut team = TeamDef { len: size as u8, ..TeamDef::default() };
    let mut set_log_w = [0.0; 6];
    for (pos, i) in order.into_iter().enumerate().take(size) {
        team.sets[pos] = sets[i].clone();
        set_log_w[pos] = weights[i];
    }
    Ok((team, set_log_w))
}

/// Compile the small public party history once, then enumerate at most 720 initial permutations.
/// Switching swaps the incoming and outgoing party positions *before* Illusion chooses the last
/// living member after the user. Different disguises therefore impose constraints at different
/// times, rather than competing for one fixed last slot. Replay still checks details and damage.
fn party_order(cx: &Ctx<'_>, sets: &[SetDef], rng: &mut SplitMix64) -> Option<[usize; 6]> {
    #[derive(Clone, Copy)]
    enum Event {
        Enter { mon: usize, shown: usize, pos: usize, lead: bool },
        Faint(usize),
        Swap(usize, usize),
        Revive(usize),
        Tera(usize),
        Transform(usize),
    }
    let index = |name: &str| sets.iter().position(|s| s.name == name);
    let entry = |st| stint(&cx, st);
    let mut events = Vec::new();
    for event in cx.tracker.party_events(cx.opp()) {
        events.push(match *event {
            PartyEvent::Enter(st) => {
                let e = entry(st)?;
                Event::Enter {
                    mon: index(cx.owner(e))?,
                    shown: index(&e.shown.name)?,
                    pos: e.position as usize,
                    lead: e.lead,
                }
            }
            PartyEvent::Faint(st) => Event::Faint(index(cx.owner(entry(st)?))?),
            PartyEvent::Swap(a, b) => Event::Swap(a as usize, b as usize),
            PartyEvent::Revive(ref name) => Event::Revive(index(name)?),
            PartyEvent::Tera(st) => Event::Tera(index(cx.owner(entry(st)?))?),
            PartyEvent::Transform(st) => Event::Transform(index(cx.owner(entry(st)?))?),
        });
    }
    let base_blocked: [bool; 6] = core::array::from_fn(|i| {
        sets.get(i).is_some_and(|s| {
            matches!(dex::species(s.species).base_species, dex::SPECIES_OGERPON | dex::SPECIES_TERAPAGOS)
        })
    });
    let valid = |initial: &[usize; 6]| {
        let mut party = *initial;
        let mut fainted = [false; 6];
        let mut tera = [false; 6];
        let mut blocked = base_blocked.map(Some);
        let mut active = [None; 2];
        for &event in &events {
            match event {
                Event::Enter { mon, shown, pos, lead } => {
                    if pos >= 2 || fainted[mon] || active.contains(&Some(mon)) {
                        return false;
                    }
                    let at = party[..sets.len()].iter().position(|&m| m == mon).unwrap();
                    if lead && at != pos {
                        return false;
                    }
                    party.swap(pos, at);
                    active[pos] = Some(mon);
                    blocked[mon] = Some(base_blocked[mon]);
                    let mut disguise = if sets[mon].ability == dex::ABILITY_ILLUSION {
                        party[pos + 1..sets.len()].iter().rev().copied().find(|&m| !fainted[m]).unwrap_or(mon)
                    } else {
                        mon
                    };
                    if tera[mon] && (blocked[disguise] == Some(true) || blocked[disguise].is_none() && shown == mon) {
                        disguise = mon;
                    }
                    if disguise != shown {
                        return false;
                    }
                }
                Event::Faint(mon) => {
                    fainted[mon] = true;
                    for a in &mut active {
                        if *a == Some(mon) {
                            *a = None;
                        }
                    }
                }
                Event::Swap(a, b) => {
                    if a >= 2 || b >= 2 {
                        return false;
                    }
                    party.swap(a, b);
                    active.swap(a, b);
                }
                Event::Revive(mon) => fainted[mon] = false,
                Event::Tera(mon) => tera[mon] = true,
                // Transform's copied species can depend on another Illusion. Allow either Tera
                // exception here; replay, using sampled teams and the viewer's own team, decides.
                Event::Transform(mon) => blocked[mon] = None,
            }
        }
        true
    };
    fn visit(
        pos: usize,
        size: usize,
        order: &mut [usize; 6],
        valid: &impl Fn(&[usize; 6]) -> bool,
        out: &mut Vec<[usize; 6]>,
    ) {
        if pos == size {
            if valid(order) {
                out.push(*order);
            }
            return;
        }
        for i in pos..size {
            order.swap(pos, i);
            visit(pos + 1, size, order, valid, out);
            order.swap(pos, i);
        }
    }
    let mut orders = Vec::new();
    visit(0, sets.len(), &mut [0, 1, 2, 3, 4, 5], &valid, &mut orders);
    if orders.is_empty() { None } else { Some(orders[rng.below(orders.len())]) }
}

fn illusion_prior(cx: &Ctx<'_>) -> f64 {
    let free = (cx.tracker.team_size(cx.opp()).unwrap_or(6) as usize).saturating_sub(cx.seen.len());
    let count: u32 = cx.pool.illusion_species().iter().map(|(_, n)| n).sum();
    (free as f64 * count as f64 / cx.pool.len().max(1) as f64).min(0.1)
}

fn prior_hypothesis<'a>(
    cx: &Ctx<'_>,
    hypotheses: &'a [Arc<IllusionHypothesis>],
    rng: &mut SplitMix64,
) -> &'a Arc<IllusionHypothesis> {
    let weight = |h: &Arc<IllusionHypothesis>| {
        cx.pool
            .illusion_species()
            .iter()
            .find(|(s, _)| dex::species(*s).name == h.species)
            .map_or(1, |(_, n)| *n as usize)
    };
    let total: usize = hypotheses.iter().map(weight).sum();
    let mut draw = rng.below(total);
    hypotheses
        .iter()
        .find(|h| {
            let n = weight(h);
            let hit = draw < n;
            draw = draw.saturating_sub(n);
            hit
        })
        .unwrap()
}

/// Bounded proposals from public ambiguous entries. Unsupported disguise constraints come first,
/// then recent evidence. Subsets of at most six entries include non-adjacent disguised stints;
/// pool support and overlapping active positions prune them before spending any replay budget.
/// Known Illusion users may explain earlier unbroken entries, but never create another clause peer.
#[cfg(test)]
fn hypotheses(cx: &Ctx<'_>) -> Vec<Arc<IllusionHypothesis>> {
    let unsupported: Vec<_> = (0..cx.seen.len()).filter(|&i| !cx.pool.consistent(&constraints(cx, i)))
        .map(|i| cx.seen[i].name.as_str()).collect();
    prepare_hypotheses(cx, &cx.tracker.illusion_source(cx.opp()), &unsupported, &[], 0, 126)
}

fn prepare_hypotheses(cx: &Ctx<'_>, source: &IllusionSource, unsupported: &[&str], evidence: &[u32],
    rotation: usize, limit: usize) -> Vec<Arc<IllusionHypothesis>> {
    if cx.pool.illusion_species().is_empty() {
        return Vec::new();
    }
    let size = cx.tracker.team_size(cx.opp()).unwrap_or(6) as usize;
    let known = cx.seen.iter().find(|m| {
        m.starting_ability == Some(dex::ABILITY_ILLUSION)
            || (m.species != EffectId::NONE
                && dex::species(m.species).abilities.iter().any(|a| a.id == dex::ABILITY_ILLUSION))
    });
    // A revealed dead user can explain older entries, but cannot be proposed at a later entry.
    // Its lead may have switched out unbroken long before its identity and faint were observed.
    let cutoff = known.filter(|m| m.fainted).and_then(|m| {
        cx.tracker.stints(cx.opp()).iter().filter(|e| e.owner == m.name).filter_map(|e| e.faint_line).max()
    });
    let users: Vec<(String, String)> = if let Some(m) = known {
        if m.fainted && cutoff.is_none() {
            return Vec::new();
        }
        vec![(m.name.clone(), m.details_species.clone())]
    } else {
        if cx.seen.len() >= size {
            return Vec::new();
        }
        cx.pool
            .illusion_species()
            .iter()
            .filter(|(s, _)| {
                !cx.seen.iter().any(|m| {
                    m.species != EffectId::NONE
                        && dex::species(m.species).base_species_name == dex::species(*s).base_species_name
                })
            })
            .map(|(s, _)| (dex::species(*s).base_species_name.to_owned(), dex::species(*s).name.to_owned()))
            .collect()
    };
    let mut entries: Vec<_> = cx
        .tracker
        .stints(cx.opp())
        .iter()
        .rev()
        .filter(|e| {
            !e.certain
                && e.owner == e.shown.name
                && !users.iter().any(|(name, _)| *name == e.owner)
                && cutoff.is_none_or(|line| e.line < line)
        })
        .collect();
    // Evidence and unresolved leads survive recency pressure. Rotate the remainder so every older
    // attribution remains reachable across rebuilds, while at most six entries enter subset search.
    entries.sort_by_key(|e| core::cmp::Reverse((evidence.contains(&e.id),
        unsupported.contains(&e.shown.name.as_str()), e.lead, e.last)));
    if entries.len() > 6 {
        let mut leads: Vec<_> = entries.iter().copied().filter(|e| e.lead).collect();
        entries.retain(|e| !e.lead);
        // Reserve a rotating slot even when more than six stints have evidence. Otherwise the
        // evidence list itself could permanently crowd out an older necessary attribution.
        let fixed = entries.iter().take_while(|e| evidence.contains(&e.id)
            || unsupported.contains(&e.shown.name.as_str())).count().min(5 - leads.len());
        let n = entries.len() - fixed;
        if n > 0 { entries[fixed..].rotate_left(rotation % n); }
        entries.truncate(6 - leads.len());
        entries.append(&mut leads);
        entries.sort_by_key(|e| core::cmp::Reverse((evidence.contains(&e.id),
            unsupported.contains(&e.shown.name.as_str()), e.lead, e.last)));
    }
    let mut out = Vec::new();
    let mut examined = 0;
    'groups: for n in 1..=entries.len() {
        for mask in 1usize..1 << entries.len() {
            if mask.count_ones() as usize != n { continue; }
            for (user, species) in &users {
                if examined == limit { break 'groups; }
                examined += 1;
                let stints = entries.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0)
                    .map(|(_, e)| e.id).collect();
                let mut h = IllusionHypothesis { user: user.clone(), species: species.clone(),
                    stints, view: Vec::new(), revision: cx.lines.len(), exact: false };
                if attributable(cx, &h) {
                    h.view = source.view(&h.stints, &h.user, &h.species);
                    let view_cx = Ctx { seen: &h.view, hypothesis: Some(&h), ..*cx };
                    h.exact = (0..h.view.len()).all(|i| cx.pool.consistent(&constraints(&view_cx, i)));
                    out.push(Arc::new(h));
                }
            }
        }
    }
    out.sort_by_key(|h| !h.exact);
    out
}

fn stint<'a>(cx: &Ctx<'a>, id: u32) -> Option<&'a Stint> {
    let entries = cx.tracker.stints(cx.opp());
    entries.binary_search_by_key(&id, |e| e.id).ok().map(|i| &entries[i])
}

fn has_illusion(p1: &TeamDef, p2: &TeamDef, opp: SideId) -> bool {
    let team = if opp.0 == 0 { p1 } else { p2 };
    team.sets[..team.len as usize].iter().any(|s| s.ability == dex::ABILITY_ILLUSION)
}

fn record_evidence(cx: &Ctx<'_>, dead: Dead, evidence: &mut Vec<u32>) {
    // The offending line often names only the viewer's Pokemon (damage a disguise dealt), so every
    // opponent entry on the field at that line is a suspect: at most one per position.
    let Some(line) = dead.line() else { return };
    let entries = cx.tracker.stints(cx.opp());
    for pos in 0..2u8 {
        if let Some(e) = entries.iter().rev().find(|e| e.position == pos && e.line <= line)
            && e.faint_line.is_none_or(|f| f >= line)
            && can_explain_entry(cx, e)
            && !evidence.contains(&e.id)
        {
            evidence.push(e.id);
        }
    }
}

fn attributable(cx: &Ctx<'_>, hypothesis: &IllusionHypothesis) -> bool {
    let cx = Ctx { hypothesis: Some(hypothesis), ..*cx };
    let mut active = [None; 2];
    let entry = |st| stint(&cx, st);
    for event in cx.tracker.party_events(cx.opp()) {
        match *event {
            PartyEvent::Enter(st) => {
                let Some(e) = entry(st) else {
                    return false;
                };
                let pos = e.position as usize;
                let owner = cx.owner(e);
                if pos >= 2 || active.contains(&Some(owner)) {
                    return false;
                }
                active[pos] = Some(owner);
            }
            PartyEvent::Faint(st) => {
                let Some(e) = entry(st) else {
                    return false;
                };
                for a in &mut active {
                    if *a == Some(cx.owner(e)) {
                        *a = None;
                    }
                }
            }
            PartyEvent::Swap(a, b) if a < 2 && b < 2 => active.swap(a as usize, b as usize),
            _ => {}
        }
    }
    true
}

fn sample_plain_team(cx: &Ctx<'_>, rng: &mut SplitMix64) -> Result<(TeamDef, [f64; 6]), WorldError> {
    let opp = cx.opp();
    let seen = cx.seen;
    let size = (cx.tracker.team_size(opp).unwrap_or(6) as usize).max(seen.len().max(1)).min(6);
    let mut slots: [Option<SetDef>; 6] = Default::default();
    let mut set_log_w = [0.0; 6];
    let leads = cx.tracker.leads(opp);
    for (pos, lead) in leads.iter().enumerate() {
        if let Some(i) = *lead {
            let (set, weight) = sample_revealed(cx, i, rng)?;
            slots[pos] = Some(set);
            set_log_w[pos] = weight;
        }
    }
    for i in 0..seen.len() {
        if leads.contains(&Some(i)) {
            continue;
        }
        let free: Vec<usize> = (0..size).filter(|&s| slots[s].is_none() && s >= 2.min(size)).collect();
        let free = if free.is_empty() {
            (0..size).filter(|&s| slots[s].is_none()).collect()
        } else {
            free
        };
        if free.is_empty() {
            return Err(WorldError::Desync("more opponent Pokemon revealed than its team size"));
        }
        let pos = free[rng.below(free.len())];
        let (set, weight) = sample_revealed(cx, i, rng)?;
        slots[pos] = Some(set);
        set_log_w[pos] = weight;
    }
    let mut team = TeamDef { len: size as u8, ..TeamDef::default() };
    let mut species: Vec<EffectId> = slots.iter().flatten().map(|s| s.species).collect();
    for (s, slot) in slots.into_iter().enumerate().take(size) {
        team.sets[s] = match slot {
            Some(set) => set,
            None => {
                let set = cx
                    .pool
                    .sample_unrevealed(&species, rng)
                    .ok_or(WorldError::Desync("the set pool is empty"))?;
                species.push(set.species);
                set
            }
        };
    }
    Ok((team, set_log_w))
}

/// The side must submit a choice now (`pyengine::mask::Req::of`).
fn needs<L: LogSink>(b: &Battle<L>, side: SideId) -> bool {
    !b.state.sides[side.0 as usize].choice.cant_undo
        && matches!(b.side_request_kind(side), RequestKind::Move | RequestKind::Switch)
}

/// The name the protocol shows for `mon` (an Illusion user shows its disguise).
fn shown_name<L: LogSink>(b: &Battle<L>, mon: MonId) -> &str {
    let disguise = b.state.pokemon[mon.0 as usize].illusion;
    let named = if disguise == MonId::NONE { mon } else { disguise };
    &b.teams().sides[named.side().0 as usize].sets[named.0 as usize % 6].name
}

fn set_name<L: LogSink>(b: &Battle<L>, mon: MonId) -> &str {
    &b.teams().sides[mon.side().0 as usize].sets[mon.0 as usize % 6].name
}

/// Replay `p` to the end of the observed stream.
fn advance(cx: &Ctx<'_>, p: &mut Particle, stats: &mut WorldStats) -> Result<(), Dead> {
    advance_capped(cx, p, u64::MAX, stats)
}

fn advance_capped(cx: &Ctx<'_>, p: &mut Particle, cap: u64, stats: &mut WorldStats) -> Result<(), Dead> {
    // Battle::clone gives copies a default empty sink; restore the capture flag for every replay.
    p.battle.log.track_entries = p.battle.teams().sides[cx.opp().0 as usize].sets.iter()
        .any(|s| s.ability == dex::ABILITY_ILLUSION);
    let hypothesis = p.hypothesis.clone();
    let cx = &Ctx { seen: hypothesis.as_ref().map_or(cx.seen, |h| &h.view), hypothesis: hypothesis.as_deref(), ..*cx };
    let mut idle = 0;
    loop {
        if stats.attempts >= cap && (p.cursor != cx.lines.len() || p.choice != cx.choices.len()) {
            p.why = format!("recovery attempt cap at line {}", p.cursor);
            return Err(Dead::Interrupted { line: p.cursor });
        }
        let before = p.cursor;
        // One snapshot per boundary (the first one before the start); copies share the one their
        // original took.
        let snap = p.history.last().is_none_or(|s| s.seg != p.seg).then(|| Arc::new(Snapshot::of(p)));
        let swaps = stats.swaps;
        let hidden = (0x3fu16 << (cx.opp().0 * 6)) & !p.battle.revealed_mons();
        let config = WorldConfig {
            segment_budget: cx.config.segment_budget.min(cap.saturating_sub(stats.attempts).min(u64::from(u32::MAX)) as u32),
            ..*cx.config
        };
        let result = step(&Ctx { config: &config, ..*cx }, p, stats);
        if (stats.swaps > swaps || hidden & p.battle.revealed_mons() != 0)
            && let Some(s) = snap.as_ref().or(p.history.last().filter(|s| s.seg == p.seg))
        {
            s.introduces.store(true, Ordering::Relaxed);
        }
        if let Some(s) = snap {
            remember(p, s);
        }
        match result {
            Ok(true) => p.seg += 1,
            Ok(false) => return Ok(()),
            Err(d) => {
                if p.why.is_empty() {
                    p.why = format!("at line {} (choice {})", p.cursor, p.choice);
                }
                return Err(if matches!(d, Dead::Budget { .. } | Dead::ExactHp { .. })
                    && config.segment_budget < cx.config.segment_budget && stats.attempts >= cap
                {
                    Dead::Interrupted { line: d.line().unwrap() }
                } else { d });
            }
        }
        // A step that reproduces no line twice in a row cannot make progress.
        idle = if p.cursor == before { idle + 1 } else { 0 };
        if idle > 2 {
            p.why = format!("no progress at line {}", p.cursor);
            return Err(Dead::Desync);
        }
    }
}

/// Add `snap` (the newest) to `p.history`, keeping about two snapshots per power-of-two age:
/// every one up to 3 segments back, every second up to 7, every fourth up to 15, and so on
/// (multiples of coarser powers of two, so a kept snapshot stays until it is too old for its
/// spacing), plus every snapshot taken before an introduction.
fn remember(p: &mut Particle, snap: Arc<Snapshot>) {
    let now = snap.seg;
    p.history.push(snap);
    p.history.retain(|s| {
        let age = now - s.seg;
        age < 4 || s.seg % (1 << (age.ilog2() - 1)) == 0 || s.introduces.load(Ordering::Relaxed)
    });
}

/// Replay one boundary. `Ok(false)`: `p` is at the end of the observed stream.
fn step(cx: &Ctx<'_>, p: &mut Particle, stats: &mut WorldStats) -> Result<bool, Dead> {
    let (viewer, opp) = (cx.viewer, cx.opp());
    let started = p.battle.state.started;
    let end = cx.lines.len();
    let mark = cx.choices.get(p.choice).map(|c| c.0);
    let (viewer_acts, opp_acts, limit);
    if !started {
        (viewer_acts, opp_acts) = (false, false);
        limit = mark.unwrap_or(end);
    } else if p.battle.state.ended {
        return if p.cursor == end && mark.is_none() { Ok(false) } else { Err(Dead::Desync) };
    } else {
        viewer_acts = needs(&p.battle, viewer);
        opp_acts = needs(&p.battle, opp);
        // The request pushed for this decision (also when the world got here before it was pushed,
        // and at the current boundary, which has no choice yet).
        if viewer_acts
            && let Some(Some(want)) = cx.requests.get(p.choice)
            && p.battle.request_json(viewer.0 as usize).as_deref() != Some(want.as_str())
        {
            p.why = format!("request differs at line {} (choice {})", p.cursor, p.choice);
            return Err(Dead::Desync);
        }
        if viewer_acts {
            match mark {
                None if p.cursor == end => return Ok(false),
                Some(m) if m == p.cursor => {}
                _ => return Err(Dead::Desync),
            }
            limit = cx.choices.get(p.choice + 1).map_or(end, |c| c.0);
        } else {
            if p.cursor == end && mark.is_none() {
                return Ok(false);
            }
            limit = mark.unwrap_or(end);
            if !opp_acts || limit == p.cursor {
                return Err(Dead::Desync);
            }
        }
    }
    // Lines after the boundary up to the end of the turn: the opponent's choices show there, even
    // past a mid-turn replacement of the viewer.
    let turn_end = cx.lines[p.cursor..]
        .iter()
        .position(|l| l.starts_with("|turn|"))
        .map_or(end, |i| p.cursor + i);
    let window = &cx.lines[p.cursor..turn_end.max(limit.min(end))];
    if started {
        introduce(cx, p, window, stats)?;
        p.battle.log.track_entries = p.battle.teams().sides[opp.0 as usize].sets.iter()
            .any(|s| s.ability == dex::ABILITY_ILLUSION);
        retera(cx, p, window)?;
    }
    let checkpoint = p.battle.clone();
    let specs = if opp_acts { Some(infer(cx, &checkpoint, window, p.cursor)) } else { None };
    let region = &cx.lines[p.cursor..limit];
    let viewer_text = viewer_acts.then(|| cx.choices[p.choice].1.as_str());
    let seg_draws = checkpoint.state.prng.draws();
    let key = p.rng.next_u64();
    // Resample points of the next replay: which stream each draw of the segment comes from.
    let mut points: Vec<(u32, u32)> = Vec::with_capacity(RESAMPLE_CAPACITY + 2);
    let mut opp_choice = match &specs {
        Some(s) => Some(sample_choice(cx, &checkpoint, s, false, &mut p.rng).ok_or_else(|| {
            p.why = format!("no legal opponent choice for {s:?} at line {}", p.cursor);
            Dead::Choice
        })?),
        None => None,
    };
    // A move the protocol shows but the world cannot choose (its set lacks it; a Choice lock, an
    // Assault Vest or PP the world got wrong) has no random outcome that reproduces it: only a
    // misread protocol could, which a few exploring choices cover.
    let unavailable = specs.as_ref().and_then(|s| unavailable_move(&checkpoint, opp, s));
    let budget =
        if unavailable.is_some() { cx.config.segment_budget.min(UNAVAILABLE_BUDGET) } else { cx.config.segment_budget };
    let subjects: Vec<Option<(SideId, &str)>> = region.iter().map(|l| hp_subject(l)).collect();
    let mut focus = Focus { l: usize::MAX, ..Focus::default() };
    // Consecutive replays that did not get past `focus.l`.
    let mut fails = 0u32;
    let mut settled = false;
    let mut produced: Vec<(usize, u32)> = Vec::new();
    for attempt in 0..budget {
        stats.attempts += 1;
        p.battle.restore_from(&checkpoint);
        p.battle.state.prng.set_resample(key, &points);
        if !started {
            if p.battle.start().is_err() {
                return Err(Dead::Desync);
            }
        } else {
            if let Some(text) = viewer_text
                && let Err(e) = p.battle.choose(viewer.0 as usize, text)
            {
                p.why = format!("viewer choice {text:?} rejected at line {}: {}", p.cursor, e.text);
                return Err(Dead::Choice);
            }
            if let Some(choice) = &opp_choice
                && p.battle.choose_typed(opp, choice).is_err()
            {
                let s = specs.as_ref().unwrap();
                opp_choice = Some(sample_choice(cx, &checkpoint, s, true, &mut p.rng).ok_or(Dead::Choice)?);
                continue;
            }
        }
        produced.clear();
        view(&p.battle.log, viewer, &mut produced);
        let at_viewer = p.battle.state.ended || needs(&p.battle, viewer);
        let entries = &p.battle.log.text.entries;
        let mismatch = produced
            .iter()
            .zip(region)
            .enumerate()
            .position(|(offset, (&(i, _), want))| {
                entries[i] != *want || !entry_matches(cx, &p.battle, i, p.cursor + offset)
            })
            .or({
                if produced.len() > region.len() {
                    Some(region.len())
                } else if produced.len() < region.len() && (at_viewer || p.battle.state.ended) {
                    Some(produced.len())
                } else if produced.len() == region.len()
                    && !at_viewer
                    && limit < end
                {
                    // The stream says the viewer acts next; the world disagrees.
                    Some(produced.len())
                } else {
                    None
                }
            });
        // At a viewer boundary with a pushed request, the request must match too.
        let next_choice = p.choice + usize::from(viewer_acts);
        let mismatch = mismatch.or_else(|| {
            let want = cx.requests.get(next_choice)?.as_deref()?;
            let reached = p.cursor + produced.len() == cx.choices.get(next_choice)?.0;
            (reached && at_viewer && p.battle.request_json(viewer.0 as usize).as_deref() != Some(want))
                .then_some(produced.len())
        });
        let Some(l) = mismatch else {
            p.log_w -= ((attempt + 1) as f64).ln();
            let cursor = p.cursor;
            p.cursor += produced.len();
            if viewer_acts {
                p.choice += 1;
            }
            stats.segments += 1;
            p.trail = Some(Arc::new(Replay {
                previous: p.trail.take(),
                seg: p.seg,
                cursor,
                end: p.cursor,
                choice: next_choice - usize::from(viewer_acts),
                viewer: viewer_acts,
                team: checkpoint.teams().sides[opp.0 as usize].clone(),
                prng: checkpoint.state.prng,
                key,
                points,
                opponent: opp_choice,
                end_draws: p.battle.state.prng.draws(),
                boundary: [needs(&p.battle, viewer), needs(&p.battle, opp), p.battle.state.ended],
            }));
            return Ok(true);
        };
        if focus.l == usize::MAX || l >= focus.l {
            // As far as the best replay or further: continue from this one.
            fails = if l == focus.l { fails + 1 } else { 1 };
            focus.points.clone_from(&points);
            focus.choice = opp_choice;
            focus.read(cx, &p.battle, region, &subjects, &produced, l, seg_draws, p.cursor);
        } else {
            // A line the best replay reproduced broke: resample from that replay again.
            fails += 1;
            points.clone_from(&focus.points);
            opp_choice = focus.choice;
        }
        // Give persistent mismatches the normal search allowance. Even the first HP change of
        // the segment can depend on a damage roll despite many identical replayed percentages.
        if fails > STUCK {
            break;
        }
        if focus.cands.is_empty() {
            // No draw precedes the line: only the opponent's choice can change it.
            if specs.is_none() || fails > SETTLED_STUCK {
                settled = true;
                break;
            }
            let s = specs.as_ref().unwrap();
            opp_choice = Some(sample_choice(cx, &checkpoint, s, true, &mut p.rng).ok_or(Dead::Choice)?);
            continue;
        }
        // The opponent's own move or target line: its sampled choice is a likely cause.
        if focus.opp_line && let Some(s) = &specs && p.rng.chance(0.5) {
            opp_choice = Some(sample_choice(cx, &checkpoint, s, true, &mut p.rng).ok_or(Dead::Choice)?);
            continue;
        }
        // One window of draws per attempt. The first right behind the line (`cands[0]`); then a mix
        // of: an earlier change of the same Pokemon's HP in this segment (its exact HP decides this
        // line), the last draws before the line alone (a damage roll, keeping the critical hit
        // and accuracy rolls), the whole current action (a multi-hit count), everything from the
        // segment start (speed ties and random targets are drawn when the turn starts), and backing
        // off over earlier lines.
        let cands = &focus.cands;
        let r = if fails <= 1 { 100 } else { p.rng.below(100) };
        let (start, stop) = if r < 20 && !focus.causes.is_empty() {
            let (start, end) = focus.causes[p.rng.below(focus.causes.len())];
            (if p.rng.chance(0.5) { end - 1 } else { start }, end)
        } else if r < 50 && !focus.open {
            let k = [1, 1, 1, 1, 1, 2, 2, 2, 3, 3][p.rng.below(10)];
            (focus.end.saturating_sub(k).max(seg_draws), focus.end)
        } else {
            let j = if fails <= 1 {
                0
            } else if r < 65 {
                focus.action
            } else if r < 75 {
                cands.len() - 1
            } else {
                let mut j = 0;
                while j + 1 < cands.len() && p.rng.chance(0.5) {
                    j += 1;
                }
                j
            };
            if cands[j] == seg_draws
                && let Some(s) = &specs
                && (j > 0 || focus.l == 0)
                && p.rng.chance(0.5)
            {
                opp_choice = Some(sample_choice(cx, &checkpoint, s, true, &mut p.rng).ok_or(Dead::Choice)?);
            }
            (cands[j], if focus.open { u32::MAX } else { focus.end })
        };
        set_window(&mut points, start, stop, attempt);
    }
    let line = p.cursor.saturating_add(focus.l);
    p.why = format!(
        "{} at line {line}: expected {:?}, replayed {:?}",
        if settled { "settled" } else if fails > STUCK { "stuck" } else { "budget" },
        region.get(focus.l),
        focus.replayed
    );
    if let Some((mon, name)) = unavailable {
        p.why.push_str(&format!("; {name} cannot choose {}", dex::move_data(mon).name));
    }
    if let Some(note) = &focus.owner {
        p.why.push_str(note);
    }
    // The line shows the HP of an opponent Pokemon whose exact HP this world fixed earlier, among
    // several the shown percentage allows: an earlier segment, not this one, may hold the cause.
    if let (Some(want), Some(replayed)) = (region.get(focus.l), &focus.replayed)
        && let Some((side, name)) = hp_subject(want).filter(|s| s.0 == opp && hp_subject(replayed) == Some(*s))
        && let Some(mon) = checkpoint.state.sides[side.0 as usize].active.iter().copied()
            .find(|&m| m != MonId::NONE && shown_name(&checkpoint, m) == name)
        && let hp = &checkpoint.state.pokemon[mon.0 as usize]
        && hp.hp > 0 && hp.hp < hp.max_hp && hp.max_hp > 100
    {
        return Err(Dead::ExactHp { line, since: hp_history(cx, side, name, p.cursor) });
    }
    Err(if settled { Dead::Settled { line } } else { Dead::Budget { line } })
}

/// The replay of a segment that reproduced the most observed lines so far, and where its first
/// mismatch says to resample.
#[derive(Default)]
struct Focus {
    /// The first observed line of the segment it did not reproduce (index into the region).
    l: usize,
    points: Vec<(u32, u32)>,
    choice: Option<[SlotChoice; 2]>,
    /// Draw count by which replayed line `l` got its text.
    end: u32,
    /// Line `l` is a move line that later draws may still edit (or missing): no `end`.
    open: bool,
    /// Draw indices where a window keeps the lines before `l` (`candidates`), latest first.
    cands: Vec<u32>,
    /// Index into `cands` of the start of the action line `l` belongs to (its user's move or
    /// `cant` line, or the line itself).
    action: usize,
    /// Windows behind the latest earlier changes, in this segment, of the HP line `l` shows.
    causes: Vec<(u32, u32)>,
    /// Line `l` is the opponent's move or switch, as observed or replayed.
    opp_line: bool,
    replayed: Option<String>,
    /// Illusion entry mismatch, for the failure message.
    owner: Option<String>,
}

impl Focus {
    #[allow(clippy::too_many_arguments)]
    fn read(&mut self, cx: &Ctx<'_>, b: &Battle<DrawLog>, region: &[String], subjects: &[Option<(SideId, &str)>],
        produced: &[(usize, u32)], l: usize, seg_draws: u32, cursor: usize) {
        let entries = &b.log.text.entries;
        let opp = cx.opp();
        self.l = l;
        // The draws that can change line `l` while keeping the lines before it: those after the
        // last earlier line got its final text and before line `l` got its own (a damage roll
        // precedes `-supereffective`). A move line is edited after its emission (`[miss]`,
        // `[still]`, the `[spread]` targets and shown target are decided by later draws).
        let total = b.state.prng.draws();
        (self.end, self.open) = match produced.get(l) {
            Some(&(i, d)) if !region.get(l).is_some_and(|want| edited(&entries[i], want)) => (d.min(total), false),
            _ => (total, true),
        };
        candidates(seg_draws, entries, produced, l, total, self.end, &mut self.cands);
        let acting = |k: usize| {
            let line = produced.get(k).map_or("", |&(i, _)| entries[i].as_str());
            ["|move|", "|-anim|", "|cant|"].iter().any(|c| line.starts_with(c))
        };
        let begins =
            (0..=l).rev().find(|&k| acting(k)).map_or(seg_draws, |k| produced.get(k).map_or(total, |&(_, d)| d));
        self.action = self.cands.iter().position(|&c| c <= begins).unwrap_or(self.cands.len().saturating_sub(1));
        let replayed = produced.get(l).map(|&(i, _)| entries[i].as_str());
        let shown = replayed.and_then(hp_subject);
        let subject = subjects.get(l).copied().flatten().or(shown);
        let before = &subjects[..l.min(region.len())];
        self.causes.clear();
        if subject.is_some() {
            for k in (0..before.len()).rev().filter(|&k| before[k] == subject).take(2) {
                let end = produced[k].1;
                let earlier = core::iter::once(seg_draws).chain(produced[..k].iter().map(|&(_, d)| d));
                if let Some(start) = earlier.filter(|&d| d < end).max() {
                    self.causes.push((start, end));
                }
            }
        }
        self.opp_line =
            region.get(l).is_some_and(|t| opp_action(t, opp)) || replayed.is_some_and(|t| opp_action(t, opp));
        self.replayed = replayed.map(str::to_owned);
        self.owner = produced.get(l).and_then(|&(raw, _)| {
            let observed = cursor + l;
            if entry_matches(cx, b, raw, observed) {
                return None;
            }
            let (e, mon) = (cx.tracker.entry(opp, observed)?, entry_actor(&b.log, raw)?);
            Some(format!("; entry owner expected {:?}, replayed {:?}", cx.owner(e), set_name(b, mon)))
        });
    }
}

/// `line` is a move line that differs from `want` only in what edits after its emission write.
fn edited(line: &str, want: &str) -> bool {
    // Command, user and move: what edits leave alone.
    let head = |l: &str| l.match_indices('|').nth(3).map_or(l.len(), |(i, _)| i);
    (line.starts_with("|move|") || line.starts_with("|-anim|")) && line[..head(line)] == want[..head(want)]
}

/// Distinct draw indices in `[seg_draws, end)` at which a replay may switch streams to change
/// replayed line `l` while keeping the lines before it: the segment start and when each of those
/// got its final text (a move line when the next one is emitted: until then `[miss]` and the like
/// may still be appended). Latest first.
fn candidates(
    seg_draws: u32,
    entries: &[String],
    produced: &[(usize, u32)],
    l: usize,
    total: u32,
    end: u32,
    out: &mut Vec<u32>,
) {
    out.clear();
    out.push(seg_draws);
    let mut next_move = total;
    for (k, &(i, d)) in produced.iter().enumerate().rev() {
        let mv = entries[i].starts_with("|move|") || entries[i].starts_with("|-anim|");
        if k < l {
            out.push(if mv { next_move } else { d });
        }
        if mv {
            next_move = d;
        }
    }
    out.retain(|&d| d < end);
    out.sort_unstable_by(|a, b| b.cmp(a));
    out.dedup();
}

/// Make draws `start..stop` (`stop` `u32::MAX`: all from `start` on) of the next replays come from
/// stream `salt` (`Prng::set_resample`), keeping where every other draw comes from. `points` are
/// increasing resample points.
fn set_window(points: &mut Vec<(u32, u32)>, start: u32, stop: u32, salt: u32) {
    let resume = points.iter().rev().find(|p| p.0 <= stop).map_or(RESUME, |p| p.1);
    points.retain(|p| p.0 < start || p.0 > stop);
    let i = points.partition_point(|p| p.0 < start);
    points.insert(i, (start, salt));
    if stop != u32::MAX {
        points.insert(i + 1, (stop, resume));
    }
    // Drop points that switch to the stream already in use.
    let mut current = RESUME;
    points.retain(|p| core::mem::replace(&mut current, p.1) != p.1);
    while points.len() > RESAMPLE_CAPACITY {
        if points.last().is_some_and(|p| p.0 > stop) { points.pop(); } else { points.remove(0); }
    }
}

/// The Pokemon whose HP a `-damage`, `-heal` or `-sethp` line shows.
fn hp_subject(line: &str) -> Option<(SideId, &str)> {
    let rest = ["|-damage|", "|-heal|", "|-sethp|"].iter().find_map(|c| line.strip_prefix(c))?;
    let id = parse_ident(rest.split('|').next()?)?;
    Some((id.side, id.name))
}

/// A line the opponent's choice at this boundary shapes: its move (target, Tera), switch or `cant`.
fn opp_action(line: &str, opp: SideId) -> bool {
    let Some(l) = Line::parse(line) else { return false };
    matches!(l.cmd(), "move" | "switch" | "cant" | "-terastallize")
        && parse_ident(l.arg(0)).is_some_and(|id| id.side == opp)
        && l.tag("from").is_none_or(|f| f == "lockedmove")
}

/// A slot read as a specific move the world's opponent cannot choose: the move and the Pokemon.
fn unavailable_move(b: &Battle<DrawLog>, opp: SideId, specs: &[SlotSpec; 2]) -> Option<(EffectId, String)> {
    let legal = b.legal_actions(opp);
    (0..legal.slot_count as usize).find_map(|k| {
        let SlotSpec::Move { id, .. } = specs[k] else { return None };
        let ls = &legal.slots[k];
        let moves = &ls.moves[..ls.move_count as usize];
        (!ls.automatic_pass && !moves.iter().any(|m| m.move_slot == 255 || m.move_id == id)).then(|| {
            let mon = b.state.sides[opp.0 as usize].active[k];
            (id, shown_name(b, mon).to_owned())
        })
    })
}

/// The first observed line, before `cursor`, of the stretch since `name`'s HP was last shown full:
/// where its exact HP became one of several.
fn hp_history(cx: &Ctx<'_>, side: SideId, name: &str, cursor: usize) -> usize {
    let mut since = cursor;
    for (i, line) in cx.lines[..cursor].iter().enumerate().rev() {
        let Some(l) = Line::parse(line) else { continue };
        let (ident, hp) = match l.cmd() {
            "-damage" | "-heal" | "-sethp" => (l.arg(0), l.arg(1)),
            "switch" | "drag" | "replace" => (l.arg(0), l.arg(2)),
            _ => continue,
        };
        if !parse_ident(ident).is_some_and(|id| id.side == side && id.name == name) {
            continue;
        }
        if protocol::parse_condition(hp).is_some_and(|c| c.max_hp == 0 || c.hp == c.max_hp) {
            break;
        }
        since = i;
    }
    since
}

fn entry_actor(log: &DrawLog, raw: usize) -> Option<MonId> {
    if !log.track_entries {
        return None;
    }
    log.entry_mons.binary_search_by_key(&raw, |(at, _)| *at).ok().map(|i| log.entry_mons[i].1)
}

fn entry_matches(cx: &Ctx<'_>, b: &Battle<DrawLog>, raw: usize, observed: usize) -> bool {
    entry_actor(&b.log, raw).is_none_or(|mon| {
        mon.side() != cx.opp() || cx.tracker.entry(cx.opp(), observed).is_none_or(|e| {
            !e.certain && cx.hypothesis.is_none() || set_name(b, mon) == cx.owner(e)
        })
    })
}

/// The viewer's player view of the sink's entries (`|split|` resolved like `extractChannelMessages`),
/// compared lines only, with the draw count at emission.
fn view(log: &DrawLog, viewer: SideId, out: &mut Vec<(usize, u32)>) {
    let entries = &log.text.entries;
    let mut i = 0;
    while i < entries.len() {
        if let Some(owner) = entries[i].strip_prefix("|split|p").and_then(|n| n.parse::<u8>().ok())
            && i + 2 < entries.len()
        {
            let pick = if owner == viewer.0 + 1 { i + 1 } else { i + 2 };
            if protocol::is_compared(&entries[pick]) {
                out.push((pick, log.draws[pick]));
            }
            i += 3;
            continue;
        }
        if protocol::is_compared(&entries[i]) {
            out.push((i, log.draws[i]));
        }
        i += 1;
    }
}

/// Give every opponent Pokemon named in `window` a place in the world: one the world does not have
/// takes over a never-revealed bench slot (`replace_hidden_set`, exact for such slots).
fn introduce(cx: &Ctx<'_>, p: &mut Particle, window: &[String], stats: &mut WorldStats) -> Result<(), Dead> {
    let opp = cx.opp();
    for (offset, line) in window.iter().enumerate() {
        let Some(l) = Line::parse(line) else { continue };
        if !matches!(l.cmd(), "switch" | "drag" | "replace") {
            continue;
        }
        let Some(id) = parse_ident(l.arg(0)).filter(|id| id.side == opp) else { continue };
        let side = &p.battle.state.sides[opp.0 as usize];
        let party = side.party[..side.pokemon_count as usize].to_vec();
        let hidden = p.battle.hidden_mons(cx.viewer);
        if let Some(&mon) = party.iter().find(|&&m| set_name(&p.battle, m) == id.name) {
            // A never-revealed set of a random-gender species got its gender by a coin, independent
            // of everything else: give it the gender now shown (exact).
            let shown = protocol::parse_details(l.arg(1)).gender;
            let set = &p.battle.teams().sides[opp.0 as usize].sets[mon.0 as usize % 6];
            if hidden & (1 << mon.0) != 0
                && matches!(shown, Gender::Male | Gender::Female)
                && set.gender != shown
                && !matches!(dex::species(set.species).gender, "M" | "F" | "N")
            {
                let set = SetDef { gender: shown, ..set.clone() };
                p.battle.replace_hidden_set(cx.viewer, mon, &set).map_err(|_| Dead::Swap)?;
            }
            continue;
        }
        let Some(index) = cx.seen.iter().position(|m| m.name == id.name) else {
            p.why = format!("new entry {} was never seen", id.name);
            return Err(Dead::Swap);
        };
        // Prefer slots holding a set nobody has been seen as.
        let mut slots: Vec<MonId> = party.iter().copied().filter(|m| hidden & (1 << m.0) != 0).collect();
        slots.sort_by_key(|&m| cx.tracker.find(opp, set_name(&p.battle, m)).is_some());
        let Some(&first) = slots.first() else {
            p.why = format!("no hidden slot for new entry {}", id.name);
            return Err(Dead::Swap);
        };
        let preferred = cx.tracker.find(opp, set_name(&p.battle, first)).is_some();
        let pool: Vec<MonId> = slots
            .iter()
            .copied()
            .filter(|&m| cx.tracker.find(opp, set_name(&p.battle, m)).is_some() == preferred)
            .collect();
        let mon = pool[p.rng.below(pool.len())];
        let exact_first = cx.tracker.entry(opp, p.cursor + offset).is_some_and(|e| can_explain_entry(cx, e));
        let sample_cx = Ctx { repair: cx.repair && !exact_first, ..*cx };
        let before_patches = cx.patches.get();
        let (set, log_w) = sample_revealed(&sample_cx, index, &mut p.rng).map_err(|e| {
            p.why = format!("new entry {} needs rebuilding before pool repair: {e}", id.name);
            Dead::Swap
        })?;
        p.battle.replace_hidden_set(cx.viewer, mon, &set).map_err(|e| {
            p.why = format!("new entry {}: {e:?}", id.name);
            Dead::Swap
        })?;
        let slot = mon.0 as usize % 6;
        p.log_w += log_w - p.set_log_w[slot];
        p.set_log_w[slot] = log_w;
        p.repaired |= cx.patches.get() > before_patches;
        p.tera_known &= !(1 << mon.0);
        if cx.seen[index].tera_type.is_some() { p.tera_known |= 1 << mon.0; }
        stats.swaps += 1;
    }
    Ok(())
}

/// Give the opponent Pokemon that terastallize in `window` the shown Tera type. Until a Pokemon
/// terastallizes its Tera type affects nothing (`set_hidden_tera_type`), so a world that drew
/// another one is not wrong yet; without this, worlds whose copies all drew the same wrong type
/// would all die at the reveal. The Tera type is integrated out instead: every world pays the
/// pool's probability of the shown type given the rest of its set, `REPAIR_LOG_PENALTY` when the
/// pool never pairs that set with it. A world built after the reveal drew its set with the Tera
/// type as a constraint and pays nothing, as does a later redraw that knew the type. An off-pool
/// set already patched by `repair` paid its penalty when it was drawn.
fn retera(cx: &Ctx<'_>, p: &mut Particle, window: &[String]) -> Result<(), Dead> {
    let opp = cx.opp();
    for (offset, line) in window.iter().enumerate() {
        let Some(l) = Line::parse(line) else { continue };
        if l.cmd() != "-terastallize" {
            continue;
        }
        let (Some(id), Some(tera)) = (parse_ident(l.arg(0)).filter(|id| id.side == opp), dex::type_id(l.arg(1)))
        else {
            continue;
        };
        // Tera is chosen with the move: the Pokemon active at this boundary terastallizes.
        let Some(pos) = id.position.filter(|&pos| pos < 2) else { continue };
        let mon = p.battle.state.sides[opp.0 as usize].active[pos as usize];
        if mon == MonId::NONE || p.battle.state.pokemon[mon.0 as usize].terastallized != TypeId::NONE {
            continue;
        }
        let set = &p.battle.teams().sides[opp.0 as usize].sets[mon.0 as usize % 6];
        let shared = set.tera_type == tera;
        let log_w = match cx.pool.tera_share(set, tera) {
            _ if p.cursor + offset < p.built_at || p.tera_known & (1 << mon.0) != 0 => 0.0,
            Some(share) if share > 0.0 => share.ln(),
            None => 0.0,
            _ if shared => 0.0,
            _ => -REPAIR_LOG_PENALTY,
        };
        p.log_w += log_w;
        p.set_log_w[mon.0 as usize % 6] += log_w;
        p.tera_known |= 1 << mon.0;
        if !shared && p.battle.set_hidden_tera_type(cx.viewer, mon, tera).is_err() {
            p.why = format!("cannot give {} Tera {}", id.name, l.arg(1));
            return Err(Dead::Swap);
        }
    }
    Ok(())
}

fn can_explain_entry(cx: &Ctx<'_>, entry: &Stint) -> bool {
    if entry.certain || cx.pool.illusion_species().is_empty() {
        return false;
    }
    if let Some(m) = cx.seen.iter().find(|m| m.starting_ability == Some(dex::ABILITY_ILLUSION)) {
        return !m.fainted
            || cx
                .tracker
                .stints(cx.opp())
                .iter()
                .any(|e| e.owner == m.name && e.faint_line.is_some_and(|line| entry.line < line));
    }
    cx.seen.len() < cx.tracker.team_size(cx.opp()).unwrap_or(6) as usize
        && cx.pool.illusion_species().iter().any(|(s, _)| {
            !cx.seen.iter().any(|m| {
                m.species != EffectId::NONE
                    && dex::species(m.species).base_species_name == dex::species(*s).base_species_name
            })
        })
}

/// Read the opponent's choice at this boundary off the protocol.
fn infer(cx: &Ctx<'_>, b: &Battle<DrawLog>, window: &[String], at: usize) -> [SlotSpec; 2] {
    let opp = cx.opp();
    let legal = b.legal_actions(opp);
    let switch_request = b.side_request_kind(opp) == RequestKind::Switch;
    core::array::from_fn(|k| {
        if k >= legal.slot_count as usize || legal.slots[k].automatic_pass {
            return SlotSpec::Pass;
        }
        if switch_request {
            return infer_replacement(opp, k as u8, legal.slots[k].revival_mask != 0, window, at);
        }
        let mon = b.state.sides[opp.0 as usize].active[k];
        infer_move(opp, k as u8, shown_name(b, mon), window, at)
    })
}

fn infer_replacement(opp: SideId, k: u8, reviving: bool, window: &[String], at: usize) -> SlotSpec {
    for (i, line) in window.iter().enumerate() {
        let Some(l) = Line::parse(line) else { continue };
        match l.cmd() {
            "switch" if !reviving => {
                if let Some(id) = parse_ident(l.arg(0))
                    && id.side == opp
                    && id.position == Some(k)
                {
                    return SlotSpec::Switch(id.name.to_owned(), at + i);
                }
            }
            "-heal" if reviving && l.tag("from") == Some("move: Revival Blessing") => {
                if let Some(id) = parse_ident(l.arg(0))
                    && id.side == opp
                {
                    return SlotSpec::Revive(id.name.to_owned());
                }
            }
            _ => {}
        }
    }
    SlotSpec::Pass
}

fn infer_move(opp: SideId, k: u8, name: &str, window: &[String], at: usize) -> SlotSpec {
    let mut acted = false;
    let mut tera = false;
    // Uses forced by Instruct (announced by `-singleturn|X|move: Instruct`) carry no `[from]`.
    let mut instructed = 0;
    let mut dancing = 0;
    let (mut actor_name, mut position) = (name, k);
    for (i, line) in window.iter().enumerate() {
        let Some(l) = Line::parse(line) else { continue };
        let ident = parse_ident(l.arg(0));
        let mine = ident.is_some_and(|id| id.side == opp && id.name == actor_name && id.position == Some(position));
        match l.cmd() {
            "replace" if ident.is_some_and(|id| id.side == opp && id.position == Some(position)) => {
                actor_name = ident.unwrap().name;
            }
            "swap" => {
                if let Some(id) = ident.filter(|id| id.side == opp)
                    && let (Some(from), Ok(to)) = (id.position, l.arg(1).parse::<u8>())
                {
                    if position == from {
                        position = to;
                    } else if position == to {
                        position = from;
                    }
                }
            }
            // Switches resolve before Tera and moves.
            "switch" if !acted => {
                if let Some(id) = ident
                    && id.side == opp
                    && id.position == Some(k)
                {
                    return SlotSpec::Switch(id.name.to_owned(), at + i);
                }
            }
            "-terastallize" if mine => tera = true,
            "-singleturn" if mine && l.arg(1) == "move: Instruct" => instructed += 1,
            "move" => {
                acted = true;
                if !mine {
                    continue;
                }
                if let Some(from) = l.tag("from")
                    && from != "lockedmove"
                {
                    continue;
                }
                if instructed > 0 {
                    instructed -= 1;
                    continue;
                }
                if dancing > 0 {
                    dancing -= 1;
                    continue;
                }
                let Some(id) = dex::lookup(EffectKind::Move, l.arg(1)) else { return SlotSpec::AnyMove { tera } };
                let loc = if l.has_tag("spread") || l.has_tag("notarget") || l.has_tag("still") {
                    None
                } else {
                    parse_ident(l.arg(2)).and_then(|t| {
                        let pos = t.position? as i8 + 1;
                        Some(if t.side == opp { -pos } else { pos })
                    })
                };
                return SlotSpec::Move { id, loc, tera };
            }
            "cant" => {
                acted = true;
                // `|cant|HOLDER|ability: Armor Tail|MOVE|[of] USER`: the holder blocked USER's move.
                let blocked = protocol::effect_kind(l.arg(1)).0 == "ability";
                let actor = if blocked { l.tag("of").and_then(parse_ident) } else { ident };
                let actor_mine =
                    actor.is_some_and(|id| id.side == opp && id.name == actor_name && id.position == Some(position));
                if actor_mine && dancing == 0 {
                    return match dex::lookup(EffectKind::Move, l.arg(2)) {
                        Some(id) if !l.arg(2).is_empty() => SlotSpec::Move { id, loc: None, tera },
                        _ => SlotSpec::AnyMove { tera },
                    };
                }
                if actor_mine {
                    dancing -= 1;
                }
            }
            // A Dancer copy that fails (choice lock) prints `move|X|MOVE||[still]` without `[from]`.
            "-activate" if mine && l.arg(1) == "ability: Dancer" => dancing += 1,
            "faint" if mine => return SlotSpec::AnyMove { tera },
            _ => {}
        }
    }
    SlotSpec::AnyMove { tera }
}

/// Consecutive replays that get no further than the best one after which a world is dropped (the
/// windows have resampled the whole segment, and the opponent's open choices, many times by then).
const STUCK: u32 = 400;

/// Consecutive failures at a line no draw precedes (only the opponent's choice can change it).
const SETTLED_STUCK: u32 = 200;

/// Segment budget when the opponent's shown move is not among the world's legal choices.
const UNAVAILABLE_BUDGET: u32 = 48;

/// One concrete legal choice consistent with `specs`, sampling what the protocol left open. With
/// `explore`, a slot read as a specific move is instead any legal move with probability
/// `EXPLORE`: the move shown can differ from the move chosen (Encore replaces it, a failed
/// Choice-locked selection, an unrecognized call), so every legal move keeps some probability.
fn sample_choice(
    cx: &Ctx<'_>,
    b: &Battle<DrawLog>,
    specs: &[SlotSpec; 2],
    explore: bool,
    rng: &mut SplitMix64,
) -> Option<[SlotChoice; 2]> {
    const EXPLORE: f64 = 0.15;
    let opp = cx.opp();
    let legal = b.legal_actions(opp);
    for _ in 0..32 {
        let pick = |k: usize, rng: &mut SplitMix64| {
            let spec = match &specs[k] {
                SlotSpec::Move { tera, .. } if explore && rng.chance(EXPLORE) => {
                    if *tera { &SlotSpec::AnyMove { tera: true } } else { &SlotSpec::AnyMove { tera: false } }
                }
                s => s,
            };
            slot_choice(cx, b, &legal, k, spec, rng)
        };
        let c0 = pick(0, rng)?;
        let c1 = pick(1, rng)?;
        let pair = [c0, c1];
        if b.is_legal_joint_choice(opp, &pair[..legal.slot_count as usize]) {
            return Some(pair);
        }
    }
    None
}

fn locs(mask: crate::sim::choices::legal::TargetMask) -> Vec<i8> {
    (-2..=2).filter(|&l| mask.contains(l)).collect()
}

fn slot_choice(
    cx: &Ctx<'_>,
    b: &Battle<DrawLog>,
    legal: &LegalActions,
    k: usize,
    spec: &SlotSpec,
    rng: &mut SplitMix64,
) -> Option<SlotChoice> {
    if k >= legal.slot_count as usize {
        return Some(SlotChoice::default());
    }
    let ls = &legal.slots[k];
    let moves = &ls.moves[..ls.move_count as usize];
    let party = &b.state.sides[cx.opp().0 as usize].party;
    let mv = |m: &crate::sim::choices::LegalMove, loc: i8, tera: bool| SlotChoice {
        kind: ChoiceKind::Move,
        move_slot: m.move_slot,
        switch_to: MonId::NONE,
        target_loc: if m.move_slot == 255 { 0 } else { loc },
        tera: tera && m.move_slot != 255,
        move_id: m.move_id,
        move_kind: m.move_kind,
    };
    let pick_loc = |m: &crate::sim::choices::LegalMove, shown: Option<i8>, rng: &mut SplitMix64| {
        let options = locs(m.targets);
        match shown {
            Some(l) if m.targets.contains(l) && rng.chance(cx.config.shown_target) => l,
            _ if options.is_empty() => 0,
            _ => options[rng.below(options.len())],
        }
    };
    Some(match spec {
        SlotSpec::Pass => {
            if ls.automatic_pass || ls.can_pass {
                SlotChoice::default()
            } else {
                return None;
            }
        }
        SlotSpec::Move { id, loc, tera } => {
            // A forced move (lock, charge, Recharge, Struggle) is the only option anyway.
            let Some(m) = moves
                .iter()
                .find(|m| m.move_slot == 255)
                .or_else(|| moves.iter().find(|m| m.move_id == *id))
            else {
                // Not in this world's moveset (or misread): any legal move keeps the world alive;
                // the replay decides.
                return slot_choice(cx, b, legal, k, &SlotSpec::AnyMove { tera: *tera }, rng);
            };
            if *tera && m.move_slot != 255 && !m.can_terastallize {
                return None;
            }
            let loc = pick_loc(m, *loc, rng);
            mv(m, loc, *tera)
        }
        SlotSpec::AnyMove { tera } => {
            let options: Vec<_> = moves.iter().filter(|m| !*tera || m.can_terastallize).collect();
            if options.is_empty() {
                return if !*tera && (ls.can_pass || ls.automatic_pass) { Some(SlotChoice::default()) } else { None };
            }
            let m = options[rng.below(options.len())];
            let loc = pick_loc(m, None, rng);
            mv(m, loc, *tera)
        }
        SlotSpec::Switch(name, _) | SlotSpec::Revive(name) => {
            let mask = if matches!(spec, SlotSpec::Revive(_)) { ls.revival_mask } else { ls.switch_mask };
            let name = match spec {
                SlotSpec::Switch(_, at) => cx.tracker.entry(cx.opp(), *at).map_or(name.as_str(), |e| cx.owner(e)),
                _ => name.as_str(),
            };
            let options: Vec<usize> = (0..6).filter(|&p| mask & (1 << p) != 0).collect();
            let exact: Vec<usize> = options.iter().copied().filter(|&p| set_name(b, party[p]) == name).collect();
            let pinned = match spec {
                SlotSpec::Switch(_, at) => cx.tracker.entry(cx.opp(), *at).is_some_and(|e| {
                    e.certain || cx.hypothesis.is_some_and(|h| h.stints.contains(&e.id))
                }),
                _ => true,
            };
            let p = if pinned {
                if exact.is_empty() { return None; }
                exact[rng.below(exact.len())]
            } else {
                // An ordinary particle keeps main's switch choices and draw stream until an
                // entry is publicly attributed or selected by a guided hypothesis.
                let disguised: Vec<_> = options.iter().copied()
                    .filter(|&p| b.state.pokemon[party[p].0 as usize].ability == dex::ABILITY_ILLUSION).collect();
                match (exact.is_empty(), disguised.is_empty()) {
                    (true, true) => return None,
                    (false, true) => exact[rng.below(exact.len())],
                    (true, false) => disguised[rng.below(disguised.len())],
                    (false, false) => {
                        if rng.chance(0.9) { exact[rng.below(exact.len())] }
                        else { disguised[rng.below(disguised.len())] }
                    }
                }
            };
            SlotChoice { kind: ChoiceKind::Switch, switch_to: party[p], ..SlotChoice::default() }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    mod recovery {
        use super::*;
        include!("world_recovery_tests.rs");
    }

    const Z: &str =
        "Zoroark||LifeOrb|Illusion|darkpulse,protect,nastyplot,sludgebomb||85,,85,85,85,85|F|,0,,,,||84|,,,,,Poison";
    const B: &str = "Bastiodon||Leftovers|Sturdy|foulplay,wideguard,irondefense,bodypress||85,,85,85,85,85|M|,0,,,,||90|,,,,,Fighting";
    const U: &str =
        "Umbreon||Leftovers|InnerFocus|snarl,protect,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark";
    const L: &str =
        "Latias||SoulDew|Levitate|mistball,recover,dracometeor,tailwind||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel";
    const M: &str =
        "Mesprit||LifeOrb|Levitate|protect,psychic,nastyplot,dazzlinggleam||85,,85,85,85,85|N|,0,,,,||86|,,,,,Psychic";
    const BL: &str = "Blastoise||WhiteHerb|Torrent|shellsmash,protect,muddywater,dragonpulse||85,,85,85,85,85|F|,0,,,,||83|,,,,,Dragon";

    fn fixture_pool() -> SetPool {
        let mut pool = SetPool::new();
        pool.add_team(&[L, U, Z, M, BL, B].join("]")).unwrap();
        pool
    }

    fn context<'a>(tracker: &'a RevealTracker, pool: &'a SetPool, config: &'a WorldConfig) -> Ctx<'a> {
        Ctx {
            viewer: SideId(0),
            lines: &[],
            choices: &[],
            requests: &[],
            move_orders: &[],
            tracker,
            pool,
            config,
            seen: tracker.mons(SideId(1)),
            hypothesis: None,
            repair: false,
            repairs: Box::leak(Box::new(Cell::new(0))),
            patches: Box::leak(Box::new(Cell::new(0))),
            rebuild_samples: None,
        }
    }

    #[test]
    fn no_evidence_keeps_the_plain_budget_and_rng() {
        let own = Arc::new(TeamDef::unpack(&[L, U].join("]")).unwrap());
        let pool = Arc::new(fixture_pool());
        let config = WorldConfig { particles: 1, segment_budget: 0, rebuild_budget: 4,
            ..WorldConfig::default() };
        let mut builder = WorldBuilder::new(SideId(0), own.clone(), ["A".into(), "B".into()],
            pool, config, 7).unwrap();
        builder.push_lines(["|teamsize|p2|6", "|switch|p2a: Latias|Latias, L80, F|100/100",
            "|switch|p2b: Umbreon|Umbreon, L85, M|100/100", "|turn|1"]);
        // Reject all four priors, so this update must be exactly the ordinary rebuild schedule.
        let cx = context(&builder.tracker, &builder.pool, &config);
        while {
            let mut prior = builder.proposal_rng;
            (0..4).any(|_| prior.chance(illusion_prior(&cx)))
        } { builder.proposal_rng.next_u64(); }
        let mut expected = SplitMix64(7);
        let mut stats = WorldStats::default();
        for _ in 0..4 {
            let p = build(&Ctx { repair: true, ..cx }, None, own.clone(), &builder.names,
                &mut expected, &mut stats).unwrap();
            assert!(p.hypothesis.is_none());
        }
        assert_eq!(builder.update(), Err(WorldError::NoConsistentWorld));
        assert_eq!(builder.stats.builds, 4);
        assert_eq!(builder.rng, expected);
    }

    #[test]
    fn older_nonlead_entries_rotate_even_when_evidence_is_crowded() {
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let mut t = RevealTracker::new();
        t.feed_all(["|teamsize|p2|6", "|turn|1"]);
        for i in 0..12 {
            t.feed(if i % 2 == 0 { "|switch|p2a: Umbreon|Umbreon, L85, M|100/100" }
                else { "|switch|p2a: Blastoise|Blastoise, L83, F|100/100" });
        }
        let cx = context(&t, &pool, &config);
        let source = t.illusion_source(cx.opp());
        let evidence: Vec<_> = t.stints(cx.opp()).iter().map(|e| e.id).collect();
        let mut reached = Vec::new();
        for rotation in 0..12 {
            for h in prepare_hypotheses(&cx, &source, &[], &evidence, rotation, 32) {
                reached.extend(h.stints.iter().copied());
            }
        }
        assert!(evidence.iter().all(|st| reached.contains(st)));
    }

    #[test]
    fn known_entries_pin_both_the_user_and_the_real_disguise() {
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|switch|p2a: Latias|Latias, L80, F|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100
            |turn|1
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |replace|p2a: Zoroark|Zoroark, L84, F
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |-ability|p2a: Bastiodon|Sturdy",
            )
            .iter()
            .map(String::as_str),
        );
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let cx = context(&t, &pool, &config);
        let team = Arc::new(TeamDef::unpack(&[L, U, Z, M, BL, B].join("]")).unwrap());
        let mut b =
            Battle::from_team_defs([1, 2, 3, 4], team.clone(), team, ["A".into(), "B".into()], DrawLog::default())
                .unwrap();
        b.start().unwrap();
        let legal = b.legal_actions(SideId(1));
        let mut rng = SplitMix64(9);
        for e in &t.stints(SideId(1))[2..] {
            let spec = SlotSpec::Switch(e.shown.name.clone(), e.line);
            for _ in 0..100 {
                let c = slot_choice(&cx, &b, &legal, 0, &spec, &mut rng).unwrap();
                assert_eq!(set_name(&b, c.switch_to), e.owner);
            }
        }
    }

    #[test]
    fn party_order_solves_one_and_two_different_disguises() {
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|6
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100
            |turn|1
            |move|p2a: Bastiodon|Nasty Plot|p2a: Bastiodon
            |replace|p2a: Zoroark|Zoroark, L84, F",
            )
            .iter()
            .map(String::as_str),
        );
        let pool = fixture_pool();
        let config = WorldConfig::default();
        for two in [false, true] {
            if two {
                t.feed_all(
                    lines(
                        "|switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
                    |switch|p2a: Latias|Latias, L80, F|100/100
                    |switch|p2a: Latias|Latias, L80, F|100/100
                    |move|p2a: Latias|Sludge Bomb|p1a: X
                    |replace|p2a: Zoroark|Zoroark, L84, F",
                    )
                    .iter()
                    .map(String::as_str),
                );
            }
            let cx = context(&t, &pool, &config);
            for seed in 0..10 {
                let mut rng = SplitMix64(seed);
                let team = sample_team(&cx, &mut rng).unwrap().0;
                assert_eq!(team.sets[0].name, "Zoroark");
                assert_eq!(team.sets[5].name, "Bastiodon");
                let z = &team.sets[0];
                assert!(z.moves.contains(&move_id("nastyplot")));
                let b = team.sets.iter().find(|s| s.name == "Bastiodon").unwrap();
                assert!(!b.moves.contains(&move_id("nastyplot")));
            }
        }
    }

    #[test]
    fn hypotheses_require_a_slot_and_respect_species_clause_and_fainting() {
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|6
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100
            |turn|1
            |move|p2a: Bastiodon|Nasty Plot|p2a: Bastiodon",
            )
            .iter()
            .map(String::as_str),
        );
        let hs = hypotheses(&context(&t, &pool, &config));
        assert!(!hs.is_empty());
        assert_eq!(hs[0].stints, [t.stints(SideId(1))[0].id]);
        t.feed("|teamsize|p2|2");
        assert!(hypotheses(&context(&t, &pool, &config)).is_empty());
        t.feed("|teamsize|p2|6");
        t.feed("|switch|p2a: Zoroark|Zoroark-Hisui, L80, M|100/100");
        let hs = hypotheses(&context(&t, &pool, &config));
        assert!(hs.iter().all(|h| h.species == "Zoroark-Hisui"));
        t.feed("|faint|p2a: Zoroark");
        assert!(hypotheses(&context(&t, &pool, &config)).is_empty());
        let empty = SetPool::new();
        assert!(hypotheses(&context(&t, &empty, &config)).is_empty());
    }

    #[test]
    fn party_order_explains_a_fainted_disguise_appearing_alive() {
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|6
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100
            |turn|1
            |move|p2a: Bastiodon|Sludge Bomb|p1a: X
            |faint|p2a: Bastiodon
            |switch|p2a: Latias|Latias, L80, F|100/100
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |move|p2a: Bastiodon|Body Press|p1a: X",
            )
            .iter()
            .map(String::as_str),
        );
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let cx = context(&t, &pool, &config);
        let h = hypotheses(&cx).into_iter().find(|h| h.stints == [t.stints(SideId(1))[0].id]).unwrap();
        let view = t.illusion_view(cx.opp(), &h.stints, &h.user, &h.species);
        let cx = Ctx { seen: &view, hypothesis: Some(&h), ..cx };
        let team = sample_team(&cx, &mut SplitMix64(10)).unwrap().0;
        assert_eq!(team.sets[0].name, "Zoroark");
        assert_eq!(team.sets[5].name, "Bastiodon");
    }

    #[test]
    fn non_adjacent_unbroken_stints_can_use_different_disguises() {
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|6
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100
            |turn|1
            |move|p2a: Bastiodon|Nasty Plot|p2a: Bastiodon
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |switch|p2a: Latias|Latias, L80, F|100/100
            |move|p2a: Latias|Draco Meteor|p1a: X
            |switch|p2b: Mesprit|Mesprit, L86|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100
            |switch|p2a: Latias|Latias, L80, F|100/100
            |move|p2a: Latias|Sludge Bomb|p1a: X",
            )
            .iter()
            .map(String::as_str),
        );
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let cx = context(&t, &pool, &config);
        let ids = [t.stints(cx.opp())[0].id, t.stints(cx.opp()).last().unwrap().id];
        let h = hypotheses(&cx)
            .into_iter()
            .find(|h| ids.iter().all(|s| h.stints.contains(s)) && h.stints.len() == 2)
            .unwrap();
        let view = t.illusion_view(cx.opp(), &h.stints, &h.user, &h.species);
        let cx = Ctx { seen: &view, hypothesis: Some(&h), ..cx };
        let team = sample_team(&cx, &mut SplitMix64(10)).unwrap().0;
        assert_eq!(team.sets[0].name, "Zoroark");
        assert_eq!(team.sets[5].name, "Bastiodon");
        assert_eq!(view.iter().find(|m| m.name == "Latias").unwrap().moves, [move_id("dracometeor")]);
    }

    #[test]
    fn copied_move_order_follows_the_attributed_stint() {
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |turn|1
            |replace|p2a: Zoroark|Zoroark, L84, F",
            )
            .iter()
            .map(String::as_str),
        );
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let orders =
            [("Bastiodon".to_owned(), t.stints(SideId(1))[0].id, vec![move_id("nastyplot"), move_id("protect")])];
        let cx = Ctx { move_orders: &orders, ..context(&t, &pool, &config) };
        let z = sample_revealed(&cx, t.find(SideId(1), "Zoroark").unwrap(), &mut SplitMix64(7)).unwrap().0;
        assert_eq!(&z.moves[..2], orders[0].2);
    }

    #[test]
    fn a_pool_without_illusion_uses_the_ordinary_sampling_path() {
        let mut pool = SetPool::new();
        pool.add_team(&[L, U, M, BL, B].join("]")).unwrap();
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|5
            |switch|p2a: Latias|Latias, L80, F|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100",
            )
            .iter()
            .map(String::as_str),
        );
        let config = WorldConfig::default();
        let cx = context(&t, &pool, &config);
        assert!(hypotheses(&cx).is_empty());
        let (mut a, mut b) = (SplitMix64(7), SplitMix64(7));
        assert_eq!(sample_team(&cx, &mut a).unwrap(), sample_plain_team(&cx, &mut b).unwrap());
        assert_eq!(a, b);
    }

    #[test]
    fn a_revealed_fainted_user_only_explains_entries_before_its_faint() {
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|6
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100
            |turn|1
            |move|p2a: Bastiodon|Nasty Plot|p2a: Bastiodon
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |replace|p2a: Zoroark|Zoroark, L84, F
            |faint|p2a: Zoroark
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |move|p2a: Bastiodon|Body Press|p1a: X",
            )
            .iter()
            .map(String::as_str),
        );
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let cx = context(&t, &pool, &config);
        let old = t.stints(cx.opp())[0].id;
        let new = t.stints(cx.opp()).last().unwrap().id;
        let hs = hypotheses(&cx);
        assert!(hs.iter().all(|h| !h.stints.contains(&new)));
        let h = hs.into_iter().find(|h| h.stints == [old]).unwrap();
        let view = t.illusion_view(cx.opp(), &h.stints, &h.user, &h.species);
        assert!(sample_team(&Ctx { seen: &view, hypothesis: Some(&h), ..cx }, &mut SplitMix64(8)).is_ok());
    }

    #[test]
    fn forced_drag_checks_the_real_entry_owner_even_when_text_matches() {
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|drag|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |replace|p2a: Zoroark|Zoroark, L84, F",
            )
            .iter()
            .map(String::as_str),
        );
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let cx = context(&t, &pool, &config);
        let team = Arc::new(TeamDef::unpack(&[L, U, Z, M, BL, B].join("]")).unwrap());
        let log = DrawLog { track_entries: true, ..DrawLog::default() };
        let mut wrong =
            Battle::from_team_defs([1, 2, 3, 4], team.clone(), team, ["A".into(), "B".into()], log).unwrap();
        wrong.start().unwrap();
        let mut right = wrong.clone_with_log(DrawLog { track_entries: true, ..DrawLog::default() });
        wrong.log.reset();
        wrong.switch_in(MonId(11), 0, crate::event::EffectRef::None, true);
        right.switch_in(MonId(8), 0, crate::event::EffectRef::None, true);
        let a = wrong.log.entry_mons[0].0;
        let b = right.log.entry_mons[0].0;
        assert_eq!(wrong.log.text.entries[a], right.log.text.entries[b]);
        assert!(!entry_matches(&cx, &wrong, a, 0));
        assert!(entry_matches(&cx, &right, b, 0));
    }

    #[test]
    fn a_terastallized_user_does_not_disguise_as_ogerpon() {
        let mut pool = SetPool::new();
        pool.add_team(&[Z, U, L].join("]")).unwrap();
        let mut ogre = TeamDef::unpack(B).unwrap().sets[0].clone();
        ogre.name = "Ogerpon".into();
        ogre.species = dex::SPECIES_OGERPON;
        ogre.ability = dex::ABILITY_DEFIANT;
        ogre.gender = crate::teams::Gender::Female;
        pool.add_set(&ogre);
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|4
            |switch|p2a: Latias|Latias, L80, F|100/100
            |switch|p2b: Umbreon|Umbreon, L85, M|100/100
            |turn|1
            |switch|p2a: Ogerpon|Ogerpon, L90, F|100/100
            |replace|p2a: Zoroark|Zoroark, L84, F
            |-terastallize|p2a: Zoroark|Poison
            |switch|p2a: Latias|Latias, L80, F|100/100
            |switch|p2a: Zoroark|Zoroark, L84, F|100/100",
            )
            .iter()
            .map(String::as_str),
        );
        let config = WorldConfig::default();
        let team = sample_team(&context(&t, &pool, &config), &mut SplitMix64(10)).unwrap().0;
        assert_eq!(team.sets[3].name, "Ogerpon");
    }

    #[test]
    fn a_transformed_disguise_can_invoke_the_tera_exception() {
        let mut pool = SetPool::new();
        pool.add_team(
            &[Z, U, "Ditto||ChoiceScarf|Imposter|transform||85,,85,85,85,85|N|,0,,,,||90|,,,,,Steel"].join("]"),
        )
        .unwrap();
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|3
            |switch|p2a: Umbreon|Umbreon, L85, M|100/100
            |switch|p2b: Ditto|Ditto, L90|100/100
            |-transform|p2b: Ditto|p1a: Ogerpon|[from] ability: Imposter
            |turn|1
            |replace|p2a: Zoroark|Zoroark, L84, F
            |-terastallize|p2a: Zoroark|Poison
            |switch|p2a: Umbreon|Umbreon, L85, M|100/100
            |faint|p2a: Umbreon
            |switch|p2a: Zoroark|Zoroark, L84, F|100/100",
            )
            .iter()
            .map(String::as_str),
        );
        let config = WorldConfig::default();
        let team = sample_team(&context(&t, &pool, &config), &mut SplitMix64(10)).unwrap().0;
        assert_eq!(team.sets[0].name, "Zoroark");
        assert_eq!(team.sets[1].name, "Ditto");
    }

    #[test]
    fn replacements_before_turn_one_keep_only_the_original_lead_fixed() {
        let mut t = RevealTracker::new();
        t.feed_all(
            lines(
                "|teamsize|p2|6
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |switch|p2b: Mesprit|Mesprit, L86|100/100
            |switch|p2b: Latias|Latias, L80, F|100/100
            |replace|p2a: Zoroark|Zoroark, L84, F
            |turn|1",
            )
            .iter()
            .map(String::as_str),
        );
        let pool = fixture_pool();
        let config = WorldConfig::default();
        let team = sample_team(&context(&t, &pool, &config), &mut SplitMix64(10)).unwrap().0;
        assert_eq!(team.sets[0].name, "Zoroark");
        assert_eq!(team.sets[1].name, "Mesprit");
    }

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect()
    }

    fn move_id(name: &str) -> EffectId {
        dex::lookup(EffectKind::Move, name).unwrap()
    }

    #[test]
    fn blocked_moves_belong_to_the_user_not_the_ability_holder() {
        let window = lines(
            "|cant|p2a: Tsareena|ability: Queenly Majesty|Fake Out|[of] p1a: Incineroar
             |move|p2a: Tsareena|Power Whip|p1b: Pelipper",
        );
        let opp = SideId(1);
        assert_eq!(
            infer_move(opp, 0, "Tsareena", &window, 0),
            SlotSpec::Move { id: move_id("powerwhip"), loc: Some(2), tera: false }
        );
        // From the other side: the blocked user chose Fake Out.
        assert_eq!(
            infer_move(SideId(0), 0, "Incineroar", &window, 0),
            SlotSpec::Move { id: move_id("fakeout"), loc: None, tera: false }
        );
    }

    #[test]
    fn coactive_disguises_replace_and_ally_switch_keep_separate_choices() {
        let window = lines(
            "|move|p2a: Klawf|Sludge Bomb|p2b: Klawf
            |move|p2b: Klawf|Knock Off|p1a: X",
        );
        assert_eq!(
            infer_move(SideId(1), 0, "Klawf", &window, 0),
            SlotSpec::Move { id: move_id("sludgebomb"), loc: Some(-2), tera: false }
        );
        assert_eq!(
            infer_move(SideId(1), 1, "Klawf", &window, 0),
            SlotSpec::Move { id: move_id("knockoff"), loc: Some(1), tera: false }
        );
        let window = lines(
            "|replace|p2a: Zoroark|Zoroark, L84
            |swap|p2b: Klawf|0
            |move|p2b: Zoroark|Sludge Bomb|p1a: X",
        );
        assert_eq!(
            infer_move(SideId(1), 0, "Klawf", &window, 0),
            SlotSpec::Move { id: move_id("sludgebomb"), loc: Some(1), tera: false }
        );
    }

    #[test]
    fn dancer_copies_instructed_uses_and_flinches_are_not_choices() {
        let opp = SideId(1);
        let window = lines(
            "|-terastallize|p2a: Kommo-o|Steel
             |cant|p2a: Kommo-o|flinch",
        );
        assert_eq!(infer_move(opp, 0, "Kommo-o", &window, 0), SlotSpec::AnyMove { tera: true });
        let window = lines(
            "|-activate|p2a: Oricorio|ability: Dancer
             |move|p2a: Oricorio|Swords Dance||[still]
             |-singleturn|p2a: Oricorio|move: Instruct|[of] p2b: Indeedee
             |move|p2a: Oricorio|Revelation Dance|p1a: X
             |move|p2a: Oricorio|Hurricane|p1b: Y",
        );
        assert_eq!(
            infer_move(opp, 0, "Oricorio", &window, 0),
            SlotSpec::Move { id: move_id("hurricane"), loc: Some(2), tera: false }
        );
    }

    #[test]
    fn requests_compare_modulo_whitespace_and_rqid() {
        let engine = r#"{"active":[{"moves":[{"move":"Protect","id":"protect","pp":16,"maxpp":16}]}],"side":{"name":"A B"}}"#;
        let client = "{\"active\": [{\"moves\": [{\"move\": \"Protect\", \"id\": \"protect\", \"pp\": 16, \"maxpp\": 16}]}],\n \"side\": {\"name\": \"A B\"}, \"rqid\": 7}";
        assert_eq!(normalize_request(client), engine);
        assert_eq!(normalize_request(engine), engine);
    }

    #[test]
    fn recorded_replays_keep_copy_reseeds_and_reject_draw_shifts() {
        let own = Arc::new(TeamDef::unpack(&[BL, M].join("]")).unwrap());
        let opp = Arc::new(TeamDef::unpack(&[L, U].join("]")).unwrap());
        let mut pool = SetPool::new();
        for set in &opp.sets[..2] {
            pool.add_set(set);
        }
        let mut real = Battle::from_team_defs([1, 2, 3, 4], own, opp, ["A".into(), "B".into()], DrawLog::default()).unwrap();
        real.start().unwrap();
        let config = WorldConfig { particles: 2, rebuild_budget: 1, fresh_per_update: 0, ..WorldConfig::default() };
        let mut builder = WorldBuilder::new(SideId(0), real.teams().sides[0].clone(), ["A".into(), "B".into()],
            Arc::new(pool), config, 5).unwrap();
        let push = |builder: &mut WorldBuilder, real: &Battle<DrawLog>| {
            let mut produced = Vec::new();
            view(&real.log, SideId(0), &mut produced);
            builder.push_lines(produced.iter().map(|&(raw, _)| real.log.text.entries[raw].as_str()));
            builder.push_request(&real.request_json(0).unwrap());
        };
        push(&mut builder, &real);
        builder.update().unwrap();
        builder.push_choice("move 2, move 1");
        real.log.reset();
        real.choose(0, "move 2, move 1").unwrap();
        real.choose(1, "move 4, move 2").unwrap();
        push(&mut builder, &real);
        builder.update().unwrap();
        // The copy was reseeded at the boundary it was made at; its record starts from that PRNG.
        let p = &builder.particles[1];
        let turn = p.trail.as_ref().unwrap();
        assert_eq!((turn.seg, turn.prng.draws()), (1, 0));
        assert!(!p.history[0].battle.state.started);
        let cx = Ctx { lines: &builder.lines, choices: &builder.choices, requests: &builder.requests,
            ..context(&builder.tracker, &builder.pool, &config) };
        let mon = MonId(6);
        let set = &p.battle.teams().sides[1].sets[0];
        let mut stats = WorldStats::default();
        let replayed = replay_set(&cx, p, 0, mon, set, 0.0, &mut stats).expect("same set and recorded randomness");
        assert_eq!(format!("{:?}", replayed.battle.state), format!("{:?}", p.battle.state));
        assert_eq!(replayed.battle.request_json(0), p.battle.request_json(0));
        assert_eq!((replayed.cursor, replayed.choice, replayed.seg), (p.cursor, p.choice, p.seg));
        assert_eq!(replayed.history.iter().map(|s| s.seg).collect::<Vec<_>>(), p.history.iter().map(|s| s.seg).collect::<Vec<_>>());
        assert_eq!(stats.repair_replays, 2);
        let mut changed = set.clone();
        // A new speed tie consumes a shuffle draw without changing any introduction line.
        changed.evs[5] = 0;
        let speed = p.battle.state.pokemon[1].base_stored_stats[4];
        changed.ivs[5] = (0..=31).find(|&iv| {
            changed.ivs[5] = iv;
            crate::battle::pokemon_from_set(&changed, 0).unwrap().base_stored_stats[4] == speed
        }).expect("Latias can tie Mesprit's speed");
        let start = turn.previous.as_ref().unwrap();
        let mut tied = p.history[0].battle.clone();
        tied.replace_hidden_set(cx.viewer, mon, &changed).unwrap();
        tied.state.prng = start.prng;
        tied.state.prng.set_resample(start.key, &start.points);
        tied.start().unwrap();
        let mut produced = Vec::new();
        view(&tied.log, cx.viewer, &mut produced);
        assert_eq!(produced.iter().map(|&(raw, _)| &tied.log.text.entries[raw]).collect::<Vec<_>>(),
            cx.lines[start.cursor..start.end].iter().collect::<Vec<_>>());
        assert_ne!(tied.state.prng.draws(), start.end_draws);
        assert!(replay_set(&cx, p, 0, mon, &changed, 0.0, &mut stats).is_none(), "the speed tie shifts draw counts");
        assert_eq!(stats.repair_draw_shifts, 1);
        changed = set.clone();
        changed.ability = dex::ABILITY_PRESSURE;
        assert!(replay_set(&cx, p, 0, mon, &changed, 0.0, &mut stats).is_none(), "Pressure changes the introduction log");
        let bad_requests = vec![Some("{}".to_owned()); builder.requests.len()];
        assert!(replay_set(&Ctx { requests: &bad_requests, ..cx }, p, 0, mon, set, 0.0, &mut stats).is_none());
    }

    #[test]
    fn off_pool_reveals_are_patched_into_the_set() {
        let mut pool = SetPool::new();
        pool.add_team("Bastiodon||Leftovers|Sturdy|foulplay,wideguard,irondefense,bodypress||85,,85,85,85,85|M|,0,,,,||90|,,,,,Fighting").unwrap();
        let revealed = [move_id("protect"), move_id("bodypress")];
        let mut c = SetConstraints::new("Bastiodon", "Bastiodon");
        c.moves = &revealed;
        c.level = Some(88);
        let sampled = pool.sample(&c, &mut SplitMix64(1)).unwrap();
        assert!(!sampled.consistent);
        let mut set = sampled.set;
        repair(&mut set, &c);
        let moves = &set.moves[..set.move_count as usize];
        assert_eq!(moves.len(), 4);
        assert!(revealed.iter().all(|m| moves.contains(m)), "{moves:?}");
        assert_eq!(set.level, 88);
    }

    #[test]
    fn windows_resample_their_draws_and_keep_the_rest() {
        let mut points = Vec::new();
        set_window(&mut points, 5, 8, 1);
        assert_eq!(points, [(5, 1), (8, RESUME)]);
        // A window inside another splits it; the outer stream continues on both sides.
        set_window(&mut points, 6, 7, 2);
        assert_eq!(points, [(5, 1), (6, 2), (7, 1), (8, RESUME)]);
        // Giving a range the stream around it merges the points again.
        set_window(&mut points, 6, 7, 1);
        assert_eq!(points, [(5, 1), (8, RESUME)]);
        set_window(&mut points, 7, u32::MAX, 3);
        assert_eq!(points, [(5, 1), (7, 3)]);
        let mut many = Vec::new();
        for i in 0..20 {
            set_window(&mut many, 10 * i, 10 * i + 5, i + 1);
            assert!(many.len() <= RESAMPLE_CAPACITY && many.windows(2).all(|w| w[0].0 < w[1].0));
            assert_eq!(many[many.len() - 2..], [(10 * i, i + 1), (10 * i + 5, RESUME)]);
        }
        // Only the draws in the window change.
        let base = crate::prng::Prng::from_seed([1, 2, 3, 4]);
        let plain: Vec<u32> = (0..12).scan(base, |p, _| Some(p.next_u32())).collect();
        let mut points = Vec::new();
        set_window(&mut points, 4, 8, 7);
        let mut windowed = base;
        windowed.set_resample(9, &points);
        let got: Vec<u32> = (0..12).map(|_| windowed.next_u32()).collect();
        assert_eq!((&got[..4], &got[8..]), (&plain[..4], &plain[8..]));
        assert!((4..8).all(|i| got[i] != plain[i]));
    }

    #[test]
    fn window_starts_keep_earlier_lines_and_move_lines_until_the_next_move() {
        let entries: Vec<String> = [
            "|move|p1a: A|Tackle|p2a: B",
            "|-damage|p2a: B|50/100",
            "|move|p2a: B|Tackle|p1a: A",
            "|-damage|p1a: A|10/100",
        ]
        .map(String::from)
        .into();
        let produced = [(0, 3), (1, 6), (2, 8), (3, 11)];
        let mut out = Vec::new();
        // B's move line may still get a `[miss]` until the end: it is no start for its damage line.
        candidates(0, &entries, &produced, 3, 12, 11, &mut out);
        assert_eq!(out, [8, 6, 0]);
        // A's damage roll follows A's move line, which is final only once B's is emitted.
        candidates(0, &entries, &produced, 1, 12, 6, &mut out);
        assert_eq!(out, [0]);
        assert!(edited("|move|p1a: A|Tackle|p2a: B|[miss]", "|move|p1a: A|Tackle|p2a: B"));
        assert!(edited("|move|p1a: A|Rock Slide|p2b: C|[spread] p2a,p2b", "|move|p1a: A|Rock Slide|p2a: B"));
        assert!(!edited("|move|p1a: A|Tackle|p2a: B", "|move|p1a: A|Protect|p1a: A"));
        assert!(!edited("|-damage|p2a: B|50/100", "|-damage|p2a: B|49/100"));
    }

    #[test]
    fn opponent_action_and_hp_lines() {
        let opp = SideId(1);
        assert!(opp_action("|move|p2a: B|Tackle|p1a: A", opp));
        assert!(opp_action("|move|p2a: B|Outrage|p1a: A|[from]lockedmove", opp));
        assert!(!opp_action("|move|p2a: B|Tackle|p1a: A|[from]move: Sleep Talk", opp));
        assert!(!opp_action("|move|p1a: A|Tackle|p2a: B", opp));
        assert!(opp_action("|switch|p2b: C|C, L80|100/100", opp));
        assert!(!opp_action("|-damage|p2a: B|50/100", opp));
        assert_eq!(hp_subject("|-heal|p2a: B|60/100|[from] item: Leftovers"), Some((opp, "B")));
        assert_eq!(hp_subject("|-damage|p1b: A|10/250"), Some((SideId(0), "A")));
        assert_eq!(hp_subject("|move|p2a: B|Recover|p2a: B"), None);
    }

    #[test]
    fn exact_hp_history_starts_after_the_hp_was_last_shown_full() {
        let lines: Vec<String> = [
            "|switch|p2a: B|B, L80|100/100",
            "|-damage|p2a: B|80/100",
            "|-heal|p2a: B|100/100|[from] item: Leftovers",
            "|switch|p2b: C|C, L80|100/100",
            "|-damage|p2a: B|60/100",
            "|-damage|p2b: C|70/100",
            "|-damage|p2a: B|40/100",
        ]
        .map(String::from)
        .into();
        let (t, pool, config) = (RevealTracker::new(), fixture_pool(), WorldConfig::default());
        let cx = Ctx { lines: &lines, ..context(&t, &pool, &config) };
        assert_eq!(hp_history(&cx, SideId(1), "B", 7), 4);
        assert_eq!(hp_history(&cx, SideId(1), "B", 3), 3);
        assert_eq!(hp_history(&cx, SideId(1), "C", 7), 5);
    }

    #[test]
    fn only_full_movesets_reveal_a_move_order() {
        let full = r#"{"active":[{"moves":[{"move":"Body Slam","id":"bodyslam","pp":24,"maxpp":24},{"move":"Curse","id":"curse","pp":16,"maxpp":16}]},{"moves":[{"move":"Outrage","id":"outrage"}]}],"side":{}}"#;
        let ids = active_move_ids(full);
        assert_eq!(ids[0], vec![move_id("bodyslam"), move_id("curse")]);
        assert!(ids[1].is_empty(), "a locked move's single entry is not a moveset");
    }
}

#[cfg(test)]
#[path = "world_bench.rs"]
mod bench;
