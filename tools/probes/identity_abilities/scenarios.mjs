// Oracle-derived full battles; Rust replays are ignored until lifecycle/choices/moves/text land.
import fs from 'node:fs';
import {loadSim} from '../../oracle/lib/common.mjs';
import {Session} from '../../oracle/lib/session.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
function mon(name,ability,moves,{item='',level=100,gender='M',tera='Normal'}={}) {return `${name}||${item}|${ability}|${moves}|Serious||${gender}|||${level}|,,,,,${tera}`;}
const team=(...p)=>p.join(']');
const scenarios=[];
const add=(name,p1,p2,steps,must)=>scenarios.push({name,p1,p2,steps,must,seed:[101+scenarios.length,202,303,404]});

const partner=()=>mon('Mudsdale','stamina','irondefense');
add('illusion_reveal',team(mon('Zoroark','illusion','flamethrower'),partner(),mon('Pikachu','static','irondefense',{level:83})),team(mon('Pikachu','static','thunderbolt'),partner()),['p1:move 1 1, move 1','p2:move 1 1, move 1'],['replace','Illusion Level Mod']);
add('imposter_mirrored_slot',team(mon('Ditto','imposter','transform'),partner()),team(mon('Pikachu','static','thunderbolt'),mon('Swampert','torrent','irondefense')),[],['-transform|p1a: Ditto|p2b: Swampert']);
add('transform_success',team(mon('Mew','synchronize','transform',{gender:'N'}),partner()),team(mon('Pikachu','static','thunderbolt'),partner()),['p1:move 1 1, move 1','p2:move 1 2, move 1'],['-transform']);
add('transform_illusion_failure',team(mon('Mew','synchronize','transform',{gender:'N'}),partner()),team(mon('Zoroark','illusion','irondefense'),partner(),mon('Pikachu','static','irondefense')),['p1:move 1 1, move 1','p2:move 1, move 1'],['|-fail|p1a: Mew']);
add('trace_single_candidate',team(mon('Gardevoir','trace','irondefense'),partner()),team(mon('Venusaur','chlorophyll','irondefense'),mon('Iron Hands','quarkdrive','irondefense',{gender:'N'})),[],['Chlorophyll|Trace|[from] ability: Trace']);
add('as_one_berries',team(mon('Calyrex-Ice','asoneglastrier','irondefense',{gender:'N'}),mon('Calyrex-Shadow','asonespectrier','irondefense',{gender:'N'})),team(mon('Snorlax','thickfat','irondefense',{item:'Sitrus Berry'}),partner()),[],['As One','Unnerve']);

add('as_one_start',team(mon('Calyrex-Ice','asoneglastrier','irondefense',{gender:'N'}),mon('Calyrex-Shadow','asonespectrier','irondefense',{gender:'N'})),team(partner(),partner()),[],['As One','Unnerve']);
let out='# name\tseed\tp1\tp2\tsteps\tfinal seed\tlog\n';
for(const s of scenarios){
 const session=new Session(sim,{seed:s.seed,teams:[s.p1,s.p2]});
 for(const step of s.steps){const r=session.choose(step.slice(0,2),step.slice(3));if(!r.ok)throw Error(`${s.name}: ${step}: ${r.error}`);}
 const log=session.battle.log.map(l=>/^\|t:\|\d+$/.test(l)?'|t:|':l);
 for(const text of s.must)if(!log.some(l=>l.includes(text)))throw Error(`${s.name}: missing ${text}\n${log.join('\n')}`);
 if(session.anomalies.length)throw Error(session.anomalies.join(';'));
 out+=[s.name,s.seed.join(','),s.p1,s.p2,s.steps.join(';'),session.currentSeed().join(','),log.join('\\n')].join('\t')+'\n';
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/identityabilities/scenarios.tsv',import.meta.url),out);
console.log(`Wrote ${scenarios.length} full-battle scenarios.`);
