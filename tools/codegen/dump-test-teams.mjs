#!/usr/bin/env node
// Constructor fixtures from the pinned oracle. No checkout files are changed.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {createRequire} from 'node:module';
import {COMMIT,assertOraclePin} from './oracle.mjs';
const ROOT=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const PS=path.resolve(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
assertOraclePin(PS);
const {Battle,Teams}=createRequire(path.join(PS,'package.json'))('./dist/sim');
const scope=JSON.parse(fs.readFileSync(path.join(ROOT,'data/scope.json')));
if(scope.meta.showdownCommit!==COMMIT)throw Error('Scope commit mismatch');
const FORMAT='gen9randomdoublesbattle';
const rows=[`# Showdown ${COMMIT}; pre-start new Battle + setPlayer x2`, '# B seed after p1Packed p2Packed; P mon gender name details hp stats species ability item types weight speed pp'];
const stats=['atk','def','spa','spd','spe'];let cases=0;
function record(packed,seed,label){
 const battle=new Battle({formatid:FORMAT,seed});
 // setPlayer's second call would start automatically. Suspend only that instance
 // at precisely this boundary; Pokemon, Side and Battle constructors remain original.
 let starts=0;battle.start=()=>{starts++};
 battle.setPlayer('p1',{team:packed[0]});battle.setPlayer('p2',{team:packed[1]});
 if(starts!==1||battle.started||battle.turn!==0)throw Error('Pre-start boundary changed');
 rows.push(['B',seed.join(','),battle.prng.getSeed(),...packed,label].join('\t'));
 for(const side of battle.sides)for(const p of side.pokemon){
  rows.push(['P',side.n*6+p.position,p.gender,p.name,p.details,p.hp,stats.map(s=>p.storedStats[s]).join(','),p.species.id,p.ability,p.item,p.types.join(','),p.weighthg,p.speed,p.moveSlots.map(m=>`${m.id}:${m.pp}:${m.maxpp}:${m.target}`).join(','),p.teraType,p.level,p.set.evs?Object.values(p.set.evs).join(','):'',p.set.ivs?['hp',...stats].map(s=>p.set.ivs[s]).join(','):''].join('\t'));
 }
 if(Object.keys(battle.field.pseudoWeather).join(',')!=='sleepclausemod'||battle.effectOrder!==0)throw Error('Constructor effects changed');
 cases++;
}
const missing=[];
for(let i=0;i<300;i++){
 const teams=[0,1].map(side=>Teams.getGenerator(FORMAT,[0x1973,side+1,i,0xabcd]).getTeam());
 record(teams.map(t=>Teams.pack(t)),[0xb00b,i,0x1234,0x5678],`generated-${i}`);
 if(i<20){
  const variants=teams.map(t=>structuredClone(t));
  for(const t of variants)for(const set of t){delete set.gender;set.nature=['Adamant','Quiet','Timid','Jolly'][i%4];if(i%2===0)set.shiny=true;}
  missing.push([variants.map(t=>Teams.pack(t)),[9,8,i,6],`missing-gender-${i}`]);
 }
}
for(const v of missing)record(...v);
// A few boundary encodings: ability slot shorthand, omitted EVs/IVs, explicit N,
// zero happiness, PP without boosts, nickname truncation and details exceptions.
const tiny=(species,moves,ability='',gender='',misc='')=>`${species}|||${ability}|${moves}|||${gender}||||${misc}`;
record([tiny('Greninja-Bond','struggle','battlebond','N',',,,,,Stellar'),tiny('Blissey','protect','healer')],[1,2,3,4],'details-and-no-pp-boost');
record(['ThisNameIsLongerThanTwentyCharacters|pikachu||0|thunderbolt|Adamant|,0,,,,|M|,0,,,,|S|99|0,,,,,Electric',tiny('Blissey','softboiled','H')],[65535,0,1,99],'packed-defaults-and-clamping');
record(['Pikachu|||0|thunderbolt|| ,0x55,0,85,85,85|F| ,0,31,31,31,31||0x4b| ,,, , ,Electric',tiny('Blissey','protect','healer')],[11,22,33,44],'numeric-packed-fields');
const body=rows.join('\n')+'\n';
fs.writeFileSync(path.join(ROOT,'crates/engine/tests/data/constructor.tsv'),body);
fs.writeFileSync(path.join(ROOT,'crates/engine/tests/data/scope-counts.tsv'),['# '+COMMIT,...['species','moves','abilities','items','conditions','rules'].map(k=>k+'\t'+scope[k].map(e=>e.id).sort().join(','))].join('\n')+'\n');
console.log(`Wrote ${cases} battles (${cases*2} teams), SHA256 ${crypto.createHash('sha256').update(body).digest('hex')}`);
