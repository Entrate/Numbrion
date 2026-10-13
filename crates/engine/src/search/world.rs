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
//! * `worlds` returns copies at the current decision boundary, reseeded from the caller's seed (the
//!   PRNG state is hidden information too).
//!
//! Everything depends only on the inputs, the pool, the builder seed and when `update`/`worlds` are
//! called (worlds are maintained incrementally): two observers with the same public information
//! and the same call sequence get the same worlds (tests/world.rs, the leak test).
use super::{
    SplitMix64,
    protocol::{self, Line, parse_ident},
    reveal::RevealTracker,
    sets::{SetConstraints, SetPool},
};
use crate::{
    Battle, dex,
    ids::*,
    log::{LogEntry, LogSink, LogView, MoveLineEdit, TextLog},
    prng::RESAMPLE_CAPACITY,
    sim::choices::LegalActions,
    state::choices::{ChoiceKind, RequestKind, SlotChoice},
    teams::{SetDef, TeamDef},
};
use std::sync::Arc;

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
}

impl core::fmt::Display for WorldError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoConsistentWorld => f.write_str("no world reproduced the observed history"),
            Self::UnknownSpecies(s) => write!(f, "revealed species {s} is not in the set pool"),
            Self::PendingChoice => f.write_str("a choice was pushed without the lines it produced"),
            Self::Desync(what) => write!(f, "inconsistent observation stream: {what}"),
            Self::Team(e) => write!(f, "invalid team: {e}"),
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
}

impl LogSink for DrawLog {
    const ENABLED: bool = true;

    fn reset(&mut self) {
        self.text.reset();
        self.draws.clear();
    }

    fn emit(&mut self, view: LogView<'_>, entry: LogEntry<'_>) {
        let draws = view.state.prng.draws();
        self.text.emit(view, entry);
        self.draws.resize(self.text.entries.len(), draws);
    }

    fn edit_move(&mut self, view: LogView<'_>, edit: MoveLineEdit<'_>) {
        let (len, line) = (self.text.entries.len(), self.text.last_move_line);
        self.text.edit_move(view, edit);
        if self.text.entries.len() < len
            && let Some(i) = line
        {
            self.draws.remove(i);
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
}

/// What the protocol says one opponent slot chose at a boundary.
#[derive(Clone, Debug, PartialEq)]
enum SlotSpec {
    Pass,
    Move { id: EffectId, loc: Option<i8>, tera: bool },
    /// The slot acted invisibly (flinch, sleep, fainted first) or not at all: any legal move,
    /// terastallizing when the protocol showed its Tera.
    AnyMove { tera: bool },
    Switch(String),
    Revive(String),
}

/// A dropped world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dead {
    Budget,
    Desync,
    Choice,
    Swap,
}

/// Borrowed inputs of a replay.
struct Ctx<'a> {
    viewer: SideId,
    lines: &'a [String],
    choices: &'a [(usize, String)],
    requests: &'a [Option<String>],
    move_orders: &'a [(String, Vec<EffectId>)],
    tracker: &'a RevealTracker,
    pool: &'a SetPool,
    config: &'a WorldConfig,
}

impl Ctx<'_> {
    fn opp(&self) -> SideId {
        SideId(1 - self.viewer.0)
    }
}

pub struct WorldBuilder {
    viewer: SideId,
    own: Arc<TeamDef>,
    names: [String; 2],
    pool: Arc<SetPool>,
    config: WorldConfig,
    rng: SplitMix64,
    tracker: RevealTracker,
    /// Observed compared lines.
    lines: Vec<String>,
    /// (index into `lines` at which the choice was made, Showdown choice text).
    choices: Vec<(usize, String)>,
    /// Viewer request JSON by the index of the choice that answered it (`None`: not pushed).
    requests: Vec<Option<String>>,
    /// Viewer Pokemon name -> the opponent Pokemon it is transformed into (`-transform` lines).
    transformed: Vec<(String, String)>,
    /// Opponent Pokemon name -> its exact moves in slot order, read off the viewer's request while a
    /// viewer Pokemon was transformed into it (Transform copies the move slots in order).
    move_orders: Vec<(String, Vec<EffectId>)>,
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
                self.transformed.retain(|(u, _)| u != user.name);
                self.transformed.push((user.name.to_owned(), target.name.to_owned()));
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
            if let Some((_, target)) = self.transformed.iter().find(|(u, _)| *u == m.name)
                && !ids.is_empty()
                && ids.len() <= 4
            {
                let target = target.clone();
                self.move_orders.retain(|(t, _)| *t != target);
                self.move_orders.push((target, ids));
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
        let cx = Ctx {
            viewer: self.viewer,
            lines: &self.lines,
            choices: &self.choices,
            requests: &self.requests,
            move_orders: &self.move_orders,
            tracker: &self.tracker,
            pool: &self.pool,
            config: &self.config,
        };
        let mut alive = Vec::with_capacity(self.config.particles);
        for mut p in std::mem::take(&mut self.particles) {
            match advance(&cx, &mut p, &mut self.stats) {
                Ok(()) => alive.push(p),
                Err(d) => {
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
            let want = if collapsed || alive.len() < self.config.particles { self.config.fresh_per_update } else { 0 };
            (alive.len() + want, want)
        };
        let mut builds = 0;
        let mut error = None;
        while alive.len() < target.max(1) && builds < budget {
            builds += 1;
            match build(&cx, self.own.clone(), &self.names, &mut self.rng, &mut self.stats) {
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
                        self.stats.deaths += 1;
                        self.last_failure = format!("{d:?}: {}", p.why);
                    }
                },
                Err(e @ (WorldError::UnknownSpecies(_) | WorldError::Team(_))) => {
                    error = Some(e);
                    break;
                }
                Err(e) => error = Some(e),
            }
        }
        if alive.is_empty() {
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
    own: Arc<TeamDef>,
    names: &[String; 2],
    rng: &mut SplitMix64,
    stats: &mut WorldStats,
) -> Result<Particle, WorldError> {
    let mut prng = rng.fork();
    let opp = Arc::new(sample_team(cx, &mut prng)?);
    let (p1, p2) = if cx.viewer.0 == 0 { (own, opp) } else { (opp, own) };
    let battle = Battle::from_team_defs(prng.seed(), p1, p2, names.clone(), DrawLog::default())
        .map_err(|e| WorldError::Team(e.0))?;
    stats.builds += 1;
    Ok(Particle { battle, cursor: 0, choice: 0, rng: prng, log_w: 0.0, why: String::new() })
}

/// Constraints for an opponent Pokemon from what the tracker saw.
fn constraints<'a>(cx: &'a Ctx<'_>, index: usize) -> SetConstraints<'a> {
    let m = &cx.tracker.mons(cx.opp())[index];
    SetConstraints {
        name: &m.name,
        shown_species: &m.details_species,
        level: Some(m.level),
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
        repair(&mut set, &c);
    }
    if let Some((_, order)) = cx.move_orders.iter().find(|(n, _)| *n == c.name) {
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
    let opp = cx.opp();
    let seen = cx.tracker.mons(opp);
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
    let specs = if opp_acts { Some(infer(cx, &checkpoint, window)) } else { None };
    let region = &cx.lines[p.cursor..limit];
    let viewer_text = viewer_acts.then(|| cx.choices[p.choice].1.as_str());
    let seg_draws = checkpoint.state.prng.draws();
    let key = p.rng.next_u64();
    let mut points: Vec<(u32, u32)> = Vec::with_capacity(RESAMPLE_CAPACITY);
    let mut opp_choice = match &specs {
        Some(s) => Some(sample_choice(cx, &checkpoint, s, false, &mut p.rng).ok_or(Dead::Choice)?),
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
            .position(|(&(i, _), want)| entries[i] != *want)
            .or_else(|| {
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
    Err(Dead::Budget)
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
    for line in window {
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
        let Some(index) = cx.tracker.find(opp, id.name) else { return Err(Dead::Swap) };
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
        let set = sample_revealed(cx, index, &mut p.rng).map_err(|_| Dead::Swap)?;
        p.battle.replace_hidden_set(cx.viewer, mon, &set).map_err(|_| Dead::Swap)?;
        stats.swaps += 1;
    }
    Ok(())
}

/// Read the opponent's choice at this boundary off the protocol.
fn infer(cx: &Ctx<'_>, b: &Battle<DrawLog>, window: &[String]) -> [SlotSpec; 2] {
    let opp = cx.opp();
    let legal = b.legal_actions(opp);
    let switch_request = b.side_request_kind(opp) == RequestKind::Switch;
    core::array::from_fn(|k| {
        if k >= legal.slot_count as usize || legal.slots[k].automatic_pass {
            return SlotSpec::Pass;
        }
        if switch_request {
            return infer_replacement(opp, k as u8, legal.slots[k].revival_mask != 0, window);
        }
        let mon = b.state.sides[opp.0 as usize].active[k];
        infer_move(opp, k as u8, shown_name(b, mon), window)
    })
}

fn infer_replacement(opp: SideId, k: u8, reviving: bool, window: &[String]) -> SlotSpec {
    for line in window {
        let Some(l) = Line::parse(line) else { continue };
        match l.cmd() {
            "switch" if !reviving => {
                if let Some(id) = parse_ident(l.arg(0))
                    && id.side == opp
                    && id.position == Some(k)
                {
                    return SlotSpec::Switch(id.name.to_owned());
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

fn infer_move(opp: SideId, k: u8, name: &str, window: &[String]) -> SlotSpec {
    let mut acted = false;
    let mut tera = false;
    // Uses forced by Instruct (announced by `-singleturn|X|move: Instruct`) carry no `[from]`.
    let mut instructed = 0;
    let mut dancing = 0;
    for line in window {
        let Some(l) = Line::parse(line) else { continue };
        let ident = parse_ident(l.arg(0));
        let mine = ident.is_some_and(|id| id.side == opp && id.name == name && id.position.is_some());
        match l.cmd() {
            // Switches resolve before Tera and moves.
            "switch" if !acted => {
                if let Some(id) = ident
                    && id.side == opp
                    && id.position == Some(k)
                {
                    return SlotSpec::Switch(id.name.to_owned());
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
                let actor_mine = actor.is_some_and(|id| id.side == opp && id.name == name);
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
        SlotSpec::Switch(name) | SlotSpec::Revive(name) => {
            let mask = if matches!(spec, SlotSpec::Revive(_)) { ls.revival_mask } else { ls.switch_mask };
            let illusion = dex::lookup(EffectKind::Ability, "illusion");
            let options: Vec<usize> = (0..6).filter(|&p| mask & (1 << p) != 0).collect();
            let exact: Vec<usize> = options.iter().copied().filter(|&p| set_name(b, party[p]) == name).collect();
            let disguised: Vec<usize> = options
                .iter()
                .copied()
                .filter(|&p| Some(b.state.pokemon[party[p].0 as usize].ability) == illusion)
                .collect();
            let p = match (exact.is_empty(), disguised.is_empty()) {
                (true, true) => return None,
                (false, true) => exact[rng.below(exact.len())],
                (true, false) => disguised[rng.below(disguised.len())],
                (false, false) => {
                    if rng.chance(0.9) {
                        exact[rng.below(exact.len())]
                    } else {
                        disguised[rng.below(disguised.len())]
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
            infer_move(opp, 0, "Tsareena", &window),
            SlotSpec::Move { id: move_id("powerwhip"), loc: Some(2), tera: false }
        );
        // From the other side: the blocked user chose Fake Out.
        assert_eq!(
            infer_move(SideId(0), 0, "Incineroar", &window),
            SlotSpec::Move { id: move_id("fakeout"), loc: None, tera: false }
        );
    }

    #[test]
    fn dancer_copies_instructed_uses_and_flinches_are_not_choices() {
        let opp = SideId(1);
        let window = lines(
            "|-terastallize|p2a: Kommo-o|Steel
             |cant|p2a: Kommo-o|flinch",
        );
        assert_eq!(infer_move(opp, 0, "Kommo-o", &window), SlotSpec::AnyMove { tera: true });
        let window = lines(
            "|-activate|p2a: Oricorio|ability: Dancer
             |move|p2a: Oricorio|Swords Dance||[still]
             |-singleturn|p2a: Oricorio|move: Instruct|[of] p2b: Indeedee
             |move|p2a: Oricorio|Revelation Dance|p1a: X
             |move|p2a: Oricorio|Hurricane|p1b: Y",
        );
        assert_eq!(
            infer_move(opp, 0, "Oricorio", &window),
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
