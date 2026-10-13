//! `RevealTracker`: the public information about every Pokemon of both sides, built only from one
//! player's player-view protocol lines (`|split|` already resolved: the owner's secret lines, the
//! public lines of the other side). The output depends on nothing but the lines fed, so the same
//! tracker serves self-play, a ladder client and the set sampler.
//!
//! Attribution follows the engine's emissions (Showdown-exact):
//! * `[from] item: X` names the holder in `[of]` when present (Rocky Helmet, Clear Amulet), else the
//!   line's subject.
//! * `[from] ability: X` names the holder in `[of]` when present (Rough Skin, Static, Drizzle,
//!   Cursed Body, ...), except on `-heal`, where `[of]` is the attacker absorbed (Water Absorb, Volt
//!   Absorb, Dry Skin, Earth Eater) and only Hospitality heals from `[of]`.
//! * `-item` / `-enditem` / `-ability` lines are handled per effect: Frisk names the frisker in
//!   `[of]`, Magician and Pickpocket name the victim in `[of]`, and `-ability|X|NEW|OLD|[from] E`
//!   is an ability change (Trace: `[of]` is the traced Pokemon, which holds NEW).
//! * Abilities in an argument (`-activate|X|ability: A`, `cant|X|ability: A`, `-block|X|ability: A`)
//!   belong to `[of]` when present, else to the subject.
//!
//! Illusion (`|replace|`): every observation made at that position since the switch-in moves to the
//! real Pokemon (its starting ability is Illusion); the disguise is rebuilt from its remaining
//! observations, and dropped when that switch-in was its only appearance. Observations are kept
//! per Pokemon (`MonInfo` is a fold over them) and tagged with the stint (one switch-in at one
//! position) they were made in, so a disguise active next to the Pokemon it imitates is split
//! correctly. Entry identities and party events are retained even when a disguise record is
//! dropped. Search can fold a stint-filtered view, moving unbroken stints to a hypothetical
//! Illusion user without changing the tracker. This also covers silent Illusion endings on faint:
//! a later living appearance of the disguise is evidence for search, not a tracker contradiction.
use super::protocol::{Line, effect_kind, parse_condition, parse_details, parse_ident, parse_side};
use crate::{
    dex,
    ids::{EffectId, EffectKind, SideId, TypeId},
    teams::Gender,
};

/// What is known about the item a Pokemon holds now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemKnowledge {
    #[default]
    Unknown,
    Held(EffectId),
    /// Known to hold nothing (consumed, knocked off, given away).
    Lost,
}

/// The public record of one Pokemon. Identity fields (`name`, `species`, `details_species`,
/// `level`, `gender`, `shiny`) come from its first appearance; everything else is the fold of
/// what was observed about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonInfo {
    /// Protocol name (nickname): the identity within its side.
    pub name: String,
    /// `dex::lookup(Species, details_species)`, `EffectId::NONE` when unknown or out of scope.
    /// The details print the stored (battle-start) forme, e.g. `Zacian-Crowned`, and Showdown
    /// prints `Greninja-Bond` / `Rockruff-Dusk` as their base species.
    pub species: EffectId,
    /// The raw species text of the details at the first switch-in (`""` before any).
    pub details_species: String,
    /// The forme now (`detailschange` / `-formechange`; back to the details forme on switch-in).
    pub current_species: String,
    /// 0 before any details were seen.
    pub level: u16,
    pub gender: Gender,
    pub shiny: bool,
    /// Distinct moves of its own moveset it was seen choosing, in first-use order: `move` lines
    /// without `[from]` (or `[from] lockedmove`) and `cant|X|REASON|MOVE`. Never Struggle, moves
    /// used while transformed, or moves gained by Mimic / Sketch.
    pub moves: Vec<EffectId>,
    pub item: ItemKnowledge,
    /// The item held at battle start, when revealed before any item transfer touched this mon.
    pub starting_item: Option<EffectId>,
    /// An item moved to or from this mon (Trick, Switcheroo, Thief, Covet, Bestow, Magician,
    /// Pickpocket, Symbiosis or an unrecognised `-item` source).
    pub item_transferred: bool,
    /// The ability at battle start, when revealed before any ability change touched this mon.
    pub starting_ability: Option<EffectId>,
    /// The ability as last shown (reset to `starting_ability` when a changed ability reverts on
    /// switch-out).
    pub ability: Option<EffectId>,
    /// Any ability change touched this mon: Skill Swap, Trace, Mummy, Transform, a forme change to
    /// a forme with other abilities, ... `starting_ability` is frozen from then on.
    pub ability_changed: bool,
    pub tera_type: Option<TypeId>,
    pub terastallized: bool,
    /// Last shown condition: exact for the viewer's own side, percent for the other side.
    pub hp: u32,
    pub max_hp: u32,
    /// `""`, `brn`, `par`, `slp`, `frz`, `psn`, `tox` or `fnt`.
    pub status: String,
    pub fainted: bool,
    /// Current position (0 = a, 1 = b).
    pub active: Option<u8>,
    pub transformed: bool,
    /// Revealed (`|replace|`) as an Illusion user.
    pub illusion: bool,
    /// Switch-ins (switch / drag), including the ones under its own Illusion once revealed.
    pub appearances: u32,
    copied: Vec<EffectId>,
    ability_temporary: bool,
    log: Vec<(u32, Obs)>,
}

/// One observation about one Pokemon; `MonInfo` is the fold of its observations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Obs {
    SwitchIn {
        pos: u8,
        species: EffectId,
        condition: Option<(u32, u32, &'static str)>,
        tera: Option<TypeId>,
    },
    SwitchOut {
        pos: u8,
    },
    Swap {
        from: u8,
        to: u8,
    },
    Hp {
        hp: u32,
        max_hp: u32,
        status: &'static str,
    },
    Status(&'static str),
    CureStatus,
    Faint {
        pos: Option<u8>,
    },
    Move(EffectId),
    Copied(EffectId),
    /// Shown as held (an item effect): reveals the item; keeps `Lost` (a consumed berry's effect).
    ItemSeen(EffectId),
    /// Shown leaving the holder (consumed, knocked off, burnt).
    ItemLost(EffectId),
    /// Held right before a transfer in progress.
    ItemBefore(EffectId),
    Transfer,
    ItemNow(ItemKnowledge),
    Ability(EffectId),
    AbilityChange(Option<EffectId>),
    /// `permanent`: a `detailschange` (kept through switches), not a `-formechange`.
    Forme {
        species: EffectId,
        abilities_differ: bool,
        permanent: bool,
    },
    Tera(TypeId),
    Transform(Option<EffectId>),
    Illusion,
}

impl MonInfo {
    fn new(name: &str) -> Self {
        MonInfo {
            name: name.to_string(),
            species: EffectId::NONE,
            details_species: String::new(),
            current_species: String::new(),
            level: 0,
            gender: Gender::Unspecified,
            shiny: false,
            moves: Vec::new(),
            item: ItemKnowledge::Unknown,
            starting_item: None,
            item_transferred: false,
            starting_ability: None,
            ability: None,
            ability_changed: false,
            tera_type: None,
            terastallized: false,
            hp: 0,
            max_hp: 0,
            status: String::new(),
            fainted: false,
            active: None,
            transformed: false,
            illusion: false,
            appearances: 0,
            copied: Vec::new(),
            ability_temporary: false,
            log: Vec::new(),
        }
    }

    fn identify(&mut self, details: &str) {
        let d = parse_details(details);
        self.species = species_id(d.species);
        self.details_species = d.species.to_string();
        self.current_species = d.species.to_string();
        self.level = d.level;
        self.gender = d.gender;
        self.shiny = d.shiny;
    }

    fn identified(&self) -> bool {
        !self.details_species.is_empty()
    }

    /// Identity plus a fresh fold, without copying the observation log.
    fn identity(&self) -> Self {
        let mut m = Self::new(&self.name);
        m.species = self.species;
        m.details_species = self.details_species.clone();
        m.current_species = self.details_species.clone();
        m.level = self.level;
        m.gender = self.gender;
        m.shiny = self.shiny;
        m
    }

    /// Rebuild every observed field from the log (identity is kept).
    fn refold(&mut self) {
        let log = std::mem::take(&mut self.log);
        let mut fresh = MonInfo::new(&self.name);
        fresh.species = self.species;
        fresh.details_species = std::mem::take(&mut self.details_species);
        fresh.current_species = fresh.details_species.clone();
        fresh.level = self.level;
        fresh.gender = self.gender;
        fresh.shiny = self.shiny;
        *self = fresh;
        for &(_, o) in &log {
            self.apply(o);
        }
        self.log = log;
    }

    fn set_species_text(&mut self, species: EffectId) {
        let text = if species == self.species || species == EffectId::NONE {
            self.details_species.as_str()
        } else {
            dex::species(species).name
        };
        if self.current_species != text {
            self.current_species.clear();
            self.current_species.push_str(text);
        }
    }

    fn set_status(&mut self, status: &str) {
        if self.status != status {
            self.status.clear();
            self.status.push_str(status);
        }
    }

    fn apply(&mut self, o: Obs) {
        match o {
            Obs::SwitchIn {
                pos,
                species,
                condition,
                tera,
            } => {
                self.appearances += 1;
                self.active = Some(pos);
                self.transformed = false;
                self.set_species_text(species);
                if let Some((hp, max_hp, status)) = condition {
                    self.apply(Obs::Hp { hp, max_hp, status });
                }
                if let Some(t) = tera {
                    self.apply(Obs::Tera(t));
                }
            }
            Obs::SwitchOut { pos } => {
                if self.active == Some(pos) {
                    self.active = None;
                }
                self.transformed = false;
                if self.ability_temporary {
                    self.ability = self.starting_ability;
                    self.ability_temporary = false;
                }
            }
            Obs::Swap { from, to } => {
                if self.active == Some(from) {
                    self.active = Some(to);
                }
            }
            Obs::Hp { hp, max_hp, status } => {
                self.hp = hp;
                self.max_hp = if status == "fnt" { self.max_hp } else { max_hp };
                self.fainted = status == "fnt";
                self.set_status(status);
            }
            Obs::Status(s) => self.set_status(s),
            Obs::CureStatus => {
                if !self.fainted {
                    self.set_status("");
                }
            }
            Obs::Faint { pos } => {
                self.hp = 0;
                self.fainted = true;
                self.set_status("fnt");
                if pos.is_some() && self.active == pos {
                    self.active = None;
                }
            }
            Obs::Move(m) => {
                if !self.transformed
                    && m != dex::MOVE_STRUGGLE
                    && !self.copied.contains(&m)
                    && !self.moves.contains(&m)
                {
                    self.moves.push(m);
                }
            }
            Obs::Copied(m) => {
                if !self.copied.contains(&m) {
                    self.copied.push(m);
                }
            }
            Obs::ItemSeen(i) => {
                self.reveal_item(i);
                if self.item != ItemKnowledge::Lost {
                    self.item = ItemKnowledge::Held(i);
                }
            }
            Obs::ItemLost(i) => {
                self.reveal_item(i);
                self.item = ItemKnowledge::Lost;
            }
            Obs::ItemBefore(i) => self.reveal_item(i),
            Obs::Transfer => self.item_transferred = true,
            Obs::ItemNow(k) => self.item = k,
            Obs::Ability(a) => {
                self.ability = Some(a);
                if !self.ability_changed && self.starting_ability.is_none() {
                    self.starting_ability = Some(a);
                }
            }
            Obs::AbilityChange(a) => {
                self.ability_changed = true;
                self.ability_temporary = true;
                self.ability = a;
            }
            Obs::Forme {
                species,
                abilities_differ,
                permanent,
            } => {
                self.set_species_text(species);
                if permanent && abilities_differ {
                    // The new forme's ability stays after switching out (Terapagos-Terastal).
                    self.ability_changed = true;
                    self.ability_temporary = false;
                    self.ability = None;
                } else if species == self.species {
                    if self.ability_temporary && !self.transformed {
                        self.ability = self.starting_ability;
                        self.ability_temporary = false;
                    }
                } else if abilities_differ {
                    self.apply(Obs::AbilityChange(None));
                }
            }
            Obs::Tera(t) => {
                self.tera_type = Some(t);
                self.terastallized = true;
            }
            Obs::Transform(a) => {
                self.transformed = true;
                self.apply(Obs::AbilityChange(a));
            }
            Obs::Illusion => self.illusion = true,
        }
    }

    fn reveal_item(&mut self, i: EffectId) {
        if !self.item_transferred && self.starting_item.is_none() {
            self.starting_item = Some(i);
        }
    }
}

/// A resolved Pokemon reference: side, index into that side's records, and the stint the
/// reference was made in (0 when not tied to an active position).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ref {
    side: SideId,
    idx: usize,
    stint: u32,
}

#[derive(Clone, Debug, Default)]
struct SideState {
    mons: Vec<MonInfo>,
    /// Mon index and stint id per position.
    at: [Option<(usize, u32)>; 3],
    leads: [Option<(usize, u32)>; 2],
    team_size: Option<u8>,
    tera_used: bool,
    entries: Vec<Stint>,
    party_events: Vec<PartyEvent>,
}

/// One public entry, indexed by its compared protocol line. `owner` changes on `replace`, while
/// `shown` keeps the disguise's identity (including when its only record was removed).
#[derive(Clone, Debug)]
pub struct Stint {
    pub id: u32,
    pub line: usize,
    pub last: usize,
    pub faint_line: Option<usize>,
    pub position: u8,
    pub lead: bool,
    pub shown: MonInfo,
    pub owner: String,
    /// A replace or an original non-Illusion ability rules out an unexplained disguise.
    pub certain: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum PartyEvent {
    Enter(u32),
    Faint(u32),
    Swap(u8, u8),
    Revive(String),
    Tera(u32),
    Transform(u32),
}

/// An item swap (Trick / Switcheroo) between its `-activate` line and its `-item` / `-enditem`
/// lines: each party's item before the swap is revealed by the other's `-item` line (or its own
/// `-enditem`), then the party is marked transferred.
#[derive(Clone, Copy, Debug)]
struct PendingSwap {
    parties: [Ref; 2],
    done: [bool; 2],
}

/// Public per-Pokemon information from one player's protocol stream. See the module docs.
#[derive(Clone, Debug, Default)]
pub struct RevealTracker {
    sides: [SideState; 2],
    turn: u32,
    stints: u32,
    compared: usize,
    swap: Option<PendingSwap>,
    /// An item that left its holder for another Pokemon to use (Bug Bite / Pluck, Fling): its
    /// effect tags on others until the next action name the item, not their own.
    foreign_item: Option<(Ref, EffectId)>,
    /// The last line was `-ability|X|As One`: the `Unnerve` line that follows is part of it.
    as_one: Option<Ref>,
    /// `-activate|X|ability: Dancer` was shown: X's next `move` / `cant` line is the copied
    /// dance. The `move` line carries `[from] ability: Dancer`, except when a choice lock stops it
    /// (`move|X|MOVE||[still]` + `-fail|X`, effects/conditions/choicelock.rs); a `cant` (Taunt,
    /// ...) names the copied move.
    dancer: Option<Ref>,
}

const STATUSES: &[&str] = &["brn", "par", "slp", "frz", "psn", "tox", "fnt"];

fn status_str(s: &str) -> &'static str {
    STATUSES.iter().copied().find(|&k| k == s).unwrap_or("")
}

fn condition(text: &str) -> Option<(u32, u32, &'static str)> {
    let c = parse_condition(text)?;
    Some((c.hp, c.max_hp, status_str(c.status)))
}

/// A positional argument: `None` when absent, empty or a `[tag]`.
fn plain(text: &str) -> Option<&str> {
    (!text.is_empty() && !text.starts_with('[')).then_some(text)
}

/// The species of a details text. Showdown prints Greninja-Bond and Rockruff-Dusk under their base
/// species name (pokemon.ts getUpdatedDetails), so those names fall back to the forme when the
/// base species is out of scope.
fn species_id(name: &str) -> EffectId {
    dex::lookup(EffectKind::Species, name)
        .or_else(|| match name {
            "Greninja" => dex::lookup(EffectKind::Species, "Greninja-Bond"),
            "Rockruff" => dex::lookup(EffectKind::Species, "Rockruff-Dusk"),
            _ => None,
        })
        .unwrap_or(EffectId::NONE)
}

fn item_id(name: &str) -> Option<EffectId> {
    dex::lookup(EffectKind::Item, name)
}

fn move_id(name: &str) -> Option<EffectId> {
    dex::lookup(EffectKind::Move, name)
}

fn abilities_differ(a: EffectId, b: EffectId) -> bool {
    if a == EffectId::NONE || b == EffectId::NONE {
        return true;
    }
    let (sa, sb) = (&dex::species(a).abilities, &dex::species(b).abilities);
    let has = |set: &[dex::AbilitySlot; 4], id: EffectId| set.iter().any(|s| s.id == id);
    sa.iter()
        .filter(|s| s.id != EffectId::NONE)
        .any(|s| !has(sb, s.id))
        || sb
            .iter()
            .filter(|s| s.id != EffectId::NONE)
            .any(|s| !has(sa, s.id))
}

const ITEM_TAKERS: &[&str] = &["move: Thief", "move: Covet", "ability: Magician", "ability: Pickpocket"];
const ITEM_SWAPS: &[&str] = &["move: Trick", "move: Switcheroo"];

impl RevealTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed_all<'a>(&mut self, lines: impl IntoIterator<Item = &'a str>) {
        for line in lines {
            self.feed(line);
        }
    }

    /// Records in first-appearance order.
    pub fn mons(&self, side: SideId) -> &[MonInfo] {
        &self.sides[side.0 as usize & 1].mons
    }

    pub fn find(&self, side: SideId, name: &str) -> Option<usize> {
        self.mons(side).iter().position(|m| m.name == name)
    }

    pub fn active(&self, side: SideId, position: u8) -> Option<usize> {
        let s = &self.sides[side.0 as usize & 1];
        s.at.get(position as usize)
            .copied()
            .flatten()
            .map(|(i, _)| i)
    }

    pub(crate) fn active_stint(&self, side: SideId, position: u8) -> Option<u32> {
        self.sides[side.0 as usize].at.get(position as usize).copied().flatten().map(|(_, st)| st)
    }

    /// The Pokemon sent out before `|turn|1` (the real Pokemon when a lead's Illusion broke).
    pub fn leads(&self, side: SideId) -> [Option<usize>; 2] {
        self.sides[side.0 as usize & 1]
            .leads
            .map(|l| l.map(|(i, _)| i))
    }

    pub fn team_size(&self, side: SideId) -> Option<u8> {
        self.sides[side.0 as usize & 1].team_size
    }

    pub fn turn(&self) -> u32 {
        self.turn
    }

    pub fn tera_used(&self, side: SideId) -> bool {
        self.sides[side.0 as usize & 1].tera_used
    }

    pub fn stints(&self, side: SideId) -> &[Stint] {
        &self.sides[side.0 as usize].entries
    }

    pub fn entry(&self, side: SideId, line: usize) -> Option<&Stint> {
        let entries = self.stints(side);
        entries.binary_search_by_key(&line, |s| s.line).ok().map(|i| &entries[i])
    }

    pub(crate) fn party_events(&self, side: SideId) -> &[PartyEvent] {
        &self.sides[side.0 as usize].party_events
    }

    /// Per-world observation view. Only the six small folds are built; neither the tracker nor
    /// its logs are cloned. Disguise details constrain the disguise, never the user's true level.
    pub(crate) fn illusion_view(&self, side: SideId, moved: &[u32], user: &str, species: &str) -> Vec<MonInfo> {
        let mut out: Vec<_> = self
            .mons(side)
            .iter()
            .map(|m| {
                let mut fresh = m.identity();
                for &(st, obs) in &m.log {
                    if !moved.contains(&st) {
                        fresh.apply(obs);
                    }
                }
                fresh
            })
            .collect();
        let index = out.iter().position(|m| m.name == user).unwrap_or_else(|| {
            let mut m = MonInfo::new(user);
            m.identify(species);
            // The pool supplies the level, gender and shiny flag of a hidden user.
            m.level = 0;
            out.push(m);
            out.len() - 1
        });
        let mut log: Vec<_> = self
            .mons(side)
            .iter()
            .flat_map(|m| m.log.iter().copied().filter(move |(st, _)| m.name == user || moved.contains(st)))
            .collect();
        log.sort_by_key(|(st, _)| *st);
        out[index] = out[index].identity();
        for (_, mut obs) in log {
            if let Obs::SwitchIn { species, .. } = &mut obs {
                *species = out[index].species;
            }
            out[index].apply(obs);
        }
        out[index].apply(Obs::Ability(dex::ABILITY_ILLUSION));
        out
    }

    /// One player-view line. Lines that are not protocol lines, unknown commands and malformed
    /// arguments are ignored.
    pub fn feed(&mut self, line: &str) {
        let at = self.compared;
        self.compared += usize::from(super::protocol::is_compared(line));
        let Some(l) = Line::parse(line) else { return };
        let cmd = l.cmd();
        if self.swap.is_some()
            && !(matches!(cmd, "-item" | "-enditem") && l.tag("from").is_some_and(|f| ITEM_SWAPS.contains(&f)))
        {
            self.finish_swap();
        }
        if matches!(cmd, "" | "move" | "cant" | "switch" | "drag" | "faint" | "turn" | "upkeep") {
            self.foreign_item = None;
        }
        let as_one = self.as_one.take();
        if !matches!(cmd, "-item" | "-enditem" | "-ability") {
            self.attribute_tags(&l);
        }
        match cmd {
            "turn" => {
                if let Ok(t) = l.arg(0).parse() {
                    self.turn = t;
                }
            }
            "teamsize" => {
                if let (Some(side), Ok(n)) = (parse_side(l.arg(0)), l.arg(1).parse()) {
                    self.sides[side.0 as usize].team_size = Some(n);
                }
            }
            "switch" | "drag" => self.switch_in(&l, at),
            "replace" => self.replace(&l),
            "detailschange" | "-formechange" => self.forme_change(&l),
            "swap" => self.swap_positions(&l),
            "move" => self.on_move(&l),
            "cant" => self.on_cant(&l),
            "-damage" | "-heal" | "-sethp" => {
                if let (Some(r), Some((hp, max_hp, status))) = (self.resolve(l.arg(0)), condition(l.arg(1))) {
                    self.observe(r, Obs::Hp { hp, max_hp, status });
                    if l.cmd() == "-heal" && l.tag("from") == Some("move: Revival Blessing") {
                        let name = self.mon(r).name.clone();
                        self.sides[r.side.0 as usize].party_events.push(PartyEvent::Revive(name));
                    }
                }
            }
            "-status" => {
                let status = status_str(l.arg(1));
                if let Some(r) = self.resolve(l.arg(0))
                    && !status.is_empty()
                {
                    self.observe(r, Obs::Status(status));
                }
            }
            "-curestatus" => {
                if let Some(r) = self.resolve(l.arg(0)) {
                    self.observe(r, Obs::CureStatus);
                }
            }
            "-cureteam" => {
                if let Some(side) = parse_side(l.arg(0)) {
                    for idx in 0..self.sides[side.0 as usize].mons.len() {
                        self.observe(Ref { side, idx, stint: 0 }, Obs::CureStatus);
                    }
                }
            }
            "faint" => {
                if let Some(id) = parse_ident(l.arg(0))
                    && let Some(r) = self.resolve(l.arg(0))
                {
                    self.observe(r, Obs::Faint { pos: id.position });
                    if let Some(e) = self.sides[r.side.0 as usize].entries.iter_mut().rev().find(|e| e.id == r.stint) {
                        e.faint_line = Some(at);
                    }
                    self.sides[r.side.0 as usize].party_events.push(PartyEvent::Faint(r.stint));
                    if let Some(p) = id.position
                        && let Some(slot) = self.sides[r.side.0 as usize].at.get_mut(p as usize)
                        && slot.is_some_and(|(i, _)| i == r.idx)
                    {
                        *slot = None;
                    }
                }
            }
            "-terastallize" => {
                if let (Some(r), Some(t)) = (self.resolve(l.arg(0)), dex::type_id(l.arg(1))) {
                    self.sides[r.side.0 as usize].tera_used = true;
                    self.observe(r, Obs::Tera(t));
                    self.sides[r.side.0 as usize].party_events.push(PartyEvent::Tera(r.stint));
                }
            }
            "-transform" => {
                if let Some(r) = self.resolve(l.arg(0)) {
                    let copied = self.resolve(l.arg(1)).and_then(|t| self.mon(t).ability);
                    self.observe(r, Obs::Transform(copied));
                    self.sides[r.side.0 as usize].party_events.push(PartyEvent::Transform(r.stint));
                }
            }
            "-item" => self.on_item(&l),
            "-enditem" => self.on_enditem(&l),
            "-ability" => self.on_ability(&l, as_one),
            "-activate" => self.on_activate(&l),
            "-start" => {
                if matches!(l.arg(1), "Mimic" | "move: Mimic")
                    && let (Some(r), Some(m)) = (self.resolve(l.arg(0)), move_id(l.arg(2)))
                {
                    self.observe(r, Obs::Copied(m));
                }
                self.arg_effect(&l);
            }
            "-block" | "-end" | "-immune" | "-fail" => self.arg_effect(&l),
            _ => {}
        }
    }

    fn mon(&self, r: Ref) -> &MonInfo {
        &self.sides[r.side.0 as usize].mons[r.idx]
    }

    /// The record named by `p2a: Name` / `p2: Name`, created on first reference.
    fn resolve(&mut self, text: &str) -> Option<Ref> {
        let id = parse_ident(text)?;
        if id.name.is_empty() {
            return None;
        }
        let side = &mut self.sides[id.side.0 as usize];
        let idx = match side.mons.iter().position(|m| m.name == id.name) {
            Some(i) => i,
            None => {
                side.mons.push(MonInfo::new(id.name));
                side.mons.len() - 1
            }
        };
        let stint = id
            .position
            .and_then(|p| side.at.get(p as usize).copied().flatten())
            .filter(|&(i, _)| i == idx)
            .map_or(0, |(_, s)| s);
        Some(Ref {
            side: id.side,
            idx,
            stint,
        })
    }

    fn observe(&mut self, r: Ref, o: Obs) {
        if let Some(entry) = self.sides[r.side.0 as usize].entries.iter_mut().rev().find(|e| e.id == r.stint) {
            entry.last = self.compared;
        }
        if matches!(o, Obs::Ability(a) if a != dex::ABILITY_ILLUSION)
            && !self.mon(r).ability_changed
            && let Some(entry) = self.sides[r.side.0 as usize].entries.iter_mut().find(|e| e.id == r.stint)
        {
            entry.certain = true;
        }
        let m = &mut self.sides[r.side.0 as usize].mons[r.idx];
        m.log.push((r.stint, o));
        m.apply(o);
    }

    /// Ability `name` shown for `r`; Calyrex's As One shows as `As One` plus its components.
    fn ability_id(&self, r: Ref, name: &str) -> Option<EffectId> {
        let species = self.mon(r).species;
        if species != EffectId::NONE && matches!(name, "As One" | "Unnerve" | "Chilling Neigh" | "Grim Neigh") {
            let as_one = dex::species(species)
                .abilities
                .iter()
                .find(|s| s.id == dex::ABILITY_ASONEGLASTRIER || s.id == dex::ABILITY_ASONESPECTRIER);
            if let Some(slot) = as_one {
                return Some(slot.id);
            }
        }
        dex::lookup(EffectKind::Ability, name)
    }

    fn reveal_ability(&mut self, r: Ref, name: &str) {
        if let Some(a) = self.ability_id(r, name) {
            self.observe(r, Obs::Ability(a));
        }
    }

    fn reveal_item(&mut self, r: Ref, name: &str) {
        if let Some(i) = item_id(name) {
            if self.foreign_item.is_some_and(|(holder, item)| item == i && (holder.side, holder.idx) != (r.side, r.idx))
            {
                return;
            }
            self.observe(r, Obs::ItemSeen(i));
        }
    }

    /// `[from] item: X` / `[from] ability: X` on lines without their own item / ability handling.
    fn attribute_tags(&mut self, l: &Line) {
        let Some(from) = l.tag("from") else { return };
        let (kind, name) = effect_kind(from);
        let of = l.tag("of").filter(|o| !o.is_empty());
        let owner = match (kind, l.cmd()) {
            ("ability", "-heal") if name != "Hospitality" => l.arg(0),
            ("ability" | "item", _) => of.unwrap_or(l.arg(0)),
            _ => return,
        };
        let Some(r) = self.resolve(owner) else { return };
        if kind == "item" {
            self.reveal_item(r, name);
        } else {
            self.reveal_ability(r, name);
        }
    }

    /// `ability: X` / `item: X` as the second argument: the subject holds it on `-activate`;
    /// elsewhere (`-block|TARGET|ability: Aroma Veil|[of] HOLDER`, `-start`, `-end`, ...) the
    /// holder is `[of]` when present, else the subject.
    fn arg_effect(&mut self, l: &Line) {
        let (kind, name) = effect_kind(l.arg(1));
        if kind != "ability" && kind != "item" {
            return;
        }
        let owner = if l.cmd() == "-activate" {
            l.arg(0)
        } else {
            l.tag("of").filter(|o| !o.is_empty()).unwrap_or(l.arg(0))
        };
        let Some(r) = self.resolve(owner) else { return };
        if kind == "ability" {
            self.reveal_ability(r, name);
        } else {
            self.reveal_item(r, name);
        }
    }

    fn switch_in(&mut self, l: &Line, line: usize) {
        let Some(id) = parse_ident(l.arg(0)) else {
            return;
        };
        let Some(pos) = id.position else { return };
        let Some(r) = self.resolve(l.arg(0)) else {
            return;
        };
        let details = parse_details(l.arg(1));
        let s = r.side.0 as usize;
        if !self.sides[s].mons[r.idx].identified() {
            self.sides[s].mons[r.idx].identify(l.arg(1));
        }
        if let Some((prev, stint)) = self.sides[s].at[pos as usize] {
            self.observe(Ref { side: r.side, idx: prev, stint }, Obs::SwitchOut { pos });
        }
        self.stints += 1;
        let stint = self.stints;
        let mut shown = MonInfo::new(id.name);
        shown.identify(l.arg(1));
        self.sides[s].entries.push(Stint {
            id: stint,
            line,
            last: line,
            faint_line: None,
            position: pos,
            lead: self.turn == 0 && pos < 2 && self.sides[s].leads[pos as usize].is_none(),
            shown,
            owner: id.name.to_owned(),
            certain: false,
        });
        self.sides[s].party_events.push(PartyEvent::Enter(stint));
        self.sides[s].at[pos as usize] = Some((r.idx, stint));
        if self.turn == 0 && (pos as usize) < 2 && self.sides[s].leads[pos as usize].is_none() {
            self.sides[s].leads[pos as usize] = Some((r.idx, stint));
        }
        let tera = details.tera.and_then(dex::type_id);
        if tera.is_some() {
            self.sides[s].tera_used = true;
        }
        let species = species_id(details.species);
        let condition = condition(l.arg(2));
        self.observe(Ref { stint, ..r }, Obs::SwitchIn { pos, species, condition, tera });
    }

    /// `|replace|POS: REAL|DETAILS`: the Pokemon at POS since its switch-in was REAL in disguise.
    fn replace(&mut self, l: &Line) {
        let Some(id) = parse_ident(l.arg(0)) else {
            return;
        };
        let Some(pos) = id.position.filter(|&p| p < 3) else {
            return;
        };
        let s = id.side.0 as usize;
        let current = self.sides[s].at[pos as usize];
        let real = match self.sides[s].mons.iter().position(|m| m.name == id.name) {
            Some(i) => i,
            None => {
                self.sides[s].mons.push(MonInfo::new(id.name));
                self.sides[s].mons.len() - 1
            }
        };
        if !self.sides[s].mons[real].identified() {
            self.sides[s].mons[real].identify(l.arg(1));
        }
        let real_species = self.sides[s].mons[real].species;
        let (moved, stint) = match current {
            Some((disguise, stint)) if disguise != real => {
                let log = std::mem::take(&mut self.sides[s].mons[disguise].log);
                let (moved, kept): (Vec<_>, Vec<_>) = log.into_iter().partition(|&(st, _)| st == stint);
                self.sides[s].mons[disguise].log = kept;
                self.sides[s].mons[disguise].refold();
                (moved, stint)
            }
            Some((_, stint)) => (Vec::new(), stint),
            None => {
                self.stints += 1;
                let condition = None;
                let tera = None;
                (vec![(self.stints, Obs::SwitchIn { pos, species: real_species, condition, tera })], self.stints)
            }
        };
        {
            let m = &mut self.sides[s].mons[real];
            m.log.push((stint, Obs::Ability(dex::ABILITY_ILLUSION)));
            m.log.push((stint, Obs::Illusion));
            for (st, mut o) in moved {
                if let Obs::SwitchIn { species, .. } = &mut o {
                    *species = real_species;
                }
                m.log.push((st, o));
            }
            m.refold();
        }
        self.sides[s].at[pos as usize] = Some((real, stint));
        if let Some(entry) = self.sides[s].entries.iter_mut().find(|e| e.id == stint) {
            entry.owner = id.name.to_owned();
            entry.certain = true;
        }
        for lead in &mut self.sides[s].leads {
            if lead.is_some_and(|(_, st)| st == stint) {
                *lead = Some((real, stint));
            }
        }
        if let Some((disguise, _)) = current
            && disguise != real
            && self.sides[s].mons[disguise].appearances == 0
        {
            self.remove(id.side, disguise);
        }
    }

    fn remove(&mut self, side: SideId, idx: usize) {
        let s = &mut self.sides[side.0 as usize];
        s.mons.remove(idx);
        let fix = |slot: &mut Option<(usize, u32)>| match *slot {
            Some((i, _)) if i == idx => *slot = None,
            Some((i, st)) if i > idx => *slot = Some((i - 1, st)),
            _ => {}
        };
        s.at.iter_mut().for_each(fix);
        s.leads.iter_mut().for_each(fix);
        self.swap = None;
        self.foreign_item = None;
    }

    fn forme_change(&mut self, l: &Line) {
        let Some(r) = self.resolve(l.arg(0)) else {
            return;
        };
        let d = parse_details(l.arg(1));
        let species = species_id(d.species);
        if species == EffectId::NONE {
            return;
        }
        let original = self.mon(r).species;
        let abilities_differ = abilities_differ(original, species);
        self.observe(
            r,
            Obs::Forme {
                species,
                abilities_differ,
                permanent: l.cmd() == "detailschange",
            },
        );
        if let Some(t) = d.tera.and_then(dex::type_id) {
            self.sides[r.side.0 as usize].tera_used = true;
            self.observe(r, Obs::Tera(t));
        }
    }

    /// `|swap|p1a: X|1`: X moves to position 1 and the Pokemon there takes X's position.
    fn swap_positions(&mut self, l: &Line) {
        let Some(id) = parse_ident(l.arg(0)) else {
            return;
        };
        let (Some(from), Ok(to)) = (id.position, l.arg(1).parse::<u8>()) else {
            return;
        };
        if from == to || to > 2 || from > 2 {
            return;
        }
        let s = id.side.0 as usize;
        let (a, b) = (self.sides[s].at[from as usize], self.sides[s].at[to as usize]);
        self.sides[s].party_events.push(PartyEvent::Swap(from, to));
        self.sides[s].at[from as usize] = b;
        self.sides[s].at[to as usize] = a;
        if let Some((idx, stint)) = a {
            self.observe(Ref { side: id.side, idx, stint }, Obs::Swap { from, to });
        }
        if let Some((idx, stint)) = b {
            self.observe(Ref { side: id.side, idx, stint }, Obs::Swap { from: to, to: from });
        }
    }

    fn on_move(&mut self, l: &Line) {
        let dancer = self.dancer.take();
        if !matches!(l.tag("from"), None | Some("lockedmove")) {
            return;
        }
        if let (Some(r), Some(m)) = (self.resolve(l.arg(0)), move_id(l.arg(1)))
            && !dancer.is_some_and(|d| (d.side, d.idx) == (r.side, r.idx))
        {
            self.observe(r, Obs::Move(m));
        }
    }

    /// `cant|X|REASON|MOVE`: X chose MOVE (`nopp`, `Disable`, `move: Taunt`, `move: Imprison`,
    /// `move: Heal Block`, `move: Throat Chop`, ...). With an ability reason
    /// (`cant|HOLDER|ability: Queenly Majesty|MOVE|[of] TARGET`) the subject is the holder and
    /// the user is not named (its `move` line was already shown).
    fn on_cant(&mut self, l: &Line) {
        let dancer = self.dancer.take();
        if let ("ability", name) = effect_kind(l.arg(1)) {
            if let Some(r) = self.resolve(l.arg(0)) {
                self.reveal_ability(r, name);
            }
            return;
        }
        if let Some(name) = plain(l.arg(2))
            && let (Some(r), Some(m)) = (self.resolve(l.arg(0)), move_id(name))
            && !dancer.is_some_and(|d| (d.side, d.idx) == (r.side, r.idx))
        {
            self.observe(r, Obs::Move(m));
        }
    }

    fn finish_swap(&mut self) {
        if let Some(p) = self.swap.take() {
            for (party, done) in p.parties.into_iter().zip(p.done) {
                if !done {
                    self.observe(party, Obs::Transfer);
                }
            }
        }
    }

    /// Index (in the pending swap) of the party that is not `r`.
    fn swap_partner(&self, r: Ref) -> Option<usize> {
        let p = self.swap?;
        p.parties
            .iter()
            .position(|q| (q.side, q.idx) != (r.side, r.idx))
    }

    /// Swap party `party` held `item` before the swap: revealed once, then marked transferred.
    fn swap_reveal(&mut self, party: usize, item: EffectId) {
        let Some(p) = self.swap.as_mut() else { return };
        let r = p.parties[party];
        let first = !p.done[party];
        p.done[party] = true;
        if first {
            self.observe(r, Obs::ItemBefore(item));
            self.observe(r, Obs::Transfer);
        }
    }

    fn on_item(&mut self, l: &Line) {
        let Some(r) = self.resolve(l.arg(0)) else {
            return;
        };
        let Some(item) = item_id(l.arg(1)) else {
            return;
        };
        let from = l.tag("from");
        let of = l.tag("of").and_then(|o| self.resolve(o));
        match from {
            None => self.observe(r, Obs::ItemSeen(item)),
            Some("ability: Frisk") => {
                self.observe(r, Obs::ItemSeen(item));
                if let Some(f) = of {
                    self.reveal_ability(f, "Frisk");
                }
            }
            Some(f @ ("ability: Harvest" | "ability: Cud Chew" | "move: Recycle")) => {
                if let ("ability", name) = effect_kind(f) {
                    self.reveal_ability(r, name);
                }
                self.observe(r, Obs::ItemSeen(item));
                self.observe(r, Obs::ItemNow(ItemKnowledge::Held(item)));
            }
            Some(f) if ITEM_SWAPS.contains(&f) => {
                // The item came from the partner; `r`'s own pre-swap item shows on the partner's
                // line, so `r` is marked transferred then (or when the swap ends).
                match self.swap_partner(r) {
                    Some(partner) => self.swap_reveal(partner, item),
                    None => self.observe(r, Obs::Transfer),
                }
                self.observe(r, Obs::ItemNow(ItemKnowledge::Held(item)));
            }
            Some(f) => {
                // Thief / Covet / Magician / Pickpocket name the victim in `[of]`; Bestow names
                // the giver there. Either way `[of]` held the item until now.
                let giver = if ITEM_TAKERS.contains(&f) || f == "move: Bestow" {
                    of
                } else {
                    None
                };
                if let Some(g) = giver {
                    self.observe(g, Obs::ItemBefore(item));
                    self.observe(g, Obs::Transfer);
                    self.observe(g, Obs::ItemNow(ItemKnowledge::Lost));
                }
                if let ("ability", name) = effect_kind(f) {
                    self.reveal_ability(r, name);
                }
                self.observe(r, Obs::Transfer);
                self.observe(r, Obs::ItemNow(ItemKnowledge::Held(item)));
            }
        }
    }

    fn on_enditem(&mut self, l: &Line) {
        let Some(r) = self.resolve(l.arg(0)) else {
            return;
        };
        let Some(item) = item_id(l.arg(1)) else {
            return;
        };
        match l.tag("from") {
            Some(f) if ITEM_SWAPS.contains(&f) => {
                let own = self.swap.and_then(|p| p.parties.iter().position(|q| (q.side, q.idx) == (r.side, r.idx)));
                match own {
                    Some(i) => self.swap_reveal(i, item),
                    None => {
                        self.observe(r, Obs::ItemBefore(item));
                        self.observe(r, Obs::Transfer);
                    }
                }
                self.observe(r, Obs::ItemNow(ItemKnowledge::Lost));
            }
            Some(f) if ITEM_TAKERS.contains(&f) => {
                self.observe(r, Obs::ItemBefore(item));
                self.observe(r, Obs::Transfer);
                self.observe(r, Obs::ItemNow(ItemKnowledge::Lost));
            }
            Some("stealeat" | "move: Fling") => {
                self.observe(r, Obs::ItemLost(item));
                self.foreign_item = Some((r, item));
            }
            _ => self.observe(r, Obs::ItemLost(item)),
        }
    }

    fn on_ability(&mut self, l: &Line, as_one: Option<Ref>) {
        let Some(r) = self.resolve(l.arg(0)) else {
            return;
        };
        let name = l.arg(1);
        let from = l.tag("from");
        // `-ability|X|NEW|OLD|[from] EFFECT|[of] SOURCE`: setAbility (OLD may be empty).
        let change = from.is_some() && l.fields.len() > 3 && !l.fields[3].starts_with('[');
        if !change {
            if name == "Unnerve" && as_one.is_some_and(|a| (a.side, a.idx) == (r.side, r.idx)) {
                return;
            }
            if name == "As One" {
                self.as_one = Some(r);
            }
            if plain(name).is_some() {
                self.reveal_ability(r, name);
            }
            return;
        }
        if let Some(old) = plain(l.arg(2)) {
            self.reveal_ability(r, old);
        }
        let new = plain(name).and_then(|n| self.ability_id(r, n));
        if let (Some(n), Some(source)) = (plain(name), l.tag("of").and_then(|o| self.resolve(o)))
            && matches!(
                from,
                Some(
                    "ability: Trace"
                        | "ability: Receiver"
                        | "ability: Power of Alchemy"
                        | "move: Role Play"
                        | "move: Doodle"
                        | "move: Entrainment"
                )
            )
            && (source.side, source.idx) != (r.side, r.idx)
        {
            self.reveal_ability(source, n);
        }
        self.observe(r, Obs::AbilityChange(new));
    }

    fn on_activate(&mut self, l: &Line) {
        let Some(r) = self.resolve(l.arg(0)) else {
            return;
        };
        match l.arg(1) {
            "move: Trick" | "move: Switcheroo" => {
                self.finish_swap();
                if let Some(t) = l.tag("of").and_then(|o| self.resolve(o)) {
                    self.swap = Some(PendingSwap {
                        parties: [r, t],
                        done: [false; 2],
                    });
                }
            }
            "ability: Dancer" => {
                self.arg_effect(l);
                self.dancer = Some(r);
            }
            "move: Poltergeist" => {
                if let Some(item) = plain(l.arg(2)) {
                    self.reveal_item(r, item);
                }
            }
            "move: Mimic" | "move: Sketch" => {
                if let Some(m) = plain(l.arg(2)).and_then(move_id) {
                    self.observe(r, Obs::Copied(m));
                }
            }
            "move: Skill Swap" => {
                let Some(t) = l.tag("of").and_then(|o| self.resolve(o)) else {
                    return;
                };
                if let Some(a) = plain(l.arg(2)) {
                    self.reveal_ability(t, a);
                }
                if let Some(b) = plain(l.arg(3)) {
                    self.reveal_ability(r, b);
                }
                let (ra, ta) = (self.mon(r).ability, self.mon(t).ability);
                self.observe(r, Obs::AbilityChange(ta));
                self.observe(t, Obs::AbilityChange(ra));
            }
            "ability: Mummy" | "ability: Lingering Aroma" => {
                self.arg_effect(l);
                let new = effect_kind(l.arg(1)).1;
                if let Some(c) = self.resolve(l.arg(2)) {
                    if let Some(old) = l.tag("ability").filter(|o| !o.is_empty()) {
                        self.reveal_ability(c, old);
                    }
                    let new = self.ability_id(c, new);
                    self.observe(c, Obs::AbilityChange(new));
                }
            }
            "ability: Symbiosis" => {
                self.arg_effect(l);
                if let (Some(item), Some(to)) = (
                    plain(l.arg(2)).and_then(item_id),
                    l.tag("of").and_then(|o| self.resolve(o)),
                ) {
                    self.observe(r, Obs::ItemBefore(item));
                    self.observe(r, Obs::Transfer);
                    self.observe(r, Obs::ItemNow(ItemKnowledge::Lost));
                    self.observe(to, Obs::Transfer);
                    self.observe(to, Obs::ItemNow(ItemKnowledge::Held(item)));
                }
            }
            _ => self.arg_effect(l),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P1: SideId = SideId(0);
    const P2: SideId = SideId(1);

    fn track(lines: &str) -> RevealTracker {
        let mut t = RevealTracker::new();
        t.feed_all(lines.lines().map(str::trim));
        t
    }

    fn mon<'a>(t: &'a RevealTracker, side: SideId, name: &str) -> &'a MonInfo {
        &t.mons(side)[t.find(side, name).unwrap_or_else(|| panic!("no {name}"))]
    }

    fn mv(name: &str) -> EffectId {
        dex::lookup(EffectKind::Move, name).unwrap()
    }

    fn item(name: &str) -> EffectId {
        dex::lookup(EffectKind::Item, name).unwrap()
    }

    fn ab(name: &str) -> EffectId {
        dex::lookup(EffectKind::Ability, name).unwrap()
    }

    const START: &str = "|teamsize|p1|6
        |teamsize|p2|6
        |start
        |switch|p1a: Garchomp|Garchomp, L80, F|100/100
        |switch|p1b: Gardevoir|Gardevoir, L84, F|100/100
        |switch|p2a: Snorlax|Snorlax, L85, M|405/405
        |switch|p2b: Furret|Furret, L95, F|300/300";

    fn started(more: &str) -> RevealTracker {
        track(&format!("{START}\n|turn|1\n{more}"))
    }

    #[test]
    fn identity_positions_hp_and_status() {
        let t = started(
            "|move|p1a: Garchomp|Earthquake|p2a: Snorlax|[spread] p2a,p2b
            |-damage|p2a: Snorlax|202/405
            |-damage|p2b: Furret|0 fnt
            |faint|p2b: Furret
            |-status|p1b: Gardevoir|par
            garbage line
            |swap|p1a: Garchomp|1|[from] move: Ally Switch
            |turn|2",
        );
        assert_eq!(
            (t.team_size(P1), t.team_size(P2), t.turn()),
            (Some(6), Some(6), 2)
        );
        let g = mon(&t, P1, "Garchomp");
        assert_eq!(
            (g.species, g.level, g.gender, g.details_species.as_str()),
            (dex::SPECIES_GARCHOMP, 80, Gender::Female, "Garchomp")
        );
        assert_eq!(g.active, Some(1));
        assert_eq!(t.active(P1, 1), t.find(P1, "Garchomp"));
        assert_eq!(t.active(P1, 0), t.find(P1, "Gardevoir"));
        assert_eq!(
            t.leads(P1),
            [t.find(P1, "Garchomp"), t.find(P1, "Gardevoir")]
        );
        assert_eq!(mon(&t, P1, "Gardevoir").status, "par");
        let s = mon(&t, P2, "Snorlax");
        assert_eq!((s.hp, s.max_hp, s.fainted), (202, 405, false));
        let f = mon(&t, P2, "Furret");
        assert!(f.fainted && f.active.is_none() && f.status == "fnt");
        assert_eq!(t.active(P2, 1), None);
        let t = started("|-status|p2a: Snorlax|slp\n|-curestatus|p2a: Snorlax|slp|[msg]");
        assert_eq!(mon(&t, P2, "Snorlax").status, "");
        let t =
            started("|-status|p2a: Snorlax|brn\n|-status|p2b: Furret|psn\n|-cureteam|p2a: Snorlax");
        assert!(t.mons(P2).iter().all(|m| m.status.is_empty()));
    }

    #[test]
    fn moves_count_own_choices_only() {
        let t = started(
            "|move|p1a: Garchomp|Earthquake|p2a: Snorlax
            |move|p1a: Garchomp|Meteor Beam|p2a: Snorlax
            |move|p1a: Garchomp|Meteor Beam|p2a: Snorlax|[from] lockedmove
            |move|p2a: Snorlax|Struggle|p1a: Garchomp
            |move|p2b: Furret|Swords Dance|p2b: Furret|[from] ability: Dancer
            |cant|p2a: Snorlax|move: Taunt|Curse
            |cant|p2b: Furret|par
            |move|p2b: Furret|Earthquake|p1a: Garchomp",
        );
        assert_eq!(
            mon(&t, P1, "Garchomp").moves,
            [mv("Earthquake"), mv("Meteor Beam")]
        );
        assert_eq!(mon(&t, P2, "Snorlax").moves, [mv("Curse")]);
        assert_eq!(mon(&t, P2, "Furret").moves, [mv("Earthquake")]);
    }

    #[test]
    fn dancer_copies_do_not_count() {
        let t = started(
            "|move|p1b: Gardevoir|Quiver Dance|p1b: Gardevoir
            |-activate|p2b: Furret|ability: Dancer
            |move|p2b: Furret|Quiver Dance||[still]
            |-fail|p2b: Furret
            |move|p1b: Gardevoir|Quiver Dance|p1b: Gardevoir
            |-activate|p2b: Furret|ability: Dancer
            |cant|p2b: Furret|move: Taunt|Quiver Dance
            |move|p2b: Furret|Tidy Up|p2b: Furret",
        );
        let f = mon(&t, P2, "Furret");
        assert_eq!(f.moves, [mv("Tidy Up")]);
        assert_eq!(f.starting_ability, Some(ab("Dancer")));
    }

    #[test]
    fn ability_blocks_in_cant_belong_to_the_holder() {
        let t = started(
            "|move|p1a: Garchomp|Sucker Punch||[still]
            |cant|p2b: Furret|ability: Armor Tail|Sucker Punch|[of] p2a: Snorlax",
        );
        assert_eq!(
            mon(&t, P2, "Furret").starting_ability,
            Some(ab("Armor Tail"))
        );
        assert!(mon(&t, P2, "Furret").moves.is_empty());
        assert_eq!(mon(&t, P2, "Snorlax").starting_ability, None);
        assert_eq!(mon(&t, P1, "Garchomp").moves, [mv("Sucker Punch")]);
    }

    #[test]
    fn transformed_and_copied_moves_do_not_count() {
        let t = track(
            "|switch|p1a: Ditto|Ditto, L90|100/100
            |switch|p2a: Snorlax|Snorlax, L85, M|405/405
            |-transform|p1a: Ditto|p2a: Snorlax|[from] ability: Imposter
            |turn|1
            |move|p1a: Ditto|Curse|p1a: Ditto
            |move|p2a: Snorlax|Curse|p2a: Snorlax
            |-activate|p2a: Snorlax|move: Sketch|Earthquake
            |move|p2a: Snorlax|Earthquake|p1a: Ditto
            |-start|p2a: Snorlax|Mimic|Rock Slide
            |move|p2a: Snorlax|Rock Slide|p1a: Ditto",
        );
        let d = mon(&t, P1, "Ditto");
        assert!(d.moves.is_empty() && d.transformed && d.ability_changed);
        assert_eq!(d.starting_ability, Some(ab("Imposter")));
        assert_eq!(mon(&t, P2, "Snorlax").moves, [mv("Curse")]);
        let t = track(
            "|switch|p1a: Ditto|Ditto, L90|100/100
            |switch|p2a: Snorlax|Snorlax, L85, M|405/405
            |-transform|p1a: Ditto|p2a: Snorlax|[from] ability: Imposter
            |switch|p1a: Garchomp|Garchomp, L80, F|100/100",
        );
        let d = mon(&t, P1, "Ditto");
        assert!(!d.transformed && d.active.is_none());
        assert_eq!(d.ability, Some(ab("Imposter")));
    }

    #[test]
    fn item_tags_name_the_holder() {
        let t = started(
            "|-damage|p1a: Garchomp|90/100|[from] item: Life Orb
            |-damage|p1a: Garchomp|80/100|[from] item: Rocky Helmet|[of] p2a: Snorlax
            |-fail|p2b: Furret|unboost|[from] item: Clear Amulet|[of] p2b: Furret",
        );
        assert_eq!(
            mon(&t, P1, "Garchomp").starting_item,
            Some(item("Life Orb"))
        );
        assert_eq!(
            mon(&t, P1, "Garchomp").item,
            ItemKnowledge::Held(item("Life Orb"))
        );
        assert_eq!(
            mon(&t, P2, "Snorlax").starting_item,
            Some(item("Rocky Helmet"))
        );
        assert_eq!(
            mon(&t, P2, "Furret").starting_item,
            Some(item("Clear Amulet"))
        );
    }

    #[test]
    fn consumed_knocked_off_and_stolen_items() {
        let t = started(
            "|-enditem|p2a: Snorlax|Sitrus Berry|[eat]
            |-heal|p2a: Snorlax|300/405|[from] item: Sitrus Berry
            |-enditem|p1a: Garchomp|Choice Scarf|[from] move: Knock Off|[of] p2b: Furret
            |move|p2b: Furret|Bug Bite|p1b: Gardevoir
            |-enditem|p1b: Gardevoir|Sitrus Berry|[from] stealeat|[move] Bug Bite|[of] p2b: Furret
            |-heal|p2b: Furret|300/300|[from] item: Sitrus Berry
            |-activate|p2b: Furret|move: Poltergeist|Leftovers",
        );
        let s = mon(&t, P2, "Snorlax");
        assert_eq!(
            (s.starting_item, s.item),
            (Some(item("Sitrus Berry")), ItemKnowledge::Lost)
        );
        let g = mon(&t, P1, "Garchomp");
        assert_eq!(
            (g.starting_item, g.item),
            (Some(item("Choice Scarf")), ItemKnowledge::Lost)
        );
        assert_eq!(
            mon(&t, P1, "Gardevoir").starting_item,
            Some(item("Sitrus Berry"))
        );
        assert_eq!(mon(&t, P2, "Furret").starting_item, Some(item("Leftovers")));
        let t = started(
            "|-enditem|p2a: Snorlax|Sitrus Berry|[eat]
            |move|p2a: Snorlax|Recycle|p2a: Snorlax
            |-item|p2a: Snorlax|Sitrus Berry|[from] move: Recycle",
        );
        let s = mon(&t, P2, "Snorlax");
        assert_eq!(
            (s.starting_item, s.item),
            (
                Some(item("Sitrus Berry")),
                ItemKnowledge::Held(item("Sitrus Berry"))
            )
        );
        let t = started(
            "|-enditem|p2a: Snorlax|Sitrus Berry|[eat]
            |-item|p2a: Snorlax|Sitrus Berry|[from] ability: Harvest",
        );
        let s = mon(&t, P2, "Snorlax");
        assert_eq!(
            (s.starting_ability, s.item),
            (
                Some(ab("Harvest")),
                ItemKnowledge::Held(item("Sitrus Berry"))
            )
        );
    }

    #[test]
    fn frisk_reveals_the_target_item_and_the_frisker_ability() {
        let t = started("|-item|p1a: Garchomp|Choice Scarf|[from] ability: Frisk|[of] p2b: Furret");
        assert_eq!(
            mon(&t, P1, "Garchomp").starting_item,
            Some(item("Choice Scarf"))
        );
        assert_eq!(mon(&t, P2, "Furret").starting_ability, Some(ab("Frisk")));
        assert_eq!(mon(&t, P2, "Furret").starting_item, None);
    }

    #[test]
    fn trick_reveals_both_starting_items_then_freezes_them() {
        let t = started(
            "|move|p1b: Gardevoir|Trick|p2a: Snorlax
            |-activate|p1b: Gardevoir|move: Trick|[of] p2a: Snorlax
            |-item|p2a: Snorlax|Choice Specs|[from] move: Trick
            |-item|p1b: Gardevoir|Leftovers|[from] move: Trick
            |move|p2b: Furret|Knock Off|p2a: Snorlax
            |-enditem|p2a: Snorlax|Choice Specs|[from] move: Knock Off|[of] p2b: Furret
            |-heal|p1b: Gardevoir|100/100|[from] item: Leftovers",
        );
        let g = mon(&t, P1, "Gardevoir");
        assert_eq!(
            (g.starting_item, g.item_transferred),
            (Some(item("Choice Specs")), true)
        );
        assert_eq!(g.item, ItemKnowledge::Held(item("Leftovers")));
        let s = mon(&t, P2, "Snorlax");
        assert_eq!(
            (s.starting_item, s.item_transferred, s.item),
            (Some(item("Leftovers")), true, ItemKnowledge::Lost)
        );
        // Only the user held an item: the target's line is `-item`, the user's `-enditem`.
        let t = started(
            "|-activate|p1b: Gardevoir|move: Trick|[of] p2a: Snorlax
            |-item|p2a: Snorlax|Choice Scarf|[from] move: Trick
            |-enditem|p1b: Gardevoir|Choice Scarf|[silent]|[from] move: Trick
            |-heal|p2a: Snorlax|405/405|[from] item: Leftovers",
        );
        let g = mon(&t, P1, "Gardevoir");
        assert_eq!(
            (g.starting_item, g.item),
            (Some(item("Choice Scarf")), ItemKnowledge::Lost)
        );
        let s = mon(&t, P2, "Snorlax");
        assert_eq!((s.starting_item, s.item_transferred), (None, true));
    }

    #[test]
    fn magician_and_pickpocket_take_the_victim_item() {
        let t = track(
            "|switch|p1a: Hoopa|Hoopa, L85|100/100
            |switch|p2a: Weavile|Weavile, L80, M|100/100
            |switch|p2b: Skuntank|Skuntank, L88, F|100/100
            |turn|1
            |-item|p1a: Hoopa|Life Orb|[from] ability: Magician|[of] p2b: Skuntank
            |-enditem|p1a: Hoopa|Life Orb|[silent]|[from] ability: Pickpocket|[of] p1a: Hoopa
            |-item|p2a: Weavile|Life Orb|[from] ability: Pickpocket|[of] p1a: Hoopa",
        );
        let h = mon(&t, P1, "Hoopa");
        assert_eq!(
            (
                h.starting_ability,
                h.starting_item,
                h.item_transferred,
                h.item
            ),
            (Some(ab("Magician")), None, true, ItemKnowledge::Lost)
        );
        let s = mon(&t, P2, "Skuntank");
        assert_eq!(
            (s.starting_item, s.item, s.starting_ability),
            (Some(item("Life Orb")), ItemKnowledge::Lost, None)
        );
        let w = mon(&t, P2, "Weavile");
        assert_eq!(
            (w.starting_ability, w.starting_item, w.item),
            (
                Some(ab("Pickpocket")),
                None,
                ItemKnowledge::Held(item("Life Orb"))
            )
        );
    }

    #[test]
    fn ability_tags_follow_the_engine_conventions() {
        let t = started(
            "|-ability|p2a: Snorlax|Intimidate|boost
            |-damage|p2b: Furret|80/100|[from] ability: Rough Skin|[of] p1a: Garchomp
            |-heal|p1b: Gardevoir|100/100|[from] ability: Water Absorb|[of] p2b: Furret
            |-heal|p2a: Snorlax|405/405|[from] ability: Hospitality|[of] p2b: Furret",
        );
        assert_eq!(
            mon(&t, P2, "Snorlax").starting_ability,
            Some(ab("Intimidate"))
        );
        assert_eq!(
            mon(&t, P1, "Garchomp").starting_ability,
            Some(ab("Rough Skin"))
        );
        assert_eq!(
            mon(&t, P1, "Gardevoir").starting_ability,
            Some(ab("Water Absorb"))
        );
        assert_eq!(
            mon(&t, P2, "Furret").starting_ability,
            Some(ab("Hospitality"))
        );
    }

    #[test]
    fn trace_reveals_both_abilities() {
        let t = started(
            "|-ability|p1b: Gardevoir|Intimidate|Trace|[from] ability: Trace|[of] p2a: Snorlax",
        );
        let g = mon(&t, P1, "Gardevoir");
        assert_eq!(
            (g.starting_ability, g.ability, g.ability_changed),
            (Some(ab("Trace")), Some(ab("Intimidate")), true)
        );
        assert_eq!(
            mon(&t, P2, "Snorlax").starting_ability,
            Some(ab("Intimidate"))
        );
        let t = started(
            "|-ability|p1b: Gardevoir|Intimidate|Trace|[from] ability: Trace|[of] p2a: Snorlax
            |-ability|p1b: Gardevoir|Intimidate|boost
            |switch|p1b: Garchomp|Garchomp, L80, F|100/100",
        );
        let g = mon(&t, P1, "Gardevoir");
        assert_eq!(
            (g.starting_ability, g.ability),
            (Some(ab("Trace")), Some(ab("Trace")))
        );
    }

    #[test]
    fn skill_swap_and_other_changes_freeze_starting_abilities() {
        let t = started(
            "|-activate|p1b: Gardevoir|move: Skill Swap|Levitate|Intimidate|[of] p2a: Snorlax
            |-ability|p1b: Gardevoir|Levitate
            |-ability|p2a: Snorlax|Intimidate|boost",
        );
        let g = mon(&t, P1, "Gardevoir");
        assert_eq!(
            (g.starting_ability, g.ability, g.ability_changed),
            (Some(ab("Intimidate")), Some(ab("Levitate")), true)
        );
        let s = mon(&t, P2, "Snorlax");
        assert_eq!(
            (s.starting_ability, s.ability, s.ability_changed),
            (Some(ab("Levitate")), Some(ab("Intimidate")), true)
        );
        let t = started(
            "|-ability|p2a: Snorlax|Insomnia||[from] move: Worry Seed|[of] p1b: Gardevoir
            |-ability|p2a: Snorlax|Insomnia",
        );
        let s = mon(&t, P2, "Snorlax");
        assert_eq!(
            (s.starting_ability, s.ability),
            (None, Some(ab("Insomnia")))
        );
        assert_eq!(mon(&t, P1, "Gardevoir").starting_ability, None);
    }

    #[test]
    fn permanent_forme_abilities_survive_switching_out() {
        let t = track(
            "|switch|p2a: Terapagos|Terapagos, L73, M|100/100
            |-activate|p2a: Terapagos|ability: Tera Shift
            |detailschange|p2a: Terapagos|Terapagos-Terastal, L73, M
            |-activate|p2a: Terapagos|ability: Tera Shell
            |switch|p2a: Muk|Muk, L82, M|100/100
            |switch|p2a: Terapagos|Terapagos-Terastal, L73, M|100/100",
        );
        let m = mon(&t, P2, "Terapagos");
        assert_eq!(m.starting_ability, dex::lookup(EffectKind::Ability, "terashift"));
        assert_eq!(m.ability, dex::lookup(EffectKind::Ability, "terashell"));
    }

    #[test]
    fn forme_changes_with_new_abilities_freeze_the_starting_ability() {
        let t = track(
            "|switch|p2a: Terapagos|Terapagos, L73, M|100/100
            |-activate|p2a: Terapagos|ability: Tera Shift
            |detailschange|p2a: Terapagos|Terapagos-Terastal, L73, M
            |-activate|p2a: Terapagos|ability: Tera Shell",
        );
        let m = mon(&t, P2, "Terapagos");
        assert_eq!(
            (m.starting_ability, m.ability),
            (Some(ab("Tera Shift")), Some(ab("Tera Shell")))
        );
        assert_eq!(m.current_species, "Terapagos-Terastal");
        let t = track(
            "|switch|p2a: Minior|Minior, L85|100/100
            |-formechange|p2a: Minior|Minior-Meteor||[from] ability: Shields Down",
        );
        let m = mon(&t, P2, "Minior");
        assert_eq!(
            (m.starting_ability, m.ability_changed),
            (Some(ab("Shields Down")), false)
        );
    }

    #[test]
    fn as_one_is_one_ability() {
        let t = track(
            "|switch|p2a: Calyrex|Calyrex-Shadow, L68|100/100
            |-ability|p2a: Calyrex|As One
            |-ability|p2a: Calyrex|Unnerve
            |-ability|p2a: Calyrex|Grim Neigh|boost",
        );
        assert_eq!(
            mon(&t, P2, "Calyrex").starting_ability,
            Some(dex::ABILITY_ASONESPECTRIER)
        );
        assert_eq!(
            mon(&t, P2, "Calyrex").ability,
            Some(dex::ABILITY_ASONESPECTRIER)
        );
    }

    #[test]
    fn tera_from_terastallize_and_details() {
        let t = started("|-terastallize|p2a: Snorlax|Ghost");
        let s = mon(&t, P2, "Snorlax");
        assert_eq!(
            (s.tera_type, s.terastallized),
            (dex::type_id("Ghost"), true)
        );
        assert!(t.tera_used(P2) && !t.tera_used(P1));
        let t = track("|switch|p1a: Garchomp|Garchomp, L80, F, tera:Steel|100/100");
        assert_eq!(mon(&t, P1, "Garchomp").tera_type, dex::type_id("Steel"));
        assert!(t.tera_used(P1));
    }

    #[test]
    fn illusion_moves_the_stint_to_the_real_pokemon() {
        // The disguise's only appearance: the record is dropped.
        let t = started(
            "|switch|p2b: Darkrai|Darkrai, L81|100/100
            |move|p2b: Darkrai|Bitter Malice|p1a: Garchomp
            |-damage|p2b: Darkrai|90/100|[from] item: Life Orb
            |-terastallize|p2b: Darkrai|Normal
            |move|p1a: Garchomp|Earthquake|p2b: Darkrai
            |-damage|p2b: Darkrai|40/100
            |replace|p2b: Zoroark|Zoroark-Hisui, L80, M
            |-end|p2b: Zoroark|Illusion",
        );
        assert!(t.find(P2, "Darkrai").is_none());
        let z = mon(&t, P2, "Zoroark");
        assert_eq!(
            (z.species, z.level, z.illusion),
            (dex::SPECIES_ZOROARKHISUI, 80, true)
        );
        assert_eq!(z.moves, [mv("Bitter Malice")]);
        assert_eq!(
            (z.starting_item, z.starting_ability),
            (Some(item("Life Orb")), Some(dex::ABILITY_ILLUSION))
        );
        assert_eq!(
            (z.tera_type, z.hp, z.active, z.appearances),
            (dex::type_id("Normal"), 40, Some(1), 1)
        );
        assert_eq!(t.active(P2, 1), t.find(P2, "Zoroark"));
    }

    #[test]
    fn illusion_restores_the_disguise_and_splits_simultaneous_stints() {
        let t = started(
            "|switch|p2b: Darkrai|Darkrai, L81|100/100
            |move|p2b: Darkrai|Dark Pulse|p1a: Garchomp
            |switch|p2b: Furret|Furret, L95, F|300/300
            |switch|p2a: Darkrai|Darkrai, L81|100/100
            |move|p2a: Darkrai|Bitter Malice|p1a: Garchomp
            |-heal|p2b: Furret|300/300|[from] item: Leftovers
            |switch|p2b: Darkrai|Darkrai, L81|100/100
            |move|p2b: Darkrai|Nasty Plot|p2b: Darkrai
            |move|p1a: Garchomp|Earthquake|p2a: Darkrai
            |-damage|p2a: Darkrai|50/100
            |replace|p2a: Zoroark|Zoroark-Hisui, L80, M",
        );
        let d = mon(&t, P2, "Darkrai");
        assert_eq!(d.moves, [mv("Dark Pulse"), mv("Nasty Plot")]);
        assert_eq!((d.appearances, d.active, d.hp), (2, Some(1), 100));
        let z = mon(&t, P2, "Zoroark");
        assert_eq!(
            (z.moves.as_slice(), z.active, z.hp, z.appearances),
            ([mv("Bitter Malice")].as_slice(), Some(0), 50, 1)
        );
        assert_eq!(t.active(P2, 0), t.find(P2, "Zoroark"));
        assert_eq!(t.active(P2, 1), t.find(P2, "Darkrai"));
    }

    #[test]
    fn illusion_lead_is_the_real_pokemon() {
        let t = track(
            "|switch|p2a: Snorlax|Snorlax, L85, M|405/405
            |switch|p2b: Furret|Furret, L95, F|300/300
            |turn|1
            |-damage|p2a: Snorlax|200/405
            |replace|p2a: Zoroark|Zoroark-Hisui, L80, M",
        );
        assert!(t.find(P2, "Snorlax").is_none());
        assert_eq!(t.leads(P2), [t.find(P2, "Zoroark"), t.find(P2, "Furret")]);
        assert_eq!(t.mons(P2).len(), 2);
    }

    #[test]
    fn a_silent_faint_can_be_refolded_without_reviving_the_user() {
        let t = track(
            "|switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |turn|1
            |move|p2a: Bastiodon|Sludge Bomb|p1a: X
            |-damage|p2a: Bastiodon|0 fnt|[from] item: Life Orb
            |faint|p2a: Bastiodon
            |switch|p2a: Latias|Latias, L80, F|100/100
            |switch|p2a: Bastiodon|Bastiodon, L90, M|100/100
            |move|p2a: Bastiodon|Body Press|p1a: X",
        );
        let stint = t.stints(P2)[0].id;
        let view = t.illusion_view(P2, &[stint], "Zoroark", "Zoroark");
        let z = view.iter().find(|m| m.name == "Zoroark").unwrap();
        let b = view.iter().find(|m| m.name == "Bastiodon").unwrap();
        assert!(z.fainted && !b.fainted);
        assert_eq!(z.moves, [mv("Sludge Bomb")]);
        assert_eq!(b.moves, [mv("Body Press")]);
        assert_eq!(z.starting_item, Some(item("Life Orb")));
        assert_eq!(z.level, 0, "a disguised level does not constrain the user");
        assert_eq!(mon(&t, P2, "Bastiodon").moves, [mv("Sludge Bomb"), mv("Body Press")]);
    }
}
