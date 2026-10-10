//! Private helpers for the `reactive_stats` effect batch (abilities that react to
//! damage, KOs, boosts and switch-in with a stat change, plus the four Ruin abilities).
//!
//! Owned by `battlebond.rs` (`pub(super) mod support`); every other file in the batch
//! reaches it through `crate::effects::registry::abilities_battlebond::support`.
//! Nothing here allocates, formats a string or draws from the PRNG. Handler bodies stay
//! in their own effect files so each one can be read next to its pinned TypeScript.
#![allow(dead_code)]
use crate::{
    Battle,
    actions::{Attribution, HitTarget, Stat},
    dex,
    effects::support::move_overlay,
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::*,
    log::{LogArg, LogEntry, LogSink, LogTag},
    state::scratch::{OrderedBoosts, move_runtime},
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Compile-time lookup of a type by its display name (`move.type === 'Dark'`).
/// Mirrors `dex::type_id`, so ids stay in lockstep with the generated table.
pub const fn type_named(name: &str) -> TypeId {
    let want = name.as_bytes();
    let mut i = 0;
    while i < dex::TYPE_NAMES.len() {
        let have = dex::TYPE_NAMES[i].as_bytes();
        if have.len() == want.len() {
            let mut j = 0;
            let mut same = true;
            while j < have.len() {
                if have[j] != want[j] {
                    same = false;
                }
                j += 1;
            }
            if same {
                return TypeId(i as u8 + 1);
            }
        }
        i += 1;
    }
    panic!("unknown type name")
}
pub const TYPE_BUG: TypeId = type_named("Bug");
pub const TYPE_DARK: TypeId = type_named("Dark");
pub const TYPE_FIRE: TypeId = type_named("Fire");
pub const TYPE_GHOST: TypeId = type_named("Ghost");
pub const TYPE_WATER: TypeId = type_named("Water");

/// Index of a stat in `Pokemon::boosts` / `OrderedBoosts` (atk, def, spa, spd, spe, ...).
pub const fn boost_index(stat: Stat) -> usize {
    stat as usize
}

/// A JS object literal such as `{ def: -1, spe: 2 }`: insertion order is the argument
/// order, and every key is distinct at all call sites.
pub fn boosts<const N: usize>(entries: [(Stat, i8); N]) -> OrderedBoosts {
    let mut out = OrderedBoosts::default();
    for (stat, amount) in entries {
        let i = boost_index(stat);
        debug_assert_eq!(out.present & (1 << i), 0, "duplicate boost key");
        out.values[i] = amount;
        out.order[out.len as usize] = i as u8;
        out.len += 1;
        out.present |= 1 << i;
    }
    out
}

/// Value of one key of a boost object, `undefined` when the key is absent.
pub fn boost_get(object: &OrderedBoosts, stat: Stat) -> Option<i8> {
    let i = boost_index(stat);
    (object.present & (1 << i) != 0).then_some(object.values[i])
}

/// `for (i in boost) if (boost[i] < 0) statsLowered = true;`
pub fn any_lowered(object: &OrderedBoosts) -> bool {
    object.order[..object.len as usize]
        .iter()
        .any(|&i| object.values[i as usize] < 0)
}

/// Boost-object relay/argument (`boost` of AfterEachBoost/AfterBoost) as a copy.
pub fn boost_arg<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> OrderedBoosts {
    match b.event_arg(cx, index) {
        EventArg::Relay(Relay::Boosts(handle)) => {
            assert!(
                handle < 8 && b.scratch.boosts_used & (1 << handle) != 0,
                "released boost object"
            );
            b.scratch.boosts[handle as usize]
        }
        _ => panic!("callback requires boost object argument"),
    }
}

/// `this.boost(...)` source argument given as a Pokemon; the effect stays `null`
/// so the mutator applies its own `effect ||= this.effect` default.
pub fn by(source: MonId) -> Attribution {
    Attribution::from_move(source, EffectRef::None)
}

/// `this.effectState.target` of the executing ability (`abilityHolder`).
pub fn owner<L: LogSink>(b: &Battle<L>, cx: HookCtx) -> MonId {
    let holder = b.hook_state(cx).target;
    assert!(holder.0 < 12, "ability state target is not a Pokemon");
    MonId(holder.0)
}

/// Required Pokemon callback argument.
pub fn mon<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> MonId {
    Battle::<L>::arg_mon(b.event_arg(cx, index)).expect("callback requires Pokemon argument")
}

/// Nullable Pokemon callback argument (`source` may be null or undefined).
pub fn mon_opt<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> Option<MonId> {
    Battle::<L>::arg_mon(b.event_arg(cx, index))
}

/// Effect-like callback argument as an `EffectRef` (`None` for null or undefined).
pub fn effect_at<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> EffectRef {
    match b.event_arg(cx, index) {
        EventArg::Effect(e) => e,
        EventArg::Move(m) | EventArg::Relay(Relay::ActiveMove(m)) => EffectRef::ActiveMove(m),
        EventArg::Relay(Relay::Effect(id)) | EventArg::Relay(Relay::Move(id)) => EffectRef::Dex(id),
        EventArg::Undefined
        | EventArg::Null
        | EventArg::Relay(Relay::Undefined)
        | EventArg::Relay(Relay::Null) => EffectRef::None,
        _ => panic!("callback requires effect argument"),
    }
}

/// `effect?.effectType === 'Move'`; an absent effect is not a move.
pub fn effect_is_move<L: LogSink>(b: &Battle<L>, effect: EffectRef) -> bool {
    effect != EffectRef::None && b.event_effect_type(effect) == dex::EffectType::Move
}

/// Truthiness of `effect.multihit` for the effect of a Damage event. ActiveMove keeps
/// absent/fixed/ranged multihit in runtime bits; dex data stores absent as `[1, 1]`.
/// Synthetic move effects (confusion, recoil, ...) have no multihit property.
pub fn effect_has_multihit<L: LogSink>(b: &Battle<L>, effect: EffectRef) -> bool {
    match effect {
        EffectRef::ActiveMove(i) => {
            let m = move_overlay(b, i);
            m.runtime_flags & move_runtime::MULTIHIT_PRESENT != 0
                && (m.runtime_flags & move_runtime::MULTIHIT_RANGE != 0 || m.multihit[0] != 0)
        }
        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
            dex::move_data(id).multihit != [1, 1]
        }
        _ => false,
    }
}

/// Truthiness of `(effect as Move)?.status` (the move-level status, never a secondary).
/// Synchronize's `{ status: status.id, id: 'synchronize' }` source object has one too.
pub fn effect_has_status<L: LogSink>(b: &Battle<L>, effect: EffectRef) -> bool {
    match effect {
        EffectRef::ActiveMove(i) => move_overlay(b, i).effects.status != EffectId::NONE,
        EffectRef::Dex(id) if id.kind() == Some(EffectKind::Move) => {
            dex::move_data(id).effects.status != EffectId::NONE
        }
        EffectRef::Synchronize(status) => status != EffectId::NONE,
        _ => false,
    }
}

/// Effect id carried by a relay or effect argument (`item.id`, `status.id`, `sideCondition.id`).
pub fn effect_id_at<L: LogSink>(b: &Battle<L>, cx: HookCtx, index: usize) -> EffectId {
    b.event_effect_id(effect_at(b, cx, index))
}

/// The entries of `pokemon.foes()` / `adjacentFoes()` are always Pokemon.
pub fn foe(entry: HitTarget) -> MonId {
    match entry {
        HitTarget::Pokemon(m) => m,
        _ => panic!("foe list contains a non-Pokemon entry"),
    }
}

/// `this.add('-ability', pokemon, 'Name')` or, with `boost`, `... 'Name', 'boost'`.
pub fn add_ability<L: LogSink>(b: &mut Battle<L>, pokemon: MonId, ability: EffectId, boost: bool) {
    let name = LogArg::Effect(EffectRef::Dex(ability));
    if boost {
        b.add(LogEntry::new(
            "-ability",
            &[LogArg::Mon(pokemon), name, LogArg::Text("boost")],
            &[],
        ));
    } else {
        b.add(LogEntry::new(
            "-ability",
            &[LogArg::Mon(pokemon), name],
            &[],
        ));
    }
}

/// `this.add('-activate', pokemon, 'ability: Name')`.
pub fn add_activate<L: LogSink>(b: &mut Battle<L>, pokemon: MonId, ability: EffectId) {
    b.add(LogEntry::new(
        "-activate",
        &[
            LogArg::Mon(pokemon),
            LogArg::EffectFullName(EffectRef::Dex(ability)),
        ],
        &[],
    ));
}

/// `this.add('-immune', target)`.
pub fn add_immune<L: LogSink>(b: &mut Battle<L>, target: MonId) {
    b.add(LogEntry::new("-immune", &[LogArg::Mon(target)], &[]));
}

/// `this.add('-immune', target, '[from] ability: Name')`.
pub fn add_immune_from<L: LogSink>(b: &mut Battle<L>, target: MonId, ability: EffectId) {
    b.add(LogEntry::new(
        "-immune",
        &[LogArg::Mon(target)],
        &[LogTag::From(EffectRef::Dex(ability))],
    ));
}

// ---------------------------------------------------------------------------------
// Ruin abilities: `move.ruinedAtk/Def/SpA/SpD` hold the first holder that applied its
// 0.75 drop to this move use (ActiveMove::ruined_stats, MonId::NONE when absent).
// ---------------------------------------------------------------------------------
pub const RUIN_ATK: usize = 0;
pub const RUIN_DEF: usize = 1;
pub const RUIN_SPA: usize = 2;
pub const RUIN_SPD: usize = 3;

fn ruined<L: LogSink>(b: &Battle<L>, mv: u8, slot: usize) -> Option<MonId> {
    let m = move_overlay(b, mv).ruined_stats[slot];
    (m != MonId::NONE).then_some(m)
}

fn set_ruined<L: LogSink>(b: &mut Battle<L>, mv: u8, slot: usize, holder: MonId) {
    b.scratch.moves[mv as usize]
        .as_mut()
        .expect("released active move")
        .ruined_stats[slot] = holder;
}

/// Beads of Ruin `onAnyModifySpD` (data/abilities.ts:389) and Sword of Ruin
/// `onAnyModifyDef` (:4826): args `(stat, target, source, move)`, target is the
/// defender whose stat is being read.
/// ```text
/// const abilityHolder = this.effectState.target;
/// if (target.hasAbility('X of Ruin')) return;
/// if (!move.ruinedStat?.hasAbility('X of Ruin')) move.ruinedStat = abilityHolder;
/// if (move.ruinedStat !== abilityHolder) return;
/// return this.chainModify(0.75);
/// ```
pub fn ruin_defender<L: LogSink>(
    ability: EffectId,
    slot: usize,
    b: &mut Battle<L>,
    cx: HookCtx,
) -> Relay {
    let holder = owner(b, cx);
    let target = mon(b, cx, 1);
    if b.has_ability(target, &[ability]) {
        return Relay::Undefined;
    }
    let mv = crate::effects::support::move_arg(b, cx, 3);
    let current = ruined(b, mv, slot);
    if !current.is_some_and(|m| b.has_ability(m, &[ability])) {
        set_ruined(b, mv, slot, holder);
    }
    if ruined(b, mv, slot) != Some(holder) {
        return Relay::Undefined;
    }
    b.chain_modify(0.75, 1.0);
    Relay::Undefined
}

/// Tablets of Ruin `onAnyModifyAtk` (data/abilities.ts:4879) and Vessel of Ruin
/// `onAnyModifySpA` (:5292): args `(stat, source, target, move)`, source is the
/// attacker. Unlike the defender pair the stored holder is not re-validated:
/// `if (!move.ruinedAtk) move.ruinedAtk = abilityHolder;`.
pub fn ruin_attacker<L: LogSink>(
    ability: EffectId,
    slot: usize,
    b: &mut Battle<L>,
    cx: HookCtx,
) -> Relay {
    let holder = owner(b, cx);
    let source = mon(b, cx, 1);
    if b.has_ability(source, &[ability]) {
        return Relay::Undefined;
    }
    let mv = crate::effects::support::move_arg(b, cx, 3);
    if ruined(b, mv, slot).is_none() {
        set_ruined(b, mv, slot, holder);
    }
    if ruined(b, mv, slot) != Some(holder) {
        return Relay::Undefined;
    }
    b.chain_modify(0.75, 1.0);
    Relay::Undefined
}

/// `onStart(pokemon) { if (this.suppressingAbility(pokemon)) return;
/// this.add('-ability', pokemon, 'X of Ruin'); }` shared by the four Ruin abilities.
pub fn ruin_start<L: LogSink>(ability: EffectId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let pokemon = mon(b, cx, 0);
    if b.suppressing_ability(Some(pokemon)) {
        return Relay::Undefined;
    }
    add_ability(b, pokemon, ability, false);
    Relay::Undefined
}
