//! `SetPool`: a sampler of opponent sets for determinized search worlds, drawn from generated
//! random-battle teams (packed team lines, one team per line, as `tools/oracle/gen-teams.mjs`
//! writes them) and consistent with what the opponent revealed.
//!
//! The pool is the generator's empirical distribution: every added set counts once, identical
//! sets are stored once with a count, and every draw is weighted by count. Two set differences
//! are not stored:
//!
//! * *Gender.* It is resolved at sampling time (shown gender, else the species' fixed gender,
//!   else a fair coin, as the generator and the battle constructor draw it), so sampled sets
//!   always carry an explicit gender and building a battle from them draws nothing.
//! * *Move order.* Moves are stored sorted by id and every sampled set gets a uniformly random
//!   order. The gen 9 generator shuffles each set's moves, so this reproduces its distribution
//!   while storing up to 24 orderings once (pool-400 shows each move about equally often in each
//!   slot). Move order is never visible to the opponent.
//!
//! A name equal to the species' default (`base_species_name`, what `TeamDef::unpack` gives an
//! un-nicknamed set and what the generator writes) is not stored either; it is restored on output.
//!
//! Determinism: the output depends only on the sets added (contents and order) and the
//! `SplitMix64` state. Hash maps are used for lookup only; every sampling walk is over `Vec`s in
//! insertion order.
use crate::{
    dex,
    ids::{EffectId, TypeId},
    search::SplitMix64,
    teams::{Gender, SetDef, TeamDef, TeamError},
};
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{BuildHasher, BuildHasherDefault, Hash, Hasher},
    sync::OnceLock,
};

/// What one player knows about an opponent Pokemon, as hard and soft constraints for
/// [`SetPool::sample`].
///
/// `shown_species` is a hard constraint: it is printed in every switch line, so a set that shows
/// another species can never produce the observed history. Everything else is soft: candidates
/// that satisfy the most of them win, so one unusual revealed fact (a set the pool never
/// generated, a move called through another move) degrades gracefully instead of failing.
#[derive(Clone, Copy, Debug)]
pub struct SetConstraints<'a> {
    /// Protocol nickname; the sampled set takes this name.
    pub name: &'a str,
    /// Species text of the first protocol details seen for this Pokemon ("Muk-Alola",
    /// "Zacian-Crowned"), i.e. [`SetPool::shown_species`] of its set. Later permanent forme
    /// changes (Palafin-Hero, Terapagos-Terastal) must be mapped back by the caller.
    pub shown_species: &'a str,
    /// Level shown in details (`None` = not observed; level 100 prints no `L`).
    pub level: Option<u16>,
    /// Gender shown in details; `Unspecified` (or `Genderless`) = use the species rule.
    pub gender: Gender,
    pub shiny: bool,
    /// Moves it was seen using; duplicates count once.
    pub moves: &'a [EffectId],
    /// Starting item, if revealed.
    pub item: Option<EffectId>,
    /// Starting ability, if revealed.
    pub ability: Option<EffectId>,
    /// Tera type, if it terastallized.
    pub tera: Option<TypeId>,
}

impl<'a> SetConstraints<'a> {
    /// Only the name and the shown species; no soft constraints.
    pub fn new(name: &'a str, shown_species: &'a str) -> Self {
        Self {
            name,
            shown_species,
            level: None,
            gender: Gender::Unspecified,
            shiny: false,
            moves: &[],
            item: None,
            ability: None,
            tera: None,
        }
    }
}

/// A set drawn by [`SetPool::sample`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sampled {
    pub set: SetDef,
    /// Every soft constraint was satisfied (the set is one the generator could have produced
    /// given everything revealed). `false` means the best candidates missed at least one.
    pub consistent: bool,
}

/// A deduplicated pool of random-battle sets with counts; see the module docs.
///
/// Memory per distinct set: 44 bytes of record, 4 of species index, 4 of cumulative weight
/// (built on first `sample_unrevealed`), plus about 18-36 bytes of deduplication index, which
/// [`SetPool::shrink_to_fit`] frees once loading is done.
#[derive(Clone, Default)]
pub struct SetPool {
    entries: Vec<Entry>,
    total: u32,
    /// Distinct-set indices per shown species, in insertion order.
    groups: Vec<Vec<u32>>,
    by_shown: HashMap<&'static str, u32>,
    extras: Vec<Extras>,
    extras_index: IndexMap,
    /// Deduplication index over `entries`; empty after `shrink_to_fit` until the next add.
    dedupe: IndexMap,
    /// `cumulative[i]` = sum of counts of `entries[..=i]`; reset by every add.
    cumulative: OnceLock<Vec<u32>>,
    /// Species and counts with Illusion, in pool order; empty pools take the ordinary world path.
    illusion: Vec<(EffectId, u32)>,
}

/// One distinct set and how many times it was added.
#[derive(Clone, Copy, Debug)]
struct Entry {
    key: Key,
    count: u32,
}

/// The stored part of a set (everything but gender, move order and a default name).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    species: EffectId,
    item: EffectId,
    ability: EffectId,
    /// `moves[..move_count]` sorted by id, `NONE` after.
    moves: [EffectId; 4],
    level: u16,
    evs: [u8; 6],
    ivs: [u8; 6],
    move_count: u8,
    nature: i8,
    happiness: u8,
    tera_type: TypeId,
    dynamax_level: u8,
    gigantamax: bool,
    shiny: bool,
    /// Index into `SetPool::extras`.
    extras: u32,
}

/// The string fields of a set, interned (generated pools have a handful of distinct values).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Extras {
    /// `None` = the species' `base_species_name`.
    name: Option<Box<str>>,
    pokeball: Box<str>,
    hp_type: Box<str>,
}

/// Rejection attempts in `sample_unrevealed` before falling back to an exact scan.
const REJECTION_TRIES: usize = 64;

impl SetPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds every set of one packed team. Nothing is added if the team does not parse.
    pub fn add_team(&mut self, packed: &str) -> Result<(), TeamError> {
        let team = TeamDef::unpack(packed)?;
        for set in &team.sets[..team.len as usize] {
            self.add_set(set);
        }
        Ok(())
    }

    /// Adds one set (its gender is ignored; see the module docs).
    ///
    /// Panics if the pool would exceed `u32::MAX` sets.
    pub fn add_set(&mut self, set: &SetDef) {
        self.ensure_index();
        let _ = self.cumulative.take();
        self.total = self
            .total
            .checked_add(1)
            .expect("SetPool holds at most u32::MAX sets");
        if set.ability == dex::ABILITY_ILLUSION {
            if let Some((_, count)) = self.illusion.iter_mut().find(|(s, _)| *s == set.species) {
                *count += 1;
            } else {
                self.illusion.push((set.species, 1));
            }
        }
        let key = self.key_of(set);
        let hash = hash_of(&key);
        let entries = &self.entries;
        if let Some(i) = self.dedupe.find(hash, |i| entries[i as usize].key == key) {
            self.entries[i as usize].count += 1;
            return;
        }
        let i = self.entries.len() as u32;
        self.entries.push(Entry { key, count: 1 });
        self.dedupe.insert(hash, i);
        let shown = Self::shown_species(set);
        let groups = &mut self.groups;
        let g = *self.by_shown.entry(shown).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() as u32 - 1
        });
        self.groups[g as usize].push(i);
    }

    /// Sets added, with multiplicity.
    pub fn len(&self) -> usize {
        self.total as usize
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Distinct sets stored.
    pub fn distinct(&self) -> usize {
        self.entries.len()
    }

    /// Frees the deduplication index and spare capacity. Sampling is unaffected; the next add
    /// rebuilds the index (one pass over the distinct sets).
    pub fn shrink_to_fit(&mut self) {
        self.dedupe = IndexMap::default();
        self.entries.shrink_to_fit();
        for g in &mut self.groups {
            g.shrink_to_fit();
        }
    }

    /// Exactly the species text `Battle::details` prints for this set in its first switch line:
    /// the species name, except Greninja-Bond and Rockruff-Dusk (printed as their base species)
    /// and Zacian/Zamazenta holding Rusted Sword/Shield (their BattleStart hook makes the Crowned
    /// forme the base species before any switch-in). Permanent forme changes after the switch
    /// line (Terapagos-Terastal via Tera Shift, Palafin-Hero) are not applied.
    pub fn shown_species(set: &SetDef) -> &'static str {
        let s = dex::species(set.species);
        let crowned = match (s.base_species, set.item) {
            (dex::SPECIES_ZACIAN, dex::ITEM_RUSTEDSWORD) => Some(dex::SPECIES_ZACIANCROWNED),
            (dex::SPECIES_ZAMAZENTA, dex::ITEM_RUSTEDSHIELD) => Some(dex::SPECIES_ZAMAZENTACROWNED),
            _ => None,
        };
        let s = crowned.map_or(s, dex::species);
        if s.name == "Greninja-Bond" || s.name == "Rockruff-Dusk" {
            s.base_species_name
        } else {
            s.name
        }
    }

    /// Among the pool's sets equal to `set` in everything but the Tera type (and gender, move order,
    /// name/extras, and shininess, which sampled sets take from the protocol), the share whose Tera type is
    /// `tera`, by count. `None` when the pool has no such set.
    pub fn tera_share(&self, set: &SetDef, tera: TypeId) -> Option<f64> {
        let g = *self.by_shown.get(Self::shown_species(set))?;
        let n = (set.move_count as usize).min(4);
        let mut moves = [EffectId::NONE; 4];
        moves[..n].copy_from_slice(&set.moves[..n]);
        moves[..n].sort_unstable_by_key(|m| m.0);
        let (mut all, mut with) = (0u64, 0u64);
        for &i in &self.groups[g as usize] {
            let e = &self.entries[i as usize];
            let k = &e.key;
            if k.species == set.species
                && k.item == set.item
                && k.ability == set.ability
                && k.level == set.level
                && k.move_count as usize == n
                && k.moves == moves
                && k.evs == set.evs
                && k.ivs == set.ivs
                && k.nature == set.nature
                && k.happiness == set.happiness
                && k.dynamax_level == set.dynamax_level
                && k.gigantamax == set.gigantamax
            {
                all += e.count as u64;
                if k.tera_type == tera {
                    with += e.count as u64;
                }
            }
        }
        (all > 0).then(|| with as f64 / all as f64)
    }

    /// Whether any set in the pool shows this species text.
    pub fn contains_species(&self, shown: &str) -> bool {
        self.by_shown.contains_key(shown)
    }

    pub(crate) fn illusion_species(&self) -> &[(EffectId, u32)] {
        &self.illusion
    }

    /// Exact support check without sampling, building a set or allocating a move list.
    pub(crate) fn consistent(&self, c: &SetConstraints<'_>) -> bool {
        self.by_shown.get(c.shown_species).is_some_and(|&g| {
            self.groups[g as usize].iter().any(|&i| {
                let k = &self.entries[i as usize].key;
                c.level.is_none_or(|x| x == k.level)
                    && c.item.is_none_or(|x| x == k.item)
                    && c.ability.is_none_or(|x| x == k.ability)
                    && c.tera.is_none_or(|x| x == k.tera_type)
                    && c.moves.iter().all(|m| k.moves[..k.move_count as usize].contains(m))
            })
        })
    }

    /// Bounded repair proposals, exact except for Tera, most frequent first, ties in pool order.
    /// Returns the pool record identity, set and share of all records satisfying the other
    /// constraints, including records outside the bound. Gender and move order follow `sample`.
    pub(crate) fn repair_candidates(
        &self,
        c: &SetConstraints<'_>,
        limit: usize,
        rng: &mut SplitMix64,
    ) -> Vec<(u32, SetDef, f64)> {
        let Some(&g) = self.by_shown.get(c.shown_species) else { return Vec::new() };
        let mut candidates: Vec<_> = self.groups[g as usize].iter().copied().filter(|&i| {
            let k = &self.entries[i as usize].key;
            c.level.is_none_or(|x| x == k.level)
                && c.item.is_none_or(|x| x == k.item)
                && c.ability.is_none_or(|x| x == k.ability)
                && c.moves.iter().all(|m| k.moves[..k.move_count as usize].contains(m))
        }).collect();
        let total: u64 = candidates.iter().map(|&i| self.entries[i as usize].count as u64).sum();
        candidates.sort_by_key(|&i| (std::cmp::Reverse(self.entries[i as usize].count), i));
        candidates.into_iter().take(limit).map(|i| {
            let e = &self.entries[i as usize];
            (i, self.build(&e.key, Some(c.name), c.shiny, c.gender, rng), e.count as f64 / total as f64)
        }).collect()
    }

    /// A set for a revealed Pokemon: among the sets showing `c.shown_species`, those that satisfy
    /// the most soft constraints (level, each distinct move, item, ability, Tera type), weighted
    /// by count. `None` if no set in the pool shows that species.
    ///
    /// The set takes `c.name` and `c.shiny`; its gender is `c.gender` if Male/Female, else the
    /// species' fixed gender, else a fair coin.
    ///
    /// PRNG: one draw for the candidate, one for a coin gender (only when drawn), then
    /// `move_count - 1` for the move order.
    pub fn sample(&self, c: &SetConstraints, rng: &mut SplitMix64) -> Option<Sampled> {
        self.sample_candidates(c, rng, None).map(|(sampled, _)| sampled)
    }

    /// Rebuild proposal: visit distinct best-fitting pool records before repeating one. `used`
    /// contains record indices from earlier proposals for this Pokemon under the same constraints.
    /// Once exhausted, start another weighted pass. The returned factor corrects the proposal's
    /// weight back to the ordinary pool prior; no candidate is ruled out by a failed replay.
    pub(crate) fn sample_distinct(
        &self,
        c: &SetConstraints,
        rng: &mut SplitMix64,
        used: &mut Vec<u32>,
    ) -> Option<(Sampled, f64)> {
        self.sample_candidates(c, rng, Some(used))
    }

    fn sample_candidates(
        &self,
        c: &SetConstraints,
        rng: &mut SplitMix64,
        mut used: Option<&mut Vec<u32>>,
    ) -> Option<(Sampled, f64)> {
        let g = *self.by_shown.get(c.shown_species)?;
        let members = &self.groups[g as usize];
        let mut moves: Vec<EffectId> = Vec::with_capacity(c.moves.len());
        for &m in c.moves {
            if !moves.contains(&m) {
                moves.push(m);
            }
        }
        let max = c.level.is_some() as u32
            + moves.len() as u32
            + c.item.is_some() as u32
            + c.ability.is_some() as u32
            + c.tera.is_some() as u32;
        let score = |k: &Key| {
            (c.level == Some(k.level)) as u32
                + moves
                    .iter()
                    .filter(|m| k.moves[..k.move_count as usize].contains(m))
                    .count() as u32
                + (c.item == Some(k.item)) as u32
                + (c.ability == Some(k.ability)) as u32
                + (c.tera == Some(k.tera_type)) as u32
        };
        let (mut best, mut weight) = (0u32, 0u64);
        for &i in members {
            let e = &self.entries[i as usize];
            let s = score(&e.key);
            if s > best || weight == 0 {
                (best, weight) = (s, 0);
            }
            if s == best {
                weight += e.count as u64;
            }
        }
        let mut remaining = weight;
        if let Some(used) = &mut used {
            remaining = members
                .iter()
                .copied()
                .filter(|i| !used.contains(i))
                .map(|i| &self.entries[i as usize])
                .filter(|e| score(&e.key) == best)
                .map(|e| e.count as u64)
                .sum();
            if remaining == 0 {
                used.clear();
                remaining = weight;
            }
        }
        let mut target = rng.below(remaining as usize) as u64;
        let pick = *members
            .iter()
            .filter(|i| used.as_ref().is_none_or(|used| !used.contains(i)))
            .filter(|&&i| score(&self.entries[i as usize].key) == best)
            .find(|&&i| {
                let e = &self.entries[i as usize];
                let hit = target < e.count as u64;
                target = target.saturating_sub(e.count as u64);
                hit
            })
            .expect("weighted pick within the best candidates");
        if let Some(used) = used {
            used.push(pick);
        }
        let set = self.build(&self.entries[pick as usize].key, Some(c.name), c.shiny, c.gender, rng);
        Some((
            Sampled {
                set,
                consistent: best == max,
            },
            remaining as f64 / weight as f64,
        ))
    }

    /// A set for a never-seen team slot, weighted by count over all sets whose Species Clause key
    /// (the dex `base_species_name`, so formes of one species collide) differs from that of every
    /// species in `exclude` (the species EffectIds of the team so far). `None` if no set
    /// qualifies.
    ///
    /// The set keeps its stored name (the species' `base_species_name` unless nicknamed) and
    /// shininess; gender is the species' fixed gender, else a fair coin.
    ///
    /// PRNG: one draw per rejection attempt (at most 64, then one draw over an exact scan), one
    /// for a coin gender (only when drawn), then `move_count - 1` for the move order.
    pub fn sample_unrevealed(&self, exclude: &[EffectId], rng: &mut SplitMix64) -> Option<SetDef> {
        if self.total == 0 {
            return None;
        }
        let excluded_keys: Vec<&str> = exclude.iter().map(|&s| clause_key(s)).collect();
        let allowed = |e: &Entry| !excluded_keys.contains(&clause_key(e.key.species));
        let cumulative = self.cumulative.get_or_init(|| {
            let mut acc = 0u32;
            self.entries
                .iter()
                .map(|e| {
                    acc += e.count;
                    acc
                })
                .collect()
        });
        for _ in 0..REJECTION_TRIES {
            let r = rng.below(self.total as usize) as u32;
            let e = &self.entries[cumulative.partition_point(|&c| c <= r)];
            if allowed(e) {
                return Some(self.build(&e.key, None, e.key.shiny, Gender::Unspecified, rng));
            }
        }
        // Almost all weight is excluded (tiny pools only): draw exactly over the allowed sets.
        let weight: u64 = self
            .entries
            .iter()
            .filter(|e| allowed(e))
            .map(|e| e.count as u64)
            .sum();
        if weight == 0 {
            return None;
        }
        let mut target = rng.below(weight as usize) as u64;
        let e = self
            .entries
            .iter()
            .filter(|e| allowed(e))
            .find(|e| {
                let hit = target < e.count as u64;
                target = target.saturating_sub(e.count as u64);
                hit
            })
            .expect("weighted pick within the allowed sets");
        Some(self.build(&e.key, None, e.key.shiny, Gender::Unspecified, rng))
    }

    /// The stored record of `set`, interning its string fields.
    fn key_of(&mut self, set: &SetDef) -> Key {
        let s = dex::species(set.species);
        let name = (set.name != s.base_species_name).then_some(set.name.as_str());
        let hash = extras_hash(name, &set.pokeball, &set.hp_type);
        let extras = &self.extras;
        let found = self.extras_index.find(hash, |i| {
            let x = &extras[i as usize];
            x.name.as_deref() == name && *x.pokeball == *set.pokeball && *x.hp_type == *set.hp_type
        });
        let extras = found.unwrap_or_else(|| {
            let i = self.extras.len() as u32;
            self.extras.push(Extras {
                name: name.map(Into::into),
                pokeball: set.pokeball.as_str().into(),
                hp_type: set.hp_type.as_str().into(),
            });
            self.extras_index.insert(hash, i);
            i
        });
        let n = (set.move_count as usize).min(4);
        let mut moves = [EffectId::NONE; 4];
        moves[..n].copy_from_slice(&set.moves[..n]);
        moves[..n].sort_unstable_by_key(|m| m.0);
        Key {
            species: set.species,
            item: set.item,
            ability: set.ability,
            moves,
            level: set.level,
            evs: set.evs,
            ivs: set.ivs,
            move_count: n as u8,
            nature: set.nature,
            happiness: set.happiness,
            tera_type: set.tera_type,
            dynamax_level: set.dynamax_level,
            gigantamax: set.gigantamax,
            shiny: set.shiny,
            extras,
        }
    }

    /// Rebuilds the deduplication index after `shrink_to_fit`.
    fn ensure_index(&mut self) {
        if self.dedupe.is_empty() && !self.entries.is_empty() {
            for (i, e) in self.entries.iter().enumerate() {
                self.dedupe.insert(hash_of(&e.key), i as u32);
            }
        }
    }

    /// A full set from a stored record: resolved gender, shuffled moves, the given or stored name.
    fn build(
        &self,
        key: &Key,
        name: Option<&str>,
        shiny: bool,
        shown_gender: Gender,
        rng: &mut SplitMix64,
    ) -> SetDef {
        let s = dex::species(key.species);
        let extras = &self.extras[key.extras as usize];
        let gender = match shown_gender {
            Gender::Male | Gender::Female => shown_gender,
            _ => match s.gender {
                "M" => Gender::Male,
                "F" => Gender::Female,
                "N" => Gender::Genderless,
                _ if rng.below(2) == 0 => Gender::Male,
                _ => Gender::Female,
            },
        };
        let mut moves = key.moves;
        for i in (1..key.move_count as usize).rev() {
            moves.swap(i, rng.below(i + 1));
        }
        SetDef {
            name: name
                .or(extras.name.as_deref())
                .unwrap_or(s.base_species_name)
                .to_owned(),
            species: key.species,
            item: key.item,
            ability: key.ability,
            moves,
            move_count: key.move_count,
            evs: key.evs,
            ivs: key.ivs,
            nature: key.nature,
            level: key.level,
            gender,
            shiny,
            happiness: key.happiness,
            tera_type: key.tera_type,
            pokeball: extras.pokeball.to_string(),
            hp_type: extras.hp_type.to_string(),
            dynamax_level: key.dynamax_level,
            gigantamax: key.gigantamax,
        }
    }
}

impl core::fmt::Debug for SetPool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SetPool")
            .field("len", &self.len())
            .field("distinct", &self.distinct())
            .field("species", &self.groups.len())
            .finish()
    }
}

/// Species Clause key: formes of one species share their `base_species_name`. (The dex
/// `base_species` id is `NONE` when the base forme is out of scope, e.g. for Greninja-Bond.)
fn clause_key(species: EffectId) -> &'static str {
    dex::species(species).base_species_name
}

fn hash_of<T: Hash>(value: &T) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

fn extras_hash(name: Option<&str>, pokeball: &str, hp_type: &str) -> u64 {
    hash_of(&(name, pokeball, hp_type))
}

/// Passes an already-mixed 64-bit hash through unchanged.
#[derive(Clone, Copy, Default)]
struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ b as u64;
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// Hash -> index map that does not store the keys (they live in the indexed `Vec`); `find`
/// confirms candidates with a closure. Hash collisions between different keys go to `more`.
/// Lookup only: nothing iterates it, so its order never affects results.
#[derive(Clone, Default)]
struct IndexMap {
    first: HashMap<u64, u32, BuildHasherDefault<IdentityHasher>>,
    more: Vec<(u64, u32)>,
}

impl IndexMap {
    fn is_empty(&self) -> bool {
        self.first.is_empty()
    }

    fn find(&self, hash: u64, eq: impl Fn(u32) -> bool) -> Option<u32> {
        let &i = self.first.get(&hash)?;
        if eq(i) {
            return Some(i);
        }
        self.more
            .iter()
            .find(|&&(h, i)| h == hash && eq(i))
            .map(|&(_, i)| i)
    }

    fn insert(&mut self, hash: u64, i: u32) {
        use std::collections::hash_map::Entry;
        match self.first.entry(hash) {
            Entry::Vacant(v) => {
                v.insert(i);
            }
            Entry::Occupied(_) => self.more.push((hash, i)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Battle, ids::MonId, log::NoLog};
    use std::sync::Arc;

    const LATIAS: &str = "Latias||SoulDew|Levitate|mistball,recover,dracometeor,tailwind||85,,85,85,85,85|F|,0,,,,||80|,,,,,Steel";
    const UMBREON_A: &str = "Umbreon||SitrusBerry|InnerFocus|snarl,thunderwave,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Poison";
    // UMBREON_A with another gender and move order: the same stored set.
    const UMBREON_A2: &str = "Umbreon||SitrusBerry|InnerFocus|moonlight,foulplay,snarl,thunderwave||85,,85,85,85,85|F|,0,,,,||85|,,,,,Poison";
    const UMBREON_B: &str = "Umbreon||Leftovers|InnerFocus|snarl,protect,foulplay,moonlight||85,,85,85,85,85|M|,0,,,,||85|,,,,,Dark";
    const MUK_ALOLA: &str = "Muk|MukAlola|SitrusBerry|PoisonTouch|poisonjab,helpinghand,knockoff,drainpunch||85,85,85,85,85,85|M|||82|,,,,,Flying";
    const MUK: &str = "Muk||Leftovers|PoisonTouch|poisonjab,protect,knockoff,drainpunch||85,85,85,85,85,85|M|||88|,,,,,Dark";
    const MESPRIT: &str = "Mesprit||LifeOrb|Levitate|protect,psychic,nastyplot,dazzlinggleam||85,,85,85,85,85|N|,0,,,,||86|,,,,,Psychic";
    const GRENINJA_BOND: &str = "Greninja|GreninjaBond|LifeOrb|BattleBond|hydropump,darkpulse,icebeam,protect||85,85,85,85,85,85|M|||80|,,,,,Water";
    const ZAMAZENTA: &str = "Zamazenta||RustedShield|DauntlessShield|bodypress,ironhead,protect,wideguard||85,85,85,85,85,85|N|||70|,,,,,Fighting";

    // The per-distinct-set memory the type docs state.
    const _: () = assert!(size_of::<Entry>() == 44);

    fn team(sets: &[&str]) -> String {
        sets.join("]")
    }

    fn pool() -> SetPool {
        let mut p = SetPool::new();
        p.add_team(&team(&[LATIAS, UMBREON_A, UMBREON_A2, MUK_ALOLA, MESPRIT]))
            .unwrap();
        p.add_team(&team(&[
            UMBREON_A,
            UMBREON_B,
            MUK,
            GRENINJA_BOND,
            ZAMAZENTA,
        ]))
        .unwrap();
        p
    }

    fn id(kind: crate::ids::EffectKind, text: &str) -> EffectId {
        dex::lookup(kind, text).unwrap()
    }

    fn mv(text: &str) -> EffectId {
        id(crate::ids::EffectKind::Move, text)
    }

    fn item(text: &str) -> EffectId {
        id(crate::ids::EffectKind::Item, text)
    }

    fn species(text: &str) -> EffectId {
        id(crate::ids::EffectKind::Species, text)
    }

    fn sorted_moves(set: &SetDef) -> Vec<u16> {
        let mut m: Vec<u16> = set.moves[..set.move_count as usize]
            .iter()
            .map(|m| m.0)
            .collect();
        m.sort();
        m
    }

    #[test]
    fn rebuild_sampling_covers_candidates_then_restarts_with_the_pool_prior() {
        let p = pool();
        let c = SetConstraints::new("Umbreon", "Umbreon");
        let mut rng = SplitMix64(7);
        let mut ordinary_rng = rng;
        let mut used = Vec::new();
        let (first, weight) = p.sample_distinct(&c, &mut rng, &mut used).unwrap();
        assert_eq!(first, p.sample(&c, &mut ordinary_rng).unwrap());
        assert_eq!(weight, 1.0);
        let (second, weight) = p.sample_distinct(&c, &mut rng, &mut used).unwrap();
        assert_ne!(first.set.item, second.set.item);
        assert!(first.consistent && second.consistent);
        // Three Sitrus Berry entries and one Leftovers entry. The proposal correction retains
        // that prior even when the next draw is forced to cover the other candidate.
        assert_eq!(
            weight,
            if second.set.item == item("Sitrus Berry") { 0.75 } else { 0.25 }
        );
        let (_, weight) = p.sample_distinct(&c, &mut rng, &mut used).unwrap();
        assert_eq!(weight, 1.0);
        assert_eq!(used.len(), 1);

        // Exhausting the best candidates must never admit a lower-scoring set.
        let moves = [mv("Protect")];
        let c = SetConstraints { moves: &moves, ..c };
        for _ in 0..3 {
            let (sampled, weight) = p.sample_distinct(&c, &mut rng, &mut used).unwrap();
            assert!(sampled.consistent);
            assert_eq!(sampled.set.item, item("Leftovers"));
            assert_eq!(weight, 1.0);
        }
    }

    #[test]
    fn dedupes_with_counts_ignoring_gender_and_move_order() {
        let p = pool();
        assert_eq!(p.len(), 10);
        // Umbreon A x3 -> one entry; 10 sets, 8 distinct.
        assert_eq!(p.distinct(), 8);
        let umbreon = p.groups[p.by_shown["Umbreon"] as usize].clone();
        let counts: Vec<u32> = umbreon
            .iter()
            .map(|&i| p.entries[i as usize].count)
            .collect();
        assert_eq!(counts, [3, 1]);
        // A nickname is part of the set.
        let mut q = SetPool::new();
        q.add_team(UMBREON_A).unwrap();
        q.add_team(&UMBREON_A.replacen("Umbreon||", "Moony|Umbreon|", 1))
            .unwrap();
        q.add_team(&UMBREON_A.replacen("Umbreon|", "Umbreon|Umbreon", 1))
            .unwrap();
        assert_eq!((q.len(), q.distinct()), (3, 2));
        // Unparseable teams add nothing.
        assert!(
            q.add_team(&team(&[UMBREON_A, "Notamon||||tackle||||||100|"]))
                .is_err()
        );
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn shown_species_matches_battle_details() {
        let a = team(&[MUK_ALOLA, GRENINJA_BOND, ZAMAZENTA, LATIAS]);
        let b = team(&[
            MUK,
            MESPRIT,
            UMBREON_A,
            UMBREON_B.replacen("|85|", "||", 1).as_str(),
        ]);
        let (ta, tb) = (TeamDef::unpack(&a).unwrap(), TeamDef::unpack(&b).unwrap());
        let mut battle = Battle::from_team_defs(
            [1, 2, 3, 4],
            Arc::new(ta.clone()),
            Arc::new(tb.clone()),
            ["A".into(), "B".into()],
            NoLog,
        )
        .unwrap();
        battle.start().unwrap();
        let shown: Vec<&str> = [&ta, &tb]
            .iter()
            .flat_map(|t| t.sets[..t.len as usize].iter().map(SetPool::shown_species))
            .collect();
        assert_eq!(
            shown,
            [
                "Muk-Alola",
                "Greninja",
                "Zamazenta-Crowned",
                "Latias",
                "Muk",
                "Mesprit",
                "Umbreon",
                "Umbreon"
            ]
        );
        for (k, &s) in shown.iter().enumerate() {
            let mon = MonId((k / 4 * 6 + k % 4) as u8);
            let details = battle.details(mon);
            assert_eq!(details.split(", ").next().unwrap(), s, "{details}");
        }
    }

    #[test]
    fn species_is_a_hard_constraint() {
        let p = pool();
        let mut rng = SplitMix64(7);
        for _ in 0..50 {
            let mut c = SetConstraints::new("Muk", "Muk-Alola");
            c.item = Some(item("Leftovers")); // only plain Muk holds it
            let s = p.sample(&c, &mut rng).unwrap();
            assert_eq!(s.set.species, species("MukAlola"));
            assert!(!s.consistent);
            let s = p
                .sample(&SetConstraints::new("Muk", "Muk"), &mut rng)
                .unwrap();
            assert_eq!(s.set.species, species("Muk"));
            assert!(s.consistent);
        }
        let s = p
            .sample(&SetConstraints::new("Greninja", "Greninja"), &mut rng)
            .unwrap();
        assert_eq!(s.set.species, species("GreninjaBond"));
        assert!(p.contains_species("Zamazenta-Crowned") && !p.contains_species("Zamazenta"));
        assert!(
            p.sample(&SetConstraints::new("Pikachu", "Pikachu"), &mut rng)
                .is_none()
        );
        assert!(!p.contains_species("Pikachu"));
    }

    #[test]
    fn picks_the_most_satisfied_candidates() {
        let p = pool();
        let mut rng = SplitMix64(11);
        let a = TeamDef::unpack(UMBREON_A).unwrap().sets[0].clone();
        let b = TeamDef::unpack(UMBREON_B).unwrap().sets[0].clone();
        // Protect is only in B: always B, consistent.
        let protect = [mv("protect"), mv("protect")];
        for _ in 0..50 {
            let c = SetConstraints {
                moves: &protect,
                ..SetConstraints::new("Umbreon", "Umbreon")
            };
            let s = p.sample(&c, &mut rng).unwrap();
            assert_eq!((s.set.item, s.consistent), (b.item, true));
        }
        // Thunder Wave (A) + Leftovers (B): both satisfy one of two; inconsistent, both drawn.
        let twave = [mv("thunderwave")];
        let mut seen = [0; 2];
        for _ in 0..400 {
            let c = SetConstraints {
                moves: &twave,
                item: Some(b.item),
                ..SetConstraints::new("Umbreon", "Umbreon")
            };
            let s = p.sample(&c, &mut rng).unwrap();
            assert!(!s.consistent);
            seen[(s.set.item == b.item) as usize] += 1;
        }
        // Counts 3:1.
        assert!(seen[0] > 220 && seen[1] > 50, "{seen:?}");
        // Thunder Wave + Sitrus + level 85 + Poison tera: A satisfies all four.
        for _ in 0..50 {
            let c = SetConstraints {
                moves: &twave,
                item: Some(a.item),
                level: Some(85),
                tera: Some(a.tera_type),
                ability: Some(a.ability),
                ..SetConstraints::new("Umbreon", "Umbreon")
            };
            let s = p.sample(&c, &mut rng).unwrap();
            assert!(s.consistent);
            assert_eq!(sorted_moves(&s.set), sorted_moves(&a));
            assert_eq!(
                SetDef {
                    moves: a.moves,
                    gender: a.gender,
                    ..s.set
                },
                a
            );
        }
        // A wrong level alone does not exclude anything: everything ties at zero.
        let c = SetConstraints {
            level: Some(50),
            ..SetConstraints::new("Umbreon", "Umbreon")
        };
        assert!(!p.sample(&c, &mut rng).unwrap().consistent);
    }

    #[test]
    fn gender_name_and_shiny() {
        let p = pool();
        let mut rng = SplitMix64(3);
        let mut genders = [0; 4];
        for _ in 0..400 {
            let s = p
                .sample(&SetConstraints::new("Nick", "Umbreon"), &mut rng)
                .unwrap()
                .set;
            assert_eq!((s.name.as_str(), s.shiny), ("Nick", false));
            genders[s.gender as usize] += 1;
            let c = SetConstraints {
                gender: Gender::Female,
                shiny: true,
                ..SetConstraints::new("U", "Umbreon")
            };
            let s = p.sample(&c, &mut rng).unwrap().set;
            assert_eq!((s.gender, s.shiny), (Gender::Female, true));
            // Fixed genders come from the dex, whatever the stored set said.
            let s = p
                .sample(&SetConstraints::new("Latias", "Latias"), &mut rng)
                .unwrap()
                .set;
            assert_eq!(s.gender, Gender::Female);
            let s = p
                .sample(&SetConstraints::new("Mesprit", "Mesprit"), &mut rng)
                .unwrap()
                .set;
            assert_eq!(s.gender, Gender::Genderless);
        }
        assert_eq!(
            genders[Gender::Unspecified as usize] + genders[Gender::Genderless as usize],
            0
        );
        assert!(genders[Gender::Male as usize] > 150 && genders[Gender::Female as usize] > 150);
        // Unrevealed slots keep the stored (default) name.
        for _ in 0..100 {
            let s = p.sample_unrevealed(&[], &mut rng).unwrap();
            assert_eq!(s.name, dex::species(s.species).base_species_name);
            assert_ne!(s.gender, Gender::Unspecified);
        }
        let mut q = SetPool::new();
        q.add_team(&UMBREON_A.replacen("Umbreon||", "Moony|Umbreon|", 1))
            .unwrap();
        assert_eq!(q.sample_unrevealed(&[], &mut rng).unwrap().name, "Moony");
    }

    #[test]
    fn unrevealed_respects_species_clause() {
        let p = pool();
        let mut rng = SplitMix64(5);
        // Muk excludes Muk-Alola too (same base species).
        let exclude = [
            species("Umbreon"),
            species("Muk"),
            species("Latias"),
            species("Mesprit"),
            species("Zamazenta"),
        ];
        for _ in 0..200 {
            let s = p.sample_unrevealed(&exclude, &mut rng).unwrap();
            assert_eq!(s.species, species("GreninjaBond"));
        }
        let mut seen_muk = 0;
        for _ in 0..200 {
            let s = p
                .sample_unrevealed(&[species("Umbreon")], &mut rng)
                .unwrap();
            assert_ne!(s.species, species("Umbreon"));
            seen_muk += (clause_key(s.species) == "Muk") as u32;
        }
        assert!(seen_muk > 0);
        let all = [
            species("Umbreon"),
            species("Muk"),
            species("Latias"),
            species("Mesprit"),
            species("Zamazenta"),
            species("GreninjaBond"),
        ];
        assert!(p.sample_unrevealed(&all, &mut rng).is_none());
        assert!(SetPool::new().sample_unrevealed(&[], &mut rng).is_none());
    }

    #[test]
    fn deterministic_and_unaffected_by_shrinking() {
        let draw = |p: &SetPool| {
            let mut rng = SplitMix64(99);
            let mut out = Vec::new();
            for _ in 0..50 {
                out.push(
                    p.sample(&SetConstraints::new("x", "Umbreon"), &mut rng)
                        .unwrap()
                        .set,
                );
                out.push(p.sample_unrevealed(&[species("Muk")], &mut rng).unwrap());
            }
            out
        };
        let mut p = pool();
        let first = draw(&p);
        assert_eq!(first, draw(&pool()));
        p.shrink_to_fit();
        assert_eq!(first, draw(&p));
        // Adding after shrinking still deduplicates.
        p.add_team(UMBREON_B).unwrap();
        assert_eq!((p.len(), p.distinct()), (11, 8));
    }

    #[test]
    fn tera_share_conditions_on_every_stored_field_except_tera_and_extras() {
        let mut p = SetPool::new();
        let a = TeamDef::unpack(&UMBREON_A.replacen("||85", "|Adamant|85", 1)).unwrap().sets[0].clone();
        let mut b = TeamDef::unpack(&UMBREON_A.replacen("||85", "|Timid|85", 1)).unwrap().sets[0].clone();
        b.tera_type = dex::type_id("Ghost").unwrap();
        assert_ne!(a.nature, b.nature);
        for _ in 0..9 { p.add_set(&a); }
        p.add_set(&b);
        assert_eq!(p.tera_share(&a, a.tera_type), Some(1.0));
        assert_eq!(p.tera_share(&b, a.tera_type), Some(0.0));
        assert_eq!(p.tera_share(&b, b.tera_type), Some(1.0));
        for field in 0..3 {
            let mut other = a.clone();
            match field {
                0 => other.happiness -= 1,
                1 => other.dynamax_level -= 1,
                _ => other.gigantamax = !other.gigantamax,
            }
            assert_eq!(p.tera_share(&other, a.tera_type), None);
        }
        // A shiny opponent takes its shininess from the protocol, not from the pool.
        let shiny = SetDef { shiny: !a.shiny, ..a.clone() };
        assert_eq!(p.tera_share(&shiny, a.tera_type), Some(1.0));
        let mut same = b;
        same.tera_type = a.tera_type;
        same.name = "nickname".into();
        same.pokeball = "Ultra Ball".into();
        same.hp_type = "Fire".into();
        same.gender = Gender::Female;
        same.moves.reverse();
        assert_eq!(p.tera_share(&same, a.tera_type), Some(0.0));
    }

    #[test]
    fn repair_proposals_are_exact_ranked_and_keep_the_full_prior_denominator() {
        let mut p = SetPool::new();
        let a = TeamDef::unpack(UMBREON_A).unwrap().sets[0].clone();
        let mut b = a.clone();
        b.item = dex::ITEM_CHOICESCARF;
        let mut c = b.clone();
        c.tera_type = dex::type_id("Water").unwrap();
        for _ in 0..10 { p.add_set(&a); }
        for _ in 0..3 { p.add_set(&b); }
        for _ in 0..2 { p.add_set(&c); }
        let mut constraints = SetConstraints::new("Umbreon", "Umbreon");
        constraints.item = Some(dex::ITEM_CHOICESCARF);
        let proposals = p.repair_candidates(&constraints, 1, &mut SplitMix64(1));
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].1.tera_type, b.tera_type);
        assert_eq!(proposals[0].2, 3.0 / 5.0);
        constraints.tera = Some(dex::type_id("Fire").unwrap());
        assert_eq!(p.repair_candidates(&constraints, 1, &mut SplitMix64(1)), proposals,
            "Tera is integrated separately, including off-pool types");
        constraints.item = None;
        let all = p.repair_candidates(&constraints, 3, &mut SplitMix64(1));
        assert_eq!(all[1].0, proposals[0].0, "record identities survive a changed filter and rank");
        constraints.moves = &[dex::MOVE_FLASHCANNON];
        assert!(p.repair_candidates(&constraints, 8, &mut SplitMix64(1)).is_empty());
    }
}
