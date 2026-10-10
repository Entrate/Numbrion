use super::{
    HOOKS, ID,
    host::{Foes, Host, Move, boosts},
    react,
};
use crate::{
    actions::Stat,
    dex,
    event::Relay,
    ids::*,
    state::{Pokemon, Status, scratch::OrderedBoosts},
};
struct RecordingHost<'a> {
    c: Vec<&'a str>,
    hook: &'a str,
    trace: Vec<String>,
    move_value: Move,
    pokemon: [Pokemon; 12],
    checked: Option<bool>,
    multihit_count: u8,
}
impl RecordingHost<'_> {
    fn at(&self, i: usize) -> &str {
        self.c[3 + i]
    }
    fn n(&self, i: usize) -> i32 {
        self.at(i).parse().unwrap()
    }
    fn source(&self) -> Option<MonId> {
        (self.n(0) >= 0).then_some(MonId(self.n(0) as u8))
    }
    fn output(r: Relay) -> String {
        match r {
            Relay::Undefined => "undefined".into(),
            Relay::Null => "null".into(),
            Relay::Bool(b) => b.to_string(),
            Relay::Number(n) => n.to_string(),
            _ => panic!("unexpected return"),
        }
    }
}
fn mon_text(m: Option<MonId>) -> String {
    m.map_or("undefined".into(), |m| m.0.to_string())
}
impl Host for RecordingHost<'_> {
    fn mon(&self, i: usize) -> Option<MonId> {
        match self.hook {
            "onStart" | "onResidual" | "onUpdate" => Some(MonId(0)),
            "onModifyMove" => Some(MonId(0)),
            "onTryHit" | "onAfterMoveSecondary" => {
                if i == 0 {
                    Some(MonId(0))
                } else {
                    self.source()
                }
            }
            "onSetStatus" | "onAfterEachBoost" | "onAfterBoost" => {
                if i == 1 {
                    Some(MonId(0))
                } else {
                    self.source()
                }
            }
            "onSourceAfterFaint" => {
                if i == 1 {
                    Some(MonId(0))
                } else {
                    Some(self.source().unwrap_or(MonId(0)))
                }
            }
            _ if self.hook.starts_with("onAnyModify") => {
                if i == 1 {
                    Some(MonId(0))
                } else {
                    self.source()
                }
            }
            _ => {
                if i == 1 {
                    Some(MonId(0))
                } else {
                    Some(self.source().unwrap_or(MonId(0)))
                }
            }
        }
    }
    fn owner(&self) -> MonId {
        MonId(self.n(1) as u8)
    }
    fn number(&self, _: usize) -> f64 {
        self.n(29) as f64
    }
    fn effect(&self, _: usize) -> Option<dex::EffectType> {
        match self.at(25) {
            "Move" => Some(dex::EffectType::Move),
            "Ability" => Some(dex::EffectType::Ability),
            _ => None,
        }
    }
    fn effect_id(&self, i: usize) -> EffectId {
        let (kind, key) = if self.hook == "onTryEatItem" {
            (EffectKind::Item, self.at(28))
        } else if self.hook == "onSetStatus" && i == 0 {
            (EffectKind::Condition, self.at(27))
        } else if self.hook == "onSideConditionStart" {
            (
                EffectKind::Condition,
                if self.n(23) != 0 {
                    "tailwind"
                } else {
                    "reflect"
                },
            )
        } else if self.hook == "onAfterBoost" && self.at(25) == "Ability" {
            (EffectKind::Ability, "intimidate")
        } else {
            (EffectKind::Move, "facade")
        };
        dex::lookup(kind, key).unwrap()
    }
    fn effect_status(&self, _: usize) -> bool {
        self.at(25) != "missing" && self.n(26) != 0
    }
    fn boost_arg(&self, _: usize) -> OrderedBoosts {
        if self.n(31) == 99 {
            boosts(&[(4, self.n(30) as i8)])
        } else {
            boosts(&[(4, self.n(30) as i8), (0, self.n(31) as i8)])
        }
    }
    fn mv(&self, _: usize) -> Move {
        self.move_value
    }
    fn pokemon(&self, m: MonId) -> Pokemon {
        self.pokemon[m.0 as usize]
    }
    fn species_name(&self, _: MonId) -> &'static str {
        match self.at(11) {
            "greninjaash" => "Greninja-Ash",
            "greninjabond" => "Greninja-Bond",
            _ => "Pikachu",
        }
    }
    fn flag(&mut self, m: MonId, flag: u32) {
        self.pokemon[m.0 as usize].flags |= flag;
    }
    fn foes(&mut self, m: MonId, a: bool) -> Foes {
        self.trace.push(format!("foes:{}:{a}", m.0));
        Foes {
            entries: [MonId(6), MonId(7), MonId::NONE, MonId::NONE],
            len: self.n(19) as u8,
        }
    }
    fn foe_left(&self, _: MonId) -> bool {
        self.n(19) != 0
    }
    fn volatile(&mut self, _: MonId, _: EffectId) -> bool {
        self.n(22) != 0
    }
    fn tailwind(&self, _: SideId) -> bool {
        self.n(23) != 0
    }
    fn stat(&mut self, m: MonId, s: Stat) -> f64 {
        let key = if s == Stat::Def { "def" } else { "spd" };
        self.trace.push(format!("stat:{}:{key}:false:true", m.0));
        self.n(if s == Stat::Def { 20 } else { 21 }) as f64
    }
    fn contact(&mut self, s: MonId, t: MonId) -> bool {
        self.trace.push(format!("contact:{}:{}:true", s.0, t.0));
        self.n(13) != 0
    }
    fn suppress_ability(&mut self, m: MonId) -> bool {
        self.trace.push(format!("suppress:{}", m.0));
        self.n(14) != 0
    }
    fn suppress_secondaries(&mut self) -> bool {
        self.trace.push("secondaries".into());
        self.n(15) != 0
    }
    fn ability(&mut self, m: MonId, id: EffectId) -> bool {
        self.trace
            .push(format!("ability:{}:{}", m.0, dex::effect(id).key));
        self.n(if m.0 == 0 { 16 } else { 18 }) != 0
    }
    fn last_damage(&mut self, m: MonId) -> Option<f64> {
        self.trace.push(format!("last:{}", m.0));
        (self.n(8) >= 0).then_some(self.n(8) as f64)
    }
    fn checked(&self) -> Option<bool> {
        self.checked
    }
    fn set_checked(&mut self, v: bool) {
        self.checked = Some(v);
    }
    fn boost(
        &mut self,
        b: OrderedBoosts,
        t: Option<MonId>,
        s: Option<MonId>,
        effect: Option<EffectId>,
        secondary: bool,
        self_boost: bool,
    ) -> Relay {
        let stats = ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"];
        let changes = b.order[..b.len as usize]
            .iter()
            .map(|i| format!("{}={}", stats[*i as usize], b.values[*i as usize]))
            .collect::<Vec<_>>()
            .join(",");
        self.trace.push(format!(
            "boost:{changes}:{}:{}:{}:{secondary}:{self_boost}",
            mon_text(t),
            mon_text(s),
            effect.map_or("undefined", |id| dex::effect(id).key)
        ));
        match self.at(24) {
            "true" => Relay::Bool(true),
            "false" => Relay::Bool(false),
            "null" => Relay::Null,
            "0" => Relay::Number(0.0),
            _ => Relay::Undefined,
        }
    }
    fn log(&mut self, cmd: &'static str, m: MonId, id: EffectId, from: bool, boost: bool) {
        let label = if id == EffectId::NONE {
            "undefined".into()
        } else if from {
            format!("[from] ability: {}", dex::effect(id).name)
        } else if cmd == "-activate" {
            format!("ability: {}", dex::effect(id).name)
        } else {
            dex::effect(id).name.into()
        };
        self.trace.push(format!(
            "log:{cmd}:{}:{label}:{}",
            m.0,
            if boost { "boost" } else { "undefined" }
        ));
    }
    fn cure(&mut self, m: MonId) {
        self.trace.push(format!("cure:{}", m.0));
    }
    fn multihit(&mut self, _: usize, n: u8) {
        self.move_value.multihit = true;
        self.multihit_count = n;
    }
    fn ruin(&mut self, _: usize, slot: usize, m: MonId) {
        self.move_value.ruined[slot] = Some(m);
    }
    fn chain(&mut self) {
        self.trace.push("chain:0.75".into());
    }
}
#[test]
fn callback_decisions_core_call_order_and_state_match_pinned_source() {
    let name = dex::effect(ID).key;
    let mut count = 0;
    let mut covered = Vec::new();
    for line in super::VECTORS
        .lines()
        .filter(|l| l.split('\t').next() == Some(name))
    {
        let c: Vec<_> = line.split('\t').collect();
        let hook = c[1];
        let manifest = &dex::MANIFESTS[ID.0 as usize];
        let hi = manifest.hooks().iter().position(|h| h.key == hook).unwrap();
        let hook_id = dex::HookId(manifest.hooks_start + hi as u16);
        let n = |i: usize| c[3 + i].parse::<i32>().unwrap();
        let old = (n(17) >= 0).then_some(MonId(n(17) as u8));
        let mv = Move {
            id: dex::MOVE_FACADE,
            key: if n(33) != 0 {
                "watershuriken"
            } else {
                "facade"
            },
            move_type: dex::type_id(c[6]).unwrap(),
            category: if n(4) != 0 {
                dex::Category::Physical
            } else {
                dex::Category::Special
            },
            flags: if n(32) != 0 { dex::FLAG_WIND } else { 0 },
            multihit: n(5) != 0,
            smart: n(6) != 0,
            total_damage: n(7) as u32,
            ruined: [old; 4],
        };
        let mut pokemon = [Pokemon::default(); 12];
        for p in &mut pokemon {
            p.hp = n(2) as u16;
            p.max_hp = 120;
            p.flags = n(10) as u32;
            p.species = if c[14] == "greninjabond" {
                dex::SPECIES_GRENINJABOND
            } else {
                dex::SPECIES_PIKACHU
            };
            p.active_turns = n(12) as u32;
            p.status = if c[30] == "brn" {
                Status::Burn
            } else {
                Status::Poison
            };
        }
        let checked = (n(9) >= 0).then_some(n(9) != 0);
        let mut h = RecordingHost {
            c,
            hook,
            trace: Vec::new(),
            move_value: mv,
            pokemon,
            checked,
            multihit_count: if mv.multihit { 2 } else { 0 },
        };
        let result = react(hook_id, &mut h);
        assert_eq!(h.trace.join("|"), h.c[37], "{line}");
        assert_eq!(RecordingHost::output(result), h.c[38], "{line}");
        assert_eq!(
            h.checked.map_or("undefined".into(), |b| b.to_string()),
            h.c[39],
            "{line}"
        );
        for (m, column) in [(0, 40), (1, 41), (6, 42)] {
            assert_eq!(h.pokemon[m].flags.to_string(), h.c[column], "{line}");
        }
        let slot = match name {
            "beadsofruin" => 3,
            "swordofruin" => 1,
            "vesselofruin" => 2,
            _ => 0,
        };
        assert_eq!(
            h.move_value.ruined[slot]
                .map_or(-1, |m| m.0 as i32)
                .to_string(),
            h.c[43],
            "{line}"
        );
        let multihit = h.multihit_count;
        assert_eq!(multihit.to_string(), h.c[44], "{line}");
        if !covered.contains(&hook_id) {
            covered.push(hook_id);
        }
        count += 1;
    }
    assert_eq!(count, HOOKS.len() * 1131);
    assert_eq!(covered.len(), HOOKS.len());
    for hook in HOOKS {
        assert!(covered.contains(hook));
        assert_eq!(
            crate::effects::registry::hook_coverage(*hook),
            crate::effects::HookCoverage::Implemented
        );
    }
}
