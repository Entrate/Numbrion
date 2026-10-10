//! data/moves.ts:14966-14991. Sleep installation draws once before overriding its timer to 3.
use crate::{Battle,actions::{Attribution, HealEffect},dex::{self,HookId},effects::{HookWaiver,support::mon_arg},event::{EffectRef,HookCtx,Relay},ids::EffectId,log::{LogArg,LogEntry,LogSink,LogTag},state::Status};
pub const ID:EffectId=dex::MOVE_REST;
pub const HOOKS:&[HookId]=&[dex::HOOK_MOVE_REST_ONTRY,dex::HOOK_MOVE_REST_ONHIT];
pub const PAYLOAD_WORDS:usize=0;
pub const WAIVERS:&[HookWaiver]=&[];
pub fn dispatch<L:LogSink>(hook:HookId,b:&mut Battle<L>,cx:HookCtx)->Relay {
    let target=mon_arg(b,cx,0);
    match hook {
        dex::HOOK_MOVE_REST_ONTRY=>{
            if b.state.pokemon[target.0 as usize].status==Status::Sleep || b.has_ability(target,&[dex::ABILITY_COMATOSE]) {return Relay::Bool(false);}
            let p=&b.state.pokemon[target.0 as usize];
            if p.hp==p.max_hp {b.add(LogEntry::new("-fail",&[LogArg::Mon(target),LogArg::Text("heal")],&[]));return Relay::Null;}
            for ability in [dex::ABILITY_INSOMNIA,dex::ABILITY_VITALSPIRIT] {
                if b.has_ability(target,&[ability]) {b.add(LogEntry::new("-fail",&[LogArg::Mon(target)],&[LogTag::From(EffectRef::Dex(ability)),LogTag::Of(target)]));return Relay::Null;}
            }
            Relay::Undefined
        }
        dex::HOOK_MOVE_REST_ONHIT=>{
            let source=mon_arg(b,cx,1);
            let effect=match b.event_arg(cx,2) {
                crate::event::EventArg::Move(h)|crate::event::EventArg::Relay(Relay::ActiveMove(h))=>EffectRef::ActiveMove(h),
                crate::event::EventArg::Effect(e)=>e,
                _=>EffectRef::Dex(ID),
            };
            let result=b.set_status(target,dex::CONDITION_SLP,Attribution::from_move(source,effect),false);
            if !result.truthy(){return result;}
            let cell=b.state.pokemon[target.0 as usize].status_state;
            let c=&mut b.state.effects.cells[cell.0 as usize];
            c.payload.words[0]=3;c.payload.words[1]=3;c.present|=(1<<8)|(1<<9);
            b.heal(f64::from(b.state.pokemon[target.0 as usize].max_hp),None,None,HealEffect::Context);
            Relay::Undefined
        }
        _=>panic!("unexpected Rest hook"),
    }
}
