//! Ports data/items.ts:4775-4790 (Power Herb). PRNG: none directly (useItem events may tie-shuffle).
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{
        HookWaiver,
        support::{mon_arg, move_arg},
    },
    event::{EffectRef, HookCtx, Relay},
    ids::EffectId,
    log::{LogArg, LogEntry, LogSink, MoveLineEdit},
};
pub const ID: EffectId = dex::ITEM_POWERHERB;
pub const HOOKS: &[HookId] = &[dex::HOOK_ITEM_POWERHERB_ONCHARGEMOVE];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/items.ts:4776-4783 onChargeMove(pokemon, target, move):
        //   if (pokemon.useItem()) {
        //     this.debug(...); this.attrLastMove('[still]');
        //     this.addMove('-anim', pokemon, move.name, target);
        //     return false;                                   // skip the charge turn
        //   }
        // `useItem()` takes no arguments (source/sourceEffect default inside the mutator).
        // runEvent('ChargeMove', attacker, defender, move) supplies (pokemon, target, move).
        // PRNG: none directly.
        dex::HOOK_ITEM_POWERHERB_ONCHARGEMOVE => {
            let pokemon = mon_arg(b, cx, 0);
            let target = mon_arg(b, cx, 1);
            let mv = move_arg(b, cx, 2);
            if b.use_item(pokemon, Attribution::DEFAULT) {
                b.attr_last_move(MoveLineEdit::Still);
                b.add_move(LogEntry::new(
                    "-anim",
                    &[
                        LogArg::Mon(pokemon),
                        LogArg::Effect(EffectRef::ActiveMove(mv)),
                        LogArg::Mon(target),
                    ],
                    &[],
                ));
                return Relay::FAIL;
            }
            Relay::Undefined
        }
        _ => panic!("unexpected Power Herb hook"),
    }
}
