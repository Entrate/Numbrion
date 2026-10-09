import fs from 'node:fs';
import {createRequire} from 'node:module';
import {loadSim} from '../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
const {Dex}=createRequire(sim.root+'/package.json')('./dist/sim');
const groups=JSON.parse(fs.readFileSync(new URL('../../docs/design/EFFECT-BATCHES.json',import.meta.url))).groups;
const oracle=new sim.Battle({formatid:'gen9randomdoublesbattle',seed:'1,2,3,4'});
const species=['pikachu','raichu','dialga','palkia','giratina','arceus','zacian','zamazenta','latios','ogerpon','ogerponcornerstone','ogerponhearthflame','ogerponwellspring'];
const types=Dex.types.names().filter(t=>t!=='???');
let lines=['# item\thook\tholder\tsource\ttype\tinitial\tnumeric\tmodifier\trelay'];
for(const k of groups.find(g=>g.batch==='items_modifiers').keys){const item=Dex.items.get(k.split(':')[1]);
 for(const [hook,fn]of Object.entries(item)){if(typeof fn!=='function'||!['onBasePower','onTakeItem','onModifyAtk','onModifySpA','onModifySpD','onModifyDef','onModifySpe','onModifyDamage','onSourceModifyAccuracy'].includes(hook))continue;
 for(const holder of species)for(const source of hook==='onTakeItem'?['none',...species]:['none'])for(const type of hook==='onBasePower'?types:['Normal'])for(const initial of [4096,6144])for(const numeric of hook==='onSourceModifyAccuracy'?[true,false]:[true]){
  const mon=id=>({baseSpecies:Dex.species.get(id),volatiles:{}});const ctx={event:{modifier:initial/4096},trunc:oracle.trunc,chainModify:sim.Battle.prototype.chainModify,debug(){}};
  const args=hook==='onTakeItem'?[item,mon(holder),source==='none'?undefined:mon(source)]:[numeric?40:true,mon(holder),{}, {type}];
  const relay=fn.apply(ctx,args);lines.push([item.id,hook,holder,source,type,initial,Number(numeric),Math.round(ctx.event.modifier*4096),relay===undefined?'undefined':relay].join('\t'));
 }
 }
}
fs.writeFileSync(new URL('../../crates/engine/src/effects/items/itemsmodifiers/vectors.tsv',import.meta.url),lines.join('\n')+'\n');console.log(`${lines.length-1} item vectors`);
