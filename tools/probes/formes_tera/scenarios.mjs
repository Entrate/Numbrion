// Oracle-derived full battles; Rust replays are ignored until lifecycle/choices/moves/text land.
import fs from 'node:fs';
import {loadSim} from '../../oracle/lib/common.mjs';
import {Session} from '../../oracle/lib/session.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
function mon(name,ability,moves,{item='',level=100,gender='M',tera='Normal'}={}) {return `${name}||${item}|${ability}|${moves}|Serious||${gender}|||${level}|,,,,,${tera}`;}
const team=(...p)=>p.join(']');
const scenarios=[];
const add=(name,p1,p2,steps,must)=>scenarios.push({name,p1,p2,steps,must,seed:[101+scenarios.length,202,303,404]});

const partner=()=>mon('Mudsdale','stamina','protect');
add('sun_to_booster',team(mon('Flutter Mane','protosynthesis','protect',{item:'Booster Energy',gender:'N'}),mon('Groudon','drought','protect',{gender:'N'})),team(mon('Tornadus','prankster','raindance'),partner()),['p1:move 1, move 1','p2:move 1, move 1'],['protosynthesisspa','-enditem|p1a: Flutter Mane|Booster Energy','[fromitem]']);
add('electric_to_booster_teraform',team(mon('Iron Bundle','quarkdrive','protect',{item:'Booster Energy',gender:'N'}),mon('Pincurchin','electricsurge','protect')),team(mon('Terapagos','terashift','terastarstorm',{tera:'Stellar'}),partner()),['p1:move 1, move 1','p2:move 1 terastallize, move 1'],['Tera Shift','Teraform Zero','Quark Drive|[fromitem]']);
add('disguise_bust',team(mon('Mimikyu','disguise','playrough'),partner()),team(mon('Pikachu','static','thunderbolt'),partner()),['p1:move 1 1, move 1','p2:move 1 1, move 1'],['ability: Disguise','Mimikyu-Busted']);
add('ice_face_break_restore',team(mon('Eiscue','iceface','iciclecrash'),partner()),team(mon('Rillaboom','overgrow','woodhammer'),partner(),mon('Abomasnow','snowwarning','protect')),['p1:move 1 1, move 1','p2:move 1 1, move 1','p1:move 1 1, move 1','p2:move 1 1, switch 3'],['Eiscue-Noice','Ice Face']);
add('morpeko_toggle',team(mon('Morpeko','hungerswitch','aurawheel'),partner()),team(partner(),partner()),['p1:move 1 1, move 1','p2:move 1, move 1','p1:move 1 1, move 1','p2:move 1, move 1'],['Morpeko-Hangry']);
add('palafin_switch',team(mon('Palafin','zerotohero','protect'),partner(),mon('Pikachu','static','protect')),team(partner(),partner()),['p1:switch 3, move 1','p2:move 1, move 1','p1:switch 3, move 1','p2:move 1, move 1'],['Palafin-Hero','ability: Zero to Hero']);
add('relic_song_toggle',team(mon('Meloetta','serenegrace','relicsong',{gender:'N'}),partner()),team(mon('Mudsdale','stamina','heavyslam'),partner()),['p1:move 1, move 1','p2:move 1 2, move 1'],['Meloetta-Pirouette']);
for(const [species,ability,item,tera,stat] of [['Ogerpon','defiant','','Grass','spe'],['Ogerpon-Wellspring','waterabsorb','Wellspring Mask','Water','spd'],['Ogerpon-Hearthflame','moldbreaker','Hearthflame Mask','Fire','atk'],['Ogerpon-Cornerstone','sturdy','Cornerstone Mask','Rock','def']]) {
 add(`ogerpon_${stat}`,team(mon(species,ability,'ivycudgel',{item,tera,gender:'F'}),partner()),team(mon('Snorlax','thickfat','bodypress'),partner()),['p1:move 1 1 terastallize, move 1','p2:move 1 2, move 1'],['-Tera','Embody Aspect',`|${stat}|1`]);
}
add('stellar_tera_blast',team(mon('Pikachu','static','terablast',{tera:'Stellar'}),partner()),team(mon('Snorlax','thickfat','bodypress'),partner()),['p1:move 1 1 terastallize, move 1','p2:move 1 2, move 1'],['Tera Blast Stellar','|-unboost|p1a: Pikachu|atk|1','|-unboost|p1a: Pikachu|spa|1']);
add('shields_down_crossing',team(mon('Minior','shieldsdown','acrobatics'),partner()),team(mon('Mudsdale','stamina','rockslide'),partner()),['p1:move 1 1, move 1','p2:move 1, move 1','p1:move 1 1, move 1','p2:move 1, move 1'],['Minior-Meteor']);

let out='# name\tseed\tp1\tp2\tsteps\tfinal seed\tlog\n';
for(const s of scenarios){
 const session=new Session(sim,{seed:s.seed,teams:[s.p1,s.p2]});
 for(const step of s.steps){const r=session.choose(step.slice(0,2),step.slice(3));if(!r.ok)throw Error(`${s.name}: ${step}: ${r.error}`);}
 const log=session.battle.log.map(l=>/^\|t:\|\d+$/.test(l)?'|t:|':l);
 for(const text of s.must)if(!log.some(l=>l.includes(text)))throw Error(`${s.name}: missing ${text}\n${log.join('\n')}`);
 if(session.anomalies.length)throw Error(session.anomalies.join(';'));
 out+=[s.name,s.seed.join(','),s.p1,s.p2,s.steps.join(';'),session.currentSeed().join(','),log.join('\\n')].join('\t')+'\n';
}
fs.writeFileSync(new URL('../../../crates/engine/src/effects/abilities/formestera/scenarios.tsv',import.meta.url),out);
console.log(`Wrote ${scenarios.length} full-battle scenarios.`);
