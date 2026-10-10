//! Private static core boundary. The same callback bodies use a recording host
//! in tests; production calls the shared mutators, with no allocation or vtable.
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{Attribution, HitTarget, MoveHandle, Stat, StatOptions},
    dex,
    effects::support::*,
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::{
        Pokemon, present,
        scratch::{OrderedBoosts, move_runtime},
    },
};
#[derive(Clone, Copy)]
pub struct Move {
    pub id: EffectId,
    pub key: &'static str,
    pub move_type: TypeId,
    pub category: dex::Category,
    pub flags: u64,
    pub multihit: bool,
    pub total_damage: u32,
    pub smart: bool,
    pub ruined: [Option<MonId>; 4],
}
pub fn boosts(values: &[(u8, i8)]) -> OrderedBoosts {
    let mut b = OrderedBoosts::default();
    for &(s, n) in values {
        b.order[b.len as usize] = s;
        b.len += 1;
        b.present |= 1 << s;
        b.values[s as usize] = n;
    }
    b
}
#[derive(Clone, Copy, Default)]
pub struct Foes {
    pub entries: [MonId; 4],
    pub len: u8,
}
impl Foes {
    pub fn as_slice(&self) -> &[MonId] {
        &self.entries[..self.len as usize]
    }
}
pub trait Host {
    fn mon(&self, i: usize) -> Option<MonId>;
    fn owner(&self) -> MonId;
    fn number(&self, i: usize) -> f64;
    fn effect(&self, i: usize) -> Option<dex::EffectType>;
    fn effect_id(&self, i: usize) -> EffectId;
    fn effect_status(&self, i: usize) -> bool;
    fn boost_arg(&self, i: usize) -> OrderedBoosts;
    fn mv(&self, i: usize) -> Move;
    fn pokemon(&self, m: MonId) -> Pokemon;
    fn species_name(&self, m: MonId) -> &'static str;
    fn flag(&mut self, m: MonId, flag: u32);
    fn foes(&mut self, m: MonId, adjacent: bool) -> Foes;
    fn foe_left(&self, m: MonId) -> bool;
    fn volatile(&mut self, m: MonId, id: EffectId) -> bool;
    fn tailwind(&self, s: SideId) -> bool;
    fn stat(&mut self, m: MonId, s: Stat) -> f64;
    fn contact(&mut self, s: MonId, t: MonId) -> bool;
    fn suppress_ability(&mut self, m: MonId) -> bool;
    fn suppress_secondaries(&mut self) -> bool;
    fn ability(&mut self, m: MonId, id: EffectId) -> bool;
    fn last_damage(&mut self, m: MonId) -> Option<f64>;
    fn checked(&self) -> Option<bool>;
    fn set_checked(&mut self, v: bool);
    fn boost(
        &mut self,
        b: OrderedBoosts,
        t: Option<MonId>,
        s: Option<MonId>,
        effect: Option<EffectId>,
        secondary: bool,
        self_boost: bool,
    ) -> Relay;
    fn log(&mut self, command: &'static str, m: MonId, id: EffectId, from: bool, boost: bool);
    fn cure(&mut self, m: MonId);
    fn multihit(&mut self, i: usize, n: u8);
    fn ruin(&mut self, i: usize, slot: usize, m: MonId);
    fn chain(&mut self);
}
pub struct BattleHost<'a, L: LogSink> {
    pub b: &'a mut Battle<L>,
    pub cx: HookCtx,
}
impl<L: LogSink> BattleHost<'_, L> {
    fn arg(&self, i: usize) -> EventArg {
        self.b.event_arg(self.cx, i)
    }
    fn effect_ref(&self, i: usize) -> EffectRef {
        match self.arg(i) {
            EventArg::Effect(e) => e,
            EventArg::Move(j) | EventArg::Relay(Relay::ActiveMove(j)) => EffectRef::ActiveMove(j),
            EventArg::Relay(Relay::Effect(id)) => EffectRef::Dex(id),
            _ => EffectRef::None,
        }
    }
}
impl<L: LogSink> Host for BattleHost<'_, L> {
    fn mon(&self, i: usize) -> Option<MonId> {
        Battle::<L>::arg_mon(self.arg(i))
    }
    fn owner(&self) -> MonId {
        let h = self.b.hook_state(self.cx).target;
        assert!(h.0 < 12);
        MonId(h.0)
    }
    fn number(&self, i: usize) -> f64 {
        relay_number(self.b, self.cx, i)
    }
    fn effect(&self, i: usize) -> Option<dex::EffectType> {
        let e = self.effect_ref(i);
        (e != EffectRef::None).then(|| self.b.event_effect_type(e))
    }
    fn effect_id(&self, i: usize) -> EffectId {
        self.b.event_effect_id(self.effect_ref(i))
    }
    fn effect_status(&self, i: usize) -> bool {
        match self.effect_ref(i) {
            EffectRef::Synchronize(status) => status != EffectId::NONE,
            EffectRef::ActiveMove(j) => move_overlay(self.b, j).effects.status != EffectId::NONE,
            EffectRef::Dex(id)
                if self.b.event_effect_type(EffectRef::Dex(id)) == dex::EffectType::Move =>
            {
                dex::move_data(id).effects.status != EffectId::NONE
            }
            _ => false,
        }
    }
    fn boost_arg(&self, i: usize) -> OrderedBoosts {
        match self.arg(i) {
            EventArg::Relay(Relay::Boosts(j)) => {
                assert!(
                    j < 8 && self.b.scratch.boosts_used & (1 << j) != 0,
                    "released boost object"
                );
                self.b.scratch.boosts[j as usize]
            }
            _ => panic!("requires ordered boost object"),
        }
    }
    fn mv(&self, i: usize) -> Move {
        let m = move_overlay(self.b, move_arg(self.b, self.cx, i));
        Move {
            id: m.id,
            key: dex::effect(m.id).key,
            move_type: m.move_type,
            category: m.category,
            flags: m.flags,
            multihit: m.runtime_flags & move_runtime::MULTIHIT_PRESENT != 0
                && (m.runtime_flags & move_runtime::MULTIHIT_RANGE != 0 || m.multihit[0] != 0),
            total_damage: m.total_damage,
            smart: m.runtime_flags & move_runtime::SMART_TARGET != 0,
            ruined: m.ruined_stats.map(|m| (m != MonId::NONE).then_some(m)),
        }
    }
    fn pokemon(&self, m: MonId) -> Pokemon {
        self.b.state.pokemon[m.0 as usize]
    }
    fn species_name(&self, m: MonId) -> &'static str {
        dex::species(self.pokemon(m).species).name
    }
    fn flag(&mut self, m: MonId, flag: u32) {
        self.b.state.pokemon[m.0 as usize].flags |= flag;
    }
    fn foes(&mut self, m: MonId, adjacent: bool) -> Foes {
        let t = if adjacent {
            self.b.adjacent_foes(m)
        } else {
            self.b.foes(m, false)
        };
        let mut f = Foes::default();
        for e in &t.entries[..t.len as usize] {
            let HitTarget::Pokemon(m) = e else {
                panic!("foes must be Pokemon")
            };
            f.entries[f.len as usize] = *m;
            f.len += 1;
        }
        f
    }
    fn foe_left(&self, m: MonId) -> bool {
        self.b.state.sides[(m.side().0 ^ 1) as usize].pokemon_left != 0
    }
    fn volatile(&mut self, m: MonId, id: EffectId) -> bool {
        self.b.get_volatile(m, id).is_some()
    }
    fn tailwind(&self, s: SideId) -> bool {
        self.b.state.sides[s.0 as usize]
            .conditions
            .as_slice()
            .iter()
            .any(|c| self.b.state.effects.cells[c.0 as usize].id == dex::CONDITION_TAILWIND)
    }
    fn stat(&mut self, m: MonId, s: Stat) -> f64 {
        self.b.get_stat(
            m,
            s,
            StatOptions {
                unboosted: false,
                unmodified: true,
            },
        )
    }
    fn contact(&mut self, s: MonId, t: MonId) -> bool {
        self.b
            .check_move_makes_contact(MoveHandle(move_arg(self.b, self.cx, 3)), s, t, true)
    }
    fn suppress_ability(&mut self, m: MonId) -> bool {
        self.b.suppressing_ability(Some(m))
    }
    fn suppress_secondaries(&mut self) -> bool {
        self.b.suppressing_secondaries()
    }
    fn ability(&mut self, m: MonId, id: EffectId) -> bool {
        self.b.has_ability(m, &[id])
    }
    fn last_damage(&mut self, m: MonId) -> Option<f64> {
        self.b
            .state
            .last_attacked_by(m)
            .map(|(_, r)| r.last_damage as f64)
    }
    fn checked(&self) -> Option<bool> {
        let s = self.b.hook_state(self.cx);
        (s.present & (1 << 8) != 0).then_some(s.payload.words[0] != 0)
    }
    fn set_checked(&mut self, v: bool) {
        let s = self.b.hook_state_mut(self.cx);
        s.present |= 1 << 8;
        s.payload.words[0] = u32::from(v);
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
        self.b.boost(
            b,
            t,
            Attribution {
                source: s.map_or(EventArg::Null, |s| EventArg::Holder(Holder::mon(s))),
                effect: effect.map_or(EffectRef::None, EffectRef::Dex),
            },
            secondary,
            self_boost,
        )
    }
    fn log(&mut self, command: &'static str, m: MonId, id: EffectId, from: bool, boost: bool) {
        if from {
            self.b.add(LogEntry::new(
                command,
                &[LogArg::Mon(m)],
                &[LogTag::From(EffectRef::Dex(id))],
            ));
        } else if id == EffectId::NONE {
            self.b.add(LogEntry::new(command, &[LogArg::Mon(m)], &[]));
        } else if command == "-activate" {
            self.b.add(LogEntry::new(
                command,
                &[LogArg::Mon(m), LogArg::EffectFullName(EffectRef::Dex(id))],
                &[],
            ));
        } else if boost {
            self.b.add(LogEntry::new(
                command,
                &[
                    LogArg::Mon(m),
                    LogArg::Effect(EffectRef::Dex(id)),
                    LogArg::Text("boost"),
                ],
                &[],
            ));
        } else {
            self.b.add(LogEntry::new(
                command,
                &[LogArg::Mon(m), LogArg::Effect(EffectRef::Dex(id))],
                &[],
            ));
        }
    }
    fn cure(&mut self, m: MonId) {
        self.b.cure_status(m, false);
    }
    fn multihit(&mut self, i: usize, n: u8) {
        let j = move_arg(self.b, self.cx, i);
        let m = self.b.scratch.moves[j as usize].as_mut().unwrap();
        m.multihit = [n, n];
        m.runtime_flags |= move_runtime::MULTIHIT_PRESENT;
        m.runtime_flags &= !move_runtime::MULTIHIT_RANGE;
    }
    fn ruin(&mut self, i: usize, slot: usize, m: MonId) {
        let j = move_arg(self.b, self.cx, i);
        self.b.scratch.moves[j as usize]
            .as_mut()
            .unwrap()
            .ruined_stats[slot] = m;
    }
    fn chain(&mut self) {
        self.b.chain_modify(3.0, 4.0);
    }
}
