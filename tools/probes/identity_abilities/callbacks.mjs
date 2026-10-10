import fs from 'node:fs';
import {loadSim} from '../../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
const dex=new sim.Battle({formatid:'gen9randomdoublesbattle',seed:'1,2,3,4'}).dex;
const out=[];
for(const tera of ['', 'Fire'])for(const pos of [0,1,2,3])for(const last of ['Pikachu','Ogerpon-Wellspring','Terapagos-Terastal'])for(const fainted of [false,true]) {
 const party=['Zoroark','Pikachu','Pikachu',last].map((s,i)=>({species:dex.species.get(s),fainted:i===3&&fainted}));
 const p=party[pos];Object.assign(p,{side:{pokemon:party},position:pos,terastallized:tera,illusion:{}});
 dex.abilities.get('illusion').onBeforeSwitchIn.call({},p);
 out.push([tera,pos,last,+fainted,p.illusion?party.indexOf(p.illusion):-1]);
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/identityabilities/illusion.tsv',import.meta.url),out.map(r=>r.join('\t')).join('\n')+'\n');
const trace=[];
for(const first of ['chlorophyll','asoneglastrier'])for(const second of ['swiftswim','asonespectrier'])for(const seek of [false,true]) {
 const prng=new sim.PRNG('1,2,3,4');let copied='trace';let source=-1;
 const foes=[first,second].map((id,i)=>({getAbility:()=>dex.abilities.get(id),ability:id,index:i}));
 const cx={effectState:{seek},sample:items=>prng.sample(items)};
 const p={adjacentFoes:()=>foes,setAbility:(a,t)=>{copied=a.id;source=t.index;}};
 const r=dex.abilities.get('trace').onUpdate.call(cx,p);
 if(r!==undefined)throw Error('Trace sentinel changed');
 trace.push([first,second,+seek,copied,source,prng.getSeed()]);
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/identityabilities/trace.tsv',import.meta.url),trace.map(r=>r.join('\t')).join('\n')+'\n');
const asone=[];
for(const key of ['asoneglastrier','asonespectrier']) {
 const state={},logs=[];let amount='';
 const cx={effectState:state,dex,add:(...a)=>logs.push(a.join('|')),boost:(obj,target,source,effect)=>{amount=Object.entries(obj).map(x=>x.join(':')).join(',')+'|'+effect.id;}};
 const a=dex.abilities.get(key);
 const initial=a.onFoeTryEatItem.call(cx);a.onStart.call(cx,'m0');a.onStart.call(cx,'m0');
 const active=a.onFoeTryEatItem.call(cx);
 a.onSourceAfterFaint.call(cx,2,null,'m0',dex.moves.get('thunderbolt'));a.onEnd.call(cx);
 asone.push([key,+initial,+active,+a.onFoeTryEatItem.call(cx),amount,logs.join('\\n')]);
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/identityabilities/asone.tsv',import.meta.url),asone.map(r=>r.join('\t')).join('\n')+'\n');
console.log(`Wrote ${out.length} Illusion, ${trace.length} Trace and ${asone.length} As One rows.`);
