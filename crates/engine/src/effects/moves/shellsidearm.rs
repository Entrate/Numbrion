//! Ports data/moves.ts:16214-16254 (Shell Side Arm): the animation/category reveal hints and the
//! `onModifyMove` category choice, which draws one `randomChance(1, 2)` only on an exact
//! physical/special estimate tie. The 20% poison secondary is declarative data in the core.
use crate::{
    Battle,
    actions::{MoveHandle, Stat, StatOptions},
    dex::{self, Category, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{HookCtx, Relay},
    ids::*,
    log::{LogArg, LogSink, LogTag, MoveLineEdit},
};
#[path = "movecallbacks/mod.rs"]
mod support;
pub const ID: EffectId = dex::MOVE_SHELLSIDEARM;
pub const HOOKS: &[HookId] = &[
    dex::HOOK_MOVE_SHELLSIDEARM_ONPREPAREHIT,
    dex::HOOK_MOVE_SHELLSIDEARM_ONMODIFYMOVE,
    dex::HOOK_MOVE_SHELLSIDEARM_ONHIT,
    dex::HOOK_MOVE_SHELLSIDEARM_ONAFTERSUBDAMAGE,
];
// Payload: none.
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_SHELLSIDEARM_ONPREPAREHIT => on_prepare_hit(b, cx),
        dex::HOOK_MOVE_SHELLSIDEARM_ONMODIFYMOVE => on_modify_move(b, cx),
        dex::HOOK_MOVE_SHELLSIDEARM_ONHIT => on_hit(b, cx),
        dex::HOOK_MOVE_SHELLSIDEARM_ONAFTERSUBDAMAGE => on_after_sub_damage(b, cx),
        _ => panic!("unexpected Shell Side Arm function site"),
    }
}

fn category_name(category: Category) -> &'static str {
    match category {
        Category::Physical => "Physical",
        Category::Special => "Special",
        Category::Status => "Status",
    }
}

// data/moves.ts:16223-16227 onPrepareHit(target, source, move): singleEvent('PrepareHit', move,
// {}, targets[0], pokemon, move) (battle-actions.ts:587), args [target, source, move]. The
// category here is the one chosen by onModifyMove, which runs earlier (battle-actions.ts:431).
//   if (!source.isAlly(target)) this.attrLastMove('[anim] Shell Side Arm ' + move.category);
// Returns undefined. PRNG: none.
fn on_prepare_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = support::opt_mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let handle = MoveHandle(move_arg(b, cx, 2));
    if !b.is_ally(source, target) {
        let category = category_name(b.active_move(handle).category);
        let parts = [LogArg::Text("Shell Side Arm "), LogArg::Text(category)];
        b.attr_last_move(MoveLineEdit::Tag(LogTag::Value(
            "anim",
            LogArg::Parts(&parts),
        )));
    }
    Relay::Undefined
}

// data/moves.ts:16228-16240 onModifyMove(move, pokemon, target): singleEvent('ModifyMove', move,
// null, pokemon, target, move, move) (battle-actions.ts:431), args [move, pokemon, target].
//   if (!target) return;
//   atk/spa = pokemon.getStat('atk'|'spa', false, true); def/spd = target.getStat('def'|'spd', false, true);
//   physical = floor(floor(floor(floor(2 * pokemon.level / 5 + 2) * 90 * atk) / def) / 50);
//   special  = the same with spa / spd;
//   if (physical > special || (physical === special && this.randomChance(1, 2))) {
//     move.category = 'Physical'; move.flags.contact = 1;
//   }
// getStat order is atk, spa, def, spd (each runs ModifyBoost events; unmodified skips stat
// modifier events). PRNG: one random(2) draw only when physical === special.
fn on_modify_move<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let handle = MoveHandle(move_arg(b, cx, 0));
    let pokemon = mon_arg(b, cx, 1);
    let Some(target) = support::opt_mon_arg(b, cx, 2) else {
        return Relay::Undefined;
    };
    let options = StatOptions {
        unboosted: false,
        unmodified: true,
    };
    let atk = b.get_stat(pokemon, Stat::Atk, options);
    let spa = b.get_stat(pokemon, Stat::SpA, options);
    let def = b.get_stat(target, Stat::Def, options);
    let spd = b.get_stat(target, Stat::SpD, options);
    let level =
        f64::from(b.teams().sides[pokemon.side().0 as usize].sets[(pokemon.0 % 6) as usize].level);
    let level_term = (2.0 * level / 5.0 + 2.0).floor();
    let physical = damage_estimate(level_term, atk, def);
    let special = damage_estimate(level_term, spa, spd);
    if physical > special || (physical == special && b.state.prng.random_chance(1, 2)) {
        let m = b.active_move_mut(handle);
        m.category = Category::Physical;
        m.flags |= dex::FLAG_CONTACT;
    }
    Relay::Undefined
}

/// `Math.floor(Math.floor(Math.floor(levelTerm * 90 * attack) / defense) / 50)` with
/// `levelTerm = Math.floor(2 * pokemon.level / 5 + 2)` (data/moves.ts:16234-16235).
fn damage_estimate(level_term: f64, attack: f64, defense: f64) -> f64 {
    (((level_term * 90.0 * attack).floor() / defense).floor() / 50.0).floor()
}

// data/moves.ts:16241-16244 onHit(target, source, move): Hit single event, args
// [target, source, move].
//   if (!source.isAlly(target)) this.hint(move.category + " Shell Side Arm");
// Returns undefined. PRNG: none.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let handle = MoveHandle(move_arg(b, cx, 2));
    reveal_category(b, source, Some(target), handle);
    Relay::Undefined
}

// data/moves.ts:16245-16247 onAfterSubDamage(damage, target, source, move): substitute's
// onTryPrimaryHit runs singleEvent('AfterSubDamage', move, null, target, source, move, damage)
// (data/moves.ts:18368), args [damage, target, source, move].
//   if (!source.isAlly(target)) this.hint(move.category + " Shell Side Arm");
// Returns undefined. PRNG: none.
fn on_after_sub_damage<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 1);
    let source = mon_arg(b, cx, 2);
    let handle = MoveHandle(move_arg(b, cx, 3));
    reveal_category(b, source, Some(target), handle);
    Relay::Undefined
}

/// `if (!source.isAlly(target)) this.hint(move.category + " Shell Side Arm")`.
fn reveal_category<L: LogSink>(
    b: &mut Battle<L>,
    source: MonId,
    target: Option<MonId>,
    handle: MoveHandle,
) {
    if !b.is_ally(source, target) {
        let category = category_name(b.active_move(handle).category);
        let parts = [LogArg::Text(category), LogArg::Text(" Shell Side Arm")];
        b.hint(LogArg::Parts(&parts), false, None);
    }
}
