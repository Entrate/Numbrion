//! Private helpers shared by the move modules. OWNER M. No PRNG draws anywhere in this file.
//!
//! Contains the ActiveMove construction from generated data, fixed-capacity array
//! helpers for `Targets`/`TargetResults` (which mirror JS arrays incl. their holes),
//! the JS result algebra (`combineResults`, falsy/zero distinctions) and the resolved
//! view of a "hit effect" object (the move itself, `self`, `selfBoost`, a secondary).
use crate::{
    actions::{HitEffect, HitTarget, MoveHandle, MoveInput, TargetResults, Targets},
    battle::Battle,
    dex::{
        self, BoostChange, EventId, HookId, HookRel, HookValue, MoveData, MoveTarget, SelfDestruct,
    },
    event::{EventArg, Relay},
    ids::*,
    log::LogSink,
    state::{
        ResultFlag, mon_flags,
        scratch::{
            ActiveMove, MoveAccuracy, MoveEffectsScratch, OrderedBoosts, SecondaryScratch,
            move_runtime as rt,
        },
    },
};

/// Private runtime bit: the creating scope has ended but the frame is still the global
/// `battle.activeMove`, so reclamation is deferred until it is no longer referenced.
/// Bits 0..20 belong to `move_runtime`; 31 is unused by every other owner.
pub(super) const PENDING_RELEASE: u32 = 1 << 31;

/// Empty immutable effects object (selfBoost payloads and the Recharge pseudo move).
pub static EMPTY_EFFECTS: dex::MoveEffects = dex::MoveEffects {
    boosts: &[],
    status: EffectId::NONE,
    volatile_status: EffectId::NONE,
    side_condition: EffectId::NONE,
    slot_condition: EffectId::NONE,
    weather: EffectId::NONE,
    terrain: EffectId::NONE,
    pseudo_weather: EffectId::NONE,
    heal: None,
    force_switch: false,
    self_switch: dex::SelfSwitch::None,
    self_effect: None,
    hooks: &[],
};

/// JS `Math.round` for finite values: ties go toward +infinity.
#[inline]
pub(super) fn js_round(x: f64) -> f64 {
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// `clampIntRange(num, min, max)` (lib/utils.ts:320): floor first, then clamp.
#[inline]
pub(super) fn clamp_int_range(num: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let mut n = num.floor();
    if let Some(min) = min {
        if n < min {
            n = min;
        }
    }
    if let Some(max) = max {
        if n > max {
            n = max;
        }
    }
    n
}

#[inline]
pub(super) fn mon_arg(m: MonId) -> EventArg {
    EventArg::Holder(Holder::mon(m))
}

/// `Pokemon | null` as an event argument: null stays an explicit null.
#[inline]
pub(super) fn opt_mon_arg(m: Option<MonId>) -> EventArg {
    m.map_or(EventArg::Null, mon_arg)
}

/// `Pokemon | null` carried by an event relay or argument.
pub(super) fn arg_to_mon(arg: EventArg) -> Option<MonId> {
    match arg {
        EventArg::Holder(h) if h.0 < 12 => Some(MonId(h.0)),
        EventArg::Relay(Relay::Pokemon(m)) => Some(m),
        _ => None,
    }
}

/// JS typeof ranking used by `combineResults` (battle-actions.ts:1548-1564).
fn combine_rank(r: Relay) -> u8 {
    match r {
        Relay::Undefined => 0,
        Relay::NotFail => 1,
        Relay::Null => 2,
        Relay::Bool(_) => 3,
        Relay::Number(_) => 4,
        other => panic!("combineResults received a non-primitive relay {other:?}"),
    }
}

/// `combineResults(left, right)`; pure.
pub(super) fn combine_results(left: Relay, right: Relay) -> Relay {
    if combine_rank(left) > combine_rank(right) {
        left
    } else if left.truthy() && !right.truthy() && right != Relay::Number(0.0) {
        left
    } else if let (Relay::Number(a), Relay::Number(b)) = (left, right) {
        Relay::Number(a + b)
    } else {
        right
    }
}

/// Strict `x === false`.
#[inline]
pub(super) fn is_false(r: Relay) -> bool {
    matches!(r, Relay::Bool(false))
}

/// The recurring `hitResults[i] || hitResults[i] === 0` survivor test.
#[inline]
pub(super) fn keeps(r: Relay) -> bool {
    r.truthy() || r == Relay::Number(0.0)
}

/// `!x && x !== 0`.
#[inline]
pub(super) fn failed_not_zero(r: Relay) -> bool {
    !keeps(r)
}

/// `pokemon.moveThisTurnResult = r` (undefined/null/false/true only).
pub(super) fn result_flag(r: Relay) -> ResultFlag {
    match r {
        Relay::Undefined => ResultFlag::Undefined,
        Relay::Null => ResultFlag::Null,
        Relay::Bool(false) => ResultFlag::False,
        Relay::Bool(true) => ResultFlag::True,
        other => panic!("moveThisTurnResult cannot represent {other:?}"),
    }
}

/// JS array helpers over the fixed four-entry target list.
pub(super) trait TargetsExt: Sized {
    fn from_mons(mons: &[MonId]) -> Self;
    fn single(target: HitTarget) -> Self;
    fn push_target(&mut self, t: HitTarget);
    fn extend_targets(&mut self, other: Targets);
    fn at(&self, i: usize) -> HitTarget;
    fn mon_at(&self, i: usize) -> Option<MonId>;
    fn contains_mon(&self, m: MonId) -> bool;
    fn count(&self) -> usize;
}
impl TargetsExt for Targets {
    fn from_mons(mons: &[MonId]) -> Self {
        let mut t = Targets::default();
        for &m in mons {
            t.push_target(HitTarget::Pokemon(m));
        }
        t
    }
    fn single(target: HitTarget) -> Self {
        let mut t = Targets::default();
        t.push_target(target);
        t
    }
    fn push_target(&mut self, t: HitTarget) {
        assert!(
            (self.len as usize) < self.entries.len(),
            "target list capacity exceeded"
        );
        self.entries[self.len as usize] = t;
        self.len += 1;
    }
    fn extend_targets(&mut self, other: Targets) {
        for i in 0..other.len as usize {
            self.push_target(other.entries[i]);
        }
    }
    #[inline]
    fn at(&self, i: usize) -> HitTarget {
        if i < self.len as usize {
            self.entries[i]
        } else {
            HitTarget::False
        }
    }
    #[inline]
    fn mon_at(&self, i: usize) -> Option<MonId> {
        match self.at(i) {
            HitTarget::Pokemon(m) if i < self.len as usize => Some(m),
            _ => None,
        }
    }
    fn contains_mon(&self, m: MonId) -> bool {
        self.entries[..self.len as usize].contains(&HitTarget::Pokemon(m))
    }
    #[inline]
    fn count(&self) -> usize {
        self.len as usize
    }
}

/// JS array semantics for per-target relays: reads beyond the length are undefined,
/// writes beyond it extend the array and fill any gap with undefined.
pub(super) trait ResultsExt: Sized {
    fn filled(len: usize, v: Relay) -> Self;
    fn at(&self, i: usize) -> Relay;
    fn set(&mut self, i: usize, v: Relay);
    fn push_value(&mut self, v: Relay);
    fn count(&self) -> usize;
    fn includes_false(&self) -> bool;
    fn some_false(&self) -> bool;
}
impl ResultsExt for TargetResults {
    fn filled(len: usize, v: Relay) -> Self {
        assert!(len <= 4);
        let mut r = TargetResults::default();
        for i in 0..len {
            r.values[i] = v;
        }
        r.len = len as u8;
        r
    }
    #[inline]
    fn at(&self, i: usize) -> Relay {
        if i < self.len as usize {
            self.values[i]
        } else {
            Relay::Undefined
        }
    }
    fn set(&mut self, i: usize, v: Relay) {
        assert!(i < self.values.len(), "target result capacity exceeded");
        while (self.len as usize) < i {
            self.values[self.len as usize] = Relay::Undefined;
            self.len += 1;
        }
        self.values[i] = v;
        if i >= self.len as usize {
            self.len = i as u8 + 1;
        }
    }
    fn push_value(&mut self, v: Relay) {
        let n = self.len as usize;
        self.set(n, v);
    }
    #[inline]
    fn count(&self) -> usize {
        self.len as usize
    }
    fn includes_false(&self) -> bool {
        self.values[..self.len as usize]
            .iter()
            .any(|v| is_false(*v))
    }
    fn some_false(&self) -> bool {
        self.includes_false()
    }
}

/// Ordered sparse boosts from generated data (insertion order = object key order).
pub(super) fn ordered_boosts(changes: &[BoostChange]) -> OrderedBoosts {
    let mut b = OrderedBoosts::default();
    for c in changes {
        // StatId::Hp is 0; boost keys start at atk.
        let i = (c.stat as usize)
            .checked_sub(1)
            .expect("HP is not a boost key");
        b.values[i] = c.delta;
        b.order[b.len as usize] = i as u8;
        b.len += 1;
        b.present |= 1 << i;
    }
    b
}

/// Fresh mutable copy of an immutable effects object.
pub(super) fn effects_scratch(base: &'static dex::MoveEffects) -> MoveEffectsScratch {
    MoveEffectsScratch {
        base,
        boosts: ordered_boosts(base.boosts),
        status: base.status,
        volatile_status: base.volatile_status,
        side_condition: base.side_condition,
        slot_condition: base.slot_condition,
        weather: base.weather,
        terrain: base.terrain,
        pseudo_weather: base.pseudo_weather,
        chance: None,
        heal: base.heal,
        force_switch: base.force_switch,
        self_switch: base.self_switch,
        suppressed_hooks: 0,
    }
}

/// `self.chance` is not in `MoveEffects`; it survives in the declarative property tree.
fn self_chance(id: EffectId) -> Option<u16> {
    match dex::effect(id)
        .data
        .get(dex::FIELD_SELF)?
        .get(dex::FIELD_CHANCE)?
    {
        dex::DataValue::Number(n) => Some(n as u16),
        _ => None,
    }
}

/// `dex.getActiveMove(id)` for a scoped move (sim/dex.ts:316-321).
pub(super) fn build_active_move(id: EffectId) -> ActiveMove {
    assert_eq!(
        id.kind(),
        Some(EffectKind::Move),
        "active moves are built from move ids"
    );
    let m: &'static MoveData = dex::move_data(id);
    let mut runtime = 0;
    let t = m.traits;
    if t & dex::MOVE_TRAIT_IGNOREABILITY != 0 {
        runtime |= rt::IGNORE_ABILITY;
    }
    if t & dex::MOVE_TRAIT_IGNOREEVASION != 0 {
        runtime |= rt::IGNORE_EVASION;
    }
    if t & dex::MOVE_TRAIT_SMARTTARGET != 0 {
        runtime |= rt::SMART_TARGET | rt::SMART_TARGET_PRESENT;
    }
    if t & dex::MOVE_TRAIT_SPREADHIT != 0 {
        runtime |= rt::SPREAD_HIT;
    }
    if t & dex::MOVE_TRAIT_WILLCRIT != 0 {
        runtime |= rt::WILL_CRIT | rt::WILL_CRIT_PRESENT;
    }
    if t & dex::MOVE_TRAIT_MULTIACCURACY != 0 {
        runtime |= rt::MULTIACCURACY;
    }
    // Dex always defines ignoreImmunity (dex-moves.ts:497): `?? category === 'Status'`.
    runtime |= rt::IGNORE_IMMUNITY_PRESENT;
    if m.ignore_immunity {
        runtime |= rt::IGNORE_IMMUNITY;
    }
    if m.multihit != [1, 1] {
        runtime |= rt::MULTIHIT_PRESENT;
        if m.multihit[0] != m.multihit[1] {
            runtime |= rt::MULTIHIT_RANGE;
        }
    }
    let mut secondaries = [None; 4];
    assert!(m.secondaries.len() <= 4, "more than four secondaries");
    for (i, s) in m.secondaries.iter().enumerate() {
        secondaries[i] = Some(SecondaryScratch {
            effects: effects_scratch(&s.effects),
            self_effect: s.effects.self_effect.map(effects_scratch),
            chance: s.chance.unwrap_or(0),
            chance_present: s.chance.is_some(),
        });
    }
    let mut effects = effects_scratch(&m.effects);
    effects.chance = None;
    let mut self_effect = m.effects.self_effect.map(effects_scratch);
    if let Some(e) = self_effect.as_mut() {
        e.chance = self_chance(id);
    }
    ActiveMove {
        flags: m.flags,
        traits: m.traits,
        runtime_flags: runtime,
        total_damage: 0,
        hit_targets: [MonId::NONE; 4],
        hit_target_len: 0,
        id,
        source_effect: crate::event::EffectRef::None,
        type_changer_boosted: EffectId::NONE,
        ruined_stats: [MonId::NONE; 4],
        base_power: f64::from(m.base_power),
        accuracy: match m.accuracy {
            dex::Accuracy::Always => MoveAccuracy::Always,
            dex::Accuracy::Percent(p) => MoveAccuracy::Percent(f64::from(p)),
        },
        hit_data: [Default::default(); 12],
        self_boosts: m
            .self_boost
            .map_or(OrderedBoosts::default(), |s| ordered_boosts(s.boosts)),
        effects,
        self_effect,
        secondaries,
        secondary_count: m.secondaries.len() as u8,
        secondaries_present: !m.secondaries.is_empty(),
        recoil: m.recoil,
        drain: m.drain,
        damage: m.damage,
        self_destruct: m.self_destruct,
        offensive_stat: m.offensive_stat,
        defensive_stat: m.defensive_stat,
        offensive_target: m.offensive_target,
        ignore_immunity_types: 0,
        move_type: m.move_type,
        category: m.category,
        target: m.target,
        priority: m.priority,
        crit_ratio: m.crit_ratio,
        hit: 0,
        multihit: m.multihit,
    }
}

/// The synthetic "recharge" move: a nonexistent Dex entry (`dex.moves.get('recharge')`,
/// exists=false) with no id, no target and no data. Never given a scoped dex id.
pub(super) fn build_recharge_move() -> ActiveMove {
    ActiveMove {
        flags: 0,
        traits: 0,
        runtime_flags: rt::IGNORE_IMMUNITY_PRESENT,
        total_damage: 0,
        hit_targets: [MonId::NONE; 4],
        hit_target_len: 0,
        id: EffectId::NONE,
        source_effect: crate::event::EffectRef::None,
        type_changer_boosted: EffectId::NONE,
        ruined_stats: [MonId::NONE; 4],
        base_power: f64::NAN,
        accuracy: MoveAccuracy::Percent(100.0),
        hit_data: [Default::default(); 12],
        self_boosts: OrderedBoosts::default(),
        effects: effects_scratch(&EMPTY_EFFECTS),
        self_effect: None,
        secondaries: [None; 4],
        secondary_count: 0,
        secondaries_present: false,
        recoil: None,
        drain: None,
        damage: dex::DamageSpec::None,
        self_destruct: SelfDestruct::None,
        offensive_stat: None,
        defensive_stat: None,
        offensive_target: false,
        ignore_immunity_types: 0,
        move_type: TypeId::NONE,
        category: dex::Category::Status,
        target: MoveTarget::Normal,
        priority: 0,
        crit_ratio: 1,
        hit: 0,
        multihit: [1, 1],
    }
}

/// Resolved fields of a "hit effect" object (`moveData` in battle-actions.ts:1014+):
/// the move itself, `move.self`, `move.selfBoost`, one secondary or its `self`.
#[derive(Clone, Copy, Debug)]
pub(super) struct HitView {
    pub effects: MoveEffectsScratch,
    /// `moveData.self` is present.
    pub has_self: bool,
    pub self_effect: Option<MoveEffectsScratch>,
    /// `moveData.secondaries` is present (only the move itself has them).
    pub has_secondaries: bool,
    /// `moveData.selfdestruct` (only the move itself has it).
    pub self_destruct: SelfDestruct,
}

/// Secondary arrays produced by ModifySecondaries live in worker scratch; a hit effect
/// names one entry as `handle << 2 | index` (handle < 8, index < 4).
pub(super) fn secondary_code(handle: u8, index: usize) -> u8 {
    assert!(handle < 8 && index < 4);
    handle << 2 | index as u8
}

impl<L: LogSink> Battle<L> {
    #[inline]
    pub(super) fn mflag(&self, h: MoveHandle, bit: u32) -> bool {
        self.active_move(h).runtime_flags & bit != 0
    }
    #[inline]
    pub(super) fn set_mflag(&mut self, h: MoveHandle, bit: u32, on: bool) {
        let m = self.active_move_mut(h);
        if on {
            m.runtime_flags |= bit;
        } else {
            m.runtime_flags &= !bit;
        }
    }
    #[inline]
    pub(super) fn mon_fainted(&self, m: MonId) -> bool {
        self.state.pokemon[m.0 as usize].flags & mon_flags::FAINTED != 0
    }
    #[inline]
    pub(super) fn mon_active(&self, m: MonId) -> bool {
        self.state.pokemon[m.0 as usize].flags & mon_flags::ACTIVE != 0
    }
    #[inline]
    pub(super) fn mon_hp(&self, m: MonId) -> u16 {
        self.state.pokemon[m.0 as usize].hp
    }

    /// Dex-data view of a move input. `None` target means the Recharge pseudo move.
    pub(super) fn input_target(&self, input: MoveInput) -> Option<MoveTarget> {
        match input {
            MoveInput::Dex(id) => Some(dex::move_data(id).target),
            MoveInput::Active(h) => {
                let m = self.active_move(h);
                (m.id != EffectId::NONE).then_some(m.target)
            }
            MoveInput::Recharge => None,
        }
    }
    pub(super) fn input_smart_target(&self, input: MoveInput) -> bool {
        match input {
            MoveInput::Dex(id) => dex::move_data(id).traits & dex::MOVE_TRAIT_SMARTTARGET != 0,
            MoveInput::Active(h) => {
                let rf = self.active_move(h).runtime_flags;
                rf & rt::SMART_TARGET_PRESENT != 0 && rf & rt::SMART_TARGET != 0
            }
            MoveInput::Recharge => false,
        }
    }
    pub(super) fn input_tracks_target(&self, input: MoveInput) -> bool {
        match input {
            // No scoped move declares tracksTarget; Stalwart/Propeller Tail set it at runtime.
            MoveInput::Dex(_) | MoveInput::Recharge => false,
            MoveInput::Active(h) => self.mflag(h, rt::TRACKS_TARGET),
        }
    }

    /// Resolve a `moveData` object to its fields. Panics on a missing nested object,
    /// which would mean the caller named an effect the move does not have.
    pub(super) fn hit_view(&self, h: MoveHandle, effect: HitEffect) -> HitView {
        let m = self.active_move(h);
        match effect {
            HitEffect::Primary => HitView {
                effects: m.effects,
                has_self: m.self_effect.is_some(),
                self_effect: m.self_effect,
                has_secondaries: m.secondaries_present,
                self_destruct: m.self_destruct,
            },
            HitEffect::SelfEffect => HitView {
                effects: m.self_effect.expect("move has no self effect"),
                has_self: false,
                self_effect: None,
                has_secondaries: false,
                self_destruct: SelfDestruct::None,
            },
            HitEffect::SelfBoost => {
                let base: &'static dex::MoveEffects = if m.id == EffectId::NONE {
                    &EMPTY_EFFECTS
                } else {
                    dex::move_data(m.id).self_boost.unwrap_or(&EMPTY_EFFECTS)
                };
                let mut effects = effects_scratch(base);
                effects.boosts = m.self_boosts;
                HitView {
                    effects,
                    has_self: false,
                    self_effect: None,
                    has_secondaries: false,
                    self_destruct: SelfDestruct::None,
                }
            }
            HitEffect::Secondary(code) => {
                let s = self.secondary_entry(code);
                HitView {
                    effects: s.effects,
                    has_self: s.self_effect.is_some(),
                    self_effect: s.self_effect,
                    has_secondaries: false,
                    self_destruct: SelfDestruct::None,
                }
            }
            HitEffect::SecondarySelf(code) => {
                let s = self.secondary_entry(code);
                HitView {
                    effects: s.self_effect.expect("secondary has no self effect"),
                    has_self: false,
                    self_effect: None,
                    has_secondaries: false,
                    self_destruct: SelfDestruct::None,
                }
            }
            HitEffect::Scratch(effects) => HitView {
                effects,
                has_self: false,
                self_effect: None,
                has_secondaries: false,
                self_destruct: SelfDestruct::None,
            },
        }
    }

    pub(super) fn secondary_entry(&self, code: u8) -> SecondaryScratch {
        let (handle, index) = ((code >> 2) as usize, (code & 3) as usize);
        assert!(
            handle < 8 && self.scratch.secondaries_used & (1 << handle) != 0,
            "released secondary array"
        );
        self.scratch.secondaries[handle][index].expect("secondary entry is absent")
    }

    /// `moveData.onX` for a hit-effect object: its callback exists and was not deleted.
    pub(super) fn hit_hook(&self, view: &HitView, event: EventId) -> Option<HookId> {
        for (i, &h) in view.effects.base.hooks.iter().enumerate() {
            if view.effects.suppressed_hooks & (1 << i) != 0 {
                continue;
            }
            let hook = &dex::HOOKS[h.0 as usize];
            if hook.event == event
                && hook.rel == HookRel::On
                && !matches!(hook.value, HookValue::Absent)
            {
                return Some(h);
            }
        }
        None
    }
}
