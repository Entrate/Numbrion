//! Static game data for the encoder, the damage features and the smart player (port of `training/dex.py`).
//!
//! Everything comes from `data/training/dex.json` (`tools/training/export-dex.mjs`). Index 0 of every vocabulary means
//! "unknown or absent"; the item vocabulary also has [`NO_ITEM`] (the Pokemon holds nothing, for example after Knock
//! Off). Tables are indexed by those vocabulary indices, entry 0 included, exactly like the numpy tables of `dex.py`.

use crate::json::Json;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub const STATS: [&str; 6] = ["hp", "atk", "def", "spa", "spd", "spe"];
pub const TARGETS: [&str; 15] = [
    "normal",
    "any",
    "adjacentFoe",
    "adjacentAlly",
    "adjacentAllyOrSelf",
    "allAdjacent",
    "allAdjacentFoes",
    "self",
    "allySide",
    "foeSide",
    "all",
    "allies",
    "randomNormal",
    "scripted",
    "allyTeam",
];
pub const NO_ITEM: i64 = 1;
/// Type index of Stellar (`types` lists it last, after the 18 real types).
pub const STELLAR: i64 = 19;
/// Type vocabulary size: unknown + 18 types + Stellar.
pub const N_TYPES: usize = 20;

/// Type multipliers granted by a defender's ability or item, applied to the attacking move's type.
const ABILITY_TYPE_MULT: &[(&str, &[(&str, f32)])] = &[
    ("levitate", &[("Ground", 0.0)]),
    ("eartheater", &[("Ground", 0.0)]),
    ("flashfire", &[("Fire", 0.0)]),
    ("wellbakedbody", &[("Fire", 0.0)]),
    ("waterabsorb", &[("Water", 0.0)]),
    ("stormdrain", &[("Water", 0.0)]),
    ("dryskin", &[("Water", 0.0), ("Fire", 1.25)]),
    ("voltabsorb", &[("Electric", 0.0)]),
    ("lightningrod", &[("Electric", 0.0)]),
    ("motordrive", &[("Electric", 0.0)]),
    ("sapsipper", &[("Grass", 0.0)]),
    ("thickfat", &[("Fire", 0.5), ("Ice", 0.5)]),
    ("heatproof", &[("Fire", 0.5)]),
    ("waterbubble", &[("Fire", 0.5)]),
    ("purifyingsalt", &[("Ghost", 0.5)]),
];
const ITEM_TYPE_MULT: &[(&str, &[(&str, f32)])] = &[("airballoon", &[("Ground", 0.0)])];

/// Variable-power moves with a closed form the damage features compute (`SPECIAL_POWER`).
pub fn special_power(move_id: &str) -> i64 {
    match move_id {
        "lowkick" | "grassknot" => 1,
        "heavyslam" | "heatcrash" => 2,
        "eruption" | "waterspout" | "dragonenergy" => 3,
        "nightshade" => 4,
        "superfang" | "ruination" => 5,
        "endeavor" | "finalgambit" => 6,
        _ => 0,
    }
}

/// Showdown's `toID`: lowercase, keep only `a-z0-9` (Python: `re.sub("[^a-z0-9]", "", text.lower())`).
pub fn to_id(text: &str) -> String {
    if text.is_ascii() {
        return text.bytes().filter(u8::is_ascii_alphanumeric).map(|b| b.to_ascii_lowercase() as char).collect();
    }
    text.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

/// One move of `dex.json` (the fields the Rust pipeline reads). Null strings are None, null boost tables empty.
#[derive(Clone, Debug, Default)]
pub struct MoveData {
    pub id: String,
    pub mtype: String,
    /// "Physical", "Special" or "Status".
    pub category: String,
    pub base_power: f64,
    /// Percent; values above 100 mean the move cannot miss.
    pub accuracy: f64,
    pub priority: f64,
    pub target: String,
    pub pp: f64,
    pub flags: Vec<String>,
    pub recoil: f64,
    pub drain: f64,
    pub heal: f64,
    pub boosts: Vec<(String, f64)>,
    pub self_boosts: Vec<(String, f64)>,
    pub secondary_boosts: Vec<(String, f64)>,
    pub status: Option<String>,
    pub volatile_status: Option<String>,
    pub side_condition: Option<String>,
    pub stalling_move: bool,
    pub self_switch: bool,
    pub force_switch: bool,
    pub selfdestruct: bool,
    pub secondary_chance: f64,
    pub secondary_status: Option<String>,
    pub secondary_volatile: Option<String>,
}

impl MoveData {
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

/// One random-battle role set of a species, as vocabulary indices (sorted sets, like `dex.py`'s frozensets).
#[derive(Clone, Debug, Default)]
pub struct SetPrior {
    pub weight: f64,
    pub moves: Vec<i64>,
    pub abilities: Vec<i64>,
    pub tera: Vec<i64>,
}

/// `Dex.prior`: team-generator marginals (dex.json order, keys resolved like a Python dict) and role sets.
#[derive(Clone, Debug, Default)]
pub struct Prior {
    pub level: i64,
    pub moves: Vec<(i64, f64)>,
    pub items: Vec<(i64, f64)>,
    pub abilities: Vec<(i64, f64)>,
    pub tera: Vec<(i64, f64)>,
    pub sets: Vec<SetPrior>,
}

/// `Dex.belief`: posterior over a foe's unrevealed moves (inclusion probabilities, sorted by probability, ties by
/// move index), its abilities (team-generator marginals) and Tera types, and the item marginals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Belief {
    pub moves: Vec<(i64, f64)>,
    pub abilities: Vec<(i64, f64)>,
    pub teras: Vec<(i64, f64)>,
    pub items: Vec<(i64, f64)>,
}

#[derive(Debug)]
pub struct Dex {
    pub path: PathBuf,
    /// Type names, Stellar last; type index = position + 1.
    pub types: Vec<String>,
    pub n_types: usize,
    /// `[attacking * n_types + defending]`, with row and column 0 (unknown) all 1.
    pub typechart: Vec<f32>,

    /// Species id per species index (entry 0 is "").
    pub species_ids: Vec<String>,
    pub species_base: Vec<[f32; 6]>,
    pub species_types: Vec<[i64; 2]>,
    pub species_weight: Vec<f32>,

    /// Per move index (entry 0 is the default "unknown move").
    pub move_data: Vec<MoveData>,
    pub move_type: Vec<i64>,
    /// 0 status, 1 physical, 2 special.
    pub move_category: Vec<i64>,
    /// Base power; 60 for damaging moves listed with power <= 1 (variable power).
    pub move_power: Vec<f32>,
    pub move_accuracy: Vec<f32>,
    pub move_priority: Vec<f32>,
    /// Index into [`TARGETS`].
    pub move_target: Vec<i64>,
    pub move_hits: Vec<f32>,
    /// [`special_power`] of the move.
    pub move_special_power: Vec<i64>,
    pub move_stalling: Vec<bool>,

    /// Item id per item index: 0 unknown and 1 [`NO_ITEM`] are "".
    pub item_ids: Vec<String>,
    /// Ability id per ability index (entry 0 is "").
    pub ability_ids: Vec<String>,
    pub ability_type_mult: Vec<[f32; N_TYPES]>,
    pub item_type_mult: Vec<[f32; N_TYPES]>,

    type_index: HashMap<String, i64>,
    species_index: HashMap<String, i64>,
    aliases: HashMap<String, String>,
    move_index: HashMap<String, i64>,
    item_index: HashMap<String, i64>,
    ability_index: HashMap<String, i64>,
    /// `battleOnly` base forme per species id.
    battle_only: HashMap<String, String>,
    /// Priors per base species id (species with dex.json data, and every `sets` key).
    priors: HashMap<String, Prior>,
}

fn opt_string(v: &Json) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn boost_table(v: &Json) -> Vec<(String, f64)> {
    v.dict().into_iter().map(|(k, x)| (k.to_string(), x.as_f64().unwrap_or(0.0))).collect()
}

/// Insert like a Python dict assignment: a new key goes last, an existing key keeps its place and takes the value.
fn dict_set(out: &mut Vec<(i64, f64)>, key: i64, value: f64) {
    match out.iter_mut().find(|(k, _)| *k == key) {
        Some(slot) => slot.1 = value,
        None => out.push((key, value)),
    }
}

fn sorted_set(mut v: Vec<i64>) -> Vec<i64> {
    v.sort_unstable();
    v.dedup();
    v
}

impl Dex {
    /// Load `dex.json`.
    pub fn load(path: &Path) -> Result<Dex, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_json(&text, path)
    }

    /// [`Dex::load`] once per path for the whole process (encoders and agents share one copy).
    pub fn shared(path: &Path) -> Result<Arc<Dex>, String> {
        static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Dex>>>> = OnceLock::new();
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let cache = CACHE.get_or_init(Default::default);
        if let Some(d) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            return Ok(d.clone());
        }
        let dex = Arc::new(Self::load(path)?);
        cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, dex.clone());
        Ok(dex)
    }

    pub fn from_json(text: &str, path: &Path) -> Result<Dex, String> {
        let data = Json::parse(text).map_err(|e| format!("{}: {e}", path.display()))?;
        let types: Vec<String> = data.at("types").items().iter().map(|t| t.str_or_empty().to_string()).collect();
        let type_index: HashMap<String, i64> = types.iter().enumerate().map(|(i, t)| (t.clone(), i as i64 + 1)).collect();
        let n_types = types.len() + 1;
        if n_types != N_TYPES {
            return Err(format!("dex.json has {} types, expected {}", types.len(), N_TYPES - 1));
        }
        let mut typechart = vec![1.0f32; n_types * n_types];
        for (a, row) in data.at("typechart").items().iter().enumerate() {
            for (d, x) in row.items().iter().enumerate() {
                typechart[(a + 1) * n_types + d + 1] = x.as_f64().unwrap_or(1.0) as f32;
            }
        }
        let tidx = |name: &str| type_index.get(name).copied().unwrap_or(0);

        // Species.
        let species = data.at("species").items();
        let n_species = species.len() + 1;
        let mut species_ids = vec![String::new(); n_species];
        let mut species_base = vec![[0.0f32; 6]; n_species];
        let mut species_types = vec![[0i64; 2]; n_species];
        let mut species_weight = vec![0.0f32; n_species];
        let mut species_index = HashMap::new();
        let mut battle_only = HashMap::new();
        for (i, s) in species.iter().enumerate() {
            let i = i + 1;
            let id = s.at("id").str_or_empty().to_string();
            for (k, stat) in STATS.iter().enumerate() {
                species_base[i][k] = s.at("baseStats").at(stat).as_f64().unwrap_or(0.0) as f32;
            }
            for (j, t) in s.at("types").items().iter().take(2).enumerate() {
                species_types[i][j] = tidx(t.str_or_empty());
            }
            species_weight[i] = s.at("weightkg").as_f64().unwrap_or(0.0) as f32;
            if let Some(b) = s.at("battleOnly").as_str() {
                battle_only.insert(id.clone(), b.to_string());
            }
            species_index.insert(id.clone(), i as i64);
            species_ids[i] = id;
        }
        let aliases: HashMap<String, String> =
            data.at("aliases").dict().into_iter().map(|(k, v)| (k.to_string(), v.str_or_empty().to_string())).collect();

        // Moves.
        let moves = data.at("moves").items();
        let n_moves = moves.len() + 1;
        let mut move_data = vec![MoveData::default(); n_moves];
        let mut move_type = vec![0i64; n_moves];
        let mut move_category = vec![0i64; n_moves];
        let mut move_power = vec![0.0f32; n_moves];
        let mut move_accuracy = vec![1.0f32; n_moves];
        let mut move_priority = vec![0.0f32; n_moves];
        let mut move_target = vec![0i64; n_moves];
        let mut move_hits = vec![1.0f32; n_moves];
        let mut move_special_power = vec![0i64; n_moves];
        let mut move_stalling = vec![false; n_moves];
        let mut move_index = HashMap::new();
        for (i, m) in moves.iter().enumerate() {
            let i = i + 1;
            let num = |k: &str| m.at(k).as_f64().unwrap_or(0.0);
            let flag = |k: &str| m.at(k).truthy();
            let d = MoveData {
                id: m.at("id").str_or_empty().to_string(),
                mtype: m.at("type").str_or_empty().to_string(),
                category: m.at("category").str_or_empty().to_string(),
                base_power: num("basePower"),
                accuracy: num("accuracy"),
                priority: num("priority"),
                target: m.at("target").str_or_empty().to_string(),
                pp: num("pp"),
                flags: m.at("flags").items().iter().map(|f| f.str_or_empty().to_string()).collect(),
                recoil: num("recoil"),
                drain: num("drain"),
                heal: num("heal"),
                boosts: boost_table(m.at("boosts")),
                self_boosts: boost_table(m.at("selfBoosts")),
                secondary_boosts: boost_table(m.at("secondaryBoosts")),
                status: opt_string(m.at("status")),
                volatile_status: opt_string(m.at("volatileStatus")),
                side_condition: opt_string(m.at("sideCondition")),
                stalling_move: flag("stallingMove"),
                self_switch: flag("selfSwitch"),
                force_switch: flag("forceSwitch"),
                selfdestruct: flag("selfdestruct"),
                secondary_chance: num("secondaryChance"),
                secondary_status: opt_string(m.at("secondaryStatus")),
                secondary_volatile: opt_string(m.at("secondaryVolatile")),
            };
            move_type[i] = tidx(&d.mtype);
            move_category[i] = match d.category.as_str() {
                "Physical" => 1,
                "Special" => 2,
                _ => 0,
            };
            let mut power = d.base_power;
            if d.category != "Status" && power <= 1.0 {
                power = 60.0; // variable power without a closed form here; the damage features override the common ones
            }
            move_power[i] = power as f32;
            move_accuracy[i] = (d.accuracy.min(100.0) / 100.0) as f32;
            move_priority[i] = d.priority as f32;
            move_target[i] = TARGETS.iter().position(|t| *t == d.target).unwrap_or(0) as i64;
            let hits = m.at("multihit");
            match hits {
                Json::Arr(h) => {
                    let lo = h.first().and_then(Json::as_f64).unwrap_or(1.0);
                    let hi = h.get(1).and_then(Json::as_f64).unwrap_or(lo);
                    move_hits[i] = if h.len() == 2 && lo == 2.0 && hi == 5.0 { 3.1 } else { ((lo + hi) / 2.0) as f32 };
                }
                h if h.truthy() => move_hits[i] = h.as_f64().unwrap_or(1.0) as f32,
                _ => {}
            }
            if d.id == "tripleaxel" {
                move_hits[i] = 5.4; // 20+40+60 power is six 20-power hits; ~90% of the follow-ups land
            }
            move_special_power[i] = special_power(&d.id);
            move_stalling[i] = d.stalling_move;
            move_index.insert(d.id.clone(), i as i64);
            move_data[i] = d;
        }

        // Items (index 0 unknown, 1 NO_ITEM) and abilities.
        let items = data.at("items").items();
        let mut item_ids = vec![String::new(); items.len() + 2];
        let mut item_index = HashMap::new();
        for (i, it) in items.iter().enumerate() {
            let id = it.at("id").str_or_empty().to_string();
            item_index.insert(id.clone(), i as i64 + 2);
            item_ids[i + 2] = id;
        }
        let abilities = data.at("abilities").items();
        let mut ability_ids = vec![String::new(); abilities.len() + 1];
        let mut ability_index = HashMap::new();
        for (i, a) in abilities.iter().enumerate() {
            let id = a.at("id").str_or_empty().to_string();
            ability_index.insert(id.clone(), i as i64 + 1);
            ability_ids[i + 1] = id;
        }
        let mut ability_type_mult = vec![[1.0f32; N_TYPES]; ability_ids.len()];
        for (ability, table) in ABILITY_TYPE_MULT {
            if let Some(&a) = ability_index.get(*ability) {
                for (t, mult) in *table {
                    ability_type_mult[a as usize][tidx(t) as usize] = *mult;
                }
            }
        }
        let mut item_type_mult = vec![[1.0f32; N_TYPES]; item_ids.len()];
        for (item, table) in ITEM_TYPE_MULT {
            if let Some(&a) = item_index.get(*item) {
                for (t, mult) in *table {
                    item_type_mult[a as usize][tidx(t) as usize] = *mult;
                }
            }
        }

        // Priors per base species id.
        let sets = data.at("sets");
        let mut priors = HashMap::new();
        let species_by_id: HashMap<&str, &Json> = species.iter().map(|s| (s.at("id").str_or_empty(), s)).collect();
        let mut keys: Vec<&str> = species_by_id.keys().copied().collect();
        keys.extend(sets.dict().into_iter().map(|(k, _)| k));
        for base in keys {
            if priors.contains_key(base) {
                continue;
            }
            let entry = species_by_id.get(base).copied().unwrap_or(&Json::Null);
            let lookup = |table: &Json, index: &HashMap<String, i64>| {
                let mut out = Vec::new();
                for (k, v) in table.dict() {
                    if let Some(&i) = index.get(k) {
                        dict_set(&mut out, i, v.as_f64().unwrap_or(0.0));
                    }
                }
                out
            };
            let mut tera = Vec::new();
            for (k, v) in entry.at("teraTypes").dict() {
                dict_set(&mut tera, tidx(k), v.as_f64().unwrap_or(0.0));
            }
            let role_sets = sets
                .at(base)
                .items()
                .iter()
                .map(|s| SetPrior {
                    weight: s.at("weight").as_f64().unwrap_or(0.0),
                    moves: sorted_set(s.at("movepool").items().iter().filter_map(|m| move_index.get(m.str_or_empty()).copied()).collect()),
                    abilities: sorted_set(s.at("abilities").items().iter().filter_map(|a| ability_index.get(a.str_or_empty()).copied()).collect()),
                    tera: sorted_set(s.at("teraTypes").items().iter().map(|t| tidx(t.str_or_empty())).collect()),
                })
                .collect();
            let level = entry.at("level");
            priors.insert(
                base.to_string(),
                Prior {
                    level: if level.truthy() { level.as_f64().unwrap_or(80.0) as i64 } else { 80 },
                    moves: lookup(entry.at("moves"), &move_index),
                    items: lookup(entry.at("items"), &item_index),
                    abilities: lookup(entry.at("abilities"), &ability_index),
                    tera,
                    sets: role_sets,
                },
            );
        }

        Ok(Dex {
            path: path.to_path_buf(),
            types,
            n_types,
            typechart,
            species_ids,
            species_base,
            species_types,
            species_weight,
            move_data,
            move_type,
            move_category,
            move_power,
            move_accuracy,
            move_priority,
            move_target,
            move_hits,
            move_special_power,
            move_stalling,
            item_ids,
            ability_ids,
            ability_type_mult,
            item_type_mult,
            type_index,
            species_index,
            aliases,
            move_index,
            item_index,
            ability_index,
            battle_only,
            priors,
        })
    }

    pub fn n_species(&self) -> usize {
        self.species_ids.len()
    }

    pub fn n_moves(&self) -> usize {
        self.move_data.len()
    }

    pub fn n_items(&self) -> usize {
        self.item_ids.len()
    }

    pub fn n_abilities(&self) -> usize {
        self.ability_ids.len()
    }

    // ---- name resolution (dex.py) -----------------------------------------------------------------------------

    /// `species_id`: the id of a species name, through the cosmetic-forme aliases.
    pub fn species_id(&self, name: &str) -> String {
        let sid = to_id(name);
        match self.aliases.get(&sid) {
            Some(a) => a.clone(),
            None => sid,
        }
    }

    /// `species_idx(name)`.
    pub fn species_idx(&self, name: &str) -> i64 {
        self.species_index(&self.species_id(name))
    }

    /// `species_index.get(sid, 0)`.
    pub fn species_index(&self, sid: &str) -> i64 {
        self.species_index.get(sid).copied().unwrap_or(0)
    }

    /// `move_index.get(id, 0)` (an exact id, no `to_id`).
    pub fn move_index(&self, id: &str) -> i64 {
        self.move_index.get(id).copied().unwrap_or(0)
    }

    /// `move_idx(name)`.
    pub fn move_idx(&self, name: &str) -> i64 {
        self.move_index(&to_id(name))
    }

    /// `item_index.get(id, 0)` (an exact id).
    pub fn item_index(&self, id: &str) -> i64 {
        self.item_index.get(id).copied().unwrap_or(0)
    }

    /// `item_idx(name)`: [`NO_ITEM`] for "".
    pub fn item_idx(&self, name: &str) -> i64 {
        if name.is_empty() { NO_ITEM } else { self.item_index(&to_id(name)) }
    }

    /// `ability_index.get(id, 0)` (an exact id).
    pub fn ability_index(&self, id: &str) -> i64 {
        self.ability_index.get(id).copied().unwrap_or(0)
    }

    /// `ability_idx(name)`.
    pub fn ability_idx(&self, name: &str) -> i64 {
        self.ability_index(&to_id(name))
    }

    /// `type_idx(name)`: an exact type name ("Fire"), 0 if unknown.
    pub fn type_idx(&self, name: &str) -> i64 {
        self.type_index.get(name).copied().unwrap_or(0)
    }

    /// `typechart[attacking, defending]`.
    pub fn effectiveness(&self, attacking: i64, defending: i64) -> f32 {
        self.typechart[attacking as usize * self.n_types + defending as usize]
    }

    // ---- random-battle priors -----------------------------------------------------------------------------------

    /// `base_species`: battle-only formes resolve to their base species id.
    pub fn base_species<'a>(&'a self, sid: &'a str) -> &'a str {
        let mut sid = sid;
        while let Some(base) = self.battle_only.get(sid) {
            if !self.species_index.contains_key(base.as_str()) || base == sid {
                break;
            }
            sid = base;
        }
        sid
    }

    /// `prior(sid)`; an empty prior (level 80) for a species without random-battle data.
    pub fn prior(&self, sid: &str) -> &Prior {
        static EMPTY: OnceLock<Prior> = OnceLock::new();
        self.priors.get(self.base_species(sid)).unwrap_or_else(|| EMPTY.get_or_init(|| Prior { level: 80, ..Prior::default() }))
    }

    /// `belief(sid, known_moves, ability, tera)`: `known_moves` are the revealed move indices (non-zero, sorted, no
    /// repeats). Sets consistent with every revealed move, ability and Tera type are weighted by their role share
    /// times the chance that a uniformly drawn 4-move subset of the movepool contains the revealed moves; inside a
    /// set every unrevealed movepool move then has probability (4 - r) / (n - r). Falls back to the team-generator
    /// marginals. Unlike the Python original this does not cache; the encoder caches whole foe states.
    pub fn belief(&self, sid: &str, known_moves: &[i64], ability: i64, tera: i64) -> Belief {
        let prior = self.prior(sid);
        let known = |m: &i64| known_moves.binary_search(m).is_ok();
        let mut consistent: Vec<(f64, &SetPrior, f64)> = Vec::new();
        for s in &prior.sets {
            if !known_moves.iter().all(|m| s.moves.binary_search(m).is_ok()) {
                continue;
            }
            if (ability != 0 && !s.abilities.is_empty() && s.abilities.binary_search(&ability).is_err())
                || (tera != 0 && !s.tera.is_empty() && s.tera.binary_search(&tera).is_err())
            {
                continue;
            }
            let (n, r) = (s.moves.len() as i64, known_moves.len() as i64);
            let picks = n.min(4);
            let mut likelihood = 1.0f64;
            for k in 0..r {
                likelihood *= (picks - k).max(0) as f64 / (n - k).max(1) as f64;
            }
            if likelihood > 0.0 {
                consistent.push((s.weight * likelihood, s, (picks - r) as f64 / (n - r).max(1) as f64));
            }
        }
        let mut moves: Vec<(i64, f64)> = Vec::new();
        let mut teras: Vec<(i64, f64)> = Vec::new();
        let total = fsum(consistent.iter().map(|c| c.0));
        for (w, s, p) in &consistent {
            let share = w / total;
            for &m in s.moves.iter().filter(|m| !known(m)) {
                match moves.iter_mut().find(|(k, _)| *k == m) {
                    Some(slot) => slot.1 += share * p,
                    None => moves.push((m, 0.0 + share * p)),
                }
            }
            for &t in &s.tera {
                let add = share / s.tera.len() as f64;
                match teras.iter_mut().find(|(k, _)| *k == t) {
                    Some(slot) => slot.1 += add,
                    None => teras.push((t, 0.0 + add)),
                }
            }
        }
        if consistent.is_empty() {
            let missing = (4 - known_moves.len() as i64).max(0) as f64;
            let free = fsum(prior.moves.iter().filter(|(m, _)| !known(m)).map(|(_, p)| *p));
            let free = if free == 0.0 { 1.0 } else { free };
            moves = prior.moves.iter().filter(|(m, _)| !known(m)).map(|&(m, p)| (m, (p * missing / free).min(1.0))).collect();
        }
        if teras.is_empty() {
            teras = prior.tera.clone();
        }
        moves.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        Belief { moves, abilities: prior.abilities.clone(), teras, items: prior.items.clone() }
    }
}

/// Python 3.12+ `sum()` of floats (Neumaier compensated summation), so tie structures match the reference.
pub fn fsum(values: impl IntoIterator<Item = f64>) -> f64 {
    let (mut sum, mut c) = (0.0f64, 0.0f64);
    for x in values {
        let t = sum + x;
        if sum.abs() >= x.abs() {
            c += (sum - t) + x;
        } else {
            c += (x - t) + sum;
        }
        sum = t;
    }
    sum + c
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn dex() -> Arc<Dex> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json");
        Dex::shared(&path).expect("dex.json")
    }

    #[test]
    fn vocabularies_and_tables() {
        let d = dex();
        assert_eq!(d.n_types, 20);
        assert_eq!(d.type_idx("Normal"), 1);
        assert_eq!(d.type_idx("Stellar"), STELLAR);
        assert_eq!(d.item_idx(""), NO_ITEM);
        assert_eq!(d.item_ids[NO_ITEM as usize], "");
        let tw = d.move_index("tripleaxel") as usize;
        assert_eq!(d.move_hits[tw], 5.4);
        let bs = d.move_index("bulletseed") as usize;
        assert_eq!(d.move_hits[bs], 3.1);
        assert_eq!(d.move_special_power[d.move_index("lowkick") as usize], 1);
        let lev = d.ability_index("levitate") as usize;
        assert_eq!(d.ability_type_mult[lev][d.type_idx("Ground") as usize], 0.0);
        assert_eq!(to_id("Flabébé"), "flabb");
        assert_eq!(to_id("Mr. Mime-Galar"), "mrmimegalar");
        assert_eq!(d.effectiveness(d.type_idx("Fire"), d.type_idx("Grass")), 2.0);
    }

    #[test]
    fn belief_narrows_with_reveals() {
        let d = dex();
        let sid = "abomasnow";
        let b = d.belief(sid, &[], 0, 0);
        assert!(!b.moves.is_empty());
        assert!(b.moves.windows(2).all(|w| w[0].1 >= w[1].1));
        let first = b.moves[0].0;
        let narrowed = d.belief(sid, &[first], 0, 0);
        assert!(narrowed.moves.iter().all(|(m, _)| *m != first));
    }
}
