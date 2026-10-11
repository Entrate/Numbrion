//! Doubles-aware scripted player, matching `training.smart.SmartHeuristicAgent`.

use crate::{damage::{any_ko, expected_max, hit_table, DamageFeatures, M_FOE, SPREAD}, dex::{Dex, STELLAR, TARGETS}, layout::*};
use std::sync::Arc;

/// Python's random scoring knobs, exposed so deterministic parity can disable them.
#[derive(Clone, Copy, Debug)]
pub struct SmartConfig {
    /// Scale of independent Gumbel perturbations on pair values.
    pub temperature: f32,
    /// Scale of uniform tie-breaking noise.
    pub noise: f32,
    /// Probability per slot of excluding Protect for this decision.
    pub protect_skip: f32,
    /// Uniform range of the avoided damage credited to Protect.
    pub protect_share: (f32, f32),
}

impl Default for SmartConfig {
    fn default() -> Self {
        Self { temperature: 0.0, noise: 0.005, protect_skip: 0.15, protect_share: (0.55, 0.85) }
    }
}

/// Full row-major observation arrays; `first` selects the requested rows.
#[derive(Clone, Copy)]
pub struct ObsRows<'a> {
    pub ids: &'a [i64],
    pub floats: &'a [f32],
    pub field: &'a [f32],
    pub action_features: &'a [f32],
    pub token_features: &'a [f32],
}

/// Xoshiro256** seeded through SplitMix64. Uniform draws exclude both endpoints for finite Gumbel samples.
struct Rng([u64; 4]);
impl Rng {
    fn new(mut seed: u64) -> Self {
        let mut state = [0; 4];
        for word in &mut state {
            seed = seed.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            *word = z ^ (z >> 31);
        }
        Self(state)
    }
    fn next(&mut self) -> u64 {
        let s = &mut self.0;
        let out = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0]; s[3] ^= s[1]; s[1] ^= s[2]; s[0] ^= s[3]; s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }
    fn uniform(&mut self) -> f64 {
        ((self.next() >> 12) as f64 + 0.5) / ((1u64 << 52) as f64)
    }
    fn index(&mut self, n: usize) -> usize {
        let limit = u64::MAX - u64::MAX % n as u64;
        loop { let x = self.next(); if x < limit { return (x % n as u64) as usize; } }
    }
}

const PROTECT: &[&str] = &["protect", "detect", "spikyshield", "kingsshield", "banefulbunker", "obstruct", "silktrap", "burningbulwark"];
const SLEEP: &[&str] = &["spore", "sleeppowder", "hypnosis", "sing", "lovelykiss", "grasswhistle", "darkvoid", "yawn"];
const SLEEP_BLOCK: &[&str] = &["insomnia", "vitalspirit", "sweetveil", "comatose", "purifyingsalt"];
const FLINCH_BLOCK: &[&str] = &["innerfocus", "shielddust"];
const PRIORITY_BLOCK: &[&str] = &["armortail", "dazzling", "queenlymajesty"];
const SETUP: &[&str] = &["swordsdance", "nastyplot", "calmmind", "dragondance", "quiverdance", "shellsmash", "bulkup", "coil", "tailglow", "noretreat", "tidyup", "victorydance", "shiftgear", "workup", "growth", "filletaway", "clangoroussoul", "geomancy", "honeclaws", "rockpolish", "agility", "curse", "bellydrum"];
const DEFENSE: &[&str] = &["irondefense", "cosmicpower", "amnesia", "acidarmor", "cottonguard", "stockpile"];
const RECOVERY: &[&str] = &["recover", "roost", "slackoff", "synthesis", "moonlight", "morningsun", "softboiled", "shoreup", "milkdrink", "healorder", "strengthsap"];
const TEAM_HEAL: &[&str] = &["lifedew", "lunarblessing", "junglehealing"];
const ALLY_HEAL: &[&str] = &["healpulse", "floralhealing", "pollenpuff"];

#[derive(Clone, Copy, PartialEq)]
enum Status { None, Protect, Setup, Defense, Recovery, TeamHeal, AllyHeal, Screen, Sleep, Hazard, Tailwind, Trickroom, Paralysis, Burn, Poison, Leechseed, Revive, Rest }
fn status_kind(name: &str) -> Status {
    for (kind, names) in [(Status::Protect, PROTECT), (Status::Setup, SETUP), (Status::Defense, DEFENSE),
        (Status::Recovery, RECOVERY), (Status::TeamHeal, TEAM_HEAL), (Status::AllyHeal, ALLY_HEAL),
        (Status::Screen, &["reflect", "lightscreen", "auroraveil"][..]), (Status::Sleep, SLEEP)] {
        if names.contains(&name) { return kind; }
    }
    match name {
        "stealthrock" | "spikes" | "toxicspikes" | "stickyweb" => Status::Hazard,
        "tailwind" => Status::Tailwind, "trickroom" => Status::Trickroom,
        "thunderwave" | "glare" => Status::Paralysis, "willowisp" => Status::Burn,
        "toxic" => Status::Poison, "leechseed" => Status::Leechseed,
        "revivalblessing" => Status::Revive, "rest" => Status::Rest, _ => Status::None,
    }
}
#[derive(Clone, Copy)]
struct MoveInfo {
    protect: bool, first_only: bool, fake_out: bool, helping: bool, redirect: bool,
    recoil: f32, destruct: bool, recharge: bool, harmful: bool, status: Status,
}

#[derive(Clone, Copy, Debug)]
struct Effect {
    off_d: [f32; 2], off_k: [f32; 2], first: [f32; 2], flinch: [f32; 2],
    inc_d: [f32; 2], inc_k: [f32; 2], ally_d: f32, ally_k: f32,
    protect: f32, helping: f32, redirect: f32, bonus: f32, act_bonus: f32,
    tera: bool, switch: i32, allowed: bool, fake_out: bool, strong: bool,
}
impl Default for Effect {
    fn default() -> Self {
        Self { off_d: [0.0; 2], off_k: [0.0; 2], first: [0.0; 2], flinch: [0.0; 2],
            inc_d: [0.0; 2], inc_k: [0.0; 2], ally_d: 0.0, ally_k: 0.0, protect: 0.0, helping: 0.0,
            redirect: 0.0, bonus: 0.0, act_bonus: 0.0, tera: false, switch: -1, allowed: true,
            fake_out: false, strong: false }
    }
}
struct Context {
    hp: [f32; 12], alive: [bool; 12], on: [bool; 12], own_on: [f32; 2], foe_on: [f32; 2],
    q: [[f32; 2]; 2], foe_protect: [f32; 2], foe_value: [f32; 2], matchup: [f32; 6],
    request_move: bool, protect_share: f32, tera_usable: [bool; 2],
    ids: Vec<i64>, floats: Vec<f32>, effects: [[Effect; N_ACTIONS]; 2], a0: usize,
}
impl Context {
    fn id(&self, t: usize, col: usize) -> i64 { self.ids[t * N_IDS + col] }
    fn p(&self, t: usize, col: usize) -> f32 { self.floats[t * N_POKEMON_FLOATS + col] }
}
struct Tables {
    dealt: [[[f32; 12]; 4]; 6], ko: [[[f32; 12]; 4]; 6], target: [[usize; 4]; 6],
    threat: [[f32; 2]; 12], kothreat: [[f32; 2]; 12],
}

pub struct SmartAgent {
    pub dex: Arc<Dex>,
    pub config: SmartConfig,
    damage: DamageFeatures,
    moves: Vec<MoveInfo>,
    hit: Box<[[[[f32; 12]; 5]; 2]; 15]>,
    rng: Rng,
    ctx: Vec<Context>,
}

impl SmartAgent {
    pub fn new(dex: Arc<Dex>, seed: u64, config: SmartConfig) -> Self {
        let mut moves = Vec::with_capacity(dex.move_data.len());
        for m in &dex.move_data {
            let status = if m.category == "Status" || m.id == "pollenpuff" { status_kind(&m.id) } else { Status::None };
            let harmful_effect = m.status.as_ref().is_some_and(|s| !s.is_empty()) || m.force_switch
                || m.boosts.iter().any(|(_, v)| *v < 0.0)
                || m.volatile_status.as_deref().is_some_and(|s| ["confusion", "taunt", "encore", "disable", "yawn", "leechseed", "curse"].contains(&s));
            let support = ALLY_HEAL.contains(&m.id.as_str()) || m.has_flag("allyanim")
                || (!m.boosts.is_empty() && m.boosts.iter().all(|(_, v)| *v >= 0.0));
            moves.push(MoveInfo { protect: PROTECT.contains(&m.id.as_str()),
                first_only: ["fakeout", "firstimpression"].contains(&m.id.as_str()), fake_out: m.id == "fakeout",
                helping: m.id == "helpinghand", redirect: ["followme", "ragepowder"].contains(&m.id.as_str()),
                recoil: m.recoil as f32, destruct: m.selfdestruct, recharge: m.has_flag("recharge"), status,
                harmful: m.category == "Status" && ["normal", "any", "adjacentFoe"].contains(&m.target.as_str()) && (harmful_effect || !support) });
        }
        Self { damage: DamageFeatures::new(&dex), hit: hit_table(), moves, dex, config, rng: Rng::new(seed), ctx: Vec::new() }
    }

    fn tables(&self, ids: &[i64], floats: &[f32], field: &[f32], threats: bool) -> Tables {
        let t = self.damage.tokens(ids, floats, field);
        let mut out = Tables { dealt: [[[0.0; 12]; 4]; 6], ko: [[[0.0; 12]; 4]; 6], target: [[0; 4]; 6],
            threat: [[0.0; 2]; 12], kothreat: [[0.0; 2]; 12] };
        for s in 0..6 { for m in 0..4 {
            let mid = ids[s * N_IDS + I_MOVE0 + m];
            let target = self.dex.move_target[mid as usize] as usize;
            out.target[s][m] = target;
            let modifier = if ["allAdjacent", "allAdjacentFoes"].contains(&TARGETS[target]) { SPREAD } else { 1.0 };
            let mv = self.damage.move_in(mid);
            // Smart's Tera tables intentionally omit forward()'s Tera Blast and Stellar adjustments.
            for d in 0..12 {
                let (dealt, ko, _) = self.damage.damage(&t[s], &t[d], &mv, field, modifier);
                out.dealt[s][m][d] = dealt; out.ko[s][m][d] = ko;
            }
        }}
        if threats {
            let attacks = self.damage.foe_attacks(ids, floats, &t, &t, field);
            for f in 0..2 { for d in 0..12 {
                let mut dealt = [0.0; M_FOE]; let mut ko = [0.0; M_FOE];
                for m in 0..M_FOE { dealt[m] = attacks.dealt(f, m, d); ko[m] = attacks.ko(f, m, d); }
                out.threat[d][f] = expected_max(&dealt, &attacks.probs[f]);
                out.kothreat[d][f] = any_ko(&ko, &attacks.probs[f]);
            }}
        }
        out
    }

    fn per_code(&self, tables: &Tables, slot: usize, code: usize, token: usize) -> (f32, f32) {
        let m = code / 10; let tc = code % 10 / 2; let target = tables.target[slot][m];
        let share = if self.hit[target][slot][tc][token] > 0.0 {
            if TARGETS[target] == "randomNormal" { 0.5 } else { 1.0 }
        } else { 0.0 };
        (share * tables.dealt[slot][m][token], share * tables.ko[slot][m][token])
    }

    fn types(c: &Context, token: usize) -> [i64; 2] {
        let tera = c.id(token, I_TERA);
        if c.p(token, pf::TERASTALLIZED) > 0.5 && tera > 0 && tera != STELLAR { [tera, 0] }
        else { [c.id(token, I_TYPE1), c.id(token, I_TYPE2)] }
    }
    fn ability_is(&self, id: i64, names: &[&str]) -> bool {
        names.iter().any(|name| id == self.ability_id(name))
    }
    fn ability_id(&self, name: &str) -> i64 {
        let id = self.dex.ability_idx(name); if id == 0 { -1 } else { id }
    }
    fn item_id(&self, name: &str) -> i64 {
        let id = self.dex.item_index(name); if id == 0 { -1 } else { id }
    }
    fn can_flinch(&self, c: &Context, field: &[f32]) -> [f32; 2] {
        let mut out = [1.0; 2];
        let side_block = (6..8).any(|t| self.ability_is(c.id(t, I_ABILITY), PRIORITY_BLOCK));
        for f in 0..2 {
            let t = 6 + f; let ability = c.id(t, I_ABILITY);
            let mut believed = 0.0; let mut levitate = 0.0;
            for k in 0..K_ABILITIES {
                let cand = c.id(t, I_CAND_ABILITIES + k); let p = c.p(t, pf::CAND_ABILITY_P0 + k);
                if self.ability_is(cand, FLINCH_BLOCK) { believed += p; }
                if cand == self.ability_id("levitate") { levitate += p; }
            }
            out[f] *= if ability > 0 { 1.0 - flag(self.ability_is(ability, FLINCH_BLOCK)) } else { 1.0 - believed.min(1.0) };
            out[f] *= flag(c.id(t, I_ITEM) != self.item_id("covertcloak"));
            out[f] *= flag(c.p(t, pf::SUBSTITUTE) < 0.5);
            out[f] *= flag(!side_block);
            let mut airborne = if ability > 0 { flag(ability == self.ability_id("levitate")) } else { levitate.clamp(0.0, 1.0) };
            if Self::types(c, t).contains(&self.dex.type_idx("Flying")) || c.id(t, I_ITEM) == self.item_id("airballoon") { airborne = 1.0; }
            if field[ff::GRAVITY] > 0.5 { airborne = 0.0; }
            if field[ff::PSYCHICTERRAIN] > 0.5 { out[f] *= airborne; }
        }
        out
    }

    fn prepare(&mut self, ids: &[i64], floats: &[f32], field: &[f32], af: &[f32], tf: &[f32]) -> Context {
        let mut c = Context { hp: [0.0; 12], alive: [false; 12], on: [false; 12], own_on: [0.0; 2], foe_on: [0.0; 2],
            q: [[0.0; 2]; 2], foe_protect: [0.0; 2], foe_value: [0.0; 2], matchup: [0.0; 6],
            request_move: field[ff::REQUEST_MOVE] > 0.5, protect_share: 0.0, tera_usable: [false; 2],
            ids: ids.to_vec(), floats: floats.to_vec(), effects: [[Effect::default(); N_ACTIONS]; 2], a0: PASS };
        let mut threat = [[0.0; 2]; 12]; let mut kothreat = [[0.0; 2]; 12]; let mut faster = [[0.0; 2]; 12];
        for t in 0..12 {
            c.hp[t] = c.p(t, pf::HP);
            c.alive[t] = c.p(t, pf::PRESENT) > 0.5 && c.p(t, pf::FAINTED) < 0.5;
            c.on[t] = c.alive[t] && c.p(t, pf::ACTIVE_A) + c.p(t, pf::ACTIVE_B) > 0.5;
            for f in 0..2 { threat[t][f] = tf[t * N_TOKEN_FEATURES + f];
                kothreat[t][f] = tf[t * N_TOKEN_FEATURES + 2 + f]; faster[t][f] = tf[t * N_TOKEN_FEATURES + 4 + f]; }
        }
        for s in 0..2 { c.own_on[s] = flag(c.on[s]); c.foe_on[s] = flag(c.on[6 + s]);
            c.tera_usable[s] = c.on[s] && c.p(s, pf::TERA_AVAILABLE) > 0.5 && field[ff::OWN_TERA_USED] < 0.5; }
        let base = self.tables(ids, floats, field, false);
        let mut tera = [None, None]; let mut threat_t = [threat[0], threat[1]]; let mut kothreat_t = [kothreat[0], kothreat[1]];
        for s in 0..2 { if c.tera_usable[s] {
            let mut changed = floats.to_vec(); changed[s * N_POKEMON_FLOATS + pf::TERASTALLIZED] = 1.0;
            let table = self.tables(ids, &changed, field, true);
            threat_t[s] = table.threat[s]; kothreat_t[s] = table.kothreat[s]; tera[s] = Some(table);
        }}
        for f in 0..2 {
            let t0 = threat[0][f] * c.own_on[0]; let t1 = threat[1][f] * c.own_on[1];
            let share = ((t0 + 0.05) / (t0 + t1 + 0.1)).clamp(0.15, 0.85);
            c.q[f] = if c.own_on[0] > 0.0 && c.own_on[1] > 0.0 { [share, 1.0 - share] } else { c.own_on };
            let t = 6 + f;
            let revealed = (0..4).any(|m| self.moves[c.id(t, I_MOVE0 + m) as usize].protect);
            let mut cand = [0.0; K_MOVES];
            for k in 0..K_MOVES { cand[k] = c.p(t, pf::CAND_MOVE_P0 + k) * flag(self.moves[c.id(t, I_CAND_MOVES + k) as usize].protect); }
            let has = if revealed { 1.0 } else { numpy_sum(&cand).min(1.0) };
            let danger = (kothreat[t][0] * c.own_on[0]).max(kothreat[t][1] * c.own_on[1]);
            c.foe_protect[f] = c.foe_on[f] * has * (1.0 - c.p(t, pf::PROTECT_LAST)) * (0.12 + 0.3 * danger);
            c.foe_value[f] = ((threat[0][f] + 0.5 * kothreat[0][f]) * c.own_on[0])
                .max((threat[1][f] + 0.5 * kothreat[1][f]) * c.own_on[1]);
        }
        let mut exposure = [0.0; 6]; let mut risk = [0.0f32; 6];
        for t in 0..6 {
            let mut terms = [0.0; 2];
            for f in 0..2 {
                let mut off: f32 = 0.0; let mut ko: f32 = 0.0;
                for m in 0..4 { off = off.max(base.dealt[t][m][6 + f]); ko = ko.max(base.ko[t][m][6 + f]); }
                terms[f] = (off + 0.5 * ko - threat[t][f] - 0.5 * kothreat[t][f] + 0.2 * (faster[t][f] - 0.5)) * c.foe_on[f];
                exposure[t] += threat[t][f] * c.foe_on[f];
                risk[t] = risk[t].max(kothreat[t][f] * c.foe_on[f]);
            }
            c.matchup[t] = if c.alive[t] { (terms[0] + terms[1]) / (c.foe_on[0] + c.foe_on[1]).max(1.0) + 0.1 * c.hp[t] } else { f32::NEG_INFINITY };
        }
        let can_flinch = self.can_flinch(&c, field);
        for s in 0..2 {
            let fresh = c.p(s, pf::FRESH) > 0.5 && c.p(s, pf::SWITCH_TURNS) < 0.3 && c.on[s];
            let tera_risk = (kothreat_t[s][0] * c.foe_on[0]).max(kothreat_t[s][1] * c.foe_on[1]);
            let saves = risk[s] >= 0.4 && risk[s] - tera_risk >= 0.25;
            let skip = self.rng.uniform() < self.config.protect_skip as f64;
            for code in 0..40 {
                let m = code / 10; let tc = code % 10 / 2; let tbit = code % 2 == 1;
                let mid = c.id(s, I_MOVE0 + m) as usize; let info = self.moves[mid];
                let table = if tbit { tera[s].as_ref().unwrap_or(&base) } else { &base };
                let mut e = Effect::default(); let mut ko_gain = f32::NEG_INFINITY;
                for f in 0..2 {
                    let (d, k) = self.per_code(table, s, code, 6 + f); e.off_d[f] = d; e.off_k[f] = k;
                    let (_, kt) = self.per_code(tera[s].as_ref().unwrap_or(&base), s, code, 6 + f);
                    let (_, kb) = self.per_code(&base, s, code, 6 + f); ko_gain = ko_gain.max(kt - kb);
                    e.first[f] = af[(s * 40 + code) * N_ACTION_FEATURES + 6 + f];
                    e.inc_d[f] = if tbit { threat_t[s][f] } else { threat[s][f] };
                    e.inc_k[f] = if tbit { kothreat_t[s][f] } else { kothreat[s][f] };
                }
                (e.ally_d, e.ally_k) = self.per_code(table, s, code, 1 - s);
                e.protect = flag(info.protect); e.helping = flag(info.helping); e.redirect = flag(info.redirect);
                e.tera = tbit; e.bonus = -0.2 * flag(tbit);
                let recoil = 0.25 * info.recoil * (e.off_d[0] + e.off_d[1]);
                // NumPy promotes the recharge bool term to float64 before rounding the in-place subtraction.
                e.act_bonus = (e.act_bonus as f64 - (recoil as f64 + 0.2 * flag(info.recharge) as f64)) as f32;
                e.act_bonus -= flag(info.destruct) * (c.hp[s] + 0.35);
                if info.first_only && !fresh { e.off_d = [0.0; 2]; e.off_k = [0.0; 2]; }
                for f in 0..2 { e.flinch[f] = flag(info.fake_out && fresh && tc == 3 + f && af[(s * 40 + code) * N_ACTION_FEATURES + 5] <= 0.5)
                    * e.first[f] * (can_flinch[f] * c.foe_on[f]); }
                e.fake_out = e.flinch[0].max(e.flinch[1]) > 0.05;
                e.allowed = (!tbit || ko_gain >= 0.25 || saves) && !(info.protect && (c.p(s, pf::PROTECT_LAST) > 0.5 || skip));
                c.effects[s][code] = e;
            }
            for t in 0..6 {
                let mut e = Effect::default(); e.inc_d = threat[t]; e.inc_k = kothreat[t]; e.switch = t as i32;
                e.allowed = !c.request_move || (risk[s] >= 0.5 && exposure[t] <= 0.5 * exposure[s] && risk[t] <= 0.25);
                let current = if c.matchup[s].is_finite() { c.matchup[s] } else { 0.0 };
                e.bonus = if c.request_move { 0.5 * (c.matchup[t] - current) - 0.12 } else { 0.0 };
                if !e.bonus.is_finite() { e.bonus = -10.0; }
                c.effects[s][40 + t] = e;
            }
            c.effects[s][PASS].inc_d = threat[s]; c.effects[s][PASS].inc_k = kothreat[s];
        }
        c.protect_share = (self.config.protect_share.0 as f64 + self.rng.uniform()
            * (self.config.protect_share.1 as f64 - self.config.protect_share.0 as f64)) as f32;
        self.status_values(&mut c, field, af, &faster, &kothreat);
        for s in 0..2 { for e in &mut c.effects[s] { e.strong = (e.off_k[0] * c.foe_on[0]).max(e.off_k[1] * c.foe_on[1]) >= 0.8; }}
        c
    }

    fn status_values(&self, c: &mut Context, field: &[f32], af: &[f32], faster: &[[f32; 2]; 12], kothreat: &[[f32; 2]; 12]) {
        let mut statused = [false; 12];
        for t in 0..12 { statused[t] = [pf::BRN, pf::PAR, pf::SLP, pf::FRZ, pf::PSN, pf::TOX].iter().any(|&col| c.p(t, col) > 0.5); }
        let foe_asleep = (6..12).any(|t| c.p(t, pf::SLP) > 0.5);
        let trick_room = field[ff::TRICKROOM] > 0.5;
        let grounded_block = field[ff::ELECTRICTERRAIN] > 0.5 || field[ff::MISTYTERRAIN] > 0.5;
        for s in 0..2 { for m in 0..4 {
            let mid = c.id(s, I_MOVE0 + m) as usize; let info = self.moves[mid];
            if !c.on[s] || !(info.status != Status::None && info.status != Status::Protect || info.harmful) { continue; }
            let name = self.dex.move_data[mid].id.as_str(); let accuracy = self.dex.move_accuracy[mid] as f64;
            let risk = c.q[0][s] * c.foe_on[0] * kothreat[s][0] + c.q[1][s] * c.foe_on[1] * kothreat[s][1];
            let safe = risk < 0.2 && c.hp[s] >= 0.6;
            let mut slower = 0.0f32; let mut quicker = 0.0f32;
            for s2 in 0..2 { for f in 0..2 {
                slower += (1.0 - faster[s2][f]) * c.own_on[s2] * c.foe_on[f];
                quicker += faster[s2][f] * c.own_on[s2] * c.foe_on[f];
            }}
            let mut value = 0.0f64; let mut per_foe = None;
            match info.status {
                Status::Tailwind => { if field[ff::OWN_TAILWIND] <= 0.5 && !trick_room { value = (0.06 + 0.1 * slower) as f64; } }
                Status::Trickroom => value = (0.1 * (slower - quicker)).max(0.0) as f64,
                Status::Setup => {
                    if safe && c.p(s, pf::BOOST_ATK).max(c.p(s, pf::BOOST_SPA)) < 0.3 { value = if ["shellsmash", "bellydrum"].contains(&name) { 0.3 } else { 0.22 }; }
                    if name == "bellydrum" && c.hp[s] < 0.75 { value = -0.5; }
                }
                Status::Defense => value = if safe { 0.08 } else { 0.0 },
                Status::Recovery => {
                    let missing = (1.0 - c.hp[s]).min(0.5);
                    value = ((if c.hp[s] < 0.6 { 0.8 } else { 0.2 }) * missing) as f64;
                }
                Status::Rest => value = if c.hp[s] < 0.4 && !statused[s] { 0.35 } else { 0.0 },
                Status::TeamHeal => {
                    let mut sum = 0.0f32;
                    for t in 0..2 { sum += (1.0 - c.hp[t]).min(0.25) * c.own_on[t]; }
                    value = (0.8 * sum) as f64;
                }
                Status::Screen => {
                    let col = match name { "reflect" => ff::OWN_REFLECT, "lightscreen" => ff::OWN_LIGHTSCREEN, _ => ff::OWN_AURORAVEIL };
                    if field[col] <= 0.5 && !(name == "auroraveil" && field[ff::SNOW] < 0.5) { value = if name == "auroraveil" { 0.25 } else { 0.15 }; }
                }
                Status::Hazard => {
                    let bench = (6.0 - 6.0 * field[ff::FOE_FAINTED] - (c.foe_on[0] + c.foe_on[1])).max(0.0);
                    let (col, weight) = match name { "stealthrock" => (ff::FOE_STEALTHROCK, 0.05), "spikes" => (ff::FOE_SPIKES, 0.035), "toxicspikes" => (ff::FOE_TOXICSPIKES, 0.025), _ => (ff::FOE_STICKYWEB, 0.04) };
                    if field[col] < 0.99 { value = (weight * bench) as f64; }
                }
                Status::Revive => value = if field[ff::OWN_ALIVE] < 0.99 { 0.4 } else { -0.5 },
                Status::AllyHeal => {},
                _ => {
                    let mut values = [0.0f32; 2];
                    for f in 0..2 {
                        let t = 6 + f;
                        if !c.on[t] || statused[t] { continue; }
                        let types = Self::types(c, t);
                        let is_type = |name| types.contains(&self.dex.type_idx(name));
                        values[f] = match info.status {
                            Status::Paralysis if !is_type("Electric") => {
                                let foe_faster = (1.0 - faster[0][f]) * c.own_on[0] + (1.0 - faster[1][f]) * c.own_on[1];
                                (accuracy as f32 * (0.12 + 0.12 * foe_faster)) as f32
                            }
                            Status::Burn if !is_type("Fire") => (accuracy * if c.p(t, pf::STAT_ATK) >= c.p(t, pf::STAT_SPA) { 0.3 } else { 0.05 }) as f32,
                            Status::Poison if !is_type("Poison") && !is_type("Steel") => (accuracy * 0.15) as f32,
                            Status::Sleep if !foe_asleep && !self.ability_is(c.id(t, I_ABILITY), SLEEP_BLOCK)
                                && !grounded_block && !(["spore", "sleeppowder"].contains(&name) && is_type("Grass")) => (accuracy * if name == "yawn" { 0.25 } else { 0.45 }) as f32,
                            Status::Leechseed if !is_type("Grass") && c.p(t, pf::LEECHSEED) <= 0.5 => (accuracy * 0.1) as f32,
                            _ => 0.0,
                        };
                    }
                    per_foe = Some(values);
                }
            }
            for code in m * 10..m * 10 + 10 {
                let tc = code % 10 / 2;
                let v = if tc <= 1 && info.harmful { (-1.5 * c.own_on[1 - tc]) as f64 }
                else if info.status == Status::AllyHeal {
                    let mut v = 0.0f32;
                    if tc <= 1 {
                        let t = 1 - tc;
                        if c.on[t] && c.hp[t] < 0.5 { v = 0.8 * (1.0 - c.hp[t]).min(0.5); }
                        if name == "pollenpuff" { c.effects[s][code].ally_d = 0.0; c.effects[s][code].ally_k = 0.0; }
                    } else if name != "pollenpuff" && (tc == 3 || tc == 4) {
                        v = -0.8 * (1.0 - c.hp[6 + tc - 3]).min(0.5) * c.foe_on[tc - 3];
                    }
                    v as f64
                } else if let Some(values) = per_foe {
                    if tc == 3 || tc == 4 {
                        if af[(s * 40 + code) * N_ACTION_FEATURES + 5] > 0.5 { 0.0 } else { values[tc - 3] as f64 }
                    } else if tc == 2 { values[0].max(values[1]) as f64 } else { 0.0 }
                } else { value };
                // NumPy scalar += rounds back into the float32 effect array.
                c.effects[s][code].act_bonus = c.effects[s][code].act_bonus + v as f32;
            }
        }}
    }

    fn estimate_codes(&self, c: &Context, s: usize) -> Vec<usize> {
        if !c.on[s] { return vec![PASS]; }
        let mut codes = Vec::with_capacity(N_ACTIONS);
        // Preserve Python's estimated partner target mapping, even though code 0 actually means own b.
        let ally = if s == 0 { 0 } else { 1 };
        for m in 0..4 {
            let mid = c.id(s, I_MOVE0 + m) as usize;
            if mid == 0 || c.p(s, pf::PP0 + m) <= 0.0 || c.p(s, pf::DISABLED0 + m) > 0.5 { continue; }
            let targets: &[usize] = match TARGETS[self.dex.move_target[mid] as usize] {
                "normal" | "any" => if ally == 0 { &[0, 3, 4] } else { &[1, 3, 4] },
                "adjacentFoe" => &[3, 4], "adjacentAlly" => if ally == 0 { &[0] } else { &[1] },
                "adjacentAllyOrSelf" => &[0, 1], _ => &[2],
            };
            for &tc in targets { codes.push(m * 10 + tc * 2); if c.tera_usable[s] { codes.push(m * 10 + tc * 2 + 1); } }
        }
        if c.p(s, pf::TRAPPED) < 0.5 { for p in 2..6 { if c.alive[p] && !c.on[p] { codes.push(40 + p); } } }
        if codes.is_empty() { codes.push(PASS); }
        codes
    }

    fn candidates(c: &Context, s: usize, codes: &[usize]) -> Vec<usize> {
        let effects = &c.effects[s];
        let fake = codes.iter().any(|&k| effects[k].fake_out && effects[k].allowed);
        let strong = codes.iter().any(|&k| effects[k].strong && effects[k].allowed && !effects[k].fake_out);
        let filtered: Vec<usize> = codes.iter().copied().filter(|&k| effects[k].allowed && (!fake || strong || effects[k].fake_out)).collect();
        // If every heuristic gate rejects the legal codes, Python restores all of them.
        if filtered.is_empty() { codes.to_vec() } else { filtered }
    }

    fn pair_value(c: &Context, x: &Effect, y: &Effect) -> f64 {
        if x.tera && y.tera || x.switch >= 0 && x.switch == y.switch { return f64::NEG_INFINITY; }
        let mut flinch = [0.0f32; 2]; let mut ax = [0.0f32; 2]; let mut ay = [0.0f32; 2];
        let mut damage_a = [0.0f32; 2]; let mut damage_b = [0.0f32; 2];
        let mut loss_a = [0.0f32; 2]; let mut loss_b = [0.0f32; 2];
        let mut land_a = [0.0f32; 2]; let mut land_b = [0.0f32; 2];
        let keep_x = 1.0 - c.protect_share * x.protect; let keep_y = 1.0 - c.protect_share * y.protect;
        for f in 0..2 {
            flinch[f] = 1.0 - (1.0 - x.flinch[f]) * (1.0 - y.flinch[f]);
            let first_ko = 1.0 - (1.0 - x.off_k[f] * x.first[f]) * (1.0 - y.off_k[f] * y.first[f]);
            let attacks = c.foe_on[f] * (1.0 - c.foe_protect[f]) * (1.0 - flinch[f]) * (1.0 - first_ko);
            let qa = if x.redirect > 0.0 { 1.0 } else if y.redirect > 0.0 { 0.0 } else { c.q[f][0] };
            let qb = if y.redirect > 0.0 { 1.0 } else if x.redirect > 0.0 { 0.0 } else { c.q[f][1] };
            ax[f] = attacks * (qa * c.own_on[0]); ay[f] = attacks * (qb * c.own_on[1]);
            damage_a[f] = ax[f] * x.inc_d[f] * keep_x; damage_b[f] = ay[f] * y.inc_d[f] * keep_y;
            loss_a[f] = 1.0 - ax[f] * x.inc_k[f] * keep_x; loss_b[f] = 1.0 - ay[f] * y.inc_k[f] * keep_y;
            land_a[f] = 1.0 - ax[f] * x.inc_k[f] * (1.0 - x.first[f]) * (1.0 - x.protect);
            land_b[f] = 1.0 - ay[f] * y.inc_k[f] * (1.0 - y.first[f]) * (1.0 - y.protect);
        }
        let la = land_a[0] * land_a[1]; let lb = land_b[0] * land_b[1];
        let mut offense = [0.0f32; 2];
        for f in 0..2 {
            let hp = c.hp[6 + f].max(0.01);
            let dx = x.off_d[f] * la * (1.0 + 0.5 * y.helping); let dy = y.off_d[f] * lb * (1.0 + 0.5 * x.helping);
            let mut kx = x.off_k[f] * la; let mut ky = y.off_k[f] * lb;
            if y.helping > 0.0 { kx = kx.max(((dx / hp - 0.95) / 0.2).clamp(0.0, 1.0) * 0.9); }
            if x.helping > 0.0 { ky = ky.max(((dy / hp - 0.95) / 0.2).clamp(0.0, 1.0) * 0.9); }
            let dealt = (dx + dy).min(hp);
            let ko = 1.0 - (1.0 - kx) * (1.0 - ky);
            let together = (((dx + dy) / hp - 0.8) / 0.4).clamp(0.0, 1.0) * 0.9 * flag(dx > 0.01 && dy > 0.01);
            let ko = ko.max(together);
            offense[f] = c.foe_on[f] * (1.0 - c.foe_protect[f]) * (dealt + 0.35 * ko * (1.0 + c.foe_value[f]) + 0.3 * flinch[f]);
        }
        let mut ally = (x.ally_d + 0.35 * x.ally_k) * (1.0 - y.protect) * la;
        ally += (y.ally_d + 0.35 * y.ally_k) * (1.0 - x.protect) * lb;
        let defense = damage_a[0] + damage_a[1] + (damage_b[0] + damage_b[1])
            + 0.35 * ((1.0 - loss_a[0] * loss_a[1]) + (1.0 - loss_b[0] * loss_b[1]));
        let bonus = x.bonus + y.bonus + x.act_bonus * la + y.act_bonus * lb;
        (offense[0] + offense[1] - defense - 1.5 * ally + bonus) as f64
    }

    fn choose(&mut self, c: &Context, xs: &[usize], ys: &[usize]) -> (usize, usize) {
        let mut best = f64::NEG_INFINITY; let mut selected = (0, 0); let mut finite = false;
        for (a, &xc) in xs.iter().enumerate() { for (b, &yc) in ys.iter().enumerate() {
            let mut value = Self::pair_value(c, &c.effects[0][xc], &c.effects[1][yc]);
            value += self.config.noise as f64 * self.rng.uniform();
            if self.config.temperature > 0.0 { value += self.config.temperature as f64 * -(-self.rng.uniform().ln()).ln(); }
            finite |= value.is_finite();
            if value > best || value.is_nan() && !best.is_nan() { best = value; selected = (a, b); }
        }}
        if finite { selected } else { (0, 0) }
    }

    fn replacement(&mut self, c: &Context, codes: &[usize], taken: usize) -> usize {
        let switches: Vec<usize> = codes.iter().copied().filter(|&k| (40..46).contains(&k) && k != taken).collect();
        if switches.is_empty() { return codes.first().copied().unwrap_or(PASS); }
        let mut best = f64::NEG_INFINITY; let mut selected = 0; let mut finite = false;
        for (i, &code) in switches.iter().enumerate() {
            let value = c.matchup[code - 40] as f64 + self.config.noise as f64 * self.rng.uniform();
            finite |= value.is_finite();
            if value > best || value.is_nan() && !best.is_nan() { best = value; selected = i; }
        }
        // Python still randomly picks a fainted Revival Blessing target when all matchup scores are nonfinite.
        if finite { switches[selected] } else { switches[self.rng.index(switches.len())] }
    }

    /// Picks slot a and keeps the selected rows' context for `second`.
    pub fn first(&mut self, obs: &ObsRows<'_>, rows: &[usize], mask0: &[bool]) -> Vec<i32> {
        assert_eq!(mask0.len(), rows.len() * N_ACTIONS);
        let mut out = Vec::with_capacity(rows.len()); let mut contexts = Vec::with_capacity(rows.len());
        for (i, &row) in rows.iter().enumerate() {
            let mut c = self.prepare(row_slice(obs.ids, row, 12 * N_IDS), row_slice(obs.floats, row, 12 * N_POKEMON_FLOATS),
                row_slice(obs.field, row, N_FIELD_FLOATS), row_slice(obs.action_features, row, 2 * 40 * N_ACTION_FEATURES),
                row_slice(obs.token_features, row, 12 * N_TOKEN_FEATURES));
            let codes = codes_of(&mask0[i * N_ACTIONS..(i + 1) * N_ACTIONS]);
            c.a0 = match codes.len() {
                0 => PASS, 1 => codes[0], _ if !c.request_move => self.replacement(&c, &codes, usize::MAX),
                _ => {
                    let xs = Self::candidates(&c, 0, &codes); let ys = Self::candidates(&c, 1, &self.estimate_codes(&c, 1));
                    let (a, _) = self.choose(&c, &xs, &ys); xs[a]
                }
            };
            out.push(c.a0 as i32); contexts.push(c);
        }
        self.ctx = contexts; out
    }

    /// Re-picks slot b using its exact mask, conditioned on the previous slot-a choice.
    pub fn second(&mut self, mask1: &[bool]) -> Vec<i32> {
        assert_eq!(mask1.len(), self.ctx.len() * N_ACTIONS);
        let contexts = std::mem::take(&mut self.ctx); let mut out = Vec::with_capacity(contexts.len());
        for (i, c) in contexts.iter().enumerate() {
            let codes = codes_of(&mask1[i * N_ACTIONS..(i + 1) * N_ACTIONS]);
            let code = match codes.len() {
                0 => PASS, 1 => codes[0], _ if !c.request_move => self.replacement(c, &codes, c.a0),
                _ => {
                    let xs = Self::candidates(c, 0, &[c.a0]); let ys = Self::candidates(c, 1, &codes);
                    let (_, b) = self.choose(c, &xs, &ys); ys[b]
                }
            };
            out.push(code as i32);
        }
        self.ctx = contexts; out
    }
}

fn flag(b: bool) -> f32 { if b { 1.0 } else { 0.0 } }
fn row_slice<T>(values: &[T], row: usize, width: usize) -> &[T] { &values[row * width..(row + 1) * width] }
fn codes_of(mask: &[bool]) -> Vec<usize> { mask.iter().enumerate().filter_map(|(k, &on)| if on { Some(k) } else { None }).collect() }

/// NumPy's short contiguous float32 sum uses eight accumulators for lengths at least eight.
fn numpy_sum(values: &[f32]) -> f32 {
    if values.len() < 8 { let mut out = -0.0; for &v in values { out += v; } return out; }
    let mut accum = [0.0; 8]; accum.copy_from_slice(&values[..8]);
    let mut i = 8;
    while i + 8 <= values.len() { for k in 0..8 { accum[k] += values[i + k]; } i += 8; }
    let mut out = ((accum[0] + accum[1]) + (accum[2] + accum[3])) + ((accum[4] + accum[5]) + (accum[6] + accum[7]));
    for &v in &values[i..] { out += v; }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        Context { hp: [1.0; 12], alive: [true; 12], on: [true; 12], own_on: [1.0; 2], foe_on: [1.0; 2],
            q: [[0.5; 2]; 2], foe_protect: [0.0; 2], foe_value: [0.0; 2], matchup: [0.0; 6],
            request_move: true, protect_share: 0.7, tera_usable: [false; 2], ids: vec![0; 12 * N_IDS],
            floats: vec![0.0; 12 * N_POKEMON_FLOATS], effects: [[Effect::default(); N_ACTIONS]; 2], a0: PASS }
    }

    #[test]
    fn generator_is_deterministic_and_open_interval() {
        let mut a = Rng::new(42); let mut b = Rng::new(42); let mut c = Rng::new(43);
        assert_ne!(a.next(), c.next());
        b.next();
        for _ in 0..1000 { let x = a.uniform(); assert_eq!(x, b.uniform()); assert!(x > 0.0 && x < 1.0); }
    }

    #[test]
    fn joint_score_rewards_focus_fire_without_overkill() {
        let c = context();
        let x = Effect { off_d: [0.7, 0.0], first: [1.0; 2], ..Effect::default() };
        let same = Effect { off_d: [0.7, 0.0], first: [1.0; 2], ..Effect::default() };
        let other = Effect { off_d: [0.0, 0.7], first: [1.0; 2], ..Effect::default() };
        assert!(SmartAgent::pair_value(&c, &x, &other) > SmartAgent::pair_value(&c, &x, &same));
        let tera = Effect { tera: true, ..x };
        assert_eq!(SmartAgent::pair_value(&c, &tera, &tera), f64::NEG_INFINITY);
        let switch = Effect { switch: 2, ..Effect::default() };
        assert_eq!(SmartAgent::pair_value(&c, &switch, &switch), f64::NEG_INFINITY);
    }

    #[test]
    fn obvious_attack_wins_on_a_hand_built_row() {
        let dex = Dex::shared(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/training/dex.json"))).unwrap();
        let mut agent = SmartAgent::new(dex.clone(), 0, SmartConfig { noise: 0.0, protect_skip: 0.0,
            protect_share: (0.7, 0.7), temperature: 0.0 });
        let mut ids = vec![0; 12 * N_IDS]; let mut floats = vec![0.0; 12 * N_POKEMON_FLOATS];
        for t in [0, 6] {
            ids[t * N_IDS + I_TYPE1] = dex.type_idx("Normal");
            for col in [pf::PRESENT, pf::ACTIVE_A, pf::HP] { floats[t * N_POKEMON_FLOATS + col] = 1.0; }
            for col in [pf::STAT_HP, pf::STAT_ATK, pf::STAT_DEF, pf::STAT_SPA, pf::STAT_SPD, pf::STAT_SPE, pf::LEVEL] {
                floats[t * N_POKEMON_FLOATS + col] = 0.5;
            }
            for ty in 0..crate::dex::N_TYPES { floats[t * N_POKEMON_FLOATS + pf::TYPE_MULT0 + ty] = 1.0; }
        }
        ids[I_MOVE0] = dex.move_idx("tackle"); ids[I_MOVE0 + 1] = dex.move_idx("splash");
        floats[pf::OWN] = 1.0;
        let mut field = vec![0.0; N_FIELD_FLOATS]; field[ff::REQUEST_MOVE] = 1.0;
        let mut action = vec![0.0; 2 * 40 * N_ACTION_FEATURES];
        for code in 0..40 { action[code * N_ACTION_FEATURES + 6] = 1.0; }
        let token = vec![0.0; 12 * N_TOKEN_FEATURES];
        let obs = ObsRows { ids: &ids, floats: &floats, field: &field, action_features: &action, token_features: &token };
        let mut mask = vec![false; N_ACTIONS]; mask[6] = true; mask[16] = true;
        assert_eq!(agent.first(&obs, &[0], &mask), [6]);
        mask.fill(false); mask[PASS] = true;
        assert_eq!(agent.second(&mask), [PASS as i32]);
    }

    #[test]
    fn candidates_keep_order_and_restore_rejected_codes() {
        let mut c = context();
        c.effects[0][6].allowed = false; c.effects[0][8].allowed = false;
        assert_eq!(SmartAgent::candidates(&c, 0, &[6, 8]), [6, 8]);
        c.effects[0][8].allowed = true; c.effects[0][8].fake_out = true;
        assert_eq!(SmartAgent::candidates(&c, 0, &[4, 6, 8]), [8]);
        c.effects[0][4].strong = true;
        assert_eq!(SmartAgent::candidates(&c, 0, &[4, 6, 8]), [4, 8]);
    }
}
