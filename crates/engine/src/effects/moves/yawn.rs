//! data/moves.ts:21141-21145. Status/immunity checks are ordered and draw no RNG directly.
use crate::{Battle,actions::ImmunityMessage,dex::{self,HookId,ImmunityId},effects::{HookWaiver,support::mon_arg},event::{HookCtx,Relay},ids::EffectId,log::LogSink,state::Status};
pub const ID:EffectId=dex::MOVE_YAWN;
pub const HOOKS:&[HookId]=&[dex::HOOK_MOVE_YAWN_ONTRYHIT];
pub const PAYLOAD_WORDS:usize=0;
pub const WAIVERS:&[HookWaiver]=&[];
pub fn dispatch<L:LogSink>(hook:HookId,b:&mut Battle<L>,cx:HookCtx)->Relay {
    assert_eq!(hook,dex::HOOK_MOVE_YAWN_ONTRYHIT);
    let target=mon_arg(b,cx,0);
    if b.state.pokemon[target.0 as usize].status!=Status::None || !b.run_status_immunity(target,ImmunityId::Sleep,ImmunityMessage::Silent) {Relay::Bool(false)} else {Relay::Undefined}
}
