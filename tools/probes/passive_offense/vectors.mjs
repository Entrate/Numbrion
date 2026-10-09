import fs from 'node:fs';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
import {loadSim} from '../../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
const {Dex}=createRequire(sim.root+'/package.json')('./dist/sim');
const groups=JSON.parse(fs.readFileSync(new URL('../../../docs/design/EFFECT-BATCHES.json',import.meta.url))).groups;
const oracle=new sim.Battle({formatid:"gen9randomdoublesbattle",seed:"1,2,3,4"});
const names=groups.find(g=>g.batch==='passive_offense').keys.map(k=>k.split(':')[1]);
let out='# ability\thook\tmoveType\tflags\thp\tactiveTurns\tbasePower\tmodifier\trecoil\tcrash\tforceSTAB\texpectedModifier\texpectedRelay\tstatus\tcategory\n';
for(const name of names){const ability=Dex.abilities.get(name);
 for(const[key,fn]of Object.entries(ability)){
  if(typeof fn!=='function'||key==='onModifyMove')continue;
  for(const type of ['Normal','Fire','Grass','Bug','Water','Dragon','Rock','Electric']){
   for(const variant of [0,1,2,3,4,5]){
    const move={type,category:variant===4?'Special':'Physical',flags:variant===0?{}:{punch:1,pulse:1,bite:1,slicing:1,contact:1},forceSTAB:true,recoil:variant===2?[1,4]:undefined,hasCrashDamage:variant===1};
    const attacker={hp:variant===0?41:40,maxhp:120,status:variant===0?'':variant===3?'brn':variant===4?'tox':'psn',hasType:()=>{throw Error('forceSTAB should short-circuit')}};
    const target={activeTurns:variant===1?0:1};const power=variant===5?2:variant===2?61:variant===4?41:40,initial=variant>=2&&variant<=4?6144:4096;
    const ctx={event:{modifier:initial/4096},trunc:oracle.trunc,chainModify:sim.Battle.prototype.chainModify,modify:sim.Battle.prototype.modify,debug(){}};
    const relay=fn.call(ctx,power,attacker,target,move);
    out+=[name,key,type,variant===0?0:1,attacker.hp,target.activeTurns,power,initial,!!move.recoil?1:0,move.hasCrashDamage?1:0,1,Math.round(ctx.event.modifier*4096),relay===undefined?'undefined':relay,attacker.status||'none',move.category].join('\t')+'\n';
   }
  }
 }
}
const dest=fileURLToPath(new URL('../../../crates/engine/src/effects/abilities/passiveoffense/vectors.tsv',import.meta.url));
fs.writeFileSync(dest,out);console.log('Wrote '+(out.trim().split('\n').length-1)+' pinned oracle callback vectors');
