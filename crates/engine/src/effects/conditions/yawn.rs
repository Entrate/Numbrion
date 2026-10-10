//! data/moves.ts:21150-21156. Expiry adds sleep; the Sleep onStart draws random(2,5).
use crate::{Battle,actions::Attribution,dex::{self,HookId},effects::{HookWaiver,support::mon_arg},event::{EffectRef,EventArg,HookCtx,Relay},ids::*,log::{LogArg,LogEntry,LogSink,LogTag}};
pub const ID:EffectId=dex::CONDITION_YAWN;
pub const HOOKS:&[HookId]=&[dex::HOOK_CONDITION_YAWN_ONSTART,dex::HOOK_CONDITION_YAWN_ONEND];
pub const PAYLOAD_WORDS:usize=0;
pub const WAIVERS:&[HookWaiver]=&[];
pub fn dispatch<L:LogSink>(hook:HookId,b:&mut Battle<L>,cx:HookCtx)->Relay {
    let target=mon_arg(b,cx,0);
    match hook {
        dex::HOOK_CONDITION_YAWN_ONSTART=> {
            let source=Battle::<L>::arg_mon(b.event_arg(cx,1));
            let tag=source.map_or(LogTag::Value("of",LogArg::Text("undefined")),LogTag::Of);
            b.add(LogEntry::new("-start",&[LogArg::Mon(target),LogArg::Text("move: Yawn")],&[tag]));
        }
        dex::HOOK_CONDITION_YAWN_ONEND=> {
            b.add(LogEntry::new("-end",&[LogArg::Mon(target),LogArg::Text("move: Yawn")],&[LogTag::Bare("silent")]));
            let source=b.hook_state(cx).source;
            let source=if source==MonId::NONE {EventArg::Undefined} else {EventArg::Holder(Holder::mon(source))};
            b.try_set_status(target,dex::CONDITION_SLP,Attribution{source,effect:EffectRef::None});
        }
        _=>panic!("unexpected Yawn hook"),
    }
    Relay::Undefined
}
