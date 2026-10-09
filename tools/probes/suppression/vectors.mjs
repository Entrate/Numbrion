import fs from 'node:fs';
import {createRequire} from 'node:module';
import {loadSim} from '../../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
const {Dex,Pokemon,Field}=createRequire(sim.root+'/package.json')('./dist/sim');
const oracle=new sim.Battle({formatid:'gen9randomdoublesbattle',seed:'1,2,3,4'});
const events=['Start','End','SwitchIn','TakeItem','SetAbility','Update','BasePower','Weather','FieldStart','FieldResidual','FieldEnd','Residual','AfterSetStatus'];
const effects=[Dex.abilities.get('furcoat'),Dex.abilities.get('fullmetalbody'),Dex.items.get('lifeorb'),Dex.conditions.get('brn'),Dex.conditions.get('snowscape'),Dex.formats.get('hppercentagemod')];
let lines=['# kind\tkey\tevent\tactive\tuser\tignore\tstatusMatch\tweather\tsingle\trun'];
for(const effect of effects)for(const event of events)for(const active of [0,1])for(const userMode of [0,1,2,3])for(const ignore of [0,1])for(const statusMatch of [0,1])for(const weather of [0,1,2,3,4]){
 const target=Object.assign(Object.create(Pokemon.prototype),{isActive:!!active,status:statusMatch?effect.id:'',hasItem:()=>false,ignoringItem:()=>!active,ignoringAbility:()=>!active});
 const other={isActive:userMode===1};let user=userMode===0?null:userMode===2?target:other;
 const carrier={fainted:weather===3,ignoringAbility:()=>weather===4,getAbility:()=>Dex.abilities.get('cloudnine'),abilityState:{ending:weather===2}};
 const field={battle:{sides:[{active:weather===0?[]:[carrier]},{active:[]}]},suppressingWeather:Object.getPrototypeOf(oracle.field).suppressingWeather};
 const ctx={log:[],sentLogPos:0,gen:9,eventDepth:0,event:null,effect:null,effectState:null,activePokemon:user,activeMove:ignore?{ignoreAbility:true}:null,suppressingAbility:sim.Battle.prototype.suppressingAbility,field,debug(){},trunc:oracle.trunc,modify:sim.Battle.prototype.modify};
 const callback=()=>18;
 const single=sim.Battle.prototype.singleEvent.call(ctx,event,effect,{},target,null,null,17,callback)===17;
 ctx.findEventHandlers=()=>[{effect,effectHolder:target,callback,state:{}}];ctx.speedSort=()=>{};
 const run=sim.Battle.prototype.runEvent.call(ctx,event,target,null,null,17)===17;
 lines.push([effect.effectType,effect.id,event,active,userMode,ignore,statusMatch,weather,+single,+run].join('\t'));
}
fs.writeFileSync(new URL('../../../crates/engine/src/event/suppression_vectors.tsv',import.meta.url),lines.join('\n')+'\n');console.log(`${lines.length-1} dispatch-time suppression vectors`);
