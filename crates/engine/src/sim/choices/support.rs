//! Private helpers for the choice module: JavaScript string/number semantics that the
//! Showdown parser depends on, and read-only views of the Copy state.
//! None of these draw from the PRNG or mutate anything.

use crate::{
    Battle, dex,
    ids::*,
    log::LogSink,
    state::{CellId, Pokemon, Status, mon_flags},
    teams::SetDef,
};

// ---------------------------------------------------------------------------------
// JavaScript semantics
// ---------------------------------------------------------------------------------

/// ECMAScript WhiteSpace + LineTerminator, i.e. what `\s` and `String.prototype.trim`
/// treat as a space. (Rust's `char::is_whitespace` differs: it includes U+0085 and
/// excludes U+FEFF.)
pub(crate) const fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'
            | '\u{a}'
            | '\u{b}'
            | '\u{c}'
            | '\u{d}'
            | '\u{20}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// `String.prototype.trim`.
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// Showdown `toID`: lowercase, then drop everything outside [a-z0-9] (sim/dex.ts toID).
pub(crate) fn to_id(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        }
    }
    out
}

/// `Utils.splitFirst(str, ' ')`: split at the first space; no space gives (str, "").
pub(crate) fn split_first_space(s: &str) -> (&str, &str) {
    match s.find(' ') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

/// `/^[0-9]+$/.test(s)`.
pub(crate) fn is_ascii_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `Number(s)` for a non-empty ASCII digit string.
pub(crate) fn digits_to_number(s: &str) -> f64 {
    s.parse::<f64>().unwrap_or(f64::INFINITY)
}

/// `parseInt(s)` with no radix: NaN when no digits. Handles leading whitespace, a
/// sign, the 0x/0X hexadecimal prefix, trailing garbage and values beyond 2^53.
pub(crate) fn js_parse_int(s: &str) -> f64 {
    let mut rest = s.trim_start_matches(is_js_space);
    let mut negative = false;
    if let Some(r) = rest.strip_prefix('-') {
        negative = true;
        rest = r;
    } else if let Some(r) = rest.strip_prefix('+') {
        rest = r;
    }
    let (radix, digits) = if let Some(r) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        (16u32, r)
    } else {
        (10u32, rest)
    };
    let end = digits
        .char_indices()
        .find(|(_, c)| !c.is_digit(radix))
        .map_or(digits.len(), |(i, _)| i);
    let digits = &digits[..end];
    if digits.is_empty() {
        return f64::NAN;
    }
    let value = if radix == 10 {
        digits.parse::<f64>().unwrap_or(f64::INFINITY)
    } else {
        let mut v = 0f64;
        for c in digits.chars() {
            v = v * 16.0 + f64::from(c.to_digit(16).unwrap());
        }
        v
    };
    if negative { -value } else { value }
}

/// `String(n)` for the integer-valued doubles the choice messages interpolate.
pub(crate) fn js_number_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if n == 0.0 {
        return "0".into();
    }
    if n.abs() >= 1e21 {
        // ECMAScript switches to exponent notation at 1e21: d.ddde+N.
        let s = format!("{:e}", n);
        return match s.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => s,
        };
    }
    format!("{n}")
}

// ---------------------------------------------------------------------------------
// State views
// ---------------------------------------------------------------------------------

impl<L: LogSink> Battle<L> {
    #[inline]
    pub(crate) fn ch_mon(&self, mon: MonId) -> &Pokemon {
        &self.state.pokemon[mon.0 as usize]
    }

    #[inline]
    pub(crate) fn ch_set(&self, mon: MonId) -> &SetDef {
        &self.teams.sides[mon.side().0 as usize].sets[mon.0 as usize % 6]
    }

    /// Pokemon.name (the nickname, not the species).
    pub(crate) fn ch_name(&self, mon: MonId) -> &str {
        &self.ch_set(mon).name
    }

    #[inline]
    pub(crate) fn ch_fainted(&self, mon: MonId) -> bool {
        self.ch_mon(mon).flags & mon_flags::FAINTED != 0
    }

    /// `volatiles['commanding']`. The Commander ability and its condition are outside
    /// the generated scope (docs/showdown/06-scope.md), so this is constantly false.
    #[inline]
    pub(crate) fn ch_commanding(&self, _mon: MonId) -> bool {
        false
    }

    /// `!!pokemon.switchFlag` (true or a move id).
    #[inline]
    pub(crate) fn ch_switch_flag(&self, mon: MonId) -> bool {
        mon != MonId::NONE && self.ch_mon(mon).switch_flag != EffectId::NONE
    }

    /// The occupant of active slot `index` (`side.active[index]`).
    #[inline]
    pub(crate) fn ch_active(&self, side: SideId, index: usize) -> MonId {
        self.state.sides[side.0 as usize].active[index]
    }

    /// Cell of the volatile with this condition id, if present.
    pub(crate) fn ch_volatile(&self, mon: MonId, id: EffectId) -> Option<CellId> {
        self.ch_mon(mon)
            .volatiles
            .as_slice()
            .iter()
            .copied()
            .find(|c| self.state.effects.cells[c.0 as usize].id == id)
    }

    /// `side.slotConditions[position]['revivalblessing']`.
    pub(crate) fn ch_has_revival_slot(&self, side: SideId, position: u8) -> bool {
        if position >= 2 {
            return false;
        }
        self.state.sides[side.0 as usize].slot_conditions[position as usize]
            .as_slice()
            .iter()
            .any(|c| self.state.effects.cells[c.0 as usize].id == dex::CONDITION_REVIVALBLESSING)
    }

    /// Pokemon.teraType: `set.teraType || types[0]` (pokemon.ts:452); never reassigned.
    pub(crate) fn ch_tera_type(&self, mon: MonId) -> TypeId {
        let t = self.ch_set(mon).tera_type;
        if t != TypeId::NONE {
            return t;
        }
        dex::species(self.ch_set(mon).species).types[0]
    }

    /// Tera eligibility (`pokemon.canTerastallize`) from lifecycle's cached query.
    /// Unit tests may swap in a state-derived model (`tests::LOCAL_TERA`) because
    /// lifecycle's implementation is a separate owner's work.
    #[inline]
    pub(crate) fn ch_can_tera(&self, mon: MonId) -> TypeId {
        #[cfg(test)]
        if super::tests::LOCAL_TERA.with(|c| c.get()) {
            let p = self.ch_mon(mon);
            let side = &self.state.sides[mon.side().0 as usize];
            return if side.tera_used || p.flags & mon_flags::TERA_BLOCKED != 0 {
                TypeId::NONE
            } else {
                self.ch_tera_type(mon)
            };
        }
        self.can_terastallize(mon)
    }

    /// Battle.canSwitch(side) (battle.ts:1570,1582-1590): living bench Pokemon from
    /// party index 2. A pure restatement of lifecycle's query so choices never depend
    /// on unfinished code for a trivial read.
    pub(crate) fn ch_can_switch_count(&self, side: SideId) -> u8 {
        let s = &self.state.sides[side.0 as usize];
        if s.pokemon_left == 0 {
            return 0;
        }
        let mut n = 0;
        for i in 2..s.pokemon_count as usize {
            if !self.ch_fainted(s.party[i]) {
                n += 1;
            }
        }
        n
    }
}

/// `Pokemon.status` string used in `condition`.
pub(crate) const fn status_name(s: Status) -> &'static str {
    match s {
        Status::None => "",
        Status::Burn => "brn",
        Status::Paralysis => "par",
        Status::Sleep => "slp",
        Status::Freeze => "frz",
        Status::Poison => "psn",
        Status::Toxic => "tox",
        Status::Fainted => "fnt",
    }
}

/// Name of a type (`TypeId` 1.. index `TYPE_NAMES`); empty for NONE.
pub(crate) fn type_name(t: TypeId) -> &'static str {
    if t == TypeId::NONE {
        ""
    } else {
        dex::TYPE_NAMES[t.0 as usize - 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_matches_ecmascript() {
        assert_eq!(js_parse_int("3"), 3.0);
        assert_eq!(js_parse_int("  -2abc"), -2.0);
        assert_eq!(js_parse_int("+7"), 7.0);
        assert_eq!(js_parse_int("0x1f"), 31.0);
        assert_eq!(js_parse_int("0X10"), 16.0);
        assert!(js_parse_int("abc").is_nan());
        assert!(js_parse_int("").is_nan());
        assert!(js_parse_int("-").is_nan());
        assert!(js_parse_int("0x").is_nan());
        assert_eq!(js_parse_int("1e3"), 1.0);
        assert_eq!(js_parse_int("12.9"), 12.0);
        assert_eq!(js_parse_int("\u{a0}\u{feff}5"), 5.0);
        assert_eq!(js_parse_int("99999999999999999999"), 1e20);
    }

    #[test]
    fn number_string_matches_ecmascript() {
        assert_eq!(js_number_string(7.0), "7");
        assert_eq!(js_number_string(-3.0), "-3");
        assert_eq!(js_number_string(1e20), "100000000000000000000");
        assert_eq!(js_number_string(1e21), "1e+21");
        assert_eq!(js_number_string(1.5e25), "1.5e+25");
        assert_eq!(js_number_string(f64::INFINITY), "Infinity");
        assert_eq!(js_number_string(f64::NAN), "NaN");
        assert_eq!(js_number_string(-0.0), "0");
    }

    #[test]
    fn to_id_strips_and_lowercases() {
        assert_eq!(to_id("Will-O-Wisp"), "willowisp");
        assert_eq!(to_id("  U-turn! "), "uturn");
        assert_eq!(to_id("PokéMon"), "pokmon");
        assert_eq!(to_id("\u{212a}ing"), "king");
    }

    #[test]
    fn trim_and_split() {
        assert_eq!(js_trim("\u{a0} x \u{3000}"), "x");
        assert_eq!(js_trim("\u{85}x"), "\u{85}x");
        assert_eq!(split_first_space("move 1 2"), ("move", "1 2"));
        assert_eq!(split_first_space("pass"), ("pass", ""));
        assert_eq!(split_first_space(" a"), ("", "a"));
    }
}
