import fs from 'node:fs';import{createRequire}from'node:module';import{loadSim}from'../../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);const {Pokemon,Side}=createRequire(sim.root+'/package.json')('./dist/sim');
let lines=['# mask\tuser\tmethod\tids'];
for(let mask=0;mask<256;mask++){
 const sides=[0,1].map(n=>Object.assign(Object.create(Side.prototype),{n,battle:{gameType:'doubles',activePerHalf:2},active:[]}));sides[0].foe=sides[1];sides[1].foe=sides[0];
 const mons=[0,1,6,7].map((id,i)=>Object.assign(Object.create(Pokemon.prototype),{id,side:sides[id<6?0:1],battle:{activePerHalf:2},hp:mask&(1<<i)?100:0,fainted:!!(mask&(1<<(i+4)))}));
 sides[0].active=[mons[0],mask%3===0?null:mons[1]];sides[1].active=[mons[2],mask%5===0?null:mons[3]];
 for(const user of [0,6]){const mon=mons[user===0?0:2];for(const name of ['alliesAndSelf','allies','adjacentAllies','foes','foesAll','adjacentFoes']){const result=name==='foesAll'?mon.foes(true):mon[name]();lines.push([mask,user,name,result.map(p=>p.id).join(',')].join('\t'));}}
}
fs.writeFileSync(new URL('../../../crates/engine/src/actions/moves/targetqueries/vectors.tsv',import.meta.url),lines.join('\n')+'\n');console.log(`${lines.length-1} ally/foe query vectors`);
