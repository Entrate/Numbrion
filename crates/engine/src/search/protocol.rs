//! Parsing helpers for Showdown player-view protocol lines (`|cmd|arg|...|[tag] value`), as
//! `Battle` emits them and a Showdown server sends them. Pure functions, no allocation.
use crate::{ids::SideId, teams::Gender};

/// One protocol line split at `|`: `fields[0]` is the command, the rest are its arguments and
/// tags in order. `None` for lines that do not start with `|`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line<'a> {
    pub fields: Vec<&'a str>,
}

impl<'a> Line<'a> {
    pub fn parse(line: &'a str) -> Option<Self> {
        let rest = line.strip_prefix('|')?;
        Some(Line { fields: rest.split('|').collect() })
    }

    pub fn cmd(&self) -> &'a str {
        self.fields[0]
    }

    /// Argument `i` (0-based, after the command), `""` when absent.
    pub fn arg(&self, i: usize) -> &'a str {
        self.fields.get(i + 1).copied().unwrap_or("")
    }

    /// The value of the first `[name] value` (or bare `[name]`) field, e.g. `tag("from")` on
    /// `|-heal|p1a: X|50/100|[from] item: Leftovers` is `Some("item: Leftovers")`.
    pub fn tag(&self, name: &str) -> Option<&'a str> {
        self.fields[1..].iter().find_map(|f| {
            let inner = f.strip_prefix('[')?;
            let (tag, value) = inner.split_once(']')?;
            (tag == name).then(|| value.trim_start())
        })
    }

    pub fn has_tag(&self, name: &str) -> bool {
        self.tag(name).is_some()
    }
}

/// `p2a: Name` (active, position 0 = a) or `p2: Name` (not active).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ident<'a> {
    pub side: SideId,
    pub position: Option<u8>,
    pub name: &'a str,
}

pub fn parse_ident(text: &str) -> Option<Ident<'_>> {
    let (head, name) = text.split_once(": ")?;
    let b = head.as_bytes();
    if b.len() < 2 || b[0] != b'p' || !(b'1'..=b'2').contains(&b[1]) {
        return None;
    }
    let side = SideId(b[1] - b'1');
    let position = match b.get(2) {
        None => None,
        Some(&c @ b'a'..=b'c') if b.len() == 3 => Some(c - b'a'),
        _ => return None,
    };
    Some(Ident { side, position, name })
}

/// A side-only reference `p2` / `p2: Name` (side conditions, `-cureteam`).
pub fn parse_side(text: &str) -> Option<SideId> {
    let b = text.as_bytes();
    (b.len() >= 2 && b[0] == b'p' && (b'1'..=b'2').contains(&b[1])).then(|| SideId(b[1] - b'1'))
}

/// `Muk-Alola, L82, M, shiny, tera:Fire`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Details<'a> {
    pub species: &'a str,
    pub level: u16,
    /// `Unspecified` when the details carry no gender (genderless or not shown).
    pub gender: Gender,
    pub shiny: bool,
    pub tera: Option<&'a str>,
}

pub fn parse_details(text: &str) -> Details<'_> {
    let mut parts = text.split(", ");
    let mut d = Details {
        species: parts.next().unwrap_or(""),
        level: 100,
        gender: Gender::Unspecified,
        shiny: false,
        tera: None,
    };
    for p in parts {
        match p {
            "M" => d.gender = Gender::Male,
            "F" => d.gender = Gender::Female,
            "shiny" => d.shiny = true,
            _ => {
                if let Some(l) = p.strip_prefix('L').and_then(|l| l.parse().ok()) {
                    d.level = l;
                } else if let Some(t) = p.strip_prefix("tera:") {
                    d.tera = Some(t);
                }
            }
        }
    }
    d
}

/// `169/260 par`, `46/100`, `0 fnt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Condition<'a> {
    pub hp: u32,
    /// 0 for `0 fnt`.
    pub max_hp: u32,
    /// `""` when healthy, `"fnt"` when fainted.
    pub status: &'a str,
}

pub fn parse_condition(text: &str) -> Option<Condition<'_>> {
    let (hp_text, status) = text.split_once(' ').unwrap_or((text, ""));
    if hp_text == "0" {
        return Some(Condition { hp: 0, max_hp: 0, status: "fnt" });
    }
    let (hp, max) = hp_text.split_once('/')?;
    Some(Condition { hp: hp.parse().ok()?, max_hp: max.parse().ok()?, status })
}

/// `item: Leftovers` / `ability: Intimidate` / `move: Protect` / `Protect` -> (`"item"`, `"Leftovers"`),
/// ..., (`""`, `"Protect"`).
pub fn effect_kind(text: &str) -> (&str, &str) {
    match text.split_once(": ") {
        Some((kind @ ("item" | "ability" | "move" | "pokemon"), name)) => (kind, name),
        _ => ("", text),
    }
}

/// Commands that carry no battle information, or whose text differs between the engine and a
/// Showdown server (timestamps, chat, player avatars and ratings, room/UI messages). The world
/// builder compares the rest line by line.
const UNCOMPARED: &[&str] = &[
    "t:", "player", "j", "J", "l", "L", "n", "N", "c", "c:", "chat", "raw", "html", "uhtml",
    "uhtmlchange", "inactive", "inactiveoff", "title", "init", "badge", "rated", "seed", "request",
    "message", "hidelines", "unlink", "bigerror", "error", "debug", "gametype", "gen", "tier",
    "rule", "teampreview", "clearpoke", "poke", "timestamp", "deinit", "noinit", "join", "leave",
    "name", "updateuser", "",
];

/// The line takes part in the world builder's comparison: a battle event. Empty lines, blank
/// `|` separators and the commands in `UNCOMPARED` do not.
pub fn is_compared(line: &str) -> bool {
    match line.strip_prefix('|') {
        None => false,
        Some(rest) => {
            let cmd = rest.split('|').next().unwrap_or("");
            !UNCOMPARED.contains(&cmd)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines_idents_details_and_conditions() {
        let l = Line::parse("|-heal|p1a: Muk|50/100 par|[from] item: Leftovers").unwrap();
        assert_eq!(l.cmd(), "-heal");
        assert_eq!(l.arg(0), "p1a: Muk");
        assert_eq!(l.arg(5), "");
        assert_eq!(l.tag("from"), Some("item: Leftovers"));
        assert_eq!(effect_kind(l.tag("from").unwrap()), ("item", "Leftovers"));
        assert!(!l.has_tag("of"));
        assert!(Line::parse("|-activate|p2a: X|move: Protect|[still]").unwrap().has_tag("still"));
        assert_eq!(
            parse_ident("p2b: Iron Crown"),
            Some(Ident { side: SideId(1), position: Some(1), name: "Iron Crown" })
        );
        assert_eq!(parse_ident("p1: Mr. Mime").unwrap().position, None);
        assert_eq!(parse_ident("p3a: X"), None);
        let d = parse_details("Muk-Alola, L82, M, shiny, tera:Fire");
        assert_eq!((d.species, d.level, d.gender, d.shiny, d.tera), ("Muk-Alola", 82, Gender::Male, true, Some("Fire")));
        assert_eq!(parse_details("Mesprit, L86").gender, Gender::Unspecified);
        assert_eq!(parse_details("Arceus").level, 100);
        assert_eq!(parse_condition("169/260 par"), Some(Condition { hp: 169, max_hp: 260, status: "par" }));
        assert_eq!(parse_condition("0 fnt").unwrap().status, "fnt");
        assert!(is_compared("|move|p1a: X|Protect|p1a: X"));
        assert!(!is_compared("|t:|1700000000"));
        assert!(!is_compared("|"));
        assert!(!is_compared("|player|p1|Alice|1|1500"));
    }
}
