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
//!   (`protocol::is_compared` lines). Random outcomes are searched, not guessed: on a mismatch the
//!   draws behind the offending line are resampled through `Prng::set_resample` (every earlier
//!   draw kept), backing off to earlier lines when that keeps failing, within a per-segment
//!   budget. Wrong hidden hypotheses (an item, stat spread or exact HP that cannot reproduce the
//!   observed order or damage) therefore die instead of being reported.
//! * every world carries a weight: a segment its hypothesis reproduced after `n` attempts
//!   multiplies it by `1/n`. This is a search-effort heuristic, not a likelihood estimate (the
//!   guided search keeps matched prefixes, so effort undercounts joint improbability); it only
//!   makes hypotheses that needed far more luck count less. `worlds` draws worlds in proportion.
//! * worlds are maintained incrementally: each new segment extends the surviving ones; an
//!   opponent Pokemon seen for the first time is swapped into a never-revealed bench slot with
//!   `replace_hidden_set` (exact by its contract); dead worlds are rebuilt from scratch within a
//!   budget, the rest refilled by copying survivors.
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
    prng::RESAMPLE_CAPACITY,
    sim::choices::LegalActions,
    state::choices::{ChoiceKind, RequestKind, SlotChoice},
    teams::{SetDef, TeamDef},
};
use std::{cell::Cell, sync::Arc};

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
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self { particles: 8, segment_budget: 1000, rebuild_budget: 16, fresh_per_update: 1, shown_target: 0.85 }
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
    /// Log weight (sum of `-ln attempts` over reproduced segments; a search-effort heuristic).
    log_w: f64,
    /// Why the world was dropped (diagnostics).
    why: String,
    hypothesis: Option<Arc<IllusionHypothesis>>,
    /// Some opponent set was patched by `repair`.
    repaired: bool,
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
    Budget { line: usize },
    Desync,
    Choice,
    Swap,
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
            evidence: Vec::new(),
            rotation: 0,
            tracker: RevealTracker::new(),
            lines: Vec::new(),
            choices: Vec::new(),
            requests: Vec::new(),
            transformed: Vec::new(),
            move_orders: Vec::new(),
            particles: Vec::new(),
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
        };
        let mut alive = Vec::with_capacity(self.config.particles);
        let mut source = None;
        let mut cached: Vec<Arc<IllusionHypothesis>> = Vec::new();
        for mut p in std::mem::take(&mut self.particles) {
            if let Some(h) = &p.hypothesis {
                if h.stints.iter().all(|st| stint(&cx, *st).is_some_and(|e| e.owner == h.user)) {
                    p.hypothesis = None;
                } else if h.revision != cx.lines.len() {
                    let view = cached.iter().find(|v| v.user == h.user && v.species == h.species && v.stints == h.stints);
                    p.hypothesis = Some(match view {
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
            match advance(&cx, &mut p, &mut self.stats) {
                Ok(()) => alive.push(p),
                Err(d) => {
                    record_evidence(&cx, d, &mut self.evidence);
                    self.stats.deaths += 1;
                    self.last_failure = format!("{d:?}: {}", p.why);
                }
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
            let build_cx = Ctx { repair: repairing, ..cx };
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
    let before = cx.repairs.get();
    let opp = Arc::new(sample_team(cx, &mut prng)?);
    let log_w = -REPAIR_LOG_PENALTY * f64::from(cx.repairs.get() - before);
    let (p1, p2) = if cx.viewer.0 == 0 { (own, opp) } else { (opp, own) };
    let log = DrawLog { track_entries: has_illusion(&p1, &p2, cx.opp()), ..DrawLog::default() };
    let battle = Battle::from_team_defs(prng.seed(), p1, p2, names.clone(), log).map_err(|e| WorldError::Team(e.0))?;
    stats.builds += 1;
    let repaired = cx.repairs.get() > before;
    Ok(Particle { battle, cursor: 0, choice: 0, rng: prng, log_w, why: String::new(), hypothesis, repaired })
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

fn sample_revealed(cx: &Ctx<'_>, index: usize, rng: &mut SplitMix64) -> Result<SetDef, WorldError> {
    let c = constraints(cx, index);
    let sampled = cx
        .pool
        .sample(&c, rng)
        .ok_or_else(|| WorldError::UnknownSpecies(c.shown_species.to_owned()))?;
    let mut set = sampled.set;
    if !sampled.consistent {
        if !cx.repair {
            return Err(WorldError::NoConsistentWorld);
        }
        repair(&mut set, &c);
        cx.repairs.set(cx.repairs.get() + 1);
    }
    if let Some(order) = move_order(cx, c.name) {
        set.moves = [EffectId::NONE; 4];
        set.moves[..order.len()].copy_from_slice(order);
        set.move_count = order.len() as u8;
    }
    Ok(set)
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
fn sample_team(cx: &Ctx<'_>, rng: &mut SplitMix64) -> Result<TeamDef, WorldError> {
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
    let mut sets: Vec<SetDef> = (0..cx.seen.len()).map(|i| sample_revealed(cx, i, rng)).collect::<Result<_, _>>()?;
    // A broken lead's disguise may have no remaining MonInfo, but the party still needs it.
    for e in entries {
        if !sets.iter().any(|s| s.name == e.shown.name) {
            let c = mon_constraints(&e.shown);
            let sampled =
                cx.pool.sample(&c, rng).ok_or_else(|| WorldError::UnknownSpecies(c.shown_species.to_owned()))?;
            let mut set = sampled.set;
            if !sampled.consistent {
                if !cx.repair {
                    return Err(WorldError::NoConsistentWorld);
                }
                repair(&mut set, &c);
                cx.repairs.set(cx.repairs.get() + 1);
            }
            sets.push(set);
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
    }
    let order = party_order(cx, &sets, rng).ok_or(WorldError::IllusionOrder)?;
    let mut team = TeamDef { len: size as u8, ..TeamDef::default() };
    for (pos, i) in order.into_iter().enumerate().take(size) {
        team.sets[pos] = sets[i].clone();
    }
    Ok(team)
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
    let Dead::Budget { line } = dead else { return };
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

fn sample_plain_team(cx: &Ctx<'_>, rng: &mut SplitMix64) -> Result<TeamDef, WorldError> {
    let opp = cx.opp();
    let seen = cx.seen;
    let size = (cx.tracker.team_size(opp).unwrap_or(6) as usize).max(seen.len().max(1)).min(6);
    let mut slots: [Option<SetDef>; 6] = Default::default();
    let leads = cx.tracker.leads(opp);
    for (pos, lead) in leads.iter().enumerate() {
        if let Some(i) = *lead {
            slots[pos] = Some(sample_revealed(cx, i, rng)?);
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
        slots[free[rng.below(free.len())]] = Some(sample_revealed(cx, i, rng)?);
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
    Ok(team)
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
    // Battle::clone gives copies a default empty sink; restore the capture flag for every replay.
    p.battle.log.track_entries = p.battle.teams().sides[cx.opp().0 as usize].sets.iter()
        .any(|s| s.ability == dex::ABILITY_ILLUSION);
    let hypothesis = p.hypothesis.clone();
    let cx = &Ctx { seen: hypothesis.as_ref().map_or(cx.seen, |h| &h.view), hypothesis: hypothesis.as_deref(), ..*cx };
    let mut idle = 0;
    loop {
        let before = p.cursor;
        match step(cx, p, stats) {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            Err(d) => {
                if p.why.is_empty() {
                    p.why = format!("at line {} (choice {})", p.cursor, p.choice);
                }
                return Err(d);
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
    }
    let checkpoint = p.battle.clone();
    let specs = if opp_acts { Some(infer(cx, &checkpoint, window, p.cursor)) } else { None };
    let region = &cx.lines[p.cursor..limit];
    let viewer_text = viewer_acts.then(|| cx.choices[p.choice].1.as_str());
    let seg_draws = checkpoint.state.prng.draws();
    let key = p.rng.next_u64();
    let mut points: Vec<(u32, u32)> = Vec::with_capacity(RESAMPLE_CAPACITY);
    let mut opp_choice = match &specs {
        Some(s) => Some(sample_choice(cx, &checkpoint, s, false, &mut p.rng).ok_or_else(|| {
            p.why = format!("no legal opponent choice for {s:?} at line {}", p.cursor);
            Dead::Choice
        })?),
        None => None,
    };
    let (mut last_l, mut fails) = (usize::MAX, 0u32);
    let mut produced: Vec<(usize, u32)> = Vec::new();
    for attempt in 0..cx.config.segment_budget {
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
            p.cursor += produced.len();
            if viewer_acts {
                p.choice += 1;
            }
            stats.segments += 1;
            return Ok(true);
        };
        // Resample the draws behind line `l`, backing off to earlier emission points when the
        // same line keeps failing (its cause may be an earlier, invisible draw).
        fails = if l == last_l { fails + 1 } else { 1 };
        last_l = l;
        // A line that resampling the whole segment cannot fix is a wrong hypothesis (a set, an
        // exact HP), not bad luck: stop spending the budget on it.
        if fails > STUCK {
            break;
        }
        let mut level = 0;
        let mut threshold = 6;
        while fails > threshold {
            level += 1;
            threshold = threshold * 2 + 6;
        }
        let mut emission: Vec<u32> = core::iter::once(seg_draws)
            .chain(produced[..l].iter().map(|&(_, d)| d))
            .collect();
        emission.dedup();
        let at = emission[emission.len() - 1 - level.min(emission.len() - 1)];
        if at == seg_draws
            && let Some(s) = &specs
            && (level > 0 || l == 0)
            && p.rng.chance(0.5)
        {
            opp_choice = Some(sample_choice(cx, &checkpoint, s, true, &mut p.rng).ok_or(Dead::Choice)?);
        }
        points.retain(|&(d, _)| d < at);
        if points.len() == RESAMPLE_CAPACITY {
            points.clear();
        }
        points.push((at, attempt));
    }
    let entries = &p.battle.log.text.entries;
    p.why = format!(
        "{} at line {}: expected {:?}, replayed {:?}",
        if fails > STUCK { "stuck" } else { "budget" },
        p.cursor + last_l,
        region.get(last_l),
        produced.get(last_l).map(|&(i, _)| &entries[i])
    );
    if let Some(&(raw, _)) = produced.get(last_l)
        && !entry_matches(cx, &p.battle, raw, p.cursor + last_l)
        && let (Some(e), Some(mon)) = (cx.tracker.entry(opp, p.cursor + last_l), entry_actor(&p.battle.log, raw))
    {
        p.why.push_str(&format!("; entry owner expected {:?}, replayed {:?}", cx.owner(e), set_name(&p.battle, mon)));
    }
    Err(Dead::Budget { line: p.cursor.saturating_add(last_l) })
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
        if party.iter().any(|&m| set_name(&p.battle, m) == id.name) {
            continue;
        }
        let Some(index) = cx.seen.iter().position(|m| m.name == id.name) else {
            return Err(Dead::Swap);
        };
        let hidden = p.battle.hidden_mons(cx.viewer);
        // Prefer slots holding a set nobody has been seen as.
        let mut slots: Vec<MonId> = party.iter().copied().filter(|m| hidden & (1 << m.0) != 0).collect();
        slots.sort_by_key(|&m| cx.tracker.find(opp, set_name(&p.battle, m)).is_some());
        let Some(&first) = slots.first() else { return Err(Dead::Swap) };
        let preferred = cx.tracker.find(opp, set_name(&p.battle, first)).is_some();
        let pool: Vec<MonId> = slots
            .iter()
            .copied()
            .filter(|&m| cx.tracker.find(opp, set_name(&p.battle, m)).is_some() == preferred)
            .collect();
        let mon = pool[p.rng.below(pool.len())];
        let exact_first = cx.tracker.entry(opp, p.cursor + offset).is_some_and(|e| can_explain_entry(cx, e));
        let sample_cx = Ctx { repair: cx.repair && !exact_first, ..*cx };
        let before = cx.repairs.get();
        let set = sample_revealed(&sample_cx, index, &mut p.rng).map_err(|e| {
            p.why = format!("new entry {} needs rebuilding before pool repair: {e}", id.name);
            Dead::Swap
        })?;
        p.battle.replace_hidden_set(cx.viewer, mon, &set).map_err(|_| Dead::Swap)?;
        p.log_w -= REPAIR_LOG_PENALTY * f64::from(cx.repairs.get() - before);
        p.repaired |= cx.repairs.get() > before;
        stats.swaps += 1;
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

/// Consecutive failures at one line after which a world is dropped (the backoff has resampled the
/// whole segment, and the opponent's open choices, many times by then).
const STUCK: u32 = 200;

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
                let team = sample_team(&cx, &mut rng).unwrap();
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
        let team = sample_team(&cx, &mut SplitMix64(10)).unwrap();
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
        let team = sample_team(&cx, &mut SplitMix64(10)).unwrap();
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
        let z = sample_revealed(&cx, t.find(SideId(1), "Zoroark").unwrap(), &mut SplitMix64(7)).unwrap();
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
        let team = sample_team(&context(&t, &pool, &config), &mut SplitMix64(10)).unwrap();
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
        let team = sample_team(&context(&t, &pool, &config), &mut SplitMix64(10)).unwrap();
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
        let team = sample_team(&context(&t, &pool, &config), &mut SplitMix64(10)).unwrap();
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
    fn only_full_movesets_reveal_a_move_order() {
        let full = r#"{"active":[{"moves":[{"move":"Body Slam","id":"bodyslam","pp":24,"maxpp":24},{"move":"Curse","id":"curse","pp":16,"maxpp":16}]},{"moves":[{"move":"Outrage","id":"outrage"}]}],"side":{}}"#;
        let ids = active_move_ids(full);
        assert_eq!(ids[0], vec![move_id("bodyslam"), move_id("curse")]);
        assert!(ids[1].is_empty(), "a locked move's single entry is not a moveset");
    }
}
