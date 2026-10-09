//! Private, statically dispatched core-call boundary. Tests record these calls
//! against the pinned JS callbacks while the real core mutators remain unfinished.
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{Attribution, ImmunityMessage, MoveHandle},
    dex::{self, ImmunityId},
    effects::support::*,
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink},
};
#[derive(Clone, Copy)]
pub struct Hit {
    pub target: MonId,
    pub source: Option<MonId>,
    pub owner: MonId,
    pub target_hp: u16,
    pub source_maxhp: f64,
    pub disabled: bool,
    pub struggle: bool,
    pub pecharunt: bool,
    pub is_move: bool,
    pub is_move_callback: bool,
    pub physical: bool,
    pub status: EffectId,
    pub effect_id: EffectId,
    pub layers: Option<u32>,
    pub side: SideId,
}
impl Hit {
    pub fn from_battle<L: LogSink>(b: &Battle<L>, cx: HookCtx, id: EffectId) -> Self {
        let status_callback = id == dex::ABILITY_SYNCHRONIZE || id == dex::ABILITY_POISONPUPPETEER;
        let target = mon_arg(b, cx, 1);
        let source = Battle::<L>::arg_mon(b.event_arg(cx, 2));
        let effect = match b.event_arg(cx, 3) {
            EventArg::Effect(e) => e,
            EventArg::Move(i) => EffectRef::ActiveMove(i),
            _ => EffectRef::None,
        };
        let status = if status_callback {
            match b.event_arg(cx, 0) {
                EventArg::Effect(e) => b.event_effect_id(e),
                EventArg::Relay(Relay::Effect(id)) => id,
                _ => panic!("status callback requires condition"),
            }
        } else {
            EffectId::NONE
        };
        let (physical, struggle) = if !status_callback {
            let mv = move_overlay(b, move_arg(b, cx, 3));
            (
                mv.category == dex::Category::Physical,
                mv.id == dex::MOVE_STRUGGLE,
            )
        } else {
            (false, false)
        };
        let owner = if id == dex::ABILITY_CURSEDBODY || id == dex::ABILITY_POISONPUPPETEER {
            let h = b.hook_state(cx).target;
            assert!(h.0 < 12);
            MonId(h.0)
        } else {
            MonId::NONE
        };
        let mut side = source.map_or(SideId(0), |m| m.side());
        if source.is_some_and(|s| s.side() == target.side()) {
            side = SideId(side.0 ^ 1);
        }
        let layers = if id == dex::ABILITY_TOXICDEBRIS {
            b.state.sides[side.0 as usize]
                .conditions
                .as_slice()
                .iter()
                .find_map(|c| {
                    let cell = &b.state.effects.cells[c.0 as usize];
                    if cell.id == dex::CONDITION_TOXICSPIKES {
                        // Contract with the pending hazard owner: Toxic Spikes.layers uses word 0,
                        // custom presence bit 8. Missing layers is JS undefined, so `< 2` is false.
                        Some(if cell.present & (1 << 8) != 0 {
                            cell.payload.words[0]
                        } else {
                            u32::MAX
                        })
                    } else {
                        None
                    }
                })
        } else {
            None
        };
        Self {
            target,
            source,
            owner,
            target_hp: b.state.pokemon[target.0 as usize].hp,
            source_maxhp: source.map_or(0.0, |m| b.state.pokemon[m.0 as usize].max_hp as f64),
            disabled: id == dex::ABILITY_CURSEDBODY
                && source.is_some_and(|s| {
                    b.state.pokemon[s.0 as usize]
                        .volatiles
                        .as_slice()
                        .iter()
                        .any(|c| b.state.effects.cells[c.0 as usize].id == dex::CONDITION_DISABLE)
                }),
            struggle,
            pecharunt: id == dex::ABILITY_POISONPUPPETEER
                && source.is_some_and(|m| {
                    dex::species(b.state.pokemon[m.0 as usize].base_species).name == "Pecharunt"
                }),
            is_move: b.event_effect_type(effect) == dex::EffectType::Move,
            is_move_callback: !status_callback,
            physical,
            status,
            effect_id: b.event_effect_id(effect),
            layers,
            side,
        }
    }
}
pub trait Host {
    fn contact(&mut self, attacker: MonId, defender: MonId, announce: bool) -> bool;
    fn powder_immunity(&mut self, target: MonId) -> bool;
    fn shield_dust(&mut self, target: MonId) -> bool;
    fn covert_cloak(&mut self, target: MonId) -> bool;
    fn chance(&mut self, n: u32, d: u32) -> bool;
    fn random(&mut self, n: u32) -> u32;
    fn damage(&mut self, target: MonId, amount: f64, source: MonId);
    fn status(&mut self, target: MonId, status: EffectId, source: MonId, synchronize: bool);
    fn volatile(&mut self, target: MonId, status: EffectId, source: Option<MonId>);
    fn activate(&mut self, target: MonId, effect: EffectId);
    fn hazard(&mut self, side: SideId, status: EffectId, source: MonId);
}
pub struct BattleHost<'a, L: LogSink> {
    pub b: &'a mut Battle<L>,
    pub move_index: Option<u8>,
}
impl<L: LogSink> Host for BattleHost<'_, L> {
    fn contact(&mut self, a: MonId, d: MonId, announce: bool) -> bool {
        self.b.check_move_makes_contact(
            MoveHandle(self.move_index.expect("contact requires move")),
            a,
            d,
            announce,
        )
    }
    fn powder_immunity(&mut self, m: MonId) -> bool {
        self.b
            .run_status_immunity(m, ImmunityId::Powder, ImmunityMessage::Silent)
    }
    fn shield_dust(&mut self, m: MonId) -> bool {
        self.b.has_ability(m, &[dex::ABILITY_SHIELDDUST])
    }
    fn covert_cloak(&mut self, m: MonId) -> bool {
        self.b.has_item(m, &[dex::ITEM_COVERTCLOAK])
    }
    fn chance(&mut self, n: u32, d: u32) -> bool {
        self.b.state.prng.random_chance(n, d)
    }
    fn random(&mut self, n: u32) -> u32 {
        self.b.state.prng.random(n)
    }
    fn damage(&mut self, m: MonId, n: f64, source: MonId) {
        self.b
            .damage(n, Some(m), Attribution::from_move(source, EffectRef::None));
    }
    fn status(&mut self, m: MonId, status: EffectId, source: MonId, sync: bool) {
        self.b.try_set_status(
            m,
            status,
            Attribution::from_move(
                source,
                if sync {
                    EffectRef::Synchronize(status)
                } else {
                    EffectRef::None
                },
            ),
        );
    }
    fn volatile(&mut self, m: MonId, id: EffectId, source: Option<MonId>) {
        self.b.add_volatile(
            m,
            id,
            source.map_or(Attribution::DEFAULT, |m| {
                Attribution::from_move(m, EffectRef::None)
            }),
            None,
        );
    }
    fn activate(&mut self, m: MonId, id: EffectId) {
        self.b.add(LogEntry::new(
            "-activate",
            &[LogArg::Mon(m), LogArg::EffectFullName(EffectRef::Dex(id))],
            &[],
        ));
    }
    fn hazard(&mut self, side: SideId, id: EffectId, source: MonId) {
        self.b
            .add_side_condition(side, id, Attribution::from_move(source, EffectRef::None));
    }
}
