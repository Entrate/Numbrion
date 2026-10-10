// Run actual pinned callbacks without lifecycle. Writes only this worktree's fixtures.
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {loadSim} from '../../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
const dex=new sim.Battle({formatid:'gen9randomdoublesbattle',seed:'1,2,3,4'}).dex;
const require=createRequire(path.join(sim.root,'package.json'));
const {Pokemon}=require('./dist/sim/pokemon');
const rows=[];
function move(key,species,{tera='',atk=100,spa=100,types='Normal',item='',ignored=false,bp=67,inputType}={}) {
 const raw=dex.moves.get(key); const m={...raw,basePower:bp,type:inputType||raw.type};
 const p={species:dex.species.get(species),terastallized:tera,teraType:tera,getStat:s=>s==='atk'?atk:spa,
 getTypes:()=>tera && tera!=='Stellar'?[tera]:types.split(','),ignoringItem:()=>ignored,getItem:()=>dex.items.get(item)};
 const cx={};
 raw.onModifyType?.call(cx,m,p);raw.onModifyMove?.call(cx,m,p);
 const power=raw.basePowerCallback?.call(cx,p,{},m)??m.basePower;
 rows.push([key,species,tera,atk,spa,types,item,+ignored,bp,inputType||raw.type,m.type,m.category,m.target,power,m.self?.boosts?Object.entries(m.self.boosts).map(x=>x.join(':')).join(','):'']);
}
for(const s of ['Morpeko','Morpeko-Hangry','Pikachu'])move('aurawheel',s);
for(const s of ['Ogerpon','Ogerpon-Teal-Tera','Ogerpon-Wellspring','Ogerpon-Wellspring-Tera','Ogerpon-Hearthflame','Ogerpon-Hearthflame-Tera','Ogerpon-Cornerstone','Ogerpon-Cornerstone-Tera','Pikachu'])move('ivycudgel',s,{inputType:'Electric'});
for(const ignored of [false,true])for(const item of ['','Mind Plate'])move('judgment','Arceus-Psychic',{item,ignored});
for(const types of ['Fire,Flying','???,Flying','???','Grass'])for(const tera of ['','Water','Stellar'])move('revelationdance','Oricorio',{types,tera});
for(const tera of ['','Fire','Stellar'])for(const [atk,spa] of [[99,100],[100,100],[101,100]])move('terablast','Pikachu',{tera,atk,spa});
for(const s of ['Terapagos','Terapagos-Terastal','Terapagos-Stellar'])for(const tera of ['','Stellar'])for(const [atk,spa] of [[100,100],[101,100]])move('terastarstorm',s,{tera,atk,spa});
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/formestera/moves.tsv',import.meta.url),rows.map(r=>r.join('\t')).join('\n')+'\n');
const stats=['atk','def','spa','spd','spe'];const out=[];
for(const key of ['protosynthesis','quarkdrive'])for(const best of stats)for(const from of [false,true])for(const ignored of [false,true]) {
 const values=stats.map(s=>s===best?201:200);const logs=[];let factor=4096;
 const state={};const cx={effectState:state,add:(...a)=>logs.push(a.map(x=>x?.marker??x).join('|')),debug:()=>{},chainModify:r=>{factor=Array.isArray(r)?r[0]:Math.floor(r*4096);}};
 const p={marker:'m0',getStat:s=>values[stats.indexOf(s)],getBestStat:(...a)=>Pokemon.prototype.getBestStat.call(p,...a),ignoringAbility:()=>ignored};
 const raw=dex.abilities.get(key).condition;
 raw.onStart.call(cx,p,null,from?dex.items.get('boosterenergy'):dex.abilities.get(key));
 const factors=stats.map(s=>{factor=4096;raw['onModify'+({atk:'Atk',def:'Def',spa:'SpA',spd:'SpD',spe:'Spe'}[s])].call(cx,100,p);return factor;});
 raw.onEnd.call(cx,p);
 out.push([key,values.join(','),+from,+ignored,state.bestStat,factors.join(','),logs.join('\\n')]);
}
// Atk wins ties in the real Pokemon method; activation caches that result.
const p={getStat:()=>200};if(Pokemon.prototype.getBestStat.call(p,false,true)!=='atk')throw Error('tie changed');
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/formestera/paradox.tsv',import.meta.url),out.map(r=>r.join('\t')).join('\n')+'\n');
const scope=JSON.parse(fs.readFileSync(new URL('../../../data/scope.json',import.meta.url)));
// Closed-format branch evidence; input boundary must never grant these moves.
const excluded=['assist','copycat','mefirst','metronome','mirrormove','naturepower','sleeptalk','snatch','futuresight','doomdesire'];
const rust=fs.readFileSync(new URL('../../../crates/engine/src/dex/ids_generated.rs',import.meta.url),'utf8');
for(const key of excluded)if(rust.includes(`pub const MOVE_${key.toUpperCase()}:`))throw Error(`branch exclusion invalid: ${key}`);
console.log(`Wrote ${rows.length} move and ${out.length} Paradox rows; closed-format exclusions verified.`);
// Exercise exact relay sentinels and Substitute/immunity guards on the pinned handlers.
const guards=[];
const show=v=>v===undefined?'undefined':v===null?'null':String(v);
for(const key of ['disguise','iceface'])for(const hook of ['onDamage','onCriticalHit','onEffectiveness'])for(const species of [key==='disguise'?'Mimikyu':'Eiscue',key==='disguise'?'Mimikyu-Busted':'Eiscue-Noice','Pikachu'])for(const category of ['Physical','Special','Status'])for(const mode of ['plain','sub','bypass','infiltrates'])for(const immune of [false,true]) {
 const logs=[];const state={};
 const p={marker:'m0',species:dex.species.get(species),volatiles:mode==='plain'?{}:{substitute:{}},runImmunity:()=>immune};
 const mv={...dex.moves.get('bodyslam'),category,flags:{bypasssub:mode==='bypass'},infiltrates:mode==='infiltrates'};
 const cx={gen:9,effectState:state,add:(...a)=>logs.push(a.map(x=>x?.marker??x).join('|'))};
 const a=dex.abilities.get(key);
 const args=hook==='onDamage'?[37,p,null,mv]:hook==='onCriticalHit'?[p,null,mv]:[1,p,'Normal',mv];
 const result=a[hook].apply(cx,args);
 guards.push([key,hook,species,category,mode,+immune,show(result),state.busted===undefined?'absent':+state.busted,logs.join('\\n')]);
}
// Confusion's source object has effectType Move, but no category property.
for(const key of ['disguise','iceface']) {
 const state={},logs=[];const p={marker:'m0',species:dex.species.get(key==='disguise'?'Mimikyu':'Eiscue')};
 const r=dex.abilities.get(key).onDamage.call({effectState:state,add:(...a)=>logs.push(a.map(x=>x?.marker??x).join('|'))},37,p,p,{id:'confused',effectType:'Move',type:'???'});
 guards.push([key,'onDamage',p.species.name,'Confused','plain',1,show(r),state.busted===undefined?'absent':+state.busted,logs.join('\\n')]);
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/formestera/guards.tsv',import.meta.url),guards.map(r=>r.join('\t')).join('\n')+'\n');
const arceus=[];
for(const sp of dex.species.all().filter(s=>s.baseSpecies==='Arceus'))for(const ability of ['multitype','static'])for(const item of ['','Mind Plate'])for(const transformed of [false,true]) {
 // Only the 18 scoped formes; Legends-Arceus has no generated species view.
 if(!rust.includes(`pub const SPECIES_${sp.id.toUpperCase()}:`))continue;
 const input=[sp.types[0],'Flying'];
 const p={transformed,ability,getItem:()=>dex.items.get(item)};
 const result=sp.onType.call({gen:9},input,p);
 arceus.push([sp.id,ability,item,+transformed,input.join(','),result.join(',')]);
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/formestera/arceus.tsv',import.meta.url),arceus.map(r=>r.join('\t')).join('\n')+'\n');
console.log(`Wrote ${guards.length} guard and ${arceus.length} inherited species-type rows.`);
const take=[];
for(const species of ['Flutter Mane','Iron Bundle','Pikachu','Mimikyu-Busted']) {
 const p={baseSpecies:dex.species.get(species)};
 take.push([species,show(dex.items.get('boosterenergy').onTakeItem.call({},dex.items.get('boosterenergy'),p))]);
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/formestera/takeitem.tsv',import.meta.url),take.map(r=>r.join('\t')).join('\n')+'\n');
const shieldStatus=[];
for(const species of ['Minior-Meteor','Minior'])for(const transformed of [false,true])for(const kind of ['statusmove','other','yawn','substitute']) {
 const logs=[];const p={marker:'m0',species:dex.species.get(species),transformed};
 const cx={add:(...a)=>logs.push(a.map(x=>x?.marker??x).join('|'))};
 const a=dex.abilities.get('shieldsdown');
 const r=['statusmove','other'].includes(kind)?a.onSetStatus.call(cx,dex.conditions.get('brn'),p,null,kind==='statusmove'?dex.moves.get('willowisp'):dex.abilities.get('static')):a.onTryAddVolatile.call(cx,dex.conditions.get(kind),p);
 shieldStatus.push([species,+transformed,kind,show(r),logs.join('\\n')]);
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/formestera/status.tsv',import.meta.url),shieldStatus.map(r=>r.join('\t')).join('\n')+'\n');
console.log(`Wrote ${take.length} Booster Energy take-item and ${shieldStatus.length} Shields Down status rows.`);
