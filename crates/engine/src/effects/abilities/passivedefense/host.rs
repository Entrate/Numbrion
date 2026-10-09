//! Generic statically dispatched core-call boundary for absorption callbacks.
use super::*;
pub trait Host {
    fn perfect_accuracy(&mut self);
    fn volatile(&mut self, m: MonId, id: EffectId) -> bool;
    fn boost(&mut self, stat: usize) -> bool;
    fn heal(&mut self, m: MonId) -> bool;
    fn immune(&mut self, m: MonId, id: EffectId);
}
pub fn absorb<H: Host>(h: &mut H, id: EffectId, target: MonId) -> Relay {
    let ok = match id {
        dex::ABILITY_FLASHFIRE => {
            h.perfect_accuracy();
            h.volatile(target, dex::CONDITION_FLASHFIRE)
        }
        dex::ABILITY_MOTORDRIVE => h.boost(4),
        dex::ABILITY_SAPSIPPER => h.boost(0),
        _ => h.heal(target),
    };
    if !ok {
        h.immune(target, id)
    }
    Relay::Null
}
pub struct BattleHost<'a, L: LogSink> {
    pub b: &'a mut Battle<L>,
    pub move_index: u8,
}
impl<L: LogSink> Host for BattleHost<'_, L> {
    fn perfect_accuracy(&mut self) {
        self.b.scratch.moves[self.move_index as usize]
            .as_mut()
            .unwrap()
            .accuracy = MoveAccuracy::Always;
    }
    fn volatile(&mut self, m: MonId, id: EffectId) -> bool {
        self.b
            .add_volatile(m, id, Attribution::DEFAULT, None)
            .truthy()
    }
    fn boost(&mut self, s: usize) -> bool {
        self.b
            .boost(singleton(s, 1), None, Attribution::DEFAULT, false, false)
            .truthy()
    }
    fn heal(&mut self, m: MonId) -> bool {
        // In this fixed format baseMaxhp == max_hp: Dynamax is absent.
        self.b
            .heal(
                self.b.state.pokemon[m.0 as usize].max_hp as f64 / 4.0,
                None,
                None,
                HealEffect::Context,
            )
            .truthy()
    }
    fn immune(&mut self, m: MonId, id: EffectId) {
        immune(self.b, m, id)
    }
}
pub trait MirrorHost {
    fn boosts(&mut self) -> &mut OrderedBoosts;
    fn stage(&mut self, stat: usize) -> i8;
    fn source_alive(&mut self) -> bool;
    fn announce(&mut self);
    fn reflect(&mut self, stat: usize, n: i8);
}
pub fn mirror<H: MirrorHost>(h: &mut H) {
    let boost = *h.boosts();
    for stat in boost.order[..boost.len as usize].iter().copied() {
        let stat = stat as usize;
        let n = h.boosts().values[stat];
        if n < 0 && h.stage(stat) != -6 {
            delete(h.boosts(), stat);
            if h.source_alive() {
                h.announce();
                h.reflect(stat, n);
            }
        }
    }
}
pub struct MirrorBattleHost<'a, L: LogSink> {
    pub b: &'a mut Battle<L>,
    pub boosts: u8,
    pub target: MonId,
    pub source: MonId,
}
impl<L: LogSink> MirrorHost for MirrorBattleHost<'_, L> {
    fn boosts(&mut self) -> &mut OrderedBoosts {
        self.b.scratch_boosts(self.boosts)
    }
    fn stage(&mut self, s: usize) -> i8 {
        self.b.state.pokemon[self.target.0 as usize].boosts[s]
    }
    fn source_alive(&mut self) -> bool {
        self.b.state.pokemon[self.source.0 as usize].hp != 0
    }
    fn announce(&mut self) {
        log(
            self.b,
            "-ability",
            self.target,
            dex::ABILITY_MIRRORARMOR,
            false,
        )
    }
    fn reflect(&mut self, s: usize, n: i8) {
        self.b.boost(
            singleton(s, n),
            Some(self.source),
            Attribution::from_move(self.target, EffectRef::None),
            true,
            false,
        );
    }
}
pub trait UpdateHost {
    fn status(&mut self) -> Status;
    fn volatile(&mut self, key: &'static str) -> bool;
    fn activate(&mut self);
    fn cure(&mut self);
    fn remove(&mut self, key: &'static str);
    fn attract_end(&mut self);
}
pub fn update<H: UpdateHost>(h: &mut H, id: EffectId) {
    match id {
        dex::ABILITY_INSOMNIA
        | dex::ABILITY_VITALSPIRIT
        | dex::ABILITY_WATERVEIL
        | dex::ABILITY_WATERBUBBLE => {
            let s = if matches!(id, dex::ABILITY_INSOMNIA | dex::ABILITY_VITALSPIRIT) {
                Status::Sleep
            } else {
                Status::Burn
            };
            if h.status() == s {
                h.activate();
                h.cure();
            }
        }
        dex::ABILITY_OWNTEMPO => {
            if h.volatile("confusion") {
                h.activate();
                h.remove("confusion");
            }
        }
        dex::ABILITY_OBLIVIOUS => {
            if h.volatile("attract") {
                h.activate();
                h.remove("attract");
                h.attract_end();
            }
            if h.volatile("taunt") {
                h.activate();
                h.remove("taunt");
            }
        }
        _ => panic!("unexpected update ability"),
    }
}
pub struct UpdateBattleHost<'a, L: LogSink> {
    pub b: &'a mut Battle<L>,
    pub target: MonId,
    pub id: EffectId,
}
impl<L: LogSink> UpdateHost for UpdateBattleHost<'_, L> {
    fn status(&mut self) -> Status {
        self.b.state.pokemon[self.target.0 as usize].status
    }
    fn volatile(&mut self, name: &'static str) -> bool {
        has_volatile(self.b, self.target, name)
    }
    fn activate(&mut self) {
        log(self.b, "-activate", self.target, self.id, true)
    }
    fn cure(&mut self) {
        self.b.cure_status(self.target, false);
    }
    fn remove(&mut self, name: &'static str) {
        let id = match name {
            "confusion" => dex::CONDITION_CONFUSION,
            "taunt" => dex::CONDITION_TAUNT,
            "attract" => {
                const ATTRACT: EffectId = optional_id(dex::CONDITIONS_DATA, "attract");
                assert!(
                    ATTRACT != EffectId::NONE,
                    "Attract state requires scoped condition"
                );
                ATTRACT
            }
            _ => panic!("unknown volatile"),
        };
        self.b.remove_volatile(self.target, id);
    }
    fn attract_end(&mut self) {
        self.b.add(LogEntry::new(
            "-end",
            &[LogArg::Mon(self.target), LogArg::Text("move: Attract")],
            &[LogTag::From(EffectRef::Dex(self.id))],
        ));
    }
}
