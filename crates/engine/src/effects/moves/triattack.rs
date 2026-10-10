//! data/moves.ts:19856 secondary.onHit. Always sample once (random(3)), even if status fails.
use crate::{Battle,actions::Attribution,dex::{self,HookId},effects::{HookWaiver,support::mon_arg},event::{EffectRef,HookCtx,Relay},ids::EffectId,log::LogSink};
pub const ID:EffectId=dex::MOVE_TRIATTACK;
pub const HOOKS:&[HookId]=&[dex::HOOK_MOVE_TRIATTACK_SECONDARY_ONHIT,dex::HOOK_MOVE_TRIATTACK_SECONDARIES_0_ONHIT];
pub const PAYLOAD_WORDS:usize=0;
pub const WAIVERS:&[HookWaiver]=&[];
pub fn dispatch<L:LogSink>(hook:HookId,b:&mut Battle<L>,cx:HookCtx)->Relay {
    assert!(HOOKS.contains(&hook));
    let target=mon_arg(b,cx,0);let source=mon_arg(b,cx,1);
    let status=[dex::CONDITION_BRN,dex::CONDITION_PAR,dex::CONDITION_FRZ][b.state.prng.random(3) as usize];
    b.try_set_status(target,status,Attribution::from_move(source,EffectRef::None));
    Relay::Undefined
}
