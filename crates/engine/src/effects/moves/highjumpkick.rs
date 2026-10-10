//! Ports data/moves.ts:8898-8915 (High Jump Kick; `hasCrashDamage` is declarative data). PRNG: none
//! directly (the nested damage event may speed-sort ties).
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::LogSink,
};
pub const ID: EffectId = dex::MOVE_HIGHJUMPKICK;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_HIGHJUMPKICK_ONMOVEFAIL];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:8908-8910 onMoveFail(target, source, move):
        //   this.damage(source.baseMaxhp / 2, source, source, this.dex.conditions.get('High Jump Kick'))
        // `conditions.get('High Jump Kick')` is the bare, nonexistent Condition named 'highjumpkick'
        // (the move has no `condition`), so the log reads `[from] highjumpkick`. baseMaxhp equals
        // maxhp in this format (no Dynamax) and the quotient stays fractional until damage truncates.
        // MoveFail is a singleEvent whose arguments are (target, source, move).
        dex::HOOK_MOVE_HIGHJUMPKICK_ONMOVEFAIL => {
            let source = mon_arg(b, cx, 1);
            let amount = f64::from(b.state.pokemon[source.0 as usize].max_hp) / 2.0;
            b.damage(
                amount,
                Some(source),
                Attribution::from_move(source, EffectRef::Dex(dex::CONDITION_HIGHJUMPKICK)),
            );
            Relay::Undefined
        }
        _ => panic!("unexpected High Jump Kick hook"),
    }
}
