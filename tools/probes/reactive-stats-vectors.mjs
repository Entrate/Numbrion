import fs from 'node:fs';
import {createRequire} from 'node:module';
import {loadSim} from '../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||'/home/aminaliu/src/pokemon-showdown');
const {Dex}=createRequire(sim.root+'/package.json')('./dist/sim');
const batch=JSON.parse(fs.readFileSync(new URL('../../docs/design/EFFECT-BATCHES.json',import.meta.url))).groups.find(g=>g.batch==='reactive_stats');
const fields={source:[6,1,0,-1],owner:[0,1,6],hp:[40,0,60,61,70],type:['Fire','Water','Dark','Bug','Ghost','Normal'],physical:[1,0],multihit:[0,1],smart:[0,1],total:[100,0,10,20],last:[20,-1,0,100],checked:[-1,0,1],flags:[0,8,2048,8192,16384,24576],species:['greninjabond','greninjaash','pikachu'],turns:[1,0],contact:[1,0],suppress:[0,1],secondaries:[0,1],ability:[0,1],old:[-1,0,1,6],oldability:[0,1],foes:[2,0,1],def:[100,0,60],spd:[100,0,160],substitute:[0,1],tailwind:[0,1],boostresult:['true','false','0','null','undefined'],effecttype:['Move','Ability','missing'],effectstatus:[1,0],status:['brn','psn'],item:['sitrusberry','aguavberry','figyberry','iapapaberry','magoberry','wikiberry','leftovers'],length:[1,2,0],lowered:[-1,0,1],atkboost:[-1,0,1,99],wind:[1,0],shuriken:[1,0]};
const keys=Object.keys(fields),profiles=[];const base=keys.map(k=>fields[k][0]);profiles.push(base);for(let i=0;i<keys.length;i++)for(const value of fields[keys[i]]){const p=[...base];p[i]=value;profiles.push(p);}
// Deterministic fixture construction only: this is independent of battle PRNG.
let fixtureSeed=123456789;for(let n=0;n<1024;n++){profiles.push(keys.map(k=>{fixtureSeed=(Math.imul(fixtureSeed,1664525)+1013904223)>>>0;const a=fields[k];return a[(fixtureSeed>>>8)%a.length];}));}
const str=(v)=>v===undefined?'undefined':v===null?'null':String(v);const id=(v)=>v?.id??'undefined';let lines=['# effect\thook\tprofile\t'+keys.join('\t')+'\ttrace\treturn\tchecked\tflags0\tflags1\tflags6\told\tmultihit'];let count=0;
for(const key of batch.keys){const name=key.split(':')[1],effect=Dex.abilities.get(name);
for(const [hook,fn] of Object.entries(effect).filter(([k,v])=>typeof v==='function'))for(let pi=0;pi<profiles.length;pi++){
 const p=Object.fromEntries(keys.map((k,i)=>[k,profiles[pi][i]])),trace=[];
 const sides=[{id:0,sideConditions:p.tailwind?{tailwind:{}}:{},foePokemonLeft(){return p.foes?1:0;}},{id:1,sideConditions:p.tailwind?{tailwind:{}}:{},foePokemonLeft(){return p.foes?1:0;}}];
 const mons=new Map();function mon(i){if(i===-1)return undefined;if(mons.has(i))return mons.get(i);const m={id:i,side:sides[i<6?0:1],hp:p.hp,maxhp:120,transformed:!!(p.flags&8),bondTriggered:!!(p.flags&2048),swordBoost:!!(p.flags&8192),shieldBoost:!!(p.flags&16384),species:{id:p.species,name:p.species==='greninjaash'?'Greninja-Ash':p.species==='greninjabond'?'Greninja-Bond':'Pikachu'},activeTurns:p.turns,status:p.status,volatiles:p.substitute?{substitute:{}}:{},isAlly(o){return this.side===o.side;},hasAbility(a){trace.push(`ability:${i}:${name}`);return i===0?!!p.ability:!!p.oldability;},foes(){trace.push(`foes:${i}:false`);return [mon(6),mon(7)].slice(0,p.foes);},adjacentFoes(){trace.push(`foes:${i}:true`);return [mon(6),mon(7)].slice(0,p.foes);},getStat(s,u,v){trace.push(`stat:${i}:${s}:${u}:${v}`);return s==='def'?p.def:p.spd;},getLastAttackedBy(){trace.push(`last:${i}`);return p.last<0?undefined:{damage:p.last};},cureStatus(){trace.push(`cure:${i}`);}};mons.set(i,m);return m;}
 const target=mon(0),source=mon(p.source),owner=mon(p.owner);const state={target:owner};if(p.checked>=0)state.checkedBerserk=!!p.checked;
 const move={id:p.shuriken?'watershuriken':'facade',type:p.type,category:p.physical?'Physical':'Special',flags:{wind:!!p.wind},totalDamage:p.total,effectType:p.effecttype,status:p.effectstatus?'brn':undefined};if(p.multihit)move.multihit=2;if(p.smart)move.smartTarget=true;
 for(const s of ['Atk','Def','SpA','SpD'])if(p.old>=0)move['ruined'+s]=mon(p.old);
 const effectArg=p.effecttype==='missing'?undefined:move;
 const ctx={effect,effectState:state,suppressingAbility(m){trace.push(`suppress:${m.id}`);return !!p.suppress;},suppressingSecondaries(){trace.push('secondaries');return !!p.secondaries;},boost(b,t,s,e,secondary=false,self=false){trace.push(`boost:${Object.entries(b).map(([k,v])=>`${k}=${v}`).join(',')}:${id(t)}:${id(s)}:${id(e)}:${secondary}:${self}`);return p.boostresult==='true'?true:p.boostresult==='false'?false:p.boostresult==='0'?0:p.boostresult==='null'?null:undefined;},add(cmd,m,a,b){trace.push(`log:${cmd}:${m.id}:${a??'undefined'}:${b??'undefined'}`);},checkMoveMakesContact(m,s,t,announce){trace.push(`contact:${s.id}:${t.id}:${announce}`);return !!p.contact;},debug(){},chainModify(n){trace.push(`chain:${n}`);}};
 const boost={spe:p.lowered};if(p.atkboost!==99)boost.atk=p.atkboost;let args;
 if(hook==='onStart'||hook==='onResidual'||hook==='onUpdate')args=[target];
 else if(hook==='onTryEatItem')args=[{id:p.item}];
 else if(hook==='onModifyMove')args=[move,target];
 else if(hook.startsWith('onAnyModify'))args=[100,target,source,move];
 else if(hook==='onSourceAfterFaint')args=[p.length,target,source??target,effectArg];
 else if(hook==='onAfterEachBoost'||hook==='onAfterBoost')args=[boost,target,source,p.effecttype==='Ability'?Dex.abilities.get('intimidate'):effectArg];
 else if(hook==='onAfterMoveSecondary')args=[target,source,move];
 else if(hook==='onSetStatus')args=[{id:p.status},target,source,effectArg];
 else if(hook==='onTryHit')args=[target,source,move];
 else if(hook==='onSideConditionStart')args=[sides[0],source,{id:p.tailwind?'tailwind':'reflect'}];
 else if(hook==='onAnyFaint')args=[];
 else args=[40,target,source??target,effectArg??move];
 const ret=fn.apply(ctx,args);const flags=m=>Number(m.transformed)*8+Number(m.bondTriggered)*2048+Number(m.swordBoost)*8192+Number(m.shieldBoost)*16384;
 const slot=name==='beadsofruin'?'SpD':name==='swordofruin'?'Def':name==='vesselofruin'?'SpA':'Atk';
 lines.push([name,hook,pi,...keys.map(k=>p[k]),trace.join('|'),str(ret),str(state.checkedBerserk),flags(mon(0)),flags(mon(1)),flags(mon(6)),move['ruined'+slot]?.id??-1,move.multihit??0].join('\t'));count++;
}}
const outDir=new URL('../../crates/engine/src/effects/abilities/reactivestats/vectors/',import.meta.url);fs.mkdirSync(outDir,{recursive:true});for(const key of batch.keys){const name=key.split(':')[1];fs.writeFileSync(new URL(name+'.tsv',outDir),[lines[0],...lines.slice(1).filter(l=>l.startsWith(name+'\t'))].join('\n')+'\n');}console.log(`${count} callback action vectors (${profiles.length} profiles per hook)`);
