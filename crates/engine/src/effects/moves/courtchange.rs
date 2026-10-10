//! Ports data/moves.ts:3041-3095 (Court Change onHitField).
//! Payload: none. No PRNG draws and no events fire while the states move.
//!
//! The free-for-all branch (data/moves.ts:3046-3065, `this.gameType === 'freeforall'`) is outside
//! gen9randomdoublesbattle (gameType `doubles`, battle.ts:218), so only the two-side swap is ported.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::MOVE_COURTCHANGE;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_COURTCHANGE_ONHITFIELD];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:3041 onHitField(target, source): singleEvent('HitField', moveData, {}, target,
        // source, move) (battle-actions.ts:1258) => args [target, source, move]. Moves every allow-listed
        // side-condition state between the source's side and its foe (see support::swap_side_conditions
        // for ordering/target rewrite), failing with `false` when neither side holds one. On success
        // logs `-swapsideconditions` then `-activate|source|move: Court Change`; returns undefined.
        // PRNG: none.
        dex::HOOK_MOVE_COURTCHANGE_ONHITFIELD => {
            let source = mon_arg(b, cx, 1);
            if !support::swap_side_conditions(b, source.side()) {
                return Relay::Bool(false);
            }
            b.add(LogEntry::new("-swapsideconditions", &[], &[]));
            b.add(LogEntry::new(
                "-activate",
                &[LogArg::Mon(source), LogArg::Text("move: Court Change")],
                &[],
            ));
            Relay::Undefined
        }
        _ => panic!("unexpected Court Change function site"),
    }
}
