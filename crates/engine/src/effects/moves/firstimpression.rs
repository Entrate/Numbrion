//! Ports data/moves.ts:5473-5491 (First Impression). PRNG: none.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogSink},
};
pub const ID: EffectId = dex::MOVE_FIRSTIMPRESSION;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_FIRSTIMPRESSION_ONTRY];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:5482-5487 onTry(source): `source.activeMoveActions > 1` -> hint, return
        // false. PRNG: none.
        dex::HOOK_MOVE_FIRSTIMPRESSION_ONTRY => {
            let source = mon_arg(b, cx, 0);
            if b.state.pokemon[source.0 as usize].active_move_actions > 1 {
                b.hint(
                    LogArg::Text("First Impression only works on your first turn out."),
                    false,
                    None,
                );
                return Relay::FAIL;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected First Impression hook"),
    }
}
