//! data/conditions.ts:91-134. BeforeMove draws randomChance(1,5) once unless defrost; other hooks no draws.
use crate::{
    Battle,
    dex::{self, HookId},
    effects::{HookWaiver, support::*},
    event::*,
    ids::*,
    log::{LogArg, LogEntry, LogSink},
    state::Status,
};
pub const ID: EffectId = dex::CONDITION_FRZ;
pub const HOOKS: &[HookId] = &[
    HookId(686),
    HookId(687),
    HookId(688),
    HookId(689),
    HookId(690),
];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const ONSTART: HookId = HookId(686);
const ONBEFOREMOVE: HookId = HookId(687);
const ONMODIFYMOVE: HookId = HookId(688);
const ONAFTERMOVESECONDARY: HookId = HookId(689);
const ONDAMAGINGHIT: HookId = HookId(690);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        ONSTART => {
            log_status(b, cx, Status::Freeze, "-status");
            let m = mon_arg(b, cx, 0);
            let p = b.state.pokemon[m.0 as usize];
            if p.species == dex::SPECIES_SHAYMINSKY
                && dex::species(p.base_species).base_species == dex::SPECIES_SHAYMIN
            {
                b.forme_change(
                    m,
                    dex::SPECIES_SHAYMIN,
                    b.scratch.current_effect,
                    crate::actions::FormeOptions {
                        permanent: true,
                        message: None,
                    },
                );
            }
        }
        ONBEFOREMOVE => {
            let m = mon_arg(b, cx, 0);
            let mov = move_arg(b, cx, 2); /* Burn Up excluded from fixed scope. */
            if move_overlay(b, mov).flags & dex::FLAG_DEFROST != 0 {
                return Relay::Undefined;
            }
            if b.state.prng.random_chance(1, 5) {
                b.cure_status(m, false);
                return Relay::Undefined;
            }
            b.add(LogEntry::new(
                "cant",
                &[LogArg::Mon(m), LogArg::Text("frz")],
                &[],
            ));
            return Relay::FAIL;
        }
        ONMODIFYMOVE => {
            let mov = move_arg(b, cx, 0);
            let m = mon_arg(b, cx, 1);
            if move_overlay(b, mov).flags & dex::FLAG_DEFROST != 0 {
                b.add(LogEntry::new(
                    "-curestatus",
                    &[LogArg::Mon(m), LogArg::Text("frz")],
                    &[crate::log::LogTag::From(EffectRef::ActiveMove(mov))],
                ));
                b.clear_status(m);
            }
        }
        ONAFTERMOVESECONDARY => {
            let m = mon_arg(b, cx, 0);
            let mov = move_arg(b, cx, 2);
            if move_overlay(b, mov).traits & dex::MOVE_TRAIT_THAWSTARGET != 0 {
                b.cure_status(m, false);
            }
        }
        ONDAMAGINGHIT => {
            let m = mon_arg(b, cx, 1);
            let mov = move_arg(b, cx, 3);
            let mov = move_overlay(b, mov); /* Polar Flare excluded from fixed scope. */
            if mov.move_type == TypeId(10) && mov.category != dex::Category::Status {
                b.cure_status(m, false);
            }
        }
        _ => unreachable!(),
    }
    Relay::Undefined
}
