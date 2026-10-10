//! data/rulesets.ts:1379-1403. No direct PRNG draws.
use crate::{Battle, dex::{self, HookId}, effects::{HookWaiver, support::mon_arg},
    event::{EventArg, HookCtx, Relay}, ids::*, log::{LogArg, LogEntry, LogSink}, state::{Status, CellId}};
pub const ID: EffectId = dex::RULE_SLEEPCLAUSEMOD;
pub const HOOKS: &[HookId] = &[dex::HOOK_RULE_SLEEPCLAUSEMOD_ONBEGIN, dex::HOOK_RULE_SLEEPCLAUSEMOD_ONSETSTATUS];
pub const PAYLOAD_WORDS: usize = 0;
pub const WAIVERS: &[HookWaiver] = &[];
pub fn dispatch<L: LogSink>(hook: HookId, b: &mut Battle<L>, cx: HookCtx) -> Relay {
    match hook {
        dex::HOOK_RULE_SLEEPCLAUSEMOD_ONBEGIN => begin(b),
        dex::HOOK_RULE_SLEEPCLAUSEMOD_ONSETSTATUS => set_status(b,cx),
        _ => panic!("unexpected Sleep Clause hook"),
    }
}
pub(super) fn begin<L:LogSink>(b:&mut Battle<L>) -> Relay {
    b.add(LogEntry::new("rule", &[LogArg::Text("Sleep Clause Mod: Limit one foe put to sleep")], &[]));
    Relay::Undefined
}
pub(super) fn set_status<L:LogSink>(b:&mut Battle<L>, cx:HookCtx) -> Relay {
    let target = mon_arg(b,cx,1);
    let source = Battle::<L>::arg_mon(b.event_arg(cx,2));
    if source.is_some_and(|s| s.side()==target.side()) {return Relay::Undefined;}
    let status = match b.event_arg(cx,0) {
        EventArg::Effect(e) => b.event_effect_id(e),
        EventArg::Relay(Relay::Effect(id)) => id,
        _ => panic!("Sleep Clause expects status object relay"),
    };
    if status != dex::CONDITION_SLP {return Relay::Undefined;}
    let side = &b.state.sides[target.side().0 as usize];
    for mon in side.party {
        if mon == MonId::NONE {continue;}
        let p = &b.state.pokemon[mon.0 as usize];
        if p.hp == 0 || p.status != Status::Sleep {continue;}
        let source = if p.status_state == CellId::NONE {MonId::NONE} else {b.state.effects.cells[p.status_state.0 as usize].source};
        if source == MonId::NONE || source.side()!=mon.side() {
            b.add(LogEntry::new("-message", &[LogArg::Text("Sleep Clause Mod activated.")], &[]));
            b.hint(LogArg::Text("Sleep Clause Mod prevents players from putting more than one of their opponent's Pokémon to sleep at a time"), false, None);
            return Relay::Bool(false);
        }
    }
    Relay::Undefined
}
