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
