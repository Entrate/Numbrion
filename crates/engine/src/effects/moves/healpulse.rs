//! Ports data/moves.ts:8399 (Heal Pulse). No direct PRNG draws; `heal` runs TryHeal/Heal
//! events whose listeners may speed-sort ties.
use crate::effects::registry::abilities_baddreams::support::base_max_hp;
use crate::{
    Battle,
    actions::HealEffect,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::MOVE_HEALPULSE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_HEALPULSE_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_MOVE_HEALPULSE_ONHIT => on_hit(b, cx),
        _ => panic!("unexpected healpulse hook"),
    }
}

// data/moves.ts:8408-8422 onHit(target, source). PRNG: none directly.
fn on_hit<L: LogSink>(b: &mut Battle<L>, cx: HookCtx) -> Relay {
    let target = mon_arg(b, cx, 0);
    let source = mon_arg(b, cx, 1);
    let base_max_hp = base_max_hp(b, target);
    // if (source.hasAbility('megalauncher')) success = !!this.heal(this.modify(target.baseMaxhp, 0.75));
    // else success = !!this.heal(Math.ceil(target.baseMaxhp * 0.5));   (effect defaults to the move)
    let success = if b.has_ability(source, &[dex::ABILITY_MEGALAUNCHER]) {
        let amount = b.modify(base_max_hp, 0.75, 1.0);
        b.heal(amount, None, None, HealEffect::Context).truthy()
    } else {
        let amount = (base_max_hp * 0.5).ceil();
        b.heal(amount, None, None, HealEffect::Context).truthy()
    };
    // if (success && !target.isAlly(source)) target.staleness = 'external';
    // Staleness only feeds the Endless Battle Clause (battle.ts:1767), which this format's
    // ruleset does not include, so it is unobservable and not part of the snapshot.
    // if (!success) { this.add('-fail', target, 'heal'); return this.NOT_FAIL; }
    if !success {
        b.add(LogEntry::new(
            "-fail",
            &[LogArg::Mon(target), LogArg::Text("heal")],
            &[],
        ));
        return Relay::NotFail;
    }
    // return success;
    Relay::Bool(true)
}
