//! The token encoder (port of `Encoder` / `Buffers` in `training/encoder.py`).
//!
//! Each row (one side of one battle, row `2 * env + side`) becomes 12 Pokemon tokens and one field token. The
//! arrays are row-major `[rows, ...]` slices with the shapes of `Buffers` in encoder.py.
//!
//! Float columns the Python computes with Python floats (f64) are computed in f64 and rounded to f32 when stored;
//! the numpy float32 parts (estimated stats, type multipliers) are computed in f32 in the same operation order.

use crate::{
    damage::DamageFeatures,
    dex::{Dex, NO_ITEM, fsum},
    json::Json,
    layout::*,
    tracker::{self, Key, Mon, Tracker},
};
use std::{collections::HashMap, fmt::Write as _, sync::Arc};

/// Per-row sizes of the output arrays (`Buffers` in encoder.py).
pub const IDS_PER_ROW: usize = N_TOKENS * N_IDS;
pub const FLOATS_PER_ROW: usize = N_TOKENS * N_POKEMON_FLOATS;
pub const FIELD_PER_ROW: usize = N_FIELD_FLOATS;
pub const FOE_MATCH_PER_ROW: usize = 6;
pub const FAINTED_PER_ROW: usize = 2;
/// `guard[row, slot, move slot, kind]`, kind 0 STALE, 1 REPEAT_PROTECT (training/guards.py).
pub const GUARD_PER_ROW: usize = 2 * 4 * 2;
pub const ACTION_PER_ROW: usize = 2 * N_CODES * N_ACTION_FEATURES;
pub const TOKEN_FEATURES_PER_ROW: usize = N_TOKENS * N_TOKEN_FEATURES;

/// Guard kinds (last axis of `guard`, training/guards.py).
pub const STALE: usize = 0;
pub const REPEAT_PROTECT: usize = 1;
/// Moves that fail after the user's first move action since switching in (`FIRST_TURN_ONLY`).
pub const FIRST_TURN_ONLY: [&str; 2] = ["fakeout", "firstimpression"];

/// Static floats per token (`STATIC_FLOATS`), the columns after the dynamic block.
const N_STATIC: usize = N_POKEMON_FLOATS - N_DYNAMIC;

/// Mutable views of the observation arrays of all rows (int64 / float32 / bool, C order).
pub struct Outputs<'a> {
    pub ids: &'a mut [i64],
    pub floats: &'a mut [f32],
    pub field: &'a mut [f32],
    pub foe_match: &'a mut [i64],
    pub fainted: &'a mut [f32],
    pub guard: &'a mut [bool],
    pub action_features: &'a mut [f32],
    pub token_features: &'a mut [f32],
}

impl Outputs<'_> {
    /// The number of rows the slices hold, or an error naming the first slice whose length is inconsistent.
    pub fn rows(&self) -> Result<usize, String> {
        let rows = self.field.len() / FIELD_PER_ROW;
        let check = |name: &str, len: usize, per: usize| {
            if len == rows * per { Ok(()) } else { Err(format!("{name} has {len} elements, expected {rows} x {per}")) }
        };
        check("field", self.field.len(), FIELD_PER_ROW)?;
        check("ids", self.ids.len(), IDS_PER_ROW)?;
        check("floats", self.floats.len(), FLOATS_PER_ROW)?;
        check("foe_match", self.foe_match.len(), FOE_MATCH_PER_ROW)?;
        check("fainted", self.fainted.len(), FAINTED_PER_ROW)?;
        check("guard", self.guard.len(), GUARD_PER_ROW)?;
        check("action_features", self.action_features.len(), ACTION_PER_ROW)?;
        check("token_features", self.token_features.len(), TOKEN_FEATURES_PER_ROW)?;
        Ok(rows)
    }
}

/// What one side of one env observes (`BatchEnv.observe`): its request JSON (None or "" for no request) and the
/// player-view protocol lines since the last observation.
pub struct Observation<'a, S: AsRef<str>> {
    pub battle_id: i64,
    pub request: Option<&'a str>,
    pub log: &'a [S],
}

/// `estimated_stats(base, level)`: random-battle stats (31 IVs, 85 EVs, neutral nature) in float32 like the numpy
/// original: `floor((2 * base + 31 + 21) * level / 100) + 5`, HP `core + level + 10`.
pub fn estimated_stats(base: &[f32; 6], level: i64) -> [f32; 6] {
    let level = level as f32;
    let mut stats = [0.0f32; 6];
    for (s, b) in stats.iter_mut().zip(base) {
        *s = ((2.0 * b + 31.0 + 21.0) * level / 100.0).floor();
    }
    let hp = stats[0] + level + 10.0;
    for s in &mut stats {
        *s += 5.0;
    }
    stats[0] = hp;
    stats
}

/// The cached static part of a token: ids, the static float columns and the (estimated) max HP.
#[derive(Clone, Debug)]
struct Static {
    ids: [i64; N_IDS],
    row: [f32; N_STATIC],
    max_hp: f64,
}

impl Static {
    fn new() -> Self {
        Static { ids: [0; N_IDS], row: [0.0; N_STATIC], max_hp: 0.0 }
    }

    /// Static float column `pf` (an index into the full per-token row).
    fn set(&mut self, pf: usize, value: f32) {
        self.row[pf - N_DYNAMIC] = value;
    }
}

/// `_foe_static`'s cache key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FoeKey {
    species: String,
    level: i64,
    moves: Vec<String>,
    ability: Option<String>,
    tera: String,
    item: Option<String>,
    item_lost: bool,
}

/// Appends a serialization of `v` that is equal for values Python's `==` treats as equal (True == 1 == 1.0,
/// 0.0 == -0.0), for the own-static cache key.
fn canon(v: &Json, out: &mut String) {
    let num = |x: f64, out: &mut String| {
        let x = if x == 0.0 { 0.0 } else { x };
        let _ = write!(out, "d{:x};", x.to_bits());
    };
    match v {
        Json::Null => out.push('n'),
        Json::Bool(b) => num(f64::from(u8::from(*b)), out),
        Json::Num(x) => num(*x, out),
        Json::Str(s) => {
            let _ = write!(out, "s{}:", s.len());
            out.push_str(s);
        }
        Json::Arr(items) => {
            out.push('[');
            items.iter().for_each(|x| canon(x, out));
            out.push(']');
        }
        Json::Obj(members) => {
            out.push('{');
            for (k, x) in members {
                let _ = write!(out, "{}:{k}", k.len());
                canon(x, out);
            }
            out.push('}');
        }
    }
}

static EMPTY_STR: Json = Json::Str(String::new());

/// Python's `==` between two JSON scalars (strings by content, numbers and booleans by value, None == None).
fn py_eq(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Str(x), Json::Str(y)) => x == y,
        (Json::Null, Json::Null) => true,
        (Json::Num(_) | Json::Bool(_), Json::Num(_) | Json::Bool(_)) => a.as_f64() == b.as_f64(),
        _ => false,
    }
}

/// Python's `max(x, 1)`: the first argument unless the second is larger.
fn max1(x: f64) -> f64 {
    if 1.0 > x { 1.0 } else { x }
}

/// The static-feature caches of `Encoder` (`_own_cache`, `_foe_cache`), shared by all rows.
#[derive(Default)]
struct Statics {
    own: HashMap<String, Static>,
    foe: HashMap<FoeKey, Static>,
    key: String,
}

impl Statics {
    /// `_own_static(pokemon)`: ids, static floats and estimated max HP of one of our request's Pokemon. Cached under
    /// Python's key, which leaves out `baseAbility` and the stat names, so a cached entry wins exactly as in Python.
    fn own(&mut self, dex: &Dex, pokemon: &Json) -> Static {
        let key = &mut self.key;
        key.clear();
        canon(pokemon.at("details"), key);
        for field in ["item", "ability", "teraType"] {
            canon(pokemon.get(field).unwrap_or(&EMPTY_STR), key);
        }
        key.push('(');
        pokemon.at("moves").items().iter().for_each(|m| canon(m, key));
        key.push_str(")(");
        pokemon.at("stats").dict().into_iter().for_each(|(_, v)| canon(v, key));
        key.push(')');
        if let Some(cached) = self.own.get(key.as_str()) {
            return cached.clone();
        }
        let (species, level, _) = tracker::details(pokemon.at("details").str_or_empty());
        let sp = dex.species_idx(species);
        let mut s = Static::new();
        s.ids[I_SPECIES] = sp;
        s.ids[I_ITEM] = dex.item_idx(pokemon.at("item").str_or_empty());
        let ability = pokemon.at("ability");
        let ability = if ability.truthy() { ability } else { pokemon.at("baseAbility") };
        s.ids[I_ABILITY] = dex.ability_idx(ability.str_or_empty());
        s.ids[I_TERA] = dex.type_idx(pokemon.at("teraType").str_or_empty());
        s.ids[I_TYPE1..=I_TYPE2].copy_from_slice(&dex.species_types[sp as usize]);
        let moves = pokemon.at("moves").items();
        let moves = &moves[..moves.len().min(4)];
        for (m, mv) in moves.iter().enumerate() {
            s.ids[I_MOVE0 + m] = dex.move_index(mv.str_or_empty());
        }
        s.set(pf::LEVEL, (level as f64 / 100.0) as f32);
        let stats = pokemon.at("stats");
        for (col, stat) in [(pf::STAT_ATK, "atk"), (pf::STAT_DEF, "def"), (pf::STAT_SPA, "spa"), (pf::STAT_SPD, "spd"), (pf::STAT_SPE, "spe")] {
            s.set(col, (stats.at(stat).as_f64().unwrap_or(0.0) / f64::from(STAT_SCALE)) as f32);
        }
        for col in [pf::ITEM_KNOWN, pf::ABILITY_KNOWN, pf::TERA_KNOWN, pf::MOVES_SEEN] {
            s.set(col, 1.0);
        }
        s.set(pf::ITEM_LOST, f32::from(u8::from(!pokemon.at("item").truthy())));
        for m in 0..moves.len() {
            s.set(pf::MOVE_KNOWN0 + m, 1.0);
        }
        s.set(pf::TERA_P0 + s.ids[I_TERA] as usize, 1.0);
        let (am, im) = (&dex.ability_type_mult[s.ids[I_ABILITY] as usize], &dex.item_type_mult[s.ids[I_ITEM] as usize]);
        for t in 0..20 {
            s.set(pf::TYPE_MULT0 + t, am[t] * im[t]);
        }
        s.max_hp = f64::from(estimated_stats(&dex.species_base[sp as usize], level)[0]);
        if self.own.len() >= CACHE_LIMIT {
            self.own.clear();
        }
        self.own.insert(key.clone(), s.clone());
        s
    }

    /// `_foe_static(mon)`: ids, static floats and estimated max HP of a revealed foe, beliefs included.
    fn foe(&mut self, dex: &Dex, mon: &Mon) -> Static {
        let key = FoeKey {
            species: mon.species.clone(),
            level: mon.level,
            moves: mon.moves.clone(),
            ability: mon.ability.clone(),
            tera: if mon.terastallized { mon.tera.clone() } else { String::new() },
            item: mon.item.clone(),
            item_lost: mon.item_lost,
        };
        if let Some(cached) = self.foe.get(&key) {
            return cached.clone();
        }
        let sid = dex.species_id(&mon.species);
        let sp = dex.species_index(&sid);
        let mut s = Static::new();
        s.ids[I_SPECIES] = sp;
        s.ids[I_TYPE1..=I_TYPE2].copy_from_slice(&dex.species_types[sp as usize]);
        let move_ids: Vec<i64> = mon.moves.iter().take(4).map(|m| dex.move_index(m)).collect();
        s.ids[I_MOVE0..I_MOVE0 + move_ids.len()].copy_from_slice(&move_ids);
        let ability = match &mon.ability {
            Some(a) if !a.is_empty() => dex.ability_index(a),
            _ => 0,
        };
        let tera = if mon.terastallized { dex.type_idx(&mon.tera) } else { 0 };
        s.ids[I_ABILITY] = ability;
        s.ids[I_TERA] = tera;
        if let Some(item) = &mon.item {
            s.ids[I_ITEM] = if item.is_empty() { NO_ITEM } else { dex.item_index(item) };
        }
        let stats = estimated_stats(&dex.species_base[sp as usize], mon.level);
        s.set(pf::LEVEL, (mon.level as f64 / 100.0) as f32);
        for i in 0..5 {
            s.set(pf::STAT_ATK + i, stats[i + 1] / STAT_SCALE);
        }
        s.set(pf::ITEM_KNOWN, f32::from(u8::from(mon.item.is_some())));
        s.set(pf::ABILITY_KNOWN, f32::from(u8::from(ability != 0)));
        s.set(pf::TERA_KNOWN, f32::from(u8::from(tera != 0)));
        s.set(pf::MOVES_SEEN, (move_ids.len() as f64 / 4.0) as f32);
        s.set(pf::ITEM_LOST, f32::from(u8::from(mon.item_lost)));
        for m in 0..move_ids.len() {
            s.set(pf::MOVE_KNOWN0 + m, 1.0);
        }
        let mut known: Vec<i64> = move_ids.iter().copied().filter(|&m| m != 0).collect();
        known.sort_unstable();
        known.dedup();
        let belief = dex.belief(&sid, &known, ability, tera);
        for (j, &(m, p)) in belief.moves.iter().take(K_MOVES).enumerate() {
            s.ids[I_CAND_MOVES + j] = m;
            s.set(pf::CAND_MOVE_P0 + j, p as f32);
        }
        if ability == 0 {
            for (j, &(a, p)) in belief.abilities.iter().take(K_ABILITIES).enumerate() {
                s.ids[I_CAND_ABILITIES + j] = a;
                s.set(pf::CAND_ABILITY_P0 + j, p as f32);
            }
        }
        if mon.item.is_none() {
            for (j, &(i, p)) in belief.items.iter().take(K_ITEMS).enumerate() {
                s.ids[I_CAND_ITEMS + j] = i;
                s.set(pf::CAND_ITEM_P0 + j, p as f32);
            }
        }
        if tera != 0 {
            s.set(pf::TERA_P0 + tera as usize, 1.0);
        } else {
            for &(t, p) in &belief.teras {
                s.set(pf::TERA_P0 + t as usize, p as f32);
            }
        }
        let mut mult = dex.item_type_mult[s.ids[I_ITEM] as usize];
        if ability != 0 {
            let am = &dex.ability_type_mult[ability as usize];
            for t in 0..20 {
                mult[t] *= am[t];
            }
        } else if !belief.abilities.is_empty() {
            // numpy: sum(p * row for ...) in float32 from 0, then / total (a Python float sum) as float32.
            let total = fsum(belief.abilities.iter().map(|&(_, p)| p)) as f32;
            let mut weighted = [0.0f32; 20];
            for &(a, p) in &belief.abilities {
                let am = &dex.ability_type_mult[a as usize];
                for t in 0..20 {
                    weighted[t] += p as f32 * am[t];
                }
            }
            for t in 0..20 {
                mult[t] *= weighted[t] / total;
            }
        }
        for (t, m) in mult.iter().enumerate() {
            s.set(pf::TYPE_MULT0 + t, *m);
        }
        s.max_hp = f64::from(stats[0]);
        if self.foe.len() >= CACHE_LIMIT {
            self.foe.clear();
        }
        self.foe.insert(key, s.clone());
        s
    }
}

/// `_dynamic`: the HP / status / slot / boost / timing / volatile columns of one token's dynamic block.
fn dynamic(d: &mut [f64; N_DYNAMIC], hp: f64, status: &str, mon: Option<&Mon>, turn: i64, is_active: bool, slot: usize) {
    d[pf::HP] = hp;
    if status == "fnt" {
        d[pf::FAINTED] = 1.0;
        return;
    }
    if let Some(i) = STATUSES.iter().position(|s| *s == status) {
        d[pf::BRN + i] = 1.0;
    }
    if !is_active {
        d[pf::BENCH] = 1.0;
        return;
    }
    d[if slot == 0 { pf::ACTIVE_A } else { pf::ACTIVE_B }] = 1.0;
    let Some(mon) = mon else { return };
    for (stat, value) in &mon.boosts {
        if let Some(i) = BOOSTS.iter().position(|b| b == stat) {
            d[pf::BOOST_ATK + i] = *value as f64 / 6.0;
        }
    }
    d[pf::SWITCH_TURNS] = (turn - mon.switch_turn).min(5) as f64 / 5.0;
    d[pf::FRESH] = f64::from(u8::from(mon.moves_since_switch == 0));
    d[pf::PROTECT_LAST] = f64::from(u8::from(mon.protect_turn >= turn - 1));
    for (name, _) in &mon.volatiles {
        if let Some(i) = VOLATILES.iter().position(|v| v == name) {
            d[pf::SUBSTITUTE + i] = 1.0;
        }
    }
    if let Some(perish) = mon.volatile("perish") {
        d[pf::PERISH] = (perish + 1) as f64 / 4.0;
    }
}

/// Writes one token: the dynamic block (f64 rounded to f32), the static block and the ids.
fn put_token(out: &mut Outputs<'_>, row: usize, token: usize, d: &[f64; N_DYNAMIC], s: Option<&Static>) {
    let f = &mut out.floats[row * FLOATS_PER_ROW + token * N_POKEMON_FLOATS..][..N_POKEMON_FLOATS];
    for (o, x) in f[..N_DYNAMIC].iter_mut().zip(d) {
        *o = *x as f32;
    }
    let ids = &mut out.ids[row * IDS_PER_ROW + token * N_IDS..][..N_IDS];
    match s {
        Some(s) => {
            f[N_DYNAMIC..].copy_from_slice(&s.row);
            ids.copy_from_slice(&s.ids);
        }
        None => {
            f[N_DYNAMIC..].fill(0.0);
            ids.fill(0);
        }
    }
}

/// Encodes rows `2 * env + side`.
pub struct Encoder {
    pub dex: Arc<Dex>,
    pub damage: DamageFeatures,
    trackers: Vec<Tracker>,
    /// Per row: the names of our request's Pokemon, in request order (`own_names`).
    own_names: Vec<Vec<String>>,
    /// Per row: the names of the foes on tokens 6-11 (`foe_names`).
    foe_names: Vec<[Option<String>; 6]>,
    statics: Statics,
}

impl Encoder {
    pub fn new(dex: Arc<Dex>, rows: usize) -> Self {
        let damage = DamageFeatures::new(&dex);
        Encoder {
            dex,
            damage,
            trackers: (0..rows).map(|r| Tracker::new(r % 2)).collect(),
            own_names: vec![Vec::new(); rows],
            foe_names: vec![Default::default(); rows],
            statics: Statics::default(),
        }
    }

    pub fn rows(&self) -> usize {
        self.trackers.len()
    }

    pub fn tracker(&self, row: usize) -> &Tracker {
        &self.trackers[row]
    }

    /// The names of `row`'s own request Pokemon from the last `encode_row`.
    pub fn own_names(&self, row: usize) -> &[String] {
        &self.own_names[row]
    }

    /// The foe names on tokens 6-11 of `row` from the last `encode_row`.
    pub fn foe_names(&self, row: usize) -> &[Option<String>; 6] {
        &self.foe_names[row]
    }

    /// encoder.py `Encoder.encode(row, observation)`: tracker update and the ids / floats / field / fainted / guard
    /// of `row` (no damage features, no partner matching). Err for a request that is not valid JSON or that the
    /// Python encoder would raise on (a Pokemon ident without ": ", an unreadable condition); the tracker has been fed
    /// by then, as in Python.
    pub fn encode_row<S: AsRef<str>>(&mut self, row: usize, obs: &Observation<'_, S>, out: &mut Outputs<'_>) -> Result<(), String> {
        let out_rows = out.rows()?;
        if row >= self.rows() || row >= out_rows {
            return Err(format!("row {row} out of range: encoder has {} rows, arrays {out_rows}", self.rows()));
        }
        let tracker = &mut self.trackers[row];
        if tracker.battle_id != obs.battle_id {
            tracker.reset(obs.battle_id);
        }
        tracker.feed(obs.log);
        let request = match obs.request {
            Some(text) if !text.is_empty() => Json::parse(text)?,
            _ => Json::Obj(Vec::new()),
        };
        let tracker = &self.trackers[row];
        let dex = &*self.dex;
        out.guard[row * GUARD_PER_ROW..(row + 1) * GUARD_PER_ROW].fill(false);
        let turn = tracker.turn;
        let (own_side, foe_side) = (tracker.own, tracker.foe());
        let active_json = request.at("active");
        let actives: &[Json] = if active_json.truthy() { active_json.items() } else { &[] };
        let move_request = !request.at("wait").truthy() && !request.at("forceSwitch").truthy();
        let own_names = &mut self.own_names[row];
        own_names.clear();
        let mut own_fainted = 0usize;
        let team = request.at("side").at("pokemon").items();
        let team = &team[..team.len().min(6)];
        for (index, pokemon) in team.iter().enumerate() {
            let ident = pokemon.at("ident").as_str().ok_or("a request Pokemon has no ident")?;
            let name = ident.split_once(": ").ok_or_else(|| format!("request ident {ident:?} has no \": \""))?.1;
            own_names.push(name.to_string());
            let s = self.statics.own(dex, pokemon);
            let text = pokemon.get("condition").map_or("0 fnt", Json::str_or_empty);
            let (hp, mut max_hp, status) = tracker::condition(text).map_err(|_| format!("bad condition {text:?}"))?;
            if status == "fnt" {
                own_fainted += 1;
                max_hp = s.max_hp;
            }
            let mut d = [0.0f64; N_DYNAMIC];
            d[pf::PRESENT] = 1.0;
            d[pf::OWN] = 1.0;
            let is_active = index < 2 && pokemon.at("active").truthy();
            let mon = tracker.get(own_side, name);
            dynamic(&mut d, hp / max_hp, status, mon, turn, is_active, index);
            d[pf::STAT_HP] = max_hp / f64::from(STAT_SCALE);
            d[pf::TERASTALLIZED] = f64::from(u8::from(pokemon.at("terastallized").truthy()));
            let moves = pokemon.at("moves").items();
            let moves = &moves[..moves.len().min(4)];
            for m in 0..moves.len() {
                d[pf::PP0 + m] = 1.0;
            }
            if is_active && index < actives.len() && actives[index].truthy() {
                let active = &actives[index];
                let active_moves = active.at("moves").items();
                // Guards only for a Pokemon the tracker places in this slot: our disguised Zoroark is on record under
                // the disguise, so its own record (if any) holds a past stint's timing.
                if let Some(mon) = mon
                    && move_request
                    && status != "fnt"
                    && tracker.active_at(&format!("{own_side}{}", ["a", "b"][index])).is_some_and(|k| k.0 == own_side && k.1 == name)
                {
                    for (m, info) in active_moves.iter().take(4).enumerate() {
                        let mv = info.get("id").unwrap_or(&EMPTY_STR).as_str().unwrap_or("");
                        let g = row * GUARD_PER_ROW + (index * 4 + m) * 2;
                        out.guard[g + STALE] = FIRST_TURN_ONLY.contains(&mv) && mon.moves_since_switch > 0;
                        out.guard[g + REPEAT_PROTECT] =
                            dex.move_stalling[dex.move_index(mv) as usize] && mon.protect_turn >= turn - 1;
                    }
                }
                d[pf::TERA_AVAILABLE] = f64::from(u8::from(active.at("canTerastallize").truthy()));
                d[pf::TRAPPED] = f64::from(u8::from(active.at("trapped").truthy() || active.at("maybeTrapped").truthy()));
                for (m, mv) in moves.iter().enumerate() {
                    // by_id = {m.get("id"): m for m in moves}: the last request move with that id.
                    if let Some(info) = active_moves.iter().rev().find(|info| py_eq(info.at("id"), mv)) {
                        let pp = info.get("pp").map_or(0.0, |v| v.as_f64().unwrap_or(0.0));
                        let maxpp = info.get("maxpp").map_or(1.0, |v| v.as_f64().unwrap_or(1.0));
                        d[pf::PP0 + m] = pp / max1(maxpp);
                        d[pf::DISABLED0 + m] = f64::from(u8::from(info.at("disabled").truthy()));
                    }
                }
            }
            put_token(out, row, OWN + index, &d, Some(&s));
        }
        let zero = [0.0f64; N_DYNAMIC];
        for index in team.len()..6 {
            put_token(out, row, OWN + index, &zero, None);
        }

        // Foe tokens: active a, active b, then the rest in reveal order.
        let foe_a = tracker.active_at(&format!("{foe_side}a"));
        let foe_b = tracker.active_at(&format!("{foe_side}b"));
        let mut foe_keys: Vec<Option<&Key>> = vec![foe_a, foe_b];
        foe_keys.extend(tracker.foe_order.iter().filter(|k| foe_a != Some(*k) && foe_b != Some(*k)).take(4).map(Some));
        let foe_names = &mut self.foe_names[row];
        *foe_names = Default::default();
        let (mut foe_fainted, mut revealed) = (0usize, 0usize);
        let foe_tera_available = f64::from(u8::from(!tracker.tera_used[usize::from(foe_side == "p2")]));
        for k in 0..6 {
            let key = foe_keys.get(k).copied().flatten();
            let mon = key.and_then(|key| tracker.get(&key.0, &key.1));
            let (Some(key), Some(mon)) = (key, mon.filter(|m| !m.species.is_empty())) else {
                put_token(out, row, FOE_A + k, &zero, None);
                continue;
            };
            revealed += 1;
            foe_names[k] = Some(key.1.clone());
            if mon.status == "fnt" {
                foe_fainted += 1;
            }
            let s = self.statics.foe(dex, mon);
            let mut d = [0.0f64; N_DYNAMIC];
            d[pf::PRESENT] = 1.0;
            dynamic(&mut d, mon.hp, &mon.status, Some(mon), turn, k < 2, k);
            d[pf::STAT_HP] = s.max_hp / f64::from(STAT_SCALE);
            d[pf::TERASTALLIZED] = f64::from(u8::from(mon.terastallized));
            d[pf::TERA_AVAILABLE] = foe_tera_available;
            put_token(out, row, FOE_A + k, &d, Some(&s));
        }

        let mut f = [0.0f64; N_FIELD_FLOATS];
        let (weather, since) = (&tracker.weather.0, tracker.weather.1);
        if let Some(col) = weather_column(weather) {
            f[col] = 1.0;
            if EXTREME_WEATHER.contains(&weather.as_str()) {
                f[ff::EXTREME_WEATHER] = 1.0;
            }
            f[ff::WEATHER_TURNS] = (turn - since).min(8) as f64 / 8.0;
        }
        let (terrain, since) = (&tracker.terrain.0, tracker.terrain.1);
        if let Some(i) = TERRAINS.iter().position(|t| t == terrain) {
            f[ff::ELECTRICTERRAIN + i] = 1.0;
            f[ff::TERRAIN_TURNS] = (turn - since).min(8) as f64 / 8.0;
        }
        if tracker.trickroom >= 0 {
            f[ff::TRICKROOM] = 1.0;
            f[ff::TRICKROOM_TURNS] = (turn - tracker.trickroom).min(5) as f64 / 5.0;
        }
        f[ff::GRAVITY] = f64::from(u8::from(tracker.gravity));
        let own_index = usize::from(own_side == "p2");
        for (conditions, turns, side) in [(ff::OWN_TAILWIND, ff::OWN_TAILWIND_TURNS, own_index), (ff::FOE_TAILWIND, ff::FOE_TAILWIND_TURNS, 1 - own_index)] {
            for (name, (start, layers)) in &tracker.sides[side] {
                let Some(i) = SIDE_CONDITIONS.iter().position(|c| c == name) else { continue };
                f[conditions + i] = match name.as_str() {
                    "spikes" => *layers as f64 / 3.0,
                    "toxicspikes" => *layers as f64 / 2.0,
                    _ => 1.0,
                };
                if let Some(t) = TIMED_SIDE.iter().position(|c| c == name) {
                    f[turns + t] = (turn - start).min(8) as f64 / 8.0;
                }
            }
        }
        f[ff::TURN] = turn.min(60) as f64 / 30.0;
        f[ff::OWN_TERA_USED] = f64::from(u8::from(tracker.tera_used[own_index]));
        f[ff::FOE_TERA_USED] = f64::from(u8::from(tracker.tera_used[1 - own_index]));
        f[ff::REQUEST_WAIT] = f64::from(u8::from(request.at("wait").truthy()));
        f[ff::REQUEST_SWITCH] = f64::from(u8::from(request.at("forceSwitch").truthy()));
        f[ff::REQUEST_MOVE] = f64::from(u8::from(active_json.truthy()));
        f[ff::OWN_ALIVE] = (6.0 - own_fainted as f64) / 6.0;
        f[ff::FOE_FAINTED] = foe_fainted as f64 / 6.0;
        f[ff::FOE_REVEALED] = revealed as f64 / 6.0;
        f[ff::FOE_UNREVEALED] = (6.0 - revealed as f64) / 6.0;
        for (o, x) in out.field[row * FIELD_PER_ROW..(row + 1) * FIELD_PER_ROW].iter_mut().zip(&f) {
            *o = *x as f32;
        }
        out.fainted[row * FAINTED_PER_ROW] = own_fainted as f32;
        out.fainted[row * FAINTED_PER_ROW + 1] = foe_fainted as f32;
        Ok(())
    }

    /// encoder.py `match_partners`: `foe_match` of every row from the names recorded by `encode_row`.
    pub fn match_partners(&self, out: &mut Outputs<'_>) {
        out.foe_match.fill(-1);
        let rows = self.rows().min(out.foe_match.len() / FOE_MATCH_PER_ROW);
        for row in (0..rows.saturating_sub(1)).step_by(2) {
            for (me, other) in [(row, row + 1), (row + 1, row)] {
                for (k, name) in self.foe_names[me].iter().enumerate() {
                    if let Some(name) = name {
                        // {name: j for j, name in enumerate(own_names)}: the last index wins.
                        let j = self.own_names[other].iter().rposition(|n| n == name);
                        out.foe_match[me * FOE_MATCH_PER_ROW + k] = j.map_or(-1, |j| j as i64);
                    }
                }
            }
        }
    }

    /// encoder.py `damage_features`: `action_features` and `token_features` of every row.
    pub fn damage_features(&self, out: &mut Outputs<'_>) {
        let rows = self.rows();
        for row in 0..rows {
            self.damage.row(
                &out.ids[row * IDS_PER_ROW..(row + 1) * IDS_PER_ROW],
                &out.floats[row * FLOATS_PER_ROW..(row + 1) * FLOATS_PER_ROW],
                &out.field[row * FIELD_PER_ROW..(row + 1) * FIELD_PER_ROW],
                &mut out.action_features[row * ACTION_PER_ROW..(row + 1) * ACTION_PER_ROW],
                &mut out.token_features[row * TOKEN_FEATURES_PER_ROW..(row + 1) * TOKEN_FEATURES_PER_ROW],
            );
        }
    }

    /// encoder.py `encode_batch`: every row (`observations[row]`), then partners and damage features.
    pub fn encode_batch<S: AsRef<str>>(&mut self, observations: &[Observation<'_, S>], out: &mut Outputs<'_>) -> Result<(), String> {
        let rows = out.rows()?;
        if rows != self.rows() || observations.len() != rows {
            return Err(format!("encoder has {} rows, got {} observations and arrays for {rows} rows", self.rows(), observations.len()));
        }
        for (row, obs) in observations.iter().enumerate() {
            self.encode_row(row, obs, out)?;
        }
        self.match_partners(out);
        self.damage_features(out);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dex() -> Arc<Dex> {
        Dex::shared(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json")).expect("dex.json")
    }

    struct Buffers(Vec<i64>, Vec<f32>, Vec<f32>, Vec<i64>, Vec<f32>, Vec<bool>, Vec<f32>, Vec<f32>);

    impl Buffers {
        fn new(rows: usize) -> Self {
            Buffers(
                vec![0; rows * IDS_PER_ROW],
                vec![0.0; rows * FLOATS_PER_ROW],
                vec![0.0; rows * FIELD_PER_ROW],
                vec![0; rows * FOE_MATCH_PER_ROW],
                vec![0.0; rows * FAINTED_PER_ROW],
                vec![false; rows * GUARD_PER_ROW],
                vec![0.0; rows * ACTION_PER_ROW],
                vec![0.0; rows * TOKEN_FEATURES_PER_ROW],
            )
        }

        fn out(&mut self) -> Outputs<'_> {
            let Buffers(ids, floats, field, foe_match, fainted, guard, action_features, token_features) = self;
            Outputs { ids, floats, field, foe_match, fainted, guard, action_features, token_features }
        }

        fn float(&self, token: usize, col: usize) -> f32 {
            self.1[token * N_POKEMON_FLOATS + col]
        }

        fn id(&self, token: usize, col: usize) -> i64 {
            self.0[token * N_IDS + col]
        }

        fn guard(&self, slot: usize, m: usize, kind: usize) -> bool {
            self.5[(slot * 4 + m) * 2 + kind]
        }
    }

    const SWITCH_IN: [&str; 4] = ["|switch|p1a: Infernape|Infernape, L85, M|250/250", "|switch|p1b: Dipplin|Dipplin, L90|200/200",
                                  "|switch|p2a: Inteleon|Inteleon, L78, F|100/100", "|switch|p2b: Gengar|Gengar, L80, M|100/100"];

    /// test_effects.request with Infernape's moves `moves` (slot a) and Dipplin (slot b); `extra` is merged into the
    /// request object.
    fn request(moves: [&str; 4], extra: &str) -> String {
        let active = |ms: &[&str]| {
            let ms: Vec<String> = ms.iter().map(|m| format!(r#"{{"id": "{m}", "pp": 8, "maxpp": 8, "disabled": false}}"#)).collect();
            format!(r#"{{"moves": [{}], "canTerastallize": "Fire"}}"#, ms.join(", "))
        };
        let mon = |name: &str, level: u32, ms: &[&str]| {
            let ms: Vec<String> = ms.iter().map(|m| format!("\"{m}\"")).collect();
            format!(r#"{{"ident": "p1: {name}", "details": "{name}, L{level}", "condition": "200/250", "active": true,
                "stats": {{"atk": 250, "def": 180, "spa": 250, "spd": 180, "spe": 260}}, "moves": [{}], "item": "lifeorb",
                "ability": "blaze", "teraType": "Fire", "terastallized": ""}}"#, ms.join(", "))
        };
        let dipplin = ["protect", "dracometeor"];
        format!(r#"{{"active": [{}, {}], "side": {{"id": "p1", "pokemon": [{}, {}]}}{extra}}}"#,
                active(&moves), active(&dipplin), mon("Infernape", 85, &moves), mon("Dipplin", 90, &dipplin))
    }

    fn encode(encoder: &mut Encoder, b: &mut Buffers, battle_id: i64, lines: &[&str], request: &str) {
        let obs = Observation { battle_id, request: Some(request), log: lines };
        encoder.encode_row(0, &obs, &mut b.out()).unwrap();
    }

    #[test]
    fn layout_names_match_the_columns_used() {
        for (i, s) in STATUSES.iter().enumerate() {
            assert_eq!(POKEMON_FLOAT_NAMES[pf::BRN + i], *s);
        }
        for (i, s) in BOOSTS.iter().enumerate() {
            assert_eq!(POKEMON_FLOAT_NAMES[pf::BOOST_ATK + i], format!("boost_{s}"));
        }
        for (i, s) in VOLATILES.iter().enumerate() {
            assert_eq!(POKEMON_FLOAT_NAMES[pf::SUBSTITUTE + i], *s);
        }
        for (i, s) in TERRAINS.iter().enumerate() {
            assert_eq!(FIELD_FLOAT_NAMES[ff::ELECTRICTERRAIN + i], *s);
        }
        for (i, s) in SIDE_CONDITIONS.iter().enumerate() {
            assert_eq!(FIELD_FLOAT_NAMES[ff::OWN_TAILWIND + i], format!("own_{s}"));
            assert_eq!(FIELD_FLOAT_NAMES[ff::FOE_TAILWIND + i], format!("foe_{s}"));
        }
        for (i, s) in TIMED_SIDE.iter().enumerate() {
            assert_eq!(FIELD_FLOAT_NAMES[ff::OWN_TAILWIND_TURNS + i], format!("own_{s}_turns"));
            assert_eq!(FIELD_FLOAT_NAMES[ff::FOE_TAILWIND_TURNS + i], format!("foe_{s}_turns"));
        }
        assert_eq!(estimated_stats(&[80.0, 100.0, 70.0, 60.0, 70.0, 105.0], 85), [275.0, 219.0, 168.0, 151.0, 168.0, 227.0]);
    }

    #[test]
    fn guards_follow_move_actions_protect_success_and_the_request_kind() {
        let dex = dex();
        let moves = ["fakeout", "closecombat", "protect", "flareblitz"];
        let mut encoder = Encoder::new(dex, 2);
        let mut b = Buffers::new(2);
        let mut lines: Vec<&str> = SWITCH_IN.to_vec();
        lines.push("|turn|1");
        encode(&mut encoder, &mut b, 1, &lines, &request(moves, ""));
        assert!(!b.5.iter().any(|g| *g));
        assert_eq!(b.float(0, pf::FRESH), 1.0);
        let turn1 = ["|move|p1a: Infernape|Protect|p1a: Infernape", "|-singleturn|p1a: Infernape|Protect", "|cant|p1b: Dipplin|flinch", "|turn|2"];
        encode(&mut encoder, &mut b, 1, &turn1, &request(moves, ""));
        assert!(b.guard(0, 0, STALE) && b.guard(0, 2, REPEAT_PROTECT));
        assert!(!b.guard(0, 1, STALE) && !b.guard(0, 0, REPEAT_PROTECT) && !b.guard(1, 0, REPEAT_PROTECT));
        assert_eq!((b.float(0, pf::PROTECT_LAST), b.float(1, pf::FRESH)), (1.0, 0.0));
        // A wait request clears the guards; a failed Protect resets the counter.
        encode(&mut encoder, &mut b, 1, &[], &request(moves, r#", "wait": true"#));
        assert!(!b.5.iter().any(|g| *g));
        encode(&mut encoder, &mut b, 1, &["|move|p1a: Infernape|Protect||[still]", "|-fail|p1a: Infernape", "|turn|3"], &request(moves, ""));
        assert!(b.guard(0, 0, STALE) && !b.guard(0, 2, REPEAT_PROTECT));
        // A new battle id starts from an empty tracker.
        encode(&mut encoder, &mut b, 2, &[], &request(moves, ""));
        assert!(!b.5.iter().any(|g| *g));
        assert_eq!(b.float(FOE_A, pf::PRESENT), 0.0);
    }

    #[test]
    fn illusion_and_swap_move_foe_tokens() {
        let dex = dex();
        let mut encoder = Encoder::new(dex.clone(), 2);
        let mut b = Buffers::new(2);
        let moves = ["fakeout", "closecombat", "protect", "flareblitz"];
        let mut lines: Vec<&str> = SWITCH_IN.to_vec();
        lines.extend(["|turn|1", "|-boost|p2b: Gengar|spa|1", "|replace|p2b: Zoroark|Zoroark, L80, M", "|turn|2"]);
        encode(&mut encoder, &mut b, 1, &lines, &request(moves, ""));
        assert_eq!(b.id(FOE_B, I_SPECIES), dex.species_idx("Zoroark"));
        assert_eq!(b.id(FOE_B, I_ABILITY), dex.ability_index("illusion"));
        assert!((b.float(FOE_B, pf::BOOST_SPA) - 1.0 / 6.0).abs() < 1e-7);
        assert_eq!((0..6).filter(|&k| b.float(FOE_A + k, pf::PRESENT) == 1.0).count(), 2); // the Gengar record is gone
        encode(&mut encoder, &mut b, 1, &["|swap|p2a: Inteleon|1", "|turn|3"], &request(moves, ""));
        assert_eq!(b.id(FOE_A, I_SPECIES), dex.species_idx("Zoroark"));
        assert_eq!(b.id(FOE_B, I_SPECIES), dex.species_idx("Inteleon"));
        assert_eq!(b.float(FOE_B, pf::ACTIVE_B), 1.0);
    }
}
