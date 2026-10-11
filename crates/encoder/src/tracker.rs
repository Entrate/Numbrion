//! Public battle state of one row, updated from that side's player-view protocol lines (port of `Tracker` and
//! `Mon` in `training/encoder.py`, Illusion bookkeeping included).
//!
//! Strings follow Python semantics: slices count characters (never bytes), `int()` / `float()` accept surrounding
//! whitespace, a sign and digit-separating underscores. A handler stops at the first missing field or bad number and
//! keeps what it already changed, like the Python `try: handler(...) except (IndexError, ValueError): pass`.

use crate::dex::to_id;

/// A tracked Pokemon key: side ("p1" / "p2") and name.
pub type Key = (String, String);

/// Protect-like moves (`Tracker.stalling`).
const STALLING: [&str; 8] = ["protect", "detect", "spikyshield", "banefulbunker", "kingsshield", "silktrap", "obstruct", "burningbulwark"];
const VOLATILES: [&str; 10] = crate::layout::VOLATILES;
/// `protect_turn` of a Pokemon without a recent successful Protect-like move.
const NO_PROTECT: i64 = -10;

/// A handler stopped early (Python's IndexError / ValueError).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Abort;

/// `parts[i]`.
fn part<'a>(parts: &[&'a str], i: usize) -> Result<&'a str, Abort> {
    parts.get(i).copied().ok_or(Abort)
}

/// Python `s[:n]`: the first `n` characters.
pub fn head(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Python `str.isspace()` for one character (Rust's White_Space plus the ASCII separators U+001C..U+001F).
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python `s.strip()`.
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Python `s.split()`.
fn py_split(s: &str) -> impl Iterator<Item = &str> {
    s.split(is_py_space).filter(|p| !p.is_empty())
}

/// Python `s.partition(": ")` as (head, tail), the tail "" when there is no separator.
fn partition(s: &str) -> (&str, &str) {
    s.split_once(": ").unwrap_or((s, ""))
}

/// Digit-separating underscores removed ("1_000" -> "1000"); None when an underscore is not between two ASCII digits.
fn without_underscores(s: &str) -> Option<String> {
    let b = s.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'_' && !(i > 0 && b[i - 1].is_ascii_digit() && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            return None;
        }
    }
    Some(s.replace('_', ""))
}

/// Largest magnitude [`py_int`] returns: Python ints are unbounded, the clamp keeps all later arithmetic in range.
const INT_LIMIT: i64 = 1 << 60;

/// Python `int(s)` (base 10, ASCII digits). Values beyond +-2^60 saturate.
pub fn py_int(s: &str) -> Result<i64, Abort> {
    let s = py_strip(s);
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit() || c == b'_') {
        return Err(Abort);
    }
    let digits = without_underscores(digits).ok_or(Abort)?;
    let mut value: i64 = 0;
    for c in digits.bytes() {
        value = value.saturating_mul(10).saturating_add(i64::from(c - b'0')).min(INT_LIMIT);
    }
    Ok(if negative { -value } else { value })
}

/// Python `float(s)` (ASCII digits; "inf", "infinity" and "nan" in any case).
pub fn py_float(s: &str) -> Result<f64, Abort> {
    let s = py_strip(s);
    let clean = if s.contains('_') { without_underscores(s).ok_or(Abort)? } else { s.to_string() };
    clean.parse::<f64>().map_err(|_| Abort)
}

/// `condition("169/260 par")` -> (hp, max hp, status); fainted gives (0, 1, "fnt").
pub fn condition(text: &str) -> Result<(f64, f64, &str), Abort> {
    let mut parts = py_split(text);
    let Some(first) = parts.next() else { return Ok((0.0, 1.0, "fnt")) };
    if first == "0" || py_split(text).any(|p| p == "fnt") {
        return Ok((0.0, 1.0, "fnt"));
    }
    let status = parts.next().unwrap_or("");
    let mut hp = first.split('/');
    let (Some(now), Some(max), None) = (hp.next(), hp.next(), hp.next()) else { return Ok((1.0, 1.0, status)) };
    let now = py_float(now)?;
    let max = py_float(max)?;
    // Python's max(max, 1.0) keeps its first argument unless the second is larger (NaN stays NaN).
    Ok((now, if 1.0 > max { 1.0 } else { max }, status))
}

/// `details("Inteleon, L78, F, tera:Water")` -> (species, level, tera type or "").
pub fn details(text: &str) -> (&str, i64, &str) {
    let mut parts = text.split(',').map(py_strip);
    let species = parts.next().unwrap_or("");
    let (mut level, mut tera) = (100, "");
    for p in parts {
        // Python's isdigit() also accepts non-ASCII digits; those never occur in Showdown details.
        if let Some(digits) = p.strip_prefix('L')
            && !digits.is_empty()
            && digits.bytes().all(|c| c.is_ascii_digit())
        {
            level = py_int(digits).unwrap_or(level);
        } else if let Some(t) = p.strip_prefix("tera:") {
            tera = t;
        }
    }
    (species, level, tera)
}

/// What is known about a Pokemon (`KNOWLEDGE`), saved at each switch-in for the Illusion bookkeeping.
#[derive(Clone, Debug, PartialEq)]
pub struct Knowledge {
    pub hp: f64,
    pub status: String,
    pub item: Option<String>,
    pub item_lost: bool,
    pub ability: Option<String>,
    pub tera: String,
    pub terastallized: bool,
}

impl Default for Knowledge {
    fn default() -> Self {
        let m = Mon::default();
        m.knowledge()
    }
}

#[derive(Clone, Debug)]
pub struct Mon {
    pub species: String,
    pub level: i64,
    /// HP fraction.
    pub hp: f64,
    pub status: String,
    /// Boost stages in insertion order (a Python dict).
    pub boosts: Vec<(String, i64)>,
    /// Volatile id -> start turn ("perish" -> remaining count), in insertion order.
    pub volatiles: Vec<(String, i64)>,
    pub switch_turn: i64,
    pub moves_since_switch: i64,
    /// Turn of the last successful Protect-like move (a failed one resets it).
    pub protect_turn: i64,
    /// A Protect-like move was just used; the next line tells success (-singleturn) or -fail.
    pub stall_pending: bool,
    /// Revealed move ids in order (at most 4).
    pub moves: Vec<String>,
    /// None unknown, "" none/lost, else the item id.
    pub item: Option<String>,
    pub item_lost: bool,
    pub ability: Option<String>,
    pub tera: String,
    pub terastallized: bool,
    /// Illusion bookkeeping: switch-ins under this name, what was known before the current one, every move used
    /// during the stint and the moves the stint added to `moves`.
    pub appearances: i64,
    pub before_stint: Option<Knowledge>,
    pub stint_moves: Vec<String>,
    pub stint_added: Vec<String>,
}

impl Default for Mon {
    fn default() -> Self {
        Mon {
            species: String::new(),
            level: 100,
            hp: 1.0,
            status: String::new(),
            boosts: Vec::new(),
            volatiles: Vec::new(),
            switch_turn: 0,
            moves_since_switch: 0,
            protect_turn: NO_PROTECT,
            stall_pending: false,
            moves: Vec::new(),
            item: None,
            item_lost: false,
            ability: None,
            tera: String::new(),
            terastallized: false,
            appearances: 0,
            before_stint: None,
            stint_moves: Vec::new(),
            stint_added: Vec::new(),
        }
    }
}

impl Mon {
    fn knowledge(&self) -> Knowledge {
        Knowledge {
            hp: self.hp,
            status: self.status.clone(),
            item: self.item.clone(),
            item_lost: self.item_lost,
            ability: self.ability.clone(),
            tera: self.tera.clone(),
            terastallized: self.terastallized,
        }
    }

    fn set_knowledge(&mut self, k: &Knowledge) {
        self.hp = k.hp;
        self.status = k.status.clone();
        self.item = k.item.clone();
        self.item_lost = k.item_lost;
        self.ability = k.ability.clone();
        self.tera = k.tera.clone();
        self.terastallized = k.terastallized;
    }

    pub fn boost(&self, stat: &str) -> Option<i64> {
        self.boosts.iter().find(|(k, _)| k == stat).map(|(_, v)| *v)
    }

    pub fn volatile(&self, name: &str) -> Option<i64> {
        self.volatiles.iter().find(|(k, _)| k == name).map(|(_, v)| *v)
    }
}

/// Python `d[key] = value` on an insertion-ordered list of pairs.
fn dict_set(d: &mut Vec<(String, i64)>, key: &str, value: i64) {
    match d.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => d.push((key.to_string(), value)),
    }
}

/// Python `d.pop(key, None)`.
fn dict_pop<V>(d: &mut Vec<(String, V)>, key: &str) {
    if let Some(i) = d.iter().position(|(k, _)| k == key) {
        d.remove(i);
    }
}

fn side_index(side: &str) -> Option<usize> {
    match side {
        "p1" => Some(0),
        "p2" => Some(1),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Tracker {
    /// 0 for p1, 1 for p2.
    pub side: usize,
    /// "p1" or "p2".
    pub own: &'static str,
    pub battle_id: i64,
    pub turn: i64,
    /// Every Pokemon seen, keyed by (side, name); the order carries no meaning.
    pub mons: Vec<(Key, Mon)>,
    /// Position ("p2a") -> the Pokemon there; `|swap|` can store None.
    pub active: Vec<(String, Option<Key>)>,
    /// Foe keys in reveal order.
    pub foe_order: Vec<Key>,
    /// (weather id, start turn), ("", 0) for none.
    pub weather: (String, i64),
    /// (terrain id, start turn), ("", 0) for none.
    pub terrain: (String, i64),
    /// Start turn of Trick Room, -1 for none.
    pub trickroom: i64,
    pub gravity: bool,
    /// Per side (p1, p2): condition id -> (start turn, layers), in insertion order.
    pub sides: [Vec<(String, (i64, i64))>; 2],
    pub tera_used: [bool; 2],
}

impl Tracker {
    /// `Tracker(side)` for side 0 (p1) or 1 (p2).
    pub fn new(side: usize) -> Self {
        let own = if side == 0 { "p1" } else { "p2" };
        let mut t = Tracker {
            side,
            own,
            battle_id: -1,
            turn: 0,
            mons: Vec::new(),
            active: Vec::new(),
            foe_order: Vec::new(),
            weather: (String::new(), 0),
            terrain: (String::new(), 0),
            trickroom: -1,
            gravity: false,
            sides: [Vec::new(), Vec::new()],
            tera_used: [false; 2],
        };
        t.reset(-1);
        t
    }

    pub fn reset(&mut self, battle_id: i64) {
        self.battle_id = battle_id;
        self.turn = 0;
        self.mons.clear();
        self.active.clear();
        self.foe_order.clear();
        self.weather = (String::new(), 0);
        self.terrain = (String::new(), 0);
        self.trickroom = -1;
        self.gravity = false;
        self.sides = [Vec::new(), Vec::new()];
        self.tera_used = [false; 2];
    }

    pub fn battle_id(&self) -> i64 {
        self.battle_id
    }

    /// The foe side, "p2" for p1 and "p1" for p2.
    pub fn foe(&self) -> &'static str {
        if self.own == "p1" { "p2" } else { "p1" }
    }

    fn find(&self, side: &str, name: &str) -> Option<usize> {
        self.mons.iter().position(|((s, n), _)| s == side && n == name)
    }

    /// `mons.get((side, name))`.
    pub fn get(&self, side: &str, name: &str) -> Option<&Mon> {
        self.find(side, name).map(|i| &self.mons[i].1)
    }

    /// `active.get(position)` (None for a missing position or a stored None).
    pub fn active_at(&self, position: &str) -> Option<&Key> {
        self.active.iter().find(|(p, _)| p == position).and_then(|(_, k)| k.as_ref())
    }

    fn set_active(&mut self, position: &str, key: Option<Key>) {
        match self.active.iter_mut().find(|(p, _)| p == position) {
            Some(slot) => slot.1 = key,
            None => self.active.push((position.to_string(), key)),
        }
    }

    /// `mon(ident)`: the index of the tracked Pokemon of `ident`, created for sides p1 / p2 (a new foe joins the
    /// reveal order).
    fn mon(&mut self, ident: &str) -> Option<usize> {
        let (side, name) = partition(ident);
        if name.is_empty() || side.chars().nth(1).is_none() {
            return None;
        }
        let side = head(side, 2);
        if let Some(i) = self.find(side, name) {
            return Some(i);
        }
        if side != "p1" && side != "p2" {
            return None;
        }
        let key = (side.to_string(), name.to_string());
        if side != self.own {
            self.foe_order.push(key.clone());
        }
        self.mons.push((key, Mon::default()));
        Some(self.mons.len() - 1)
    }

    /// `known(ident)`: the tracked Pokemon of `ident`, without creating one.
    fn known(&self, ident: &str) -> Option<usize> {
        let (side, name) = partition(ident);
        if name.is_empty() { None } else { self.find(head(side, 2), name) }
    }

    /// `feed(lines)`.
    pub fn feed<S: AsRef<str>>(&mut self, lines: &[S]) {
        let mut parts: Vec<&str> = Vec::new();
        for line in lines {
            let line = line.as_ref();
            if line.len() < 2 || !line.starts_with('|') {
                continue;
            }
            parts.clear();
            parts.extend(line.split('|'));
            // An Err is the Python handler's swallowed IndexError / ValueError: its earlier effects stay.
            let _ = self.handle(&parts);
            if line.contains("[from] ability:") || line.contains("[from] item:") {
                self.attribute(&parts);
            }
        }
    }

    /// `HANDLERS[parts[1]](self, parts)`.
    fn handle(&mut self, parts: &[&str]) -> Result<(), Abort> {
        match parts[1] {
            "switch" | "drag" | "replace" => self.switch(parts),
            "detailschange" | "-formechange" => self.detailschange(parts),
            "-damage" | "-heal" | "-sethp" => self.hp(parts),
            "faint" => self.faint(parts),
            "-status" => self.status(parts),
            "-curestatus" => self.curestatus(parts),
            "-cureteam" => self.cureteam(parts),
            "-boost" | "-unboost" => self.boost(parts),
            "-setboost" => self.setboost(parts),
            "-clearboost" | "-clearnegativeboost" | "-clearpositiveboost" | "-invertboost" => self.clearboost(parts),
            "-clearallboost" => {
                for (_, mon) in &mut self.mons {
                    mon.boosts.clear();
                }
                Ok(())
            }
            "-copyboost" => self.copyboost(parts),
            "move" => self.r#move(parts),
            "-item" => self.item(parts),
            "-enditem" => self.enditem(parts),
            "-ability" => self.ability(parts),
            "-terastallize" => self.terastallize(parts),
            "-start" => self.start(parts),
            "-end" => self.end(parts),
            "-weather" => self.weather(parts),
            "-fieldstart" => self.fieldstart(parts),
            "-fieldend" => self.fieldend(parts),
            "-sidestart" => self.sidestart(parts),
            "-sideend" => self.sideend(parts),
            "turn" => {
                self.turn = py_int(part(parts, 2)?)?;
                Ok(())
            }
            "swap" => self.swap(parts),
            "cant" => self.cant(parts),
            "-singleturn" => {
                if let Some(i) = self.known(part(parts, 2)?) {
                    self.mons[i].1.stall_pending = false;
                }
                Ok(())
            }
            "-fail" => self.fail(parts),
            _ => Ok(()),
        }
    }

    /// `_attribute`: an "[from] ability: X" / "[from] item: X" source reveals the ability or item of its owner
    /// ("[of] owner", else the line's Pokemon).
    fn attribute(&mut self, parts: &[&str]) {
        let (mut source, mut owner) = (None, None);
        for p in parts.iter().skip(2) {
            if p.starts_with("[from] ability: ") || p.starts_with("[from] item: ") {
                source = Some(*p);
            } else if let Some(o) = p.strip_prefix("[of] ") {
                owner = Some(o);
            }
        }
        let Some(source) = source else { return };
        if owner.is_none() && parts.len() > 2 && matches!(head(parts[2], 2), "p1" | "p2") && parts[2].contains(": ") {
            owner = Some(parts[2]);
        }
        let Some(owner) = owner.filter(|o| !o.is_empty()) else { return };
        let Some(i) = self.mon(owner) else { return };
        let (kind, name) = partition(&source[7..]);
        let mon = &mut self.mons[i].1;
        if kind == "ability" {
            mon.ability = Some(to_id(name));
        } else if kind == "item" && mon.item.is_none() {
            mon.item = Some(to_id(name));
        }
    }

    fn switch(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let ident = part(parts, 2)?;
        let position = ident.split(':').next().unwrap_or("");
        let replace = parts[1] == "replace";
        if replace {
            self.reveal_illusion(head(position, 3), ident)?;
        }
        let Some(i) = self.mon(ident) else { return Ok(()) };
        let turn = self.turn;
        let mon = &mut self.mons[i].1;
        if !replace {
            mon.before_stint = Some(mon.knowledge());
            mon.appearances += 1;
            mon.stint_moves.clear();
            mon.stint_added.clear();
        }
        let (species, level, tera) = details(part(parts, 3)?);
        mon.species = species.to_string();
        mon.level = level;
        if !tera.is_empty() {
            mon.tera = tera.to_string();
            mon.terastallized = true;
        }
        if parts.len() > 4 {
            let (hp, max_hp, status) = condition(parts[4])?;
            mon.hp = hp / max_hp;
            mon.status = status.to_string();
        }
        if !replace {
            mon.boosts.clear();
            mon.volatiles.clear();
            mon.switch_turn = turn;
            mon.moves_since_switch = 0;
            mon.protect_turn = NO_PROTECT; // switching out clears the Protect counter
            mon.stall_pending = false;
        }
        let name = ident.split_once(": ").ok_or(Abort)?.1;
        let key = (head(position, 2).to_string(), name.to_string());
        self.set_active(head(position, 3), Some(key));
        Ok(())
    }

    /// `_reveal_illusion`: `|replace|` (Illusion broke). What was observed under the disguise belongs to the real
    /// Pokemon: the stint's HP, status, boosts, volatiles, timing, every move used, and any item, ability or Tera
    /// observed during the stint; its ability is Illusion. The disguise's record is dropped when that switch-in was its
    /// only appearance, otherwise what was known before the switch-in returns.
    fn reveal_illusion(&mut self, position: &str, ident: &str) -> Result<(), Abort> {
        let old_key = self.active_at(position).cloned();
        let real = self.mon(ident);
        let new_name = ident.split_once(": ").ok_or(Abort)?.1;
        let Some(old_key) = old_key else { return Ok(()) };
        if old_key.0 == head(ident, 2) && old_key.1 == new_name {
            return Ok(());
        }
        let (Some(d), Some(r)) = (self.find(&old_key.0, &old_key.1), real) else { return Ok(()) };
        let disguise = &mut self.mons[d].1;
        let before = disguise.before_stint.clone().unwrap_or_default();
        let gained = disguise.knowledge();
        // Python aliases the disguise's dicts into the real record, then rebinds or drops the disguise's.
        let boosts = std::mem::take(&mut disguise.boosts);
        let volatiles = std::mem::take(&mut disguise.volatiles);
        let (switch_turn, moves_since_switch, protect_turn) = (disguise.switch_turn, disguise.moves_since_switch, disguise.protect_turn);
        let stint_moves = disguise.stint_moves.clone();
        let real = &mut self.mons[r].1;
        real.hp = gained.hp;
        real.status = gained.status;
        if gained.item != before.item {
            real.item = gained.item;
        }
        if gained.item_lost != before.item_lost {
            real.item_lost = gained.item_lost;
        }
        if gained.ability != before.ability {
            real.ability = gained.ability;
        }
        if gained.tera != before.tera {
            real.tera = gained.tera;
        }
        if gained.terastallized != before.terastallized {
            real.terastallized = gained.terastallized;
        }
        real.ability = Some("illusion".to_string());
        real.boosts = boosts;
        real.volatiles = volatiles;
        real.switch_turn = switch_turn;
        real.moves_since_switch = moves_since_switch;
        real.protect_turn = protect_turn;
        real.appearances += 1;
        real.before_stint = None;
        real.stint_added.clear();
        for m in &stint_moves {
            if !real.moves.contains(m) && real.moves.len() < 4 {
                real.moves.push(m.clone());
                real.stint_added.push(m.clone());
            }
        }
        real.stint_moves = stint_moves;
        let disguise = &mut self.mons[d].1;
        if disguise.appearances <= 1 {
            self.mons.remove(d);
            if let Some(j) = self.foe_order.iter().position(|k| *k == old_key) {
                self.foe_order.remove(j);
            }
        } else {
            disguise.appearances -= 1;
            disguise.set_knowledge(&before);
            let added = std::mem::take(&mut disguise.stint_added);
            disguise.moves.retain(|m| !added.contains(m));
            disguise.boosts.clear();
            disguise.volatiles.clear();
            disguise.stint_moves.clear();
        }
        Ok(())
    }

    fn detailschange(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let (species, _, tera) = details(part(parts, 3)?);
            let mon = &mut self.mons[i].1;
            mon.species = species.to_string();
            if !tera.is_empty() {
                mon.tera = tera.to_string();
                mon.terastallized = true;
            }
        }
        Ok(())
    }

    fn hp(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?)
            && parts.len() > 3
        {
            let (hp, max_hp, status) = condition(parts[3])?;
            let mon = &mut self.mons[i].1;
            mon.hp = hp / max_hp;
            mon.status = status.to_string();
        }
        Ok(())
    }

    fn faint(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let mon = &mut self.mons[i].1;
            mon.hp = 0.0;
            mon.status = "fnt".to_string();
        }
        Ok(())
    }

    fn status(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            self.mons[i].1.status = part(parts, 3)?.to_string();
        }
        Ok(())
    }

    fn curestatus(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let mon = &mut self.mons[i].1;
            if mon.status != "fnt" {
                mon.status.clear();
            }
        }
        Ok(())
    }

    fn cureteam(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let side = head(part(parts, 2)?, 2);
        for ((s, _), mon) in &mut self.mons {
            if s == side && mon.status != "fnt" {
                mon.status.clear();
            }
        }
        Ok(())
    }

    fn boost(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let sign = if parts[1] == "-boost" { 1 } else { -1 };
            let stat = part(parts, 3)?;
            let amount = py_int(part(parts, 4)?)?;
            let mon = &mut self.mons[i].1;
            let value = (mon.boost(stat).unwrap_or(0) + sign * amount).clamp(-6, 6);
            dict_set(&mut mon.boosts, stat, value);
        }
        Ok(())
    }

    fn setboost(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let value = py_int(part(parts, 4)?)?.clamp(-6, 6);
            dict_set(&mut self.mons[i].1.boosts, parts[3], value);
        }
        Ok(())
    }

    fn clearboost(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let boosts = &mut self.mons[i].1.boosts;
            match parts[1] {
                "-clearnegativeboost" => boosts.retain(|(_, v)| *v > 0),
                "-clearpositiveboost" => boosts.retain(|(_, v)| *v < 0),
                "-invertboost" => boosts.iter_mut().for_each(|(_, v)| *v = -*v),
                _ => boosts.clear(),
            }
        }
        Ok(())
    }

    fn copyboost(&mut self, parts: &[&str]) -> Result<(), Abort> {
        // The source is created before parts[3] is read, as in Python.
        let source = self.mon(part(parts, 2)?);
        let target = self.mon(part(parts, 3)?);
        if let (Some(s), Some(t)) = (source, target) {
            self.mons[s].1.boosts = self.mons[t].1.boosts.clone();
        }
        Ok(())
    }

    fn r#move(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let Some(i) = self.mon(part(parts, 2)?) else { return Ok(()) };
        let mv = to_id(part(parts, 3)?);
        let rest = parts[4.min(parts.len())..].concat();
        let turn = self.turn;
        let mon = &mut self.mons[i].1;
        if !rest.contains("Magic Bounce") {
            // A bounced move is not a move action (Dancer's copy is).
            mon.moves_since_switch += 1;
        }
        if STALLING.contains(&mv.as_str()) {
            mon.protect_turn = turn;
            mon.stall_pending = true;
        }
        if mv == "struggle" || rest.contains("[from]") {
            return Ok(());
        }
        if !mon.stint_moves.contains(&mv) {
            mon.stint_moves.push(mv.clone());
        }
        if !mon.moves.contains(&mv) && mon.moves.len() < 4 {
            mon.moves.push(mv.clone());
            mon.stint_added.push(mv);
        }
        Ok(())
    }

    fn item(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let item = to_id(part(parts, 3)?);
            let mon = &mut self.mons[i].1;
            mon.item = Some(item);
            mon.item_lost = false;
        }
        Ok(())
    }

    fn enditem(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let mon = &mut self.mons[i].1;
            mon.item = Some(String::new());
            mon.item_lost = true;
        }
        Ok(())
    }

    fn ability(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?)
            && parts.len() > 3
        {
            self.mons[i].1.ability = Some(to_id(parts[3]));
        }
        Ok(())
    }

    fn terastallize(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let ident = part(parts, 2)?;
        if let Some(i) = self.mon(ident) {
            let tera = part(parts, 3)?;
            let mon = &mut self.mons[i].1;
            mon.tera = tera.to_string();
            mon.terastallized = true;
            // A tracked Pokemon's side is p1 or p2.
            if let Some(s) = side_index(head(ident, 2)) {
                self.tera_used[s] = true;
            }
        }
        Ok(())
    }

    fn start(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let Some(i) = self.mon(part(parts, 2)?) else { return Ok(()) };
        let effect = effect_id(part(parts, 3)?, true);
        let turn = self.turn;
        let mon = &mut self.mons[i].1;
        if let Some(count) = crate::layout::perish_count(&effect) {
            dict_set(&mut mon.volatiles, "perish", i64::from(count));
        } else if VOLATILES.contains(&effect.as_str()) {
            dict_set(&mut mon.volatiles, &effect, turn);
        }
        Ok(())
    }

    fn end(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.mon(part(parts, 2)?) {
            let effect = effect_id(part(parts, 3)?, true);
            dict_pop(&mut self.mons[i].1.volatiles, &effect);
        }
        Ok(())
    }

    fn weather(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let name = to_id(part(parts, 2)?);
        if name == "none" {
            self.weather = (String::new(), 0);
        } else if !parts.contains(&"[upkeep]") && name != self.weather.0 {
            self.weather = (name, self.turn);
        }
        Ok(())
    }

    fn fieldstart(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let name = effect_id(part(parts, 2)?, false);
        if crate::layout::TERRAINS.contains(&name.as_str()) {
            self.terrain = (name, self.turn);
        } else if name == "trickroom" {
            self.trickroom = self.turn;
        } else if name == "gravity" {
            self.gravity = true;
        }
        Ok(())
    }

    fn fieldend(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let name = effect_id(part(parts, 2)?, false);
        if crate::layout::TERRAINS.contains(&name.as_str()) {
            self.terrain = (String::new(), 0);
        } else if name == "trickroom" {
            self.trickroom = -1;
        } else if name == "gravity" {
            self.gravity = false;
        }
        Ok(())
    }

    fn sidestart(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let side = head(part(parts, 2)?, 2);
        let name = effect_id(part(parts, 3)?, false);
        // Python raises KeyError (uncaught) for a side other than p1 / p2; such lines do not occur.
        let Some(s) = side_index(side) else { return Ok(()) };
        let turn = self.turn;
        let conditions = &mut self.sides[s];
        match conditions.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1.1 += 1,
            None => conditions.push((name, (turn, 1))),
        }
        Ok(())
    }

    fn sideend(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let side = head(part(parts, 2)?, 2);
        let Some(s) = side_index(side) else { return Ok(()) };
        let name = effect_id(part(parts, 3)?, false);
        dict_pop(&mut self.sides[s], &name);
        Ok(())
    }

    fn swap(&mut self, parts: &[&str]) -> Result<(), Abort> {
        let side = head(part(parts, 2)?, 2);
        let (pa, pb) = (format!("{side}a"), format!("{side}b"));
        let a = self.active_at(&pa).cloned();
        let b = self.active_at(&pb).cloned();
        self.set_active(&pa, b);
        self.set_active(&pb, a);
        Ok(())
    }

    /// `|cant|X|flinch` (also par, slp, frz, recharge, Truant, Taunt...): X used up a move action without a `|move|`
    /// line, so Fake Out and First Impression fail from now on. With `[of]`, X is the Pokemon whose ability blocked
    /// another's move (Armor Tail, Queenly Majesty, Dazzling); that move's user already has its `|move|` line.
    fn cant(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if parts.iter().skip(3).any(|p| p.starts_with("[of]")) {
            return Ok(());
        }
        if let Some(i) = self.known(part(parts, 2)?) {
            self.mons[i].1.moves_since_switch += 1;
        }
        Ok(())
    }

    /// A Protect-like move that fails (the 1/3 chance after a successful one) resets the counter.
    fn fail(&mut self, parts: &[&str]) -> Result<(), Abort> {
        if let Some(i) = self.known(part(parts, 2)?) {
            let turn = self.turn;
            let mon = &mut self.mons[i].1;
            if mon.stall_pending && mon.protect_turn == turn {
                mon.protect_turn = NO_PROTECT;
            }
            mon.stall_pending = false;
        }
        Ok(())
    }
}

/// `to_id(text.removeprefix("move: "))`, then also `.removeprefix("ability: ")` when `ability` is set.
fn effect_id(text: &str, ability: bool) -> String {
    let text = text.strip_prefix("move: ").unwrap_or(text);
    let text = if ability { text.strip_prefix("ability: ").unwrap_or(text) } else { text };
    to_id(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(t: &mut Tracker, lines: &[&str]) {
        t.feed(lines);
    }

    #[test]
    fn python_string_semantics() {
        assert_eq!(head("Flabébé", 5), "Flabé");
        assert_eq!(head("é", 3), "é");
        assert_eq!(py_int(" +12 "), Ok(12));
        assert_eq!(py_int("1_0"), Ok(10));
        assert_eq!(py_int("-3"), Ok(-3));
        assert!(py_int("").is_err() && py_int("1__0").is_err() && py_int("_1").is_err() && py_int("1.0").is_err());
        assert_eq!(py_int("99999999999999999999999"), Ok(INT_LIMIT));
        assert_eq!(py_float(" 1_0.5 "), Ok(10.5));
        assert!(py_float("inf").unwrap().is_infinite() && py_float("NaN").unwrap().is_nan());
        assert!(py_float("abc").is_err() && py_float("").is_err());
        assert_eq!(condition("169/260 par"), Ok((169.0, 260.0, "par")));
        assert_eq!(condition("0 fnt"), Ok((0.0, 1.0, "fnt")));
        assert_eq!(condition(""), Ok((0.0, 1.0, "fnt")));
        assert_eq!(condition("50/0"), Ok((50.0, 1.0, "")));
        assert_eq!(condition("100 tox"), Ok((1.0, 1.0, "tox")));
        assert!(condition("x/100").is_err());
        assert_eq!(details("Inteleon, L78, F, tera:Water"), ("Inteleon", 78, "Water"));
        assert_eq!(details("Zoroark"), ("Zoroark", 100, ""));
        assert_eq!(details(" Mew , Lx, L "), ("Mew", 100, ""));
    }

    #[test]
    fn malformed_lines_keep_earlier_effects() {
        let mut t = Tracker::new(0);
        feed(&mut t, &["|switch|p2a: Inteleon", "|-boost|p2a: Inteleon|atk", "|turn|x", "|-boost|p2b: Gengar|spa|2",
                       "|move|p2a: Café", "|-sidestart|p3: Nobody|Spikes", "|", "x"]);
        // The switch without details created the record (species unknown) and joined the reveal order.
        let inteleon = t.get("p2", "Inteleon").unwrap();
        assert_eq!(inteleon.species, "");
        assert_eq!(inteleon.appearances, 1);
        assert!(inteleon.boosts.is_empty());
        assert_eq!(t.get("p2", "Gengar").unwrap().boost("spa"), Some(2));
        assert!(t.get("p2", "Café").is_some());
        assert_eq!(t.foe_order.len(), 3);
        assert_eq!(t.turn, 0);
        assert!(t.active_at("p2a").is_none());
    }

    #[test]
    fn swap_exchanges_positions_and_can_store_none() {
        let mut t = Tracker::new(0);
        feed(&mut t, &["|switch|p1a: A|A, L80|100/100", "|switch|p1b: B|B, L80|100/100", "|swap|p1a: A|1",
                       "|swap|p2a: X|1"]);
        assert_eq!(t.active_at("p1a"), Some(&("p1".to_string(), "B".to_string())));
        assert_eq!(t.active_at("p1b"), Some(&("p1".to_string(), "A".to_string())));
        assert_eq!(t.active.iter().filter(|(p, k)| p.starts_with("p2") && k.is_none()).count(), 2);
    }

    #[test]
    fn broken_illusion_moves_observations_to_the_real_pokemon() {
        // The disguise was never really seen: its record disappears.
        let mut t = Tracker::new(0);
        feed(&mut t, &["|switch|p2a: Inteleon|Inteleon, L78|100/100", "|switch|p2b: Raichu|Raichu, L85|100/100",
                       "|turn|1", "|move|p2b: Raichu|Knock Off|p1a: Dipplin", "|-boost|p2b: Raichu|spa|1",
                       "|-damage|p2b: Raichu|60/100", "|-damage|p2b: Raichu|48/100|[from] item: Life Orb",
                       "|replace|p2b: Zoroark|Zoroark, L80, M", "|-end|p2b: Zoroark|Illusion", "|turn|2"]);
        assert!(t.get("p2", "Raichu").is_none());
        assert_eq!(t.foe_order, vec![("p2".into(), "Inteleon".into()), ("p2".into(), "Zoroark".into())]);
        let z = t.get("p2", "Zoroark").unwrap();
        assert!((z.hp - 0.48).abs() < 1e-12);
        assert_eq!(z.boost("spa"), Some(1));
        assert_eq!(z.moves, ["knockoff"]);
        assert_eq!(z.item.as_deref(), Some("lifeorb"));
        assert_eq!(z.ability.as_deref(), Some("illusion"));
        // The real Raichu was seen before: its record returns to what was known before the disguise.
        let mut t = Tracker::new(0);
        feed(&mut t, &["|switch|p2a: Inteleon|Inteleon, L78|100/100", "|switch|p2b: Raichu|Raichu, L85|100/100",
                       "|turn|1", "|move|p2b: Raichu|Thunderbolt|p1a: Dipplin", "|-damage|p2b: Raichu|70/100",
                       "|switch|p2b: Garchomp|Garchomp, L80|100/100", "|turn|2",
                       "|switch|p2b: Raichu|Raichu, L85|70/100", "|move|p2b: Raichu|Night Daze|p1a: Dipplin",
                       "|move|p2b: Raichu|Thunderbolt|p1a: Dipplin", "|-damage|p2b: Raichu|30/100",
                       "|replace|p2b: Zoroark|Zoroark, L80, M", "|turn|3"]);
        let raichu = t.get("p2", "Raichu").unwrap();
        assert!((raichu.hp - 0.70).abs() < 1e-12);
        assert_eq!(raichu.moves, ["thunderbolt"]);
        assert_eq!(raichu.appearances, 1);
        let z = t.get("p2", "Zoroark").unwrap();
        assert!((z.hp - 0.30).abs() < 1e-12);
        assert_eq!(z.moves, ["nightdaze", "thunderbolt"]);
        assert_eq!(t.active_at("p2b"), Some(&("p2".to_string(), "Zoroark".to_string())));
    }

    #[test]
    fn protect_counter_and_move_actions() {
        let mut t = Tracker::new(0);
        feed(&mut t, &["|switch|p1a: Infernape|Infernape, L85|250/250", "|turn|1",
                       "|move|p1a: Infernape|Protect|p1a: Infernape", "|-singleturn|p1a: Infernape|Protect",
                       "|cant|p1a: Infernape|ability: Armor Tail|Encore|[of] p2a: X", "|turn|2"]);
        let m = t.get("p1", "Infernape").unwrap();
        assert_eq!((m.protect_turn, m.stall_pending, m.moves_since_switch), (1, false, 1));
        feed(&mut t, &["|move|p1a: Infernape|Protect||[still]", "|-fail|p1a: Infernape", "|cant|p1a: Infernape|flinch"]);
        let m = t.get("p1", "Infernape").unwrap();
        assert_eq!((m.protect_turn, m.moves_since_switch), (NO_PROTECT, 3));
    }
}
