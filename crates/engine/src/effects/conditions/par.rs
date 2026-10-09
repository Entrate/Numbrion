//! data/conditions.ts:24-54. BeforeMove draws randomChance(1,4) exactly once; other hooks none.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink},
    state::Status,
};
pub const ID: EffectId = dex::CONDITION_PAR;
pub const HOOKS: &[HookId] = &[HookId(734), HookId(735), HookId(736)];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const ONSTART: HookId = HookId(734);
const ONMODIFYSPE: HookId = HookId(735);
const ONBEFOREMOVE: HookId = HookId(736);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        ONSTART => {
            log_status(b, cx, Status::Paralysis, "-status");
            Relay::Undefined
        }
        ONMODIFYSPE => {
            let m = mon_arg(b, cx, 1);
            let spe = b.final_modify(relay_number(b, cx, 0));
            const QUICKFEET: EffectId = optional_id(dex::ABILITIES_DATA, "quickfeet");
            Relay::Number(if has_ability(b, m, QUICKFEET) {
                spe
            } else {
                (spe * 0.5).floor()
            })
        }
        ONBEFOREMOVE => {
            let m = mon_arg(b, cx, 0);
            if b.state.prng.random_chance(1, 4) {
                b.add(LogEntry::new(
                    "cant",
                    &[LogArg::Mon(m), LogArg::Text("par")],
                    &[],
                ));
                Relay::FAIL
            } else {
                Relay::Undefined
            }
        }
        _ => unreachable!(),
    }
}
