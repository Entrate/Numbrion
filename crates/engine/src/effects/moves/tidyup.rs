//! Ports data/moves.ts:19634-19656 (Tidy Up onHit).
//! Payload: none. No direct PRNG draws; Substitute End, SideEnd and boost events retain their own.
use crate::effects::registry::conditions_reflect::support;
use crate::{
    Battle,
    actions::Attribution,
    dex::{self, HookId},
    effects::{HookWaiver, support::mon_arg},
    event::{EffectRef, EventArg, HookCtx, Relay},
    ids::{EffectId, Holder},
    log::{LogArg, LogEntry, LogSink},
};
pub const ID: EffectId = dex::MOVE_TIDYUP;
pub const HOOKS: &[HookId] = &[dex::HOOK_MOVE_TIDYUP_ONHIT];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
const _: () = assert!(PAYLOAD_WORDS <= 4);
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        // data/moves.ts:19634 onHit(pokemon): singleEvent('Hit', moveData, {}, target, source, move)
        // => args [target(self), source, move]. In order:
        //  1. for every active Pokemon (getAllActive(): p1a,p1b,p2a,p2b, fainted skipped)
        //     removeVolatile('substitute') (its End logs `-end`); any removal sets success.
        //  2. for [own side, foe side] x [spikes, toxicspikes, stealthrock, stickyweb, (gmaxsteelsurge,
        //     never present here)]: removeSideCondition -> `-sideend|side|<Name>`, success = true.
        //  3. if success: `-activate|mon|move: Tidy Up`.
        //  4. returns `!!this.boost({atk: 1, spe: 1}, pokemon, pokemon, null, false, true) || success`
        //     (boost always runs; effect null defaults to the current effect). A real boolean.
        // PRNG: none directly.
        dex::HOOK_MOVE_TIDYUP_ONHIT => {
            let pokemon = mon_arg(b, cx, 0);
            let mut success = false;
            let actives = b.get_all_active(false);
            for &active in actives.as_slice() {
                if b.remove_volatile(active, dex::CONDITION_SUBSTITUTE) {
                    success = true;
                }
            }
            let own = pokemon.side();
            for side in [own, support::foe_of(own)] {
                for condition in support::TIDY_UP_HAZARDS {
                    if b.remove_side_condition(side, condition) {
                        b.add(LogEntry::new(
                            "-sideend",
                            &[
                                LogArg::Side(side),
                                LogArg::Effect(EffectRef::Dex(condition)),
                            ],
                            &[],
                        ));
                        success = true;
                    }
                }
            }
            if success {
                b.add(LogEntry::new(
                    "-activate",
                    &[LogArg::Mon(pokemon), LogArg::Text("move: Tidy Up")],
                    &[],
                ));
            }
            let boosted = b.boost(
                support::boosts_of(&[(support::STAT_ATK, 1), (support::STAT_SPE, 1)]),
                Some(pokemon),
                Attribution {
                    source: EventArg::Holder(Holder::mon(pokemon)),
                    effect: EffectRef::None,
                },
                false,
                true,
            );
            Relay::Bool(boosted.truthy() || success)
        }
        _ => panic!("unexpected Tidy Up function site"),
    }
}
