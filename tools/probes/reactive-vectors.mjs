import fs from 'node:fs';
import {createRequire} from 'node:module';
import {loadSim} from '../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
const {Dex}=createRequire(sim.root+'/package.json')('./dist/sim');
const group=JSON.parse(fs.readFileSync(new URL('../../docs/design/EFFECT-BATCHES.json',import.meta.url))).groups.find(g=>g.batch==='reactive_contact');
let lines=['# effect\tscenario\tseed4\ttrace\tfinalSeed'];
for(const key of group.keys){const [family,name]=key.split(':');const effect=Dex[family].get(name);const fn=Object.entries(effect).find(([k,v])=>typeof v==='function')[1];
for(let scenario=0;scenario<21;scenario++)for(let seed4=0;seed4<128;seed4++){
 const prng=new sim.PRNG(`1,2,3,${seed4}`), trace=[];const flags={contact:scenario!==2,powder:scenario!==3,shield:scenario===4,cloak:scenario===5};
 const make=(id,side)=>({id,side,hp:scenario===1?0:40,baseMaxhp:120,baseSpecies:{name:scenario===13?'Pikachu':'Pecharunt'},volatiles:scenario===6?{disable:{}}:{},isAlly(other){return this.side===other.side;},hasAbility(x){trace.push(`ability:${id}:${x}`);return flags.shield;},hasItem(x){trace.push(`item:${id}:${x}`);return flags.cloak;},runStatusImmunity(x){trace.push(`powder:${id}`);return flags.powder;},addVolatile(status,source){trace.push(`volatile:${id}:${status}:${source?.id??'undefined'}`);},trySetStatus(status,source,e){trace.push(`status:${id}:${status.id??status}:${source.id}:${e?.id??'undefined'}:${e?.status??'undefined'}`);}});
 const sides=[{id:0,sideConditions:{}},{id:1,sideConditions:{}}];sides[0].foe=sides[1];sides[1].foe=sides[0];for(const s of sides){s.addSideCondition=(status,source)=>trace.push(`hazard:${s.id}:${status}:${source.id}`);if(scenario===9)s.sideConditions.toxicspikes={layers:2};if(scenario===20)s.sideConditions.toxicspikes={};}
 const target=make(0,sides[0]);let source=scenario===11?target:make(scenario===10?1:6,sides[scenario===10?0:1]);if(scenario===12&&name==='synchronize')source=undefined;
 const move={id:scenario===7?'struggle':'facade',category:scenario===8?'Special':'Physical',effectType:scenario===19?'Ability':'Move',flags:{}};
 const status={id:scenario===15?'slp':scenario===16?'frz':scenario===17?'psn':'tox'};let sourceEffect=scenario===18?{id:'toxicspikes',effectType:'Condition'}:move;
 const ctx={effectState:{target:scenario===14?target:source},checkMoveMakesContact(m,a,d,announce=false){trace.push(`contact:${a.id}:${d.id}:${announce}`);return flags.contact;},random(n){trace.push(`random:${n}`);return prng.random(n);},randomChance(n,d){trace.push(`chance:${n}:${d}`);return prng.randomChance(n,d);},damage(n,t,s){trace.push(`damage:${t.id}:${n}:${s.id}`);},add(cmd,t,label){trace.push(`activate:${t.id}:${name}`);}};
 if(name==='synchronize'||name==='poisonpuppeteer')fn.call(ctx,status,target,source,sourceEffect);else fn.call(ctx,40,target,source,move);
 lines.push([name,scenario,seed4,trace.join('|'),prng.getSeed().replace(/^gen5,/, '')].join('\t'));
}}
fs.writeFileSync(new URL('../../crates/engine/src/effects/abilities/reactivecontact/vectors.tsv',import.meta.url),lines.join('\n')+'\n');console.log(`${lines.length-1} callback action/PRNG vectors`);
