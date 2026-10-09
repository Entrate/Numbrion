use super::{
    ID,
    host::{Hit, Host},
    react,
};
use crate::{dex, ids::*, prng::Prng};
struct RecordingHost {
    scenario: u8,
    prng: Prng,
    trace: Vec<String>,
}
impl Host for RecordingHost {
    fn contact(&mut self, a: MonId, d: MonId, announce: bool) -> bool {
        self.trace
            .push(format!("contact:{}:{}:{announce}", a.0, d.0));
        self.scenario != 2
    }
    fn powder_immunity(&mut self, m: MonId) -> bool {
        self.trace.push(format!("powder:{}", m.0));
        self.scenario != 3
    }
    fn shield_dust(&mut self, m: MonId) -> bool {
        self.trace.push(format!("ability:{}:shielddust", m.0));
        self.scenario == 4
    }
    fn covert_cloak(&mut self, m: MonId) -> bool {
        self.trace.push(format!("item:{}:covertcloak", m.0));
        self.scenario == 5
    }
    fn chance(&mut self, n: u32, d: u32) -> bool {
        self.trace.push(format!("chance:{n}:{d}"));
        self.prng.random_chance(n, d)
    }
    fn random(&mut self, n: u32) -> u32 {
        self.trace.push(format!("random:{n}"));
        self.prng.random(n)
    }
    fn damage(&mut self, m: MonId, n: f64, source: MonId) {
        self.trace.push(format!("damage:{}:{n}:{}", m.0, source.0));
    }
    fn status(&mut self, m: MonId, id: EffectId, source: MonId, sync: bool) {
        let status = dex::effect(id).key;
        self.trace.push(format!(
            "status:{}:{status}:{}:{}:{}",
            m.0,
            source.0,
            if sync { "synchronize" } else { "undefined" },
            if sync { status } else { "undefined" }
        ));
    }
    fn volatile(&mut self, m: MonId, id: EffectId, source: Option<MonId>) {
        self.trace.push(format!(
            "volatile:{}:{}:{}",
            m.0,
            dex::effect(id).key,
            source.map_or("undefined".into(), |m| m.0.to_string())
        ));
    }
    fn activate(&mut self, m: MonId, id: EffectId) {
        self.trace
            .push(format!("activate:{}:{}", m.0, dex::effect(id).key));
    }
    fn hazard(&mut self, side: SideId, id: EffectId, source: MonId) {
        self.trace.push(format!(
            "hazard:{}:{}:{}",
            side.0,
            dex::effect(id).key,
            source.0
        ));
    }
}
#[test]
fn callback_core_calls_and_prng_match_pinned_oracle() {
    let name = dex::effect(ID).key;
    let mut count = 0;
    for line in include_str!("vectors.tsv")
        .lines()
        .filter(|l| l.split('\t').next() == Some(name))
    {
        let c: Vec<_> = line.split('\t').collect();
        let s = c[1].parse::<u8>().unwrap();
        let seed4 = c[2].parse::<u16>().unwrap();
        let source = if s == 12 && name == "synchronize" {
            None
        } else {
            Some(MonId(if s == 11 {
                0
            } else if s == 10 {
                1
            } else {
                6
            }))
        };
        let status = if s == 15 {
            dex::CONDITION_SLP
        } else if s == 16 {
            dex::CONDITION_FRZ
        } else if s == 17 {
            dex::CONDITION_PSN
        } else {
            dex::CONDITION_TOX
        };
        let x = Hit {
            target: MonId(0),
            source,
            owner: if s == 14 {
                MonId(0)
            } else {
                source.unwrap_or(MonId::NONE)
            },
            target_hp: if s == 1 { 0 } else { 40 },
            source_maxhp: 120.0,
            disabled: s == 6,
            struggle: s == 7,
            pecharunt: s != 13,
            is_move: s != 18 && s != 19,
            is_move_callback: name != "synchronize" && name != "poisonpuppeteer",
            physical: s != 8,
            status,
            effect_id: if s == 18 {
                dex::CONDITION_TOXICSPIKES
            } else {
                dex::MOVE_FACADE
            },
            layers: if s == 9 {
                Some(2)
            } else if s == 20 {
                Some(u32::MAX)
            } else {
                None
            },
            side: SideId(1),
        };
        let mut host = RecordingHost {
            scenario: s,
            prng: Prng::from_seed([1, 2, 3, seed4]),
            trace: Vec::new(),
        };
        react(&mut host, x);
        assert_eq!(host.trace.join("|"), c[3], "{line}");
        let seed: Vec<u16> = c[4].split(',').map(|s| s.parse().unwrap()).collect();
        assert_eq!(host.prng.seed().as_slice(), seed, "{line}");
        count += 1;
    }
    assert_eq!(count, 21 * 128);
    assert_eq!(
        crate::effects::registry::hook_coverage(super::HOOKS[0]),
        crate::effects::HookCoverage::Implemented
    );
}
