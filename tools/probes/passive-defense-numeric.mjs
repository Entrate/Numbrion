// Pinned source callback oracle; never writes the Showdown checkout.
import fs from 'node:fs';import{createRequire}from'node:module';import{loadSim}from'../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2]||'/home/aminaliu/src/pokemon-showdown');const {Dex}=createRequire(sim.root+'/package.json')('./dist/sim');const oracle=new sim.Battle({formatid:'gen9randomdoublesbattle',seed:'1,2,3,4'});
const sites=JSON.parse(fs.readFileSync('crates/engine/src/effects/abilities/passivedefense/sites.json'));const hooks=new Set(['onSourceModifyDamage','onModifyDamage','onModifyDef','onModifyAtk','onModifySpA','onSourceModifyAtk','onSourceModifySpA','onSourceModifyAccuracy','onBasePower','onModifySpe']);
let out='# family\tid\thook\ttype\tvariant\tmodifier\tresult\n';let count=0;
for(const{family,id,hooks:hs}of sites){const a=family==='abilities'?Dex.abilities.get(id):Dex.abilities.get(id).condition;for(const hook of hs){if(!hooks.has(hook))continue;for(const type of ['Normal','Fire','Water','Ice','Ghost','Grass','Electric','Fairy'])for(let v=0;v<12;v++){
 const initial=v%3?6144:4096;const counter=v%2?5:0;const move={id:'tackle',type,category:v%2?'Special':'Physical',flags:v%3?{contact:1,sound:1}:{},typeChangerBoosted:v%3===1?a:undefined};const p={hp:v%3?120:119,maxhp:120,item:v%2?'lightball':'',ignoringAbility:()=>v%3===2,hasAbility:()=>v%3!==2,getMoveHitData:()=>({typeMod:v%3-1})};const ctx={effect:a,effectState:{counter},event:{modifier:initial/4096},trunc:oracle.trunc,chainModify:sim.Battle.prototype.chainModify,modify:sim.Battle.prototype.modify,debug(){}};
 const n=hook==='onSourceModifyAccuracy'&&v>=6?true:101;const r=a[hook].call(ctx,n,p,p,move);out+=[family,id,hook,type,v,Math.round(ctx.event.modifier*4096),r===undefined?'undefined':r].map(x=>x===''?'<empty>':x).join('\t')+'\n';count++;
 }}}
const scope=JSON.parse(fs.readFileSync('data/scope.json')); // Scope generated move closure, not all Dex.
const moves=scope.moves.map(x=>typeof x==='string'?x:x.id);if(moves.some(id=>Dex.moves.get(id).ohko))throw Error('OHKO scope changed');
fs.writeFileSync('crates/engine/src/effects/abilities/passivedefense/numeric.tsv',out);console.log(`${count} numeric vectors; ${moves.length} scoped moves have no ohko`);
