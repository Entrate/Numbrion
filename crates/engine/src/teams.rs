//! Showdown packed team input; allocations are confined to this input boundary.
use crate::{dex, ids::*};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gender {
    #[default]
    Unspecified,
    Male,
    Female,
    Genderless,
}
impl Gender {
    pub fn protocol(self) -> &'static str {
        match self {
            Self::Male => "M",
            Self::Female => "F",
            _ => "",
        }
    }
}
#[derive(Clone, Debug)]
pub struct SetDef {
    pub name: String,
    pub species: EffectId,
    pub item: EffectId,
    pub ability: EffectId,
    pub moves: [EffectId; 4],
    pub move_count: u8,
    pub evs: [u8; 6],
    pub ivs: [u8; 6],
    pub nature: i8,
    pub level: u16,
    pub gender: Gender,
    pub shiny: bool,
    pub happiness: u8,
    pub tera_type: TypeId,
    pub pokeball: String,
    pub hp_type: String,
    pub dynamax_level: u8,
    pub gigantamax: bool,
}
impl Default for SetDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            species: EffectId::NONE,
            item: EffectId::NONE,
            ability: EffectId::NONE,
            moves: [EffectId::NONE; 4],
            move_count: 0,
            evs: [0; 6],
            ivs: [31; 6],
            nature: -1,
            level: 100,
            gender: Gender::Unspecified,
            shiny: false,
            happiness: 255,
            tera_type: TypeId::NONE,
            pokeball: "pokeball".into(),
            hp_type: String::new(),
            dynamax_level: 10,
            gigantamax: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct TeamDef {
    pub sets: [SetDef; 6],
    pub len: u8,
}
impl Default for TeamDef {
    fn default() -> Self {
        Self {
            sets: core::array::from_fn(|_| SetDef::default()),
            len: 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct TeamDefs {
    pub sides: [TeamDef; 2],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamError(pub String);
impl core::fmt::Display for TeamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for TeamError {}
fn lookup(kind: EffectKind, text: &str) -> Result<EffectId, TeamError> {
    dex::lookup(kind, text)
        .ok_or_else(|| TeamError(format!("Unknown or out-of-scope {kind:?}: {text}")))
}
fn number(text: &str, default: f64) -> f64 {
    if text.is_empty() {
        return default;
    }
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    let radix = if text.starts_with("0x") || text.starts_with("0X") {
        Some(16)
    } else if text.starts_with("0b") || text.starts_with("0B") {
        Some(2)
    } else if text.starts_with("0o") || text.starts_with("0O") {
        Some(8)
    } else {
        None
    };
    if let Some(radix) = radix {
        return u64::from_str_radix(&text[2..], radix).map_or(0.0, |n| n as f64);
    }
    match text.parse::<f64>() {
        Ok(n) if !n.is_nan() => n,
        _ => 0.0,
    }
}
fn vector(text: &str, default: u8, max: u8) -> [u8; 6] {
    let mut values = [default; 6];
    if !text.is_empty() {
        let parts: Vec<_> = text.split(',').take(6).collect();
        for (i, v) in values.iter_mut().enumerate() {
            let n = parts.get(i).map_or(0.0, |s| number(s, default as f64));
            *v = n.floor().clamp(0.0, max as f64) as u8;
        }
    }
    values
}
// Teams.unpack uses parseInt rather than Number for level.
fn level(text: &str) -> u16 {
    let mut s = text.trim_start();
    let negative = s.starts_with('-');
    if s.starts_with(['+', '-']) {
        s = &s[1..];
    }
    let radix = if s.starts_with("0x") || s.starts_with("0X") {
        s = &s[2..];
        16
    } else {
        10
    };
    let mut value = 0u32;
    let mut seen = false;
    for c in s.chars() {
        let Some(d) = c.to_digit(radix) else { break };
        seen = true;
        value = value.saturating_mul(radix).saturating_add(d);
    }
    if !seen || value == 0 {
        100
    } else if negative {
        1
    } else {
        value.min(9999) as u16
    }
}
fn truncate_utf16(text: &str, n: usize) -> String {
    let mut count = 0;
    text.chars()
        .take_while(|c| {
            count += c.len_utf16();
            count <= n
        })
        .collect()
}
impl TeamDef {
    pub fn unpack(packed: &str) -> Result<Self, TeamError> {
        if packed.is_empty() {
            return Err(TeamError("Explicit packed teams must be nonempty".into()));
        }
        let mut team = Self::default();
        for (index, record) in packed.split(']').enumerate() {
            if index >= 6 {
                return Err(TeamError("This format supports at most six Pokemon".into()));
            }
            let f: Vec<_> = record.splitn(12, '|').collect();
            if f.len() != 12 {
                return Err(TeamError(format!("Malformed packed set {}", index + 1)));
            }
            let species = lookup(
                EffectKind::Species,
                if f[1].is_empty() { f[0] } else { f[1] },
            )?;
            let s = dex::species(species);
            // Teams.unpack expands packed species names before constructor name comparison.
            let input_species = if f[1].is_empty() { f[0] } else { s.name };
            let name = if f[0].is_empty() || f[0] == input_species {
                s.base_species_name
            } else {
                f[0]
            };
            let mut set = SetDef {
                species,
                name: truncate_utf16(name, 20),
                ..SetDef::default()
            };
            if !f[2].is_empty() {
                set.item = lookup(EffectKind::Item, f[2])?
            }
            let slot = match f[3] {
                "" | "0" => Some(0),
                "1" => Some(1),
                "H" => Some(2),
                "S" => Some(3),
                _ => None,
            };
            if let Some(slot) = slot {
                let a = s.abilities[slot];
                if !a.key.is_empty() {
                    set.ability = lookup(EffectKind::Ability, a.key)?
                } else if !f[3].is_empty() {
                    return Err(TeamError("Unavailable ability slot".into()));
                }
            } else {
                set.ability = lookup(EffectKind::Ability, f[3])?
            }
            for (i, m) in f[4].split(',').enumerate() {
                if i >= 4 {
                    return Err(TeamError("This format supports at most four moves".into()));
                }
                set.moves[i] = lookup(EffectKind::Move, m)?;
                set.move_count += 1;
            }
            if !f[5].is_empty() {
                let normalized: String = f[5]
                    .bytes()
                    .filter(u8::is_ascii_alphanumeric)
                    .map(|b| b.to_ascii_lowercase() as char)
                    .collect();
                set.nature = dex::NATURES
                    .iter()
                    .position(|n| n.key == normalized)
                    .map_or(-1, |n| n as i8);
            }
            set.evs = vector(f[6], 0, 255);
            set.ivs = vector(f[8], 31, 31);
            set.gender = match f[7] {
                "M" => Gender::Male,
                "F" => Gender::Female,
                "N" => Gender::Genderless,
                _ => Gender::Unspecified,
            };
            set.shiny = !f[9].is_empty();
            set.level = level(f[10]);
            let misc: Vec<_> = f[11].split(',').take(6).collect();
            if let Some(h) = misc.first() {
                set.happiness = number(h, 255.0).floor().clamp(0.0, 255.0) as u8;
            }
            set.hp_type = misc.get(1).unwrap_or(&"").to_string();
            if let Some(ball) = misc.get(2).filter(|s| !s.is_empty()) {
                set.pokeball = ball
                    .bytes()
                    .filter(u8::is_ascii_alphanumeric)
                    .map(|b| b.to_ascii_lowercase() as char)
                    .collect();
            }
            set.gigantamax = misc.get(3).is_some_and(|s| !s.is_empty());
            if let Some(d) = misc.get(4) {
                set.dynamax_level = number(d, 10.0).floor().clamp(0.0, 10.0) as u8;
            }
            set.tera_type = if let Some(t) = misc.get(5).filter(|s| !s.is_empty()) {
                dex::type_id(t).ok_or_else(|| TeamError(format!("Unknown Tera type {t}")))?
            } else {
                s.types[0]
            };
            team.sets[index] = set;
            team.len += 1;
        }
        Ok(team)
    }
}
