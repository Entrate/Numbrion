//! Approximate damage-calc features (port of `training/damage.py`), one observation row at a time.
//!
//! The Python module works on `[B, ...]` tensors; this port computes the same elementwise formulas per row with
//! scalar loops in f32, in the same operation order where it matters, so the outputs match the torch ones to float
//! rounding. Outputs per row: `action` `[2, 40, N_ACTION_FEATURES]` and `token` `[12, N_TOKEN_FEATURES]`.
//!
//! Index conventions shared by every function here: tokens 0-5 are the own side (0 and 1 the actives in slots a and
//! b), 6-11 the foe side (6 and 7 the foe actives a and b); a move code of an own active is
//! `move_slot * 10 + target_code * 2 + tera_bit` with target code 0 own slot b, 1 own slot a, 2 no target / spread,
//! 3 foe a, 4 foe b.

use crate::{
    dex::{Dex, STELLAR, TARGETS},
    layout::*,
};

/// Mean damage roll.
pub const ROLL: f32 = 0.925;
/// Damage multiplier of spread moves (`allAdjacentFoes`, `allAdjacent`) in doubles.
pub const SPREAD: f32 = 0.75;
/// Foe moves considered by [`DamageFeatures::foe_attacks`]: 4 revealed slots, K_MOVES belief candidates, and
/// three generic 80-power STAB attacks.
pub const M_FOE: usize = 4 + K_MOVES + 3;
/// The foe moves read from `ids` (revealed slots, then belief candidates): the first `M_IDS` of the [`M_FOE`].
pub const M_IDS: usize = 4 + K_MOVES;
/// `OWN_TOKENS`: the own actives (slots a, b).
pub const OWN_TOKENS: [usize; 2] = [0, 1];
/// `FOE_TOKENS`: the foe actives (slots a, b).
pub const FOE_TOKENS: [usize; 2] = [6, 7];
/// `FLINCH_BLOCK`: abilities that prevent flinching (ignored by [`MOLD_BREAKER`]).
pub const FLINCH_BLOCK: [&str; 2] = ["innerfocus", "shielddust"];
/// `PRIORITY_BLOCK`: abilities that block priority moves aimed at their side.
pub const PRIORITY_BLOCK: [&str; 3] = ["armortail", "dazzling", "queenlymajesty"];
/// `MOLD_BREAKER`: abilities that ignore the target's [`FLINCH_BLOCK`] ability.
pub const MOLD_BREAKER: [&str; 3] = ["moldbreaker", "teravolt", "turboblaze"];
/// `REPEAT_PROTECT`: success chance of a Protect-like move right after a successful one.
pub const REPEAT_PROTECT: f64 = 1.0 / 3.0;
/// Protect choice prior's baseline; the model learns how far to trust this prior.
pub const PROTECT_BASE: f32 = 0.1;
/// Protect choice prior's damage weight; the model learns how far to trust this prior.
pub const PROTECT_PER_DAMAGE: f32 = 0.2;
/// Protect choice prior's KO weight; the model learns how far to trust this prior.
pub const PROTECT_PER_KO: f32 = 0.3;
/// Protect choice prior's upper bound; the model learns how far to trust this prior.
pub const PROTECT_MAX: f32 = 0.8;
/// `guards.FIRST_TURN_ONLY`: moves that fail after the user's first move action since it switched in.
pub const FIRST_TURN_ONLY: [&str; 2] = ["fakeout", "firstimpression"];
/// `FLINCH_CHANCE`: flinch chances that override dex.json's `secondaryChance` (the largest of a move's secondaries).
pub const FLINCH_CHANCE: [(&str, f32); 1] = [("triplearrows", 0.3)];

const FIRE: i64 = 10;
const WATER: i64 = 11;
const N_TYPE_MULT: usize = 20;
/// Defenders of the foes' moves in [`DamageFeatures::row`]: the 12 tokens, then the own actives terastallized.
const N_TARGETS: usize = N_TOKENS + 2;

/// Per-token attributes (`DamageFeatures.tokens`) of one token.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tok {
    pub item_phys: f32,
    pub item_spec: f32,
    pub level: f32,
    pub atk: f32,
    pub spa: f32,
    pub dfn: f32,
    pub spd: f32,
    pub spe: f32,
    pub hp: f32,
    pub hp_frac: f32,
    pub max_hp: f32,
    pub alive: f32,
    pub burned: f32,
    pub base_types: [i64; 2],
    pub def_types: [i64; 2],
    pub tera: i64,
    pub terastallized: f32,
    pub type_mult: [f32; 20],
    pub weight: f32,
    pub item: i64,
    pub screen_phys: f32,
    pub screen_spec: f32,
    /// 1 when already Stellar-terastallized, or for an own active's prospective Stellar Tera row.
    pub stellar: f32,
}

/// The move columns `damage()` takes (`mtype`, `category`, `power`, `accuracy`, `hits`, `special`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveIn {
    pub mtype: i64,
    pub category: i64,
    pub power: f32,
    pub accuracy: f32,
    pub hits: f32,
    pub special: i64,
}

/// Every per-move buffer of `DamageFeatures.__init__` for one move index, plus derived target flags.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveRow {
    /// `move_type`, `move_category` (0 status, 1 physical, 2 special), `move_power`, `move_accuracy`, `move_hits`,
    /// `move_special` as [`DamageFeatures::damage`] takes them.
    pub input: MoveIn,
    /// `move_priority`.
    pub priority: f32,
    /// `move_target`: index into [`TARGETS`].
    pub target: usize,
    /// The target is `allAdjacentFoes` or `allAdjacent` (damage times [`SPREAD`] in `foe_attacks`).
    pub spread: bool,
    /// The target is `randomNormal` (`share` 0.5 in `effects`).
    pub random_normal: bool,
    /// `move_flinch`: [`FLINCH_CHANCE`], else `secondaryChance` when `secondaryVolatile` is "flinch", else 0.
    pub flinch: f32,
    /// `move_first_only`: 1 for [`FIRST_TURN_ONLY`] moves.
    pub first_only: f32,
    /// `move_blockable`: 1 when the move has the `protect` flag.
    pub blockable: f32,
    /// `move_contact`: 1 when the move has the `contact` flag.
    pub contact: f32,
    /// `move_protect`: 1 for stalling (Protect-like) moves.
    pub protect: f32,
    /// `move_fake_out`: 1 for Fake Out.
    pub fake_out: f32,
}

/// `foe_attacks` of one row: for foe f in 0..2 (tokens 6, 7), move m in 0..M_FOE and defender d in 0..n_def,
/// `dealt[(f * M_FOE + m) * n_def + d]`, the same for `ko`, and the move probabilities `probs[f][m]`.
#[derive(Clone, Debug, Default)]
pub struct FoeAttacks {
    pub n_def: usize,
    pub dealt: Vec<f32>,
    pub ko: Vec<f32>,
    pub probs: [[f32; M_FOE]; 2],
}

impl FoeAttacks {
    pub fn dealt(&self, f: usize, m: usize, d: usize) -> f32 {
        self.dealt[(f * M_FOE + m) * self.n_def + d]
    }

    pub fn ko(&self, f: usize, m: usize, d: usize) -> f32 {
        self.ko[(f * M_FOE + m) * self.n_def + d]
    }
}

/// `_hit_table()`: `HIT[target class][actor slot][target code][token]`, the share of the move's damage that lands
/// on each token (target code 0 own slot b, 1 own slot a, 2 none/spread, 3 foe a, 4 foe b).
///
/// Single-target classes (`normal`, `any`, `adjacentFoe`, `adjacentAlly`, `adjacentAllyOrSelf`) put 1.0 on token 1
/// for code 0, token 0 for code 1, token 6 for code 3 and token 7 for code 4, whatever the actor slot (so a slot-a
/// actor's code 1 and a slot-b actor's code 0 are the user itself). `allAdjacentFoes` puts [`SPREAD`] on tokens 6
/// and 7 for code 2, `allAdjacent` also on the actor's ally (token `1 - slot`), `randomNormal` 0.5 on tokens 6 and 7
/// for code 2. Every other class and code is all zero.
pub fn hit_table() -> Box<[[[[f32; 12]; 5]; 2]; 15]> {
    const CHOSEN: [(usize, usize); 4] = [(0, 1), (1, 0), (3, 6), (4, 7)]; // target code -> token
    let mut table = Box::new([[[[0.0f32; 12]; 5]; 2]; 15]);
    for (c, name) in TARGETS.iter().enumerate() {
        for s in 0..2 {
            let ally = 1 - s;
            let cell = &mut table[c][s];
            match *name {
                "normal" | "any" | "adjacentFoe" | "adjacentAlly" | "adjacentAllyOrSelf" => {
                    for (code, token) in CHOSEN {
                        cell[code][token] = 1.0;
                    }
                }
                "allAdjacentFoes" => {
                    cell[2][6] = SPREAD;
                    cell[2][7] = SPREAD;
                }
                "allAdjacent" => {
                    cell[2][6] = SPREAD;
                    cell[2][7] = SPREAD;
                    cell[2][ally] = SPREAD;
                }
                "randomNormal" => {
                    cell[2][6] = 0.5;
                    cell[2][7] = 0.5;
                }
                _ => {}
            }
        }
    }
    table
}

/// `expected_max` for one (attacker, defender) column: E[max over present moves] of `dealt[m]` when move m is
/// present independently with probability `probs[m]` (clamped to [0, 1]); absent moves deal nothing.
///
/// With the moves sorted by `dealt` descending: `sum_i d_i q_i prod_{j<i} (1 - q_j)`, the running product kept in
/// f64 and rounded to f32 at every step like torch's CPU `cumprod`, the sum in f32. Moves with `q == 0` are skipped
/// (they add exactly nothing). `dealt` and `probs` have the same length (any length).
pub fn expected_max(dealt: &[f32], probs: &[f32]) -> f32 {
    assert_eq!(dealt.len(), probs.len(), "expected_max: dealt and probs differ in length");
    const STACK: usize = 32;
    let mut stack = [(0.0f32, 0.0f32); STACK];
    let mut heap = Vec::new();
    let pairs: &mut [(f32, f32)] = if dealt.len() <= STACK {
        &mut stack[..]
    } else {
        heap.resize(dealt.len(), (0.0, 0.0));
        &mut heap[..]
    };
    let mut n = 0;
    for (&d, &p) in dealt.iter().zip(probs) {
        let q = p.clamp(0.0, 1.0);
        if q == 0.0 {
            continue;
        }
        // Insertion sort by dealt, descending (ties may go either way, as with torch's argsort).
        let mut i = n;
        while i > 0 && pairs[i - 1].0 < d {
            pairs[i] = pairs[i - 1];
            i -= 1;
        }
        pairs[i] = (d, q);
        n += 1;
    }
    let (mut sum, mut survive, mut before) = (0.0f32, 1.0f64, 1.0f32);
    for &(d, q) in &pairs[..n] {
        sum += d * q * before;
        survive *= (1.0 - q) as f64;
        before = survive as f32;
    }
    sum
}

/// `any_ko` for one column: 1 - prod_m (1 - p_m KO_m), p clamped to [0, 1] (product in f32, in move order).
pub fn any_ko(ko: &[f32], probs: &[f32]) -> f32 {
    assert_eq!(ko.len(), probs.len(), "any_ko: ko and probs differ in length");
    let mut survive = 1.0f32;
    for (&k, &p) in ko.iter().zip(probs) {
        let q = p.clamp(0.0, 1.0);
        if q != 0.0 {
            survive *= 1.0 - q * k;
        }
    }
    1.0 - survive
}

/// `stage(boost)`: the stat multiplier of a boost stage.
pub fn stage(boost: f32) -> f32 {
    (2.0 + boost.max(0.0)) / (2.0 + (-boost).max(0.0))
}

/// Chance of choosing and successfully using Protect, before a move's blocking factor.
pub fn protect_prior(can_protect: f32, damage: f32, ko: f32) -> f32 {
    can_protect * (PROTECT_BASE + PROTECT_PER_DAMAGE * damage + PROTECT_PER_KO * ko).clamp(0.0, PROTECT_MAX)
}

/// `_terastallized` for one own active: `t` as `tokens` gives it once terastallized: `terastallized` 1, the Tera
/// type alone as the defensive type (`[tera, 0]`) when the Tera type is known and not Stellar (else the base types),
/// and `stellar` 1 for a Stellar Tera type. Every other attribute is unchanged.
pub fn terastallized(t: &Tok) -> Tok {
    let mut out = *t;
    let use_tera = t.tera > 0 && t.tera != STELLAR;
    out.def_types = if use_tera { [t.tera, 0] } else { t.base_types };
    out.terastallized = 1.0;
    out.stellar = f32::from(t.tera == STELLAR);
    out
}

/// `_per_token`: the per-Pokemon state shared by the effect columns, each `[12]`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PerToken {
    /// Alive and active (`active_a + active_b > 0.5`).
    pub on: [f32; 12],
    /// `own > 0.5`.
    pub own: [bool; 12],
    /// `fresh * on`: has not used a move action since switching in.
    pub fresh: [f32; 12],
    /// A flinch is possible: no flinch-blocking ability (by belief), no Covert Cloak, no Substitute.
    pub flinch_ok: [f32; 12],
    /// `flinch_ok` without the ability term (what a Mold Breaker attacker faces).
    pub flinch_other_ok: [f32; 12],
    /// Chance of being grounded (1 under Gravity): not Flying-type, no Levitate, no Air Balloon.
    pub grounded: [f32; 12],
    /// Chance that an opposing priority move is not blocked by Psychic Terrain or the side's priority blockers.
    pub priority_ok: [f32; 12],
    /// `1 - (1 - REPEAT_PROTECT) * protect_last`: success chance of a Protect-like move now.
    pub repeat: [f32; 12],
    /// Chance of having a Protect-like move, times `repeat` and `on`.
    pub can_protect: [f32; 12],
    /// Chance of having Fake Out, times `fresh`.
    pub fake_out_ready: [f32; 12],
    /// Chance of having Unseen Fist.
    pub unseen_fist: [f32; 12],
}

pub struct DamageFeatures {
    /// Per move index (entry 0 is the unknown move).
    pub moves: Vec<MoveRow>,
    species_weight: Vec<f32>,
    /// `[attacking * n_types + defending]`.
    typechart: Vec<f32>,
    n_types: usize,
    hit: Box<[[[[f32; 12]; 5]; 2]; 15]>,
    /// First maximum per target class, actor slot and target code, preferring foes on positive ties.
    hit_main: [[[usize; 5]; 2]; 15],
    /// `hit.sum(-1) > 0` per target class, actor slot and target code.
    hit_any: [[[bool; 5]; 2]; 15],
    scarf: i64,
    band: i64,
    specs: i64,
    life_orb: i64,
    huge_power: [i64; 2],
    ability_flinch_block: Vec<f32>,
    ability_priority_block: Vec<f32>,
    ability_levitate: Vec<f32>,
    ability_unseen_fist: Vec<f32>,
    ability_mold_breaker: Vec<f32>,
    sheer_force: i64,
    serene_grace: i64,
    prankster: i64,
    gale_wings: i64,
    tera_blast: i64,
    grassy_glide: i64,
    covert_cloak: i64,
    air_balloon: i64,
    boots: i64,
    magic_guard: i64,
    rock: i64,
    flying: i64,
    normal: i64,
    dark: i64,
}

/// `dict.get(id, -1)` over a dex lookup that returns 0 for a missing id (index 0 is never a named entry).
fn or_missing(index: i64) -> i64 {
    if index > 0 { index } else { -1 }
}

impl DamageFeatures {
    pub fn new(dex: &Dex) -> Self {
        let target = |name: &str| TARGETS.iter().position(|t| *t == name).expect("target class");
        let (all_adjacent, all_adjacent_foes, random_normal) =
            (target("allAdjacent"), target("allAdjacentFoes"), target("randomNormal"));
        let fake_out = dex.move_index("fakeout");
        let moves = (0..dex.n_moves())
            .map(|i| {
                let target = dex.move_target[i] as usize;
                let mut row = MoveRow {
                    input: MoveIn {
                        mtype: dex.move_type[i],
                        category: dex.move_category[i],
                        power: dex.move_power[i],
                        accuracy: dex.move_accuracy[i],
                        hits: dex.move_hits[i],
                        special: dex.move_special_power[i],
                    },
                    priority: dex.move_priority[i],
                    target,
                    spread: target == all_adjacent_foes || target == all_adjacent,
                    random_normal: target == random_normal,
                    protect: f32::from(dex.move_stalling[i]),
                    fake_out: f32::from(fake_out > 0 && i as i64 == fake_out),
                    ..MoveRow::default()
                };
                if i > 0 {
                    let m = &dex.move_data[i];
                    let secondary = if m.secondary_volatile.as_deref() == Some("flinch") { m.secondary_chance as f32 } else { 0.0 };
                    row.flinch = FLINCH_CHANCE.iter().find(|(id, _)| *id == m.id).map_or(secondary, |&(_, chance)| chance);
                    row.first_only = f32::from(FIRST_TURN_ONLY.contains(&m.id.as_str()));
                    row.blockable = f32::from(m.has_flag("protect"));
                    row.contact = f32::from(m.has_flag("contact"));
                }
                row
            })
            .collect();
        let ability_table = |names: &[&str]| -> Vec<f32> {
            dex.ability_ids.iter().enumerate().map(|(i, id)| f32::from(i > 0 && names.contains(&id.as_str()))).collect()
        };
        let hit = hit_table();
        let mut hit_main = [[[0usize; 5]; 2]; 15];
        let mut hit_any = [[[false; 5]; 2]; 15];
        for c in 0..15 {
            for s in 0..2 {
                for code in 0..5 {
                    let row = &hit[c][s][code];
                    let mut best = 0;
                    for (k, &x) in row.iter().enumerate() {
                        if x > row[best] || (x > 0.0 && x == row[best] && k >= 6 && best < 6) {
                            best = k;
                        }
                    }
                    hit_main[c][s][code] = best;
                    hit_any[c][s][code] = row.iter().sum::<f32>() > 0.0;
                }
            }
        }
        let ability = |name: &str| or_missing(dex.ability_index(name));
        let item = |name: &str| or_missing(dex.item_index(name));
        DamageFeatures {
            moves,
            species_weight: dex.species_weight.clone(),
            typechart: dex.typechart.clone(),
            n_types: dex.n_types,
            hit,
            hit_main,
            hit_any,
            scarf: item("choicescarf"),
            band: item("choiceband"),
            specs: item("choicespecs"),
            life_orb: item("lifeorb"),
            huge_power: [ability("hugepower"), ability("purepower")],
            ability_flinch_block: ability_table(&FLINCH_BLOCK),
            ability_priority_block: ability_table(&PRIORITY_BLOCK),
            ability_levitate: ability_table(&["levitate"]),
            ability_unseen_fist: ability_table(&["unseenfist"]),
            ability_mold_breaker: ability_table(&MOLD_BREAKER),
            sheer_force: ability("sheerforce"),
            serene_grace: ability("serenegrace"),
            prankster: ability("prankster"),
            gale_wings: ability("galewings"),
            tera_blast: or_missing(dex.move_index("terablast")),
            grassy_glide: or_missing(dex.move_index("grassyglide")),
            covert_cloak: item("covertcloak"),
            air_balloon: item("airballoon"),
            boots: item("heavydutyboots"),
            magic_guard: ability("magicguard"),
            rock: dex.type_idx("Rock"),
            flying: dex.type_idx("Flying"),
            normal: dex.type_idx("Normal"),
            dark: dex.type_idx("Dark"),
        }
    }

    /// `tokens(ids, floats, field)` for one row: `ids` `[12 * N_IDS]`, `floats` `[12 * N_POKEMON_FLOATS]`,
    /// `field` `[N_FIELD_FLOATS]`.
    ///
    /// Per token: stats are `stat_* * STAT_SCALE` times `stage(boost * 6)`; `atk` doubled for Huge Power / Pure
    /// Power; `spe` halved when paralyzed, times 1.5 with Choice Scarf, times `1 + tailwind` of its side; `hp` is
    /// `hp_frac * max_hp` with `max_hp` (stored clamped to >= 1) `stat_hp * STAT_SCALE`; `alive` is
    /// `present * (1 - fainted)`; `def_types` is `[tera, 0]` when terastallized (> 0.5) into a known non-Stellar
    /// type, else the base types; `item_phys`/`item_spec` are Choice Band/Specs 1.5 times Life Orb 1.3; the screens
    /// are its side's `max(reflect, auroraveil)` and `max(lightscreen, auroraveil)`; `stellar` is 1 when already
    /// terastallized (> 0.5) into Stellar, else 0.
    pub fn tokens(&self, ids: &[i64], floats: &[f32], field: &[f32]) -> [Tok; 12] {
        let mut out = [Tok::default(); 12];
        for (i, tok) in out.iter_mut().enumerate() {
            *tok = self.token(&ids[i * N_IDS..(i + 1) * N_IDS], &floats[i * N_POKEMON_FLOATS..(i + 1) * N_POKEMON_FLOATS], field);
        }
        out
    }

    /// One token of [`DamageFeatures::tokens`] from its `ids` `[N_IDS]` and `floats` `[N_POKEMON_FLOATS]`.
    pub fn token(&self, id: &[i64], f: &[f32], field: &[f32]) -> Tok {
        let terastallized = f[pf::TERASTALLIZED];
        let tera = id[I_TERA];
        let base_types = [id[I_TYPE1], id[I_TYPE1 + 1]];
        let use_tera = terastallized > 0.5 && tera > 0 && tera != STELLAR;
        let def_types = if use_tera { [tera, 0] } else { base_types };
        let (ability, item) = (id[I_ABILITY], id[I_ITEM]);
        let mut atk = f[pf::STAT_ATK] * STAT_SCALE * stage(f[pf::BOOST_ATK] * 6.0);
        if self.huge_power.contains(&ability) {
            atk = atk * 2.0;
        }
        let paralysis = if f[pf::PAR] > 0.0 { 0.5 } else { 1.0 };
        let mut spe = f[pf::STAT_SPE] * STAT_SCALE * stage(f[pf::BOOST_SPE] * 6.0) * paralysis;
        if item == self.scarf {
            spe = spe * 1.5;
        }
        let own = f[pf::OWN] > 0.5;
        let side = |own_col: usize, foe_col: usize| if own { field[own_col] } else { field[foe_col] };
        spe = spe * (1.0 + side(ff::OWN_TAILWIND, ff::FOE_TAILWIND));
        let max_hp = f[pf::STAT_HP] * STAT_SCALE;
        let reflect = side(ff::OWN_REFLECT, ff::FOE_REFLECT);
        let light = side(ff::OWN_LIGHTSCREEN, ff::FOE_LIGHTSCREEN);
        let veil = side(ff::OWN_AURORAVEIL, ff::FOE_AURORAVEIL);
        let orb = if item == self.life_orb { 1.3 } else { 1.0 };
        let mut type_mult = [0.0f32; N_TYPE_MULT];
        type_mult.copy_from_slice(&f[pf::TYPE_MULT0..pf::TYPE_MULT0 + N_TYPE_MULT]);
        Tok {
            item_phys: (if item == self.band { 1.5 } else { 1.0 }) * orb,
            item_spec: (if item == self.specs { 1.5 } else { 1.0 }) * orb,
            level: f[pf::LEVEL] * 100.0,
            atk,
            spa: f[pf::STAT_SPA] * STAT_SCALE * stage(f[pf::BOOST_SPA] * 6.0),
            dfn: f[pf::STAT_DEF] * STAT_SCALE * stage(f[pf::BOOST_DEF] * 6.0),
            spd: f[pf::STAT_SPD] * STAT_SCALE * stage(f[pf::BOOST_SPD] * 6.0),
            spe,
            hp: f[pf::HP] * max_hp,
            hp_frac: f[pf::HP],
            max_hp: max_hp.max(1.0),
            alive: f[pf::PRESENT] * (1.0 - f[pf::FAINTED]),
            burned: f[pf::BRN],
            base_types,
            def_types,
            tera,
            terastallized,
            type_mult,
            weight: self.species_weight[id[I_SPECIES] as usize],
            item,
            screen_phys: reflect.max(veil),
            screen_spec: light.max(veil),
            stellar: f32::from(terastallized > 0.5 && tera == STELLAR),
        }
    }

    /// The move columns of move index `m` (`move_type[m]`, `move_category[m]`, ...).
    pub fn move_in(&self, m: i64) -> MoveIn {
        self.moves[m as usize].input
    }

    /// The core formula `damage(att, dfn, ...)` for one attacker, defender and move: (expected dealt fraction of the
    /// defender's max HP capped at its current HP, KO chance, effectiveness times the ability/item multiplier).
    /// `modifier` scales the damage (1 for none, SPREAD for spread moves); `field` is the row's field floats.
    ///
    /// Gen 9 formula `((2 L / 5 + 2) P A / D / 50 + 2)` times STAB (1.5 per original-type and per Tera match; with
    /// `att.stellar`, 2 on original-type moves else 1.2), the type chart against `dfn.def_types`, `dfn.type_mult`,
    /// sun/rain on Fire/Water, burn (physical), screens (1/3 off), ROLL and hits; special powers: 1 target weight,
    /// 2 weight ratio, 3 `150 * hp_frac`, 4 Night Shade (= level), 5 Super Fang (= half the defender's HP), 6 and
    /// status moves deal 0. KO chance is `clamp((damage / ROLL - hp) / max(0.15 damage / ROLL, 1e-3), 0, 1)` times
    /// accuracy; dealt and KO chance are times `dfn.alive`.
    #[inline]
    pub fn damage(&self, att: &Tok, dfn: &Tok, mv: &MoveIn, field: &[f32], modifier: f32) -> (f32, f32, f32) {
        self.damage_with(att, dfn, mv, field[ff::SUN], field[ff::RAIN], modifier)
    }

    #[inline]
    fn damage_with(&self, att: &Tok, dfn: &Tok, mv: &MoveIn, sun: f32, rain: f32, modifier: f32) -> (f32, f32, f32) {
        let physical = mv.category == 1;
        let attack = if physical { att.atk * att.item_phys } else { att.spa * att.item_spec };
        let defense = (if physical { dfn.dfn } else { dfn.spd }).max(1.0);
        let power = match mv.special {
            1 => {
                let w = dfn.weight;
                if w < 10.0 {
                    20.0
                } else if w < 25.0 {
                    40.0
                } else if w < 50.0 {
                    60.0
                } else if w < 100.0 {
                    80.0
                } else if w < 200.0 {
                    100.0
                } else {
                    120.0
                }
            }
            2 => {
                let ratio = att.weight / dfn.weight.max(0.1);
                if ratio >= 5.0 {
                    120.0
                } else if ratio >= 4.0 {
                    100.0
                } else if ratio >= 3.0 {
                    80.0
                } else if ratio >= 2.0 {
                    60.0
                } else {
                    40.0
                }
            }
            3 => 150.0 * att.hp_frac,
            _ => mv.power,
        };
        let base = (2.0 * att.level / 5.0 + 2.0) * power * attack / defense / 50.0 + 2.0;
        let tera_match = att.terastallized > 0.5 && att.tera == mv.mtype;
        let orig_match = att.base_types[0] == mv.mtype || att.base_types[1] == mv.mtype;
        let mut stab = 1.0 + 0.5 * f32::from(orig_match) + 0.5 * f32::from(tera_match);
        if att.stellar > 0.5 {
            stab = if orig_match { 2.0 } else { 1.2 };
        }
        let row = mv.mtype as usize * self.n_types;
        let eff = self.typechart[row + dfn.def_types[0] as usize] * self.typechart[row + dfn.def_types[1] as usize];
        // The one-hot sum over the 20 type columns picks one entry (nothing for a type id outside them).
        let immune_mult = if (0..N_TYPE_MULT as i64).contains(&mv.mtype) { dfn.type_mult[mv.mtype as usize] } else { 0.0 };
        let (fire, water) = (f32::from(mv.mtype == FIRE), f32::from(mv.mtype == WATER));
        let weather = 1.0 + sun * (0.5 * fire - 0.5 * water) + rain * (0.5 * water - 0.5 * fire);
        let burn = if physical { 1.0 - 0.5 * att.burned } else { 1.0 };
        let screen = 1.0 - (if physical { dfn.screen_phys } else { dfn.screen_spec }) / 3.0;
        let mut damage = base * stab * eff * immune_mult * weather * burn * screen * ROLL * mv.hits;
        let lands = f32::from(eff > 0.0);
        if mv.special == 4 {
            damage = att.level * lands;
        }
        if mv.special == 5 {
            damage = dfn.hp / 2.0 * lands;
        }
        if mv.special == 6 || mv.category == 0 {
            damage = 0.0;
        }
        damage = damage * modifier;
        let top = damage / ROLL;
        let ko = ((top - dfn.hp) / (0.15 * top).max(1e-3)).clamp(0.0, 1.0) * mv.accuracy;
        let dealt = damage.min(dfn.hp) / dfn.max_hp * mv.accuracy;
        (dealt * dfn.alive, ko * dfn.alive, eff * immune_mult)
    }

    /// `foe_attacks(ids, floats, t, defenders, field)` for one row.
    ///
    /// For each foe active f (token 6 + f) the [`M_FOE`] moves are its 4 revealed slots (probability `move_known`),
    /// its K_MOVES belief candidates (probability `cand_move_p`), both times `move id > 0`, then three generic
    /// attacks of its two base types and its Tera type (only once terastallized; type 0 deals nothing): power 80,
    /// accuracy 1, physical when `atk >= spa` else special, probability 1 only when the revealed and candidate
    /// probabilities sum to 0 before masking by `att.alive`. All move probabilities are times `att.alive`.
    /// Spread moves get the [`SPREAD`] modifier. `dealt`/`ko` are
    /// [`DamageFeatures::damage`] of each move against every `defenders` entry; `probs` are not clamped.
    pub fn foe_attacks(&self, ids: &[i64], floats: &[f32], t: &[Tok; 12], defenders: &[Tok], field: &[f32]) -> FoeAttacks {
        let n_def = defenders.len();
        let mut out = FoeAttacks {
            n_def,
            dealt: vec![0.0; 2 * M_FOE * n_def],
            ko: vec![0.0; 2 * M_FOE * n_def],
            probs: [[0.0; M_FOE]; 2],
        };
        self.foe_attacks_into(ids, floats, t, defenders, field, &[], false, &mut out.dealt, &mut out.ko, &mut out.probs);
        out
    }

    /// [`DamageFeatures::foe_attacks`] into caller buffers (`[2 * M_FOE * defenders.len()]`). Only the defenders
    /// with `need[d]` are computed (all of them for an empty `need`), and with `skip_absent` only the moves with
    /// probability > 0 (the others do not change [`expected_max`] or [`any_ko`]); skipped entries are left as they are.
    #[allow(clippy::too_many_arguments)]
    fn foe_attacks_into(
        &self,
        ids: &[i64],
        floats: &[f32],
        t: &[Tok; 12],
        defenders: &[Tok],
        field: &[f32],
        need: &[bool],
        skip_absent: bool,
        dealt: &mut [f32],
        ko: &mut [f32],
        probs: &mut [[f32; M_FOE]; 2],
    ) {
        let n_def = defenders.len();
        let (sun, rain) = (field[ff::SUN], field[ff::RAIN]);
        for (f, &token) in FOE_TOKENS.iter().enumerate() {
            let id = &ids[token * N_IDS..(token + 1) * N_IDS];
            let fl = &floats[token * N_POKEMON_FLOATS..(token + 1) * N_POKEMON_FLOATS];
            let att = &t[token];
            let mut moves = [0i64; M_IDS];
            moves[..4].copy_from_slice(&id[I_MOVE0..I_MOVE0 + 4]);
            moves[4..].copy_from_slice(&id[I_CAND_MOVES..I_CAND_MOVES + K_MOVES]);
            let mut sum = 0.0f32;
            for (m, &mid) in moves.iter().enumerate() {
                let p = if m < 4 { fl[pf::MOVE_KNOWN0 + m] } else { fl[pf::CAND_MOVE_P0 + m - 4] };
                let p = p * f32::from(mid > 0);
                sum += p;
                probs[f][m] = p * att.alive;
            }
            let stab_types = [att.base_types[0], att.base_types[1], att.tera * i64::from(att.terastallized > 0.5)];
            let better = if att.atk >= att.spa { 1 } else { 2 };
            let mut inputs = [MoveIn::default(); M_FOE];
            let mut modifier = [1.0f32; M_FOE];
            for (m, &mid) in moves.iter().enumerate() {
                let row = &self.moves[mid as usize];
                inputs[m] = row.input;
                if row.spread {
                    modifier[m] = SPREAD;
                }
            }
            for (j, &stab) in stab_types.iter().enumerate() {
                probs[f][M_IDS + j] = f32::from(sum == 0.0) * f32::from(stab > 0) * att.alive;
                inputs[M_IDS + j] =
                    MoveIn { mtype: stab, category: better * i64::from(stab > 0), power: 80.0, accuracy: 1.0, hits: 1.0, special: 0 };
            }
            for m in 0..M_FOE {
                if skip_absent && !(probs[f][m] > 0.0) {
                    continue;
                }
                let base = (f * M_FOE + m) * n_def;
                for (d, dfn) in defenders.iter().enumerate() {
                    if need.is_empty() || need[d] {
                        let (x, k, _) = self.damage_with(att, dfn, &inputs[m], sun, rain, modifier[m]);
                        dealt[base + d] = x;
                        ko[base + d] = k;
                    }
                }
            }
        }
    }

    /// `_has_move(table)` for one token: 1 when a revealed slot holds a move of `table` (times `move_known`), else
    /// the summed belief over the candidates (capped at 1): `known + (1 - known) * believed`.
    fn has_move(&self, id: &[i64], f: &[f32], table: impl Fn(&MoveRow) -> f32) -> f32 {
        let mut known = f32::NEG_INFINITY;
        for k in 0..4 {
            known = known.max(table(&self.moves[id[I_MOVE0 + k] as usize]) * f[pf::MOVE_KNOWN0 + k]);
        }
        let mut believed = 0.0f32;
        for k in 0..K_MOVES {
            believed += table(&self.moves[id[I_CAND_MOVES + k] as usize]) * f[pf::CAND_MOVE_P0 + k];
        }
        known + (1.0 - known) * believed.min(1.0)
    }

    /// `_ability_p(table)` for one token: `table[ability]` when the ability is known, else the belief over the
    /// candidate abilities (capped at 1).
    fn ability_p(id: &[i64], f: &[f32], table: &[f32]) -> f32 {
        let ability = id[I_ABILITY];
        if ability > 0 {
            return table[ability as usize];
        }
        let mut believed = 0.0f32;
        for k in 0..K_ABILITIES {
            believed += table[id[I_CAND_ABILITIES + k] as usize] * f[pf::CAND_ABILITY_P0 + k];
        }
        believed.min(1.0)
    }

    /// `_item_p(item)` for one token: whether it holds `item` when the item is known, else the belief over the
    /// candidate items (capped at 1).
    fn item_p(id: &[i64], f: &[f32], item: i64) -> f32 {
        let held = id[I_ITEM];
        if held > 0 {
            return f32::from(held == item);
        }
        let mut believed = 0.0f32;
        for k in 0..K_ITEMS {
            believed += f32::from(id[I_CAND_ITEMS + k] == item) * f[pf::CAND_ITEM_P0 + k];
        }
        believed.min(1.0)
    }

    /// `_per_token(ids, floats, field, t)` for one row.
    pub fn per_token(&self, ids: &[i64], floats: &[f32], field: &[f32], t: &[Tok; 12]) -> PerToken {
        let mut p = PerToken::default();
        let mut blocker = [0.0f32; 12];
        let gravity = field[ff::GRAVITY] > 0.5;
        let psychic = field[ff::PSYCHICTERRAIN];
        let repeat = (1.0 - REPEAT_PROTECT) as f32; // a Python float, rounded once to f32 by torch
        for i in 0..N_TOKENS {
            let id = &ids[i * N_IDS..(i + 1) * N_IDS];
            let f = &floats[i * N_POKEMON_FLOATS..(i + 1) * N_POKEMON_FLOATS];
            let on = t[i].alive * f32::from(f[pf::ACTIVE_A] + f[pf::ACTIVE_B] > 0.5);
            p.on[i] = on;
            p.own[i] = f[pf::OWN] > 0.5;
            let flinch_ability_ok = 1.0 - Self::ability_p(id, f, &self.ability_flinch_block);
            let flinch_other_ok = (1.0 - Self::item_p(id, f, self.covert_cloak)) * (1.0 - f[pf::SUBSTITUTE]);
            p.flinch_ok[i] = flinch_ability_ok * flinch_other_ok;
            p.flinch_other_ok[i] = flinch_other_ok;
            let flying = f32::from(t[i].def_types.contains(&self.flying));
            let airborne = 1.0
                - (1.0 - flying)
                    * (1.0 - Self::ability_p(id, f, &self.ability_levitate))
                    * (1.0 - Self::item_p(id, f, self.air_balloon));
            p.grounded[i] = if gravity { 1.0 } else { 1.0 - airborne };
            blocker[i] = Self::ability_p(id, f, &self.ability_priority_block) * on;
            p.repeat[i] = 1.0 - repeat * f[pf::PROTECT_LAST];
            p.fresh[i] = f[pf::FRESH] * on;
            p.can_protect[i] = self.has_move(id, f, |m| m.protect) * p.repeat[i] * on;
            p.fake_out_ready[i] = self.has_move(id, f, |m| m.fake_out) * p.fresh[i];
            p.unseen_fist[i] = Self::ability_p(id, f, &self.ability_unseen_fist);
        }
        let own_block = 1.0 - (1.0 - blocker[0]) * (1.0 - blocker[1]);
        let foe_block = 1.0 - (1.0 - blocker[6]) * (1.0 - blocker[7]);
        for i in 0..N_TOKENS {
            let side_block = if p.own[i] { own_block } else { foe_block };
            p.priority_ok[i] = (1.0 - psychic * p.grounded[i]) * (1.0 - side_block);
        }
        p
    }

    /// Own-side entry hazards as a fraction of max HP, capped at current HP. Immunities are known ids only.
    fn entry_damage(&self, t: &Tok, ability: i64, grounded: f32, field: &[f32]) -> f32 {
        if t.item == self.boots || ability == self.magic_guard || t.alive == 0.0 {
            return 0.0;
        }
        let row = self.rock as usize * self.n_types;
        let rock = field[ff::OWN_STEALTHROCK] / 8.0
            * self.typechart[row + t.def_types[0] as usize]
            * self.typechart[row + t.def_types[1] as usize];
        let layers = (field[ff::OWN_SPIKES] * 3.0).round() as usize;
        let spikes = match layers {
            0 => 0.0,
            1 => 1.0 / 8.0,
            2 => 1.0 / 6.0,
            _ => 1.0 / 4.0,
        };
        (rock + spikes * grounded).min(t.hp_frac)
    }

    /// `forward` for one row: writes `action` `[2 * 40 * N_ACTION_FEATURES]` and `token` `[12 * N_TOKEN_FEATURES]`.
    ///
    /// `action[slot][code]` (code = move_slot * 10 + target_code * 2 + tera_bit): damage dealt to the foe and own
    /// side, KO chances on them, the main target's log2 effectiveness / 2 and immunity, moving first against each
    /// foe active, priority / 5, then the ACTION_COLUMNS of damage.py (fail, flinch a/b, stop damage/KO, protect
    /// and the damage/KO it blocks, foe a/b can Protect; the Tera deltas only on tera_bit 1 codes, zero otherwise).
    /// `token[i]`: threat and KO chance from each opposing active, outspeeding each, then fake_out_ready,
    /// flinchable, fake_out_threat, protect_ready; all times `present`. Rust-only columns append own-side offense
    /// (including switching vision for the bench), entry damage, and the foe actives' Protect choice prior.
    pub fn row(&self, ids: &[i64], floats: &[f32], field: &[f32], action: &mut [f32], token: &mut [f32]) {
        assert_eq!(action.len(), 2 * N_CODES * N_ACTION_FEATURES, "action buffer");
        assert_eq!(token.len(), N_TOKENS * N_TOKEN_FEATURES, "token buffer");
        let fl = |i: usize, c: usize| floats[i * N_POKEMON_FLOATS + c];
        let (sun, rain) = (field[ff::SUN], field[ff::RAIN]);
        let t = self.tokens(ids, floats, field);
        let tera_t = [terastallized(&t[0]), terastallized(&t[1])];
        let is_own: [bool; 12] = std::array::from_fn(|i| fl(i, pf::OWN) > 0.5);
        let present: [f32; 12] = std::array::from_fn(|i| fl(i, pf::PRESENT));

        // Own actives (attackers 0, 1) and the same terastallized (2, 3) with their real moves against every token.
        let attackers = [t[0], t[1], tera_t[0], tera_t[1]];
        let moves: [[i64; 4]; 2] = std::array::from_fn(|s| std::array::from_fn(|k| ids[s * N_IDS + I_MOVE0 + k]));
        // Defenders whose damage reaches an output: the hit tokens 0, 1, 6, 7 (also the main target of a code that
        // hits nothing, token 0), and the present opposing tokens for threat_from_own; the Tera attackers only feed
        // the foe-side Tera deltas.
        let own_need: [bool; 12] = std::array::from_fn(|d| matches!(d, 0 | 1 | 6 | 7) || (!is_own[d] && present[d] != 0.0));
        let mut all_dealt = [[[0.0f32; 12]; 4]; 4];
        let mut all_ko = [[[0.0f32; 12]; 4]; 4];
        let mut own_eff = [[[0.0f32; 12]; 4]; 2];
        for (a, att) in attackers.iter().enumerate() {
            let s = a % 2;
            for k in 0..4 {
                let m = moves[s][k];
                let mut mv = self.move_in(m);
                if a >= 2 && m == self.tera_blast {
                    // Tera Blast takes the Tera type (not Stellar), and is physical when Attack is higher.
                    let tera = tera_t[s].tera;
                    if tera > 0 && tera != STELLAR {
                        mv.mtype = tera;
                    }
                    mv.category = if tera_t[s].atk > tera_t[s].spa { 1 } else { 2 };
                }
                for d in 0..N_TOKENS {
                    if (a < 2 && own_need[d]) || d == 6 || d == 7 {
                        let (x, ko, eff) = self.damage_with(att, &t[d], &mv, sun, rain, 1.0);
                        all_dealt[a][k][d] = x;
                        all_ko[a][k][d] = ko;
                        if a < 2 {
                            own_eff[a][k][d] = eff;
                        }
                    }
                }
            }
        }

        // The foes' moves against the 12 tokens and the own actives terastallized (12, 13). Needed: the own actives
        // (effects), their Tera versions, and the present own tokens (token threat).
        let mut targets = [Tok::default(); N_TARGETS];
        targets[..N_TOKENS].copy_from_slice(&t);
        targets[N_TOKENS..].copy_from_slice(&tera_t);
        let foe_need: [bool; N_TARGETS] =
            std::array::from_fn(|d| d < 2 || d >= N_TOKENS || (is_own[d] && present[d] != 0.0));
        let mut foe_dealt = [0.0f32; 2 * M_FOE * N_TARGETS];
        let mut foe_ko = [0.0f32; 2 * M_FOE * N_TARGETS];
        let mut foe_probs = [[0.0f32; M_FOE]; 2];
        self.foe_attacks_into(ids, floats, &t, &targets, field, &foe_need, true, &mut foe_dealt, &mut foe_ko, &mut foe_probs);
        let column = |table: &[f32], f: usize, d: usize| -> [f32; M_FOE] { std::array::from_fn(|m| table[(f * M_FOE + m) * N_TARGETS + d]) };

        // Speed: does each token outspeed each opposing active (Trick Room flips).
        let trick_room = field[ff::TRICKROOM] > 0.5;
        let faster = |a: f32, b: f32| {
            let x = f32::from(a > b) + 0.5 * f32::from(a == b);
            if trick_room { 1.0 - x } else { x }
        };
        let faster_foe: [[f32; 2]; 12] = std::array::from_fn(|i| std::array::from_fn(|f| faster(t[i].spe, t[FOE_TOKENS[f]].spe)));
        let faster_own: [[f32; 2]; 12] = std::array::from_fn(|i| std::array::from_fn(|s| faster(t[i].spe, t[OWN_TOKENS[s]].spe)));
        let mut threat_all = [[0.0f32; 2]; N_TARGETS];
        let mut ko_all = [[0.0f32; 2]; N_TARGETS];
        for d in 0..N_TARGETS {
            if foe_need[d] {
                for f in 0..2 {
                    threat_all[d][f] = expected_max(&column(&foe_dealt, f, d), &foe_probs[f]);
                    ko_all[d][f] = any_ko(&column(&foe_ko, f, d), &foe_probs[f]);
                }
            }
        }
        let max4 = |v: &[[f32; 12]; 4], d: usize| v[0][d].max(v[1][d]).max(v[2][d]).max(v[3][d]);
        for i in 0..N_TOKENS {
            let out = &mut token[i * N_TOKEN_FEATURES..(i + 1) * N_TOKEN_FEATURES];
            for j in 0..2 {
                let (threat, ko, first) = if is_own[i] {
                    (threat_all[i][j], ko_all[i][j], faster_foe[i][j])
                } else {
                    (max4(&all_dealt[j], i), max4(&all_ko[j], i), faster_own[i][j])
                };
                out[j] = threat * present[i];
                out[2 + j] = ko * present[i];
                out[4 + j] = first * present[i];
            }
        }

        // What still lands through Protect: moves without the protect flag, and Unseen Fist contact moves.
        let p = self.per_token(ids, floats, field, &t);
        // Switching vision: reuse the actives' damage and the hit-table shares, including spread reduction.
        // Bench Pokemon use the same formula with their current stats, moves, types and items.
        let mut offense = [[0.0f32; 2]; 6];
        let mut offense_ko = [[0.0f32; 2]; 6];
        for i in 0..6 {
            if !is_own[i] || t[i].alive == 0.0 {
                continue;
            }
            for k in 0..4 {
                let m = ids[i * N_IDS + I_MOVE0 + k];
                let row = &self.moves[m as usize];
                for (f, &foe) in FOE_TOKENS.iter().enumerate() {
                    let share = self.hit[row.target][i.min(1)].iter().map(|h| h[foe]).fold(0.0f32, f32::max);
                    if share == 0.0 {
                        continue;
                    }
                    let (d, ko) = if i < 2 {
                        (all_dealt[i][k][foe], all_ko[i][k][foe])
                    } else {
                        let (d, ko, _) = self.damage_with(&t[i], &t[foe], &row.input, sun, rain, 1.0);
                        (d, ko)
                    };
                    offense[i][f] = offense[i][f].max(d * share);
                    offense_ko[i][f] = offense_ko[i][f].max(ko * share);
                }
            }
        }
        let protect_now: [f32; 2] = std::array::from_fn(|f| {
            let damage = (offense[0][f] * p.on[0] + offense[1][f] * p.on[1]).min(1.0);
            let ko = 1.0 - (1.0 - offense_ko[0][f] * p.on[0]) * (1.0 - offense_ko[1][f] * p.on[1]);
            protect_prior(p.can_protect[FOE_TOKENS[f]], damage, ko)
        });
        let mut through = [[0.0f32; 2]; 2]; // [user slot][foe]
        let mut through_ko = [[0.0f32; 2]; 2];
        for (f, &foe) in FOE_TOKENS.iter().enumerate() {
            let unseen = p.unseen_fist[foe];
            let mut bypass = [0.0f32; M_FOE]; // generic attacks: blocked
            for m in 0..M_IDS {
                let mid = if m < 4 { ids[foe * N_IDS + I_MOVE0 + m] } else { ids[foe * N_IDS + I_CAND_MOVES + m - 4] };
                let row = &self.moves[mid as usize];
                bypass[m] = 1.0 - row.blockable * (1.0 - unseen * row.contact);
            }
            for u in 0..2 {
                let (d, k) = (column(&foe_dealt, f, u), column(&foe_ko, f, u));
                through[u][f] = expected_max(&std::array::from_fn::<f32, M_FOE, _>(|m| d[m] * bypass[m]), &foe_probs[f]);
                through_ko[u][f] = any_ko(&std::array::from_fn::<f32, M_FOE, _>(|m| k[m] * bypass[m]), &foe_probs[f]);
            }
        }

        // Row-level terms of effects and tera_effects.
        let foe_on = [p.on[6], p.on[7]];
        let own_on = [p.on[0], p.on[1]];
        let foe_threat: [f32; 2] =
            std::array::from_fn(|f| (threat_all[0][f] * own_on[0]).max(threat_all[1][f] * own_on[1]) * foe_on[f]);
        let foe_kos: [f32; 2] = std::array::from_fn(|f| (ko_all[0][f] * own_on[0]).max(ko_all[1][f] * own_on[1]) * foe_on[f]);
        let faced = |threat: [f32; 2]| (threat[0] * foe_on[0] + threat[1] * foe_on[1]).min(1.0);
        let faced_ko = |ko: [f32; 2]| 1.0 - (1.0 - ko[0] * foe_on[0]) * (1.0 - ko[1] * foe_on[1]);
        let dark: [f32; 2] = std::array::from_fn(|f| f32::from(t[FOE_TOKENS[f]].def_types.contains(&self.dark)));
        let grassy = field[ff::GRASSYTERRAIN] > 0.5;

        for s in 0..2 {
            let ability = ids[s * N_IDS + I_ABILITY];
            let sheer = f32::from(ability == self.sheer_force);
            let serene = f32::from(ability == self.serene_grace);
            let mold = self.ability_mold_breaker[ability as usize];
            let can_flinch: [f32; 2] = std::array::from_fn(|f| {
                let foe = FOE_TOKENS[f];
                (p.flinch_ok[foe] + (p.flinch_other_ok[foe] - p.flinch_ok[foe]) * mold) * foe_on[f]
            });
            let protect_faced = faced(threat_all[s]) - faced(through[s]);
            let protect_faced_ko = faced_ko(ko_all[s]) - faced_ko(through_ko[s]);
            let can_tera = fl(s, pf::TERA_AVAILABLE) * (1.0 - fl(s, pf::TERASTALLIZED));
            let defence = [
                faced(threat_all[N_TOKENS + s]) - faced(threat_all[s]),
                faced_ko(ko_all[N_TOKENS + s]) - faced_ko(ko_all[s]),
            ];
            for k in 0..4 {
                let m = moves[s][k];
                let row = &self.moves[m as usize];
                let hits = &self.hit[row.target][s];
                let (dealt, kos, eff) = (&all_dealt[s][k], &all_ko[s][k], &own_eff[s][k]);
                let (tera_dealt, tera_kos) = (&all_dealt[2 + s][k], &all_ko[2 + s][k]);

                // Legacy columns: raw move priority against the speed order.
                let legacy_first: [f32; 2] = std::array::from_fn(|f| {
                    if row.priority > 0.0 {
                        1.0
                    } else if row.priority < 0.0 {
                        0.0
                    } else {
                        faster_foe[s][f]
                    }
                });

                // Priority as used: Prankster status moves, Gale Wings at full HP, Grassy Glide in Grassy Terrain.
                let prankster = ability == self.prankster && row.input.category == 0;
                let gale = ability == self.gale_wings && row.input.mtype == self.flying && t[s].hp_frac >= 1.0;
                let glide = m == self.grassy_glide && grassy && p.grounded[s] > 0.5;
                let priority = row.priority + f32::from(prankster) + f32::from(gale) + f32::from(glide);
                let stale = row.first_only * (1.0 - p.fresh[s]);

                // Acting before each foe: priority, then speed; a foe's first-turn Fake Out (+3) can come first.
                let first: [f32; 2] = std::array::from_fn(|f| {
                    let speed = faster_foe[s][f];
                    let before = if priority > 0.0 {
                        1.0
                    } else if priority < 0.0 {
                        0.0
                    } else {
                        speed
                    };
                    let race = if priority > 3.0 {
                        0.0
                    } else if priority == 3.0 {
                        1.0 - speed
                    } else {
                        1.0
                    };
                    before * (1.0 - p.fake_out_ready[FOE_TOKENS[f]] * race)
                });
                let chance = (row.flinch * (1.0 + serene)).min(1.0) * (1.0 - sheer);
                let share = if row.random_normal { 0.5 } else { 1.0 };
                let prio_ok: [f32; 2] = std::array::from_fn(|f| if priority > 0.0 { p.priority_ok[FOE_TOKENS[f]] } else { 1.0 });
                let landed: [f32; 2] = std::array::from_fn(|f| f32::from(eff[FOE_TOKENS[f]] > 0.0));
                let protect = row.protect * p.repeat[s] * p.on[s];
                let unseen = p.unseen_fist[s] * row.contact;
                let blockable = row.blockable * (1.0 - unseen);

                for c in 0..5 {
                    let h = &hits[c];
                    let (mut dealt_foe, mut dealt_own, mut ko_foe, mut ko_own) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
                    let (mut tera_d, mut tera_k) = (0.0f32, 0.0f32);
                    for d in 0..N_TOKENS {
                        let x = h[d] * dealt[d];
                        let y = h[d] * kos[d];
                        if d >= 6 {
                            dealt_foe += x;
                            ko_foe += y;
                            tera_d += h[d] * (tera_dealt[d] - dealt[d]);
                            tera_k += h[d] * (tera_kos[d] - kos[d]);
                        } else {
                            dealt_own += x;
                            ko_own += y;
                        }
                    }
                    let any_hit = f32::from(self.hit_any[row.target][s][c]);
                    let eff_main = eff[self.hit_main[row.target][s][c]];
                    let on_target: [f32; 2] = std::array::from_fn(|f| f32::from(h[FOE_TOKENS[f]] > 0.0));

                    // Fails outright: first-turn-only moves after a move action; priority moves at a protected foe;
                    // Prankster status moves at a Dark-type foe.
                    let blocked = (on_target[0] * (1.0 - p.priority_ok[6])).max(on_target[1] * (1.0 - p.priority_ok[7]));
                    let prankster_dark = f32::from(prankster) * (on_target[0] * dark[0]).max(on_target[1] * dark[1]);
                    let fail = 1.0 - (1.0 - stale) * (1.0 - f32::from(priority > 0.0) * blocked) * (1.0 - prankster_dark);
                    let flinch: [f32; 2] = std::array::from_fn(|f| {
                        chance * row.input.accuracy * on_target[f] * share * first[f] * can_flinch[f] * prio_ok[f] * landed[f]
                            * (1.0 - stale)
                    });
                    let foe_protect: [f32; 2] =
                        std::array::from_fn(|f| on_target[f] * p.can_protect[FOE_TOKENS[f]] * blockable);
                    let protect_chance: [f32; 2] = std::array::from_fn(|f| on_target[f] * protect_now[f] * blockable);
                    let after_damage: f32 = FOE_TOKENS.iter().enumerate()
                        .map(|(f, &foe)| h[foe] * dealt[foe] * (1.0 - protect_chance[f])).sum();
                    let after_ko: f32 = FOE_TOKENS.iter().enumerate()
                        .map(|(f, &foe)| h[foe] * kos[foe] * (1.0 - protect_chance[f])).sum();

                    let columns = [
                        dealt_foe,
                        dealt_own,
                        ko_foe,
                        ko_own,
                        eff_main.max(0.125).log2() / 2.0 * any_hit,
                        f32::from(eff_main == 0.0) * any_hit,
                        legacy_first[0],
                        legacy_first[1],
                        row.priority / 5.0,
                        fail,
                        flinch[0],
                        flinch[1],
                        flinch[0] * foe_threat[0] + flinch[1] * foe_threat[1],
                        flinch[0] * foe_kos[0] + flinch[1] * foe_kos[1],
                        protect,
                        protect * protect_faced,
                        protect * protect_faced_ko,
                        foe_protect[0],
                        foe_protect[1],
                    ];
                    let tera = [tera_d * can_tera, tera_k * can_tera, defence[0] * can_tera, defence[1] * can_tera];
                    for tera_bit in 0..2 {
                        let code = k * 10 + c * 2 + tera_bit;
                        let out = &mut action[(s * N_CODES + code) * N_ACTION_FEATURES..(s * N_CODES + code + 1) * N_ACTION_FEATURES];
                        out[..columns.len()].copy_from_slice(&columns);
                        if tera_bit == 1 {
                            out[columns.len()..REFERENCE_ACTION_FEATURES].copy_from_slice(&tera);
                        } else {
                            out[columns.len()..REFERENCE_ACTION_FEATURES].fill(0.0);
                        }
                        out[REFERENCE_ACTION_FEATURES..].copy_from_slice(&[
                            protect_chance[0], protect_chance[1], after_damage, after_ko,
                        ]);
                    }
                }
            }
        }

        // token_effects: fake_out_ready, flinchable, fake_out_threat, protect_ready.
        let ready = p.fake_out_ready;
        let from_own = 1.0 - (1.0 - ready[0]) * (1.0 - ready[1]);
        let from_foe = 1.0 - (1.0 - ready[6]) * (1.0 - ready[7]);
        let row = self.normal as usize * self.n_types;
        for i in 0..N_TOKENS {
            let normal = self.typechart[row + t[i].def_types[0] as usize]
                * self.typechart[row + t[i].def_types[1] as usize]
                * t[i].type_mult[self.normal as usize];
            let flinchable = p.flinch_ok[i] * p.priority_ok[i] * f32::from(normal > 0.0) * t[i].alive;
            let threat = flinchable * p.on[i] * if p.own[i] { from_foe } else { from_own };
            let out = &mut token[i * N_TOKEN_FEATURES + LEGACY_TOKEN_FEATURES..i * N_TOKEN_FEATURES + REFERENCE_TOKEN_FEATURES];
            out.copy_from_slice(&[ready[i] * present[i], flinchable * present[i], threat * present[i], p.can_protect[i] * present[i]]);
            let out = &mut token[i * N_TOKEN_FEATURES + REFERENCE_TOKEN_FEATURES..(i + 1) * N_TOKEN_FEATURES];
            out.fill(0.0);
            if i < 6 && is_own[i] {
                out[..2].copy_from_slice(&offense[i]);
                out[2..4].copy_from_slice(&offense_ko[i]);
                out[4] = self.entry_damage(&t[i], ids[i * N_IDS + I_ABILITY], p.grounded[i], field);
            } else if i == FOE_A || i == FOE_B {
                out[5] = protect_now[i - FOE_A];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn protect_prior_tracks_threat_and_success() {
        assert_eq!(protect_prior(0.0, 1.0, 1.0), 0.0);
        assert_eq!(protect_prior(1.0, 0.0, 0.0), PROTECT_BASE);
        assert!((protect_prior(1.0, 1.0, 1.0) - 0.6).abs() < 1e-6);
        assert!((protect_prior(REPEAT_PROTECT as f32, 1.0, 1.0) - 0.2).abs() < 1e-6);
        assert_eq!(protect_prior(1.0, 10.0, 10.0), PROTECT_MAX);
        assert_eq!(protect_prior(1.0, -10.0, -10.0), 0.0);
    }

    #[test]
    fn entry_hazards_use_types_grounding_immunities_and_hp() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json");
        let dex = Dex::shared(&path).expect("dex.json");
        let dm = DamageFeatures::new(&dex);
        let fire = dex.type_idx("Fire");
        let flying = dex.type_idx("Flying");
        let mut t = Tok { def_types: [fire, flying], hp_frac: 1.0, alive: 1.0, ..Tok::default() };
        let mut field = [0.0; N_FIELD_FLOATS];
        field[ff::OWN_STEALTHROCK] = 1.0;
        assert_eq!(dm.entry_damage(&t, 0, 0.0, &field), 0.5);
        t.def_types = [fire, 0];
        assert_eq!(dm.entry_damage(&t, 0, 1.0, &field), 0.25);
        t.def_types = [dex.type_idx("Fighting"), dex.type_idx("Steel")];
        assert_eq!(dm.entry_damage(&t, 0, 1.0, &field), 1.0 / 32.0);
        t.item = dm.boots;
        assert_eq!(dm.entry_damage(&t, 0, 1.0, &field), 0.0);
        t.item = 0;
        assert_eq!(dm.entry_damage(&t, dm.magic_guard, 1.0, &field), 0.0);
        field[ff::OWN_STEALTHROCK] = 0.0;
        for (layers, loss) in [(1, 0.125), (2, 1.0 / 6.0), (3, 0.25)] {
            field[ff::OWN_SPIKES] = layers as f32 / 3.0;
            assert_eq!(dm.entry_damage(&t, 0, 1.0, &field), loss);
            assert_eq!(dm.entry_damage(&t, 0, 0.0, &field), 0.0);
        }
        t.hp_frac = 0.05;
        assert_eq!(dm.entry_damage(&t, 0, 1.0, &field), 0.05);
        t.alive = 0.0;
        assert_eq!(dm.entry_damage(&t, 0, 1.0, &field), 0.0);
    }

    #[test]
    fn hit_table_entries() {
        let hit = hit_table();
        let class = |name: &str| TARGETS.iter().position(|t| *t == name).unwrap();
        for s in 0..2 {
            let normal = &hit[class("normal")][s];
            assert_eq!((normal[0][1], normal[1][0], normal[3][6], normal[4][7]), (1.0, 1.0, 1.0, 1.0));
            assert_eq!(normal.iter().flatten().sum::<f32>(), 4.0);
            assert!(normal[2].iter().all(|&x| x == 0.0));
            let all = &hit[class("allAdjacent")][s][2];
            assert_eq!((all[6], all[7], all[1 - s], all[s]), (SPREAD, SPREAD, SPREAD, 0.0));
            let foes = &hit[class("allAdjacentFoes")][s][2];
            assert_eq!((foes[6], foes[7], foes.iter().sum::<f32>()), (SPREAD, SPREAD, 1.5));
            let random = &hit[class("randomNormal")][s][2];
            assert_eq!((random[6], random[7]), (0.5, 0.5));
            assert!(hit[class("self")][s].iter().flatten().all(|&x| x == 0.0));
        }
    }

    #[test]
    fn hit_main_prefers_the_first_foe_on_positive_ties() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json");
        let dex = Dex::shared(&path).expect("dex.json");
        let dm = DamageFeatures::new(&dex);
        let class = |name: &str| TARGETS.iter().position(|t| *t == name).unwrap();
        for s in 0..2 {
            for name in ["allAdjacent", "allAdjacentFoes", "randomNormal"] {
                assert_eq!(dm.hit_main[class(name)][s][2], 6, "{name}, slot {s}");
            }
            for name in ["normal", "any", "adjacentFoe", "adjacentAlly", "adjacentAllyOrSelf"] {
                assert_eq!(dm.hit_main[class(name)][s], [1, 0, 0, 6, 7], "{name}, slot {s}");
            }
            assert_eq!(dm.hit_main[class("self")][s], [0; 5]);
        }
    }

    #[test]
    fn foe_attacks_mask_fainted_and_empty_slots() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json");
        let dex = Dex::shared(&path).expect("dex.json");
        let dm = DamageFeatures::new(&dex);
        let mut ids = [0; N_TOKENS * N_IDS];
        let mut floats = [0.0; N_TOKENS * N_POKEMON_FLOATS];
        let field = [0.0; N_FIELD_FLOATS];
        let defender = Tok { dfn: 100.0, spd: 100.0, hp: 1.0, max_hp: 100.0, alive: 1.0, type_mult: [1.0; 20], ..Tok::default() };
        for token in FOE_TOKENS {
            let id = &mut ids[token * N_IDS..(token + 1) * N_IDS];
            id[I_TYPE1] = dex.type_idx("Normal");
            id[I_TYPE2] = dex.type_idx("Fire");
            id[I_TERA] = dex.type_idx("Water");
            let fl = &mut floats[token * N_POKEMON_FLOATS..(token + 1) * N_POKEMON_FLOATS];
            fl[pf::LEVEL] = 1.0;
            fl[pf::STAT_ATK] = 100.0 / STAT_SCALE;
            fl[pf::STAT_SPA] = 100.0 / STAT_SCALE;
            fl[pf::TERASTALLIZED] = 1.0;
        }
        // Foe a has a revealed move and a candidate; foe b uses all three generic fallback attacks.
        ids[FOE_A * N_IDS + I_MOVE0] = dex.move_index("fakeout");
        ids[FOE_A * N_IDS + I_CAND_MOVES] = dex.move_index("flamethrower");
        floats[FOE_A * N_POKEMON_FLOATS + pf::MOVE_KNOWN0] = 1.0;
        floats[FOE_A * N_POKEMON_FLOATS + pf::CAND_MOVE_P0] = 0.25;
        for (present, fainted) in [(1.0, 0.0), (1.0, 1.0), (0.0, 0.0)] {
            for token in FOE_TOKENS {
                floats[token * N_POKEMON_FLOATS + pf::PRESENT] = present;
                floats[token * N_POKEMON_FLOATS + pf::FAINTED] = fainted;
            }
            let t = dm.tokens(&ids, &floats, &field);
            let attacks = dm.foe_attacks(&ids, &floats, &t, &[defender], &field);
            let alive = present * (1.0 - fainted);
            assert_eq!((attacks.probs[0][0], attacks.probs[0][4]), (alive, 0.25 * alive));
            assert_eq!(&attacks.probs[0][M_IDS..], &[0.0; 3]);
            assert_eq!(&attacks.probs[1][..M_IDS], &[0.0; M_IDS]);
            assert_eq!(&attacks.probs[1][M_IDS..], &[alive; 3]);
            let mut dealt = [-1.0; 2 * M_FOE];
            let mut ko = [-1.0; 2 * M_FOE];
            let mut probs = [[0.0; M_FOE]; 2];
            dm.foe_attacks_into(&ids, &floats, &t, &[defender], &field, &[], true, &mut dealt, &mut ko, &mut probs);
            assert_eq!(probs, attacks.probs);
            for f in 0..2 {
                let range = f * M_FOE..(f + 1) * M_FOE;
                let expected = expected_max(&attacks.dealt[range.clone()], &attacks.probs[f]);
                let risk = any_ko(&attacks.ko[range.clone()], &attacks.probs[f]);
                assert_eq!(expected_max(&dealt[range.clone()], &probs[f]), expected);
                assert_eq!(any_ko(&ko[range], &probs[f]), risk);
                if alive == 0.0 {
                    assert!(attacks.probs[f].iter().all(|&p| p == 0.0));
                    assert_eq!((expected, risk), (0.0, 0.0));
                } else {
                    assert!(expected > 0.0 && risk > 0.0);
                }
            }
        }
    }

    #[test]
    fn expected_max_and_any_ko_by_hand() {
        // Sorted 0.8 (p 0.5), 0.5 (p 1), 0.2 (p 0.5): 0.8 * 0.5 + 0.5 * 1 * 0.5 + 0.2 * 0.5 * 0 = 0.65.
        let e = expected_max(&[0.2, 0.8, 0.5], &[0.5, 0.5, 1.0]);
        assert!((e - 0.65).abs() < 1e-6, "{e}");
        // Probabilities are clamped to [0, 1]; absent moves deal nothing.
        let e = expected_max(&[0.4, 0.9, 0.3], &[2.0, 0.0, -1.0]);
        assert!((e - 0.4).abs() < 1e-6, "{e}");
        assert_eq!(expected_max(&[], &[]), 0.0);
        // 0.7 * 0.25 + 0.3 * 0.25 * 0.75 = 0.23125.
        let e = expected_max(&[0.3, 0.7], &[0.25, 0.25]);
        assert!((e - 0.23125).abs() < 1e-6, "{e}");
        // 1 - (1 - 0.5 * 0.4) * (1 - 1 * 0.5) = 0.6.
        let k = any_ko(&[0.4, 0.5, 1.0], &[0.5, 1.0, 0.0]);
        assert!((k - 0.6).abs() < 1e-6, "{k}");
        assert_eq!(any_ko(&[1.0], &[1.5]), 1.0);
    }

    #[test]
    fn stage_and_terastallized() {
        assert_eq!(stage(2.0), 2.0);
        assert_eq!(stage(-2.0), 0.5);
        assert_eq!(stage(1.0), 1.5);
        let t = Tok { tera: 5, base_types: [1, 3], def_types: [1, 3], ..Tok::default() };
        let tt = terastallized(&t);
        assert_eq!((tt.def_types, tt.terastallized, tt.stellar), ([5, 0], 1.0, 0.0));
        let s = terastallized(&Tok { tera: STELLAR, ..t });
        assert_eq!((s.def_types, s.stellar), ([1, 3], 1.0));
    }

    #[test]
    fn tables_match_the_python_lookups() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/training/dex.json");
        let dex = Dex::shared(&path).expect("dex.json");
        let dm = DamageFeatures::new(&dex);
        let mv = |id: &str| dm.moves[dex.move_index(id) as usize];
        assert_eq!(mv("triplearrows").flinch, 0.3);
        assert_eq!(mv("fakeout").flinch, 1.0);
        assert_eq!((mv("fakeout").fake_out, mv("fakeout").first_only), (1.0, 1.0));
        assert_eq!(mv("firstimpression").first_only, 1.0);
        assert_eq!(mv("protect").protect, 1.0);
        assert!(mv("earthquake").spread && mv("rockslide").spread && !mv("fakeout").spread);
        assert_eq!((mv("fakeout").contact, mv("fakeout").blockable), (1.0, 1.0));
        assert_eq!(dm.moves[0], MoveRow { input: MoveIn { accuracy: 1.0, hits: 1.0, ..MoveIn::default() }, ..MoveRow::default() });
        // Missing ids are -1 like dict.get(..., -1).
        assert_eq!(dm.air_balloon, if dex.item_index("airballoon") > 0 { dex.item_index("airballoon") } else { -1 });
        assert_eq!(dm.ability_priority_block[0], 0.0);
        assert_eq!(dm.ability_mold_breaker[dex.ability_index("teravolt") as usize], 1.0);
    }
}
