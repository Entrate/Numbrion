// Callback branch expectations; mutate test state only, never the oracle source.
import fs from 'node:fs';
import {loadSim} from '../../oracle/lib/common.mjs';
import {Session} from '../../oracle/lib/session.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
const packed='Mew|||earlybird|irondefense|Serious||N|||100|,,,,,Normal';
const cases=[
 ['incinerate_berry','incinerate','sitrusberry','','earlybird',false,''],
 ['incinerate_nonberry','incinerate','heavydutyboots','','earlybird',false,''],
 ['incinerate_stickyhold','incinerate','sitrusberry','','stickyhold',false,''],
 ['bugbite_nonberry','bugbite','heavydutyboots','','earlybird',false,''],
 ['bugbite_fainted_source','bugbite','sitrusberry','','earlybird',true,''],
 ['bugbite_stickyhold','bugbite','sitrusberry','','stickyhold',false,''],
 ['recycle_restore','recycle','','','earlybird',false,'sitrusberry'],
 ['recycle_occupied','recycle','','heavydutyboots','earlybird',false,'sitrusberry'],
];
let out='# name\tmove\ttarget item\tsource item\ttarget ability\tfainted source\tlast item\tresult\ttarget item after\tsource item after\tlast item after\tseed\tlog\n';
for(const c of cases){
 const session=new Session(sim,{seed:[777,888,999,111],teams:[`${packed}]${packed}`,`${packed}]${packed}`]});
 const b=session.battle,s=b.sides[0].active[0],t=c[1]==='recycle'?s:b.sides[1].active[0];
 t.item=c[2]; s.item=c[3]; t.ability=c[4]; s.lastItem=c[6]; if(c[5])s.hp=0;
 const move=b.dex.getActiveMove(c[1]); b.setActiveMove(move,s,t); const start=b.log.length;
 const result=b.singleEvent('Hit',move,null,t,s,move);
 out+=[...c,JSON.stringify(result),t.item,s.item,s.lastItem,session.currentSeed().join(','),b.log.slice(start).join('\\n')].join('\t')+'\n';
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/moves/batonpass/callbacks.tsv',import.meta.url),out);
console.log(`Wrote ${cases.length} item branch expectations.`);
