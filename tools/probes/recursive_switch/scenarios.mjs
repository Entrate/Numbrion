// Full protocol/PRNG expectations from the read-only pinned simulator.
import fs from 'node:fs';
import {loadSim} from '../../oracle/lib/common.mjs';
import {Session} from '../../oracle/lib/session.mjs';
const sim=loadSim(process.argv[2]||`${process.env.HOME}/src/pokemon-showdown`);
const mon=(name,moves,{ability='earlybird',item='',level=100,gender='N'}={})=>`${name}||${item}|${ability}|${moves}|Serious||${gender}|||${level}|,,,,,Normal`;
const team=(...p)=>p.join(']');
const quiet=()=>mon('Mew','irondefense');
const turn=(a,b)=>[`p1:${a}`,`p2:${b}`];
const idle='move 1, move 1';
const rows={recursive_switch:[],item_exchange:[]};
function add(batch,name,p1,p2,steps,must=[]){
 const seed=[101+rows[batch].length,202,303,404]; const s=new Session(sim,{seed,teams:[p1,p2]});
 for(const step of steps){const r=s.choose(step.slice(0,2),step.slice(3));if(!r.ok)throw Error(`${name}: ${step}: ${r.error}`);}
 const log=s.battle.log.map(l=>/^\|t:\|\d+$/.test(l)?'|t:|':l);
 for(const text of must)if(!log.some(l=>l.includes(text)))throw Error(`${name}: missing ${text}\n${log.join('\n')}`);
 if(s.anomalies.length)throw Error(s.anomalies.join(';'));
 rows[batch].push([name,seed.join(','),p1,p2,steps.join(';'),s.currentSeed().join(','),log.join('\\n')].join('\t'));
}
const rec=(...args)=>add('recursive_switch',...args), item=(...args)=>add('item_exchange',...args);
rec('substitute_status_and_restart',team(mon('Mew','substitute'),quiet()),team(mon('Mew','stunspore'),quiet()),[...turn(idle,'move 1 1, move 1'),...turn(idle,'move 1 1, move 1')],['|-fail|p1a: Mew|move: Substitute','|-fail|p2a: Mew']);
rec('substitute_break_and_recoil',team(mon('Mew','substitute'),quiet()),team(mon('Mew','doubleedge'),quiet()),[...turn(idle,'move 1 1, move 1')],['Substitute','[from] Recoil']);
rec('substitute_drain',team(mon('Mew','substitute'),quiet()),team(mon('Mew','substitute,gigadrain'),quiet()),[...turn(idle,idle),...turn(idle,'move 2 1, move 1')],['[from] drain']);
rec('substitute_weak',team(mon('Mew','substitute'),quiet()),team(mon('Rampardos','doubleedge'),quiet()),Array.from({length:5},()=>turn(idle,'move 1 1, move 1')).flat(),['[weak]']);
rec('batonpass_transfer',team(mon('Mew','substitute,irondefense,batonpass'),quiet(),mon('Pikachu','irondefense')),team(quiet(),quiet()),[...turn('move 1, move 1',idle),...turn('move 2, move 1',idle),...turn('move 3, move 1',idle),'p1:switch 3, pass'],['[from] Baton Pass']);
rec('batonpass_no_bench',team(mon('Mew','batonpass'),quiet()),team(quiet(),quiet()),turn(idle,idle),['|-fail|p1a: Mew']);
rec('shedtail_transfer',team(mon('Mew','shedtail'),quiet(),mon('Pikachu','irondefense')),team(quiet(),quiet()),[...turn(idle,idle),'p1:switch 3, pass'],['[from] move: Shed Tail','[from] Shed Tail']);
rec('shedtail_weak',team(mon('Mew','shedtail'),quiet(),quiet()),team(quiet(),quiet()),[...turn(idle,idle),'p1:switch 3, pass',...turn('switch 3, move 1',idle),...turn(idle,idle)],['[weak]']);
rec('magicbounce_status',team(mon('Mew','stunspore'),quiet()),team(mon('Mew','irondefense',{ability:'magicbounce'}),quiet()),turn('move 1 1, move 1',idle),['[from] ability: Magic Bounce']);
rec('magicbounce_side_once',team(mon('Mew','spikes'),quiet()),team(mon('Mew','irondefense',{ability:'magicbounce'}),mon('Mew','irondefense',{ability:'magicbounce'})),turn(idle,idle),['|-sidestart|p1: Alice|Spikes']);
rec('magicbounce_partingshot',team(mon('Mew','partingshot'),quiet(),quiet()),team(mon('Mew','irondefense',{ability:'magicbounce'}),quiet(),quiet()),[...turn('move 1 1, move 1',idle),'p2:switch 3, pass'],['[from] Parting Shot']);
rec('instruct_repeat_target',team(mon('Mew','shadowball',{level:100}),mon('Oranguru','instruct')),team(quiet(),quiet()),turn('move 1 2, move 1 -1',idle),['|-singleturn|p1a: Mew|move: Instruct']);
rec('instruct_missing_lastmove',team(mon('Mew','instruct'),mon('Oranguru','irondefense')),team(quiet(),quiet()),turn('move 1 -2, move 1',idle),['|-fail|p1a: Mew']);
rec('revival_no_fainted',team(mon('Mew','revivalblessing'),quiet()),team(quiet(),quiet()),turn(idle,idle),['|-fail|p1a: Mew']);
rec('revival_success',team(mon('Mew','revivalblessing,irondefense'),mon('Pikachu','irondefense',{level:1,gender:'M'}),quiet()),team(quiet(),mon('Mew','shadowball')),[...turn('move 2, move 1','move 1, move 1 2'),'p1:pass, switch 3',...turn(idle,'move 1, move 1 2'),'p1:switch 3, pass'],['[from] move: Revival Blessing']);
item('trick_swap',team(mon('Mew','trick',{item:'Heavy-Duty Boots'}),quiet()),team(mon('Mew','irondefense',{item:'Silk Scarf'}),quiet()),turn('move 1 1, move 1',idle),['|-item|p1a: Mew|Silk Scarf|[from] move: Trick']);
item('trick_give_only',team(mon('Mew','trick',{item:'Heavy-Duty Boots'}),quiet()),team(quiet(),quiet()),turn('move 1 1, move 1',idle),['|-enditem|p1a: Mew|Heavy-Duty Boots|[silent]|[from] move: Trick']);
item('trick_take_only',team(mon('Mew','trick'),quiet()),team(mon('Mew','irondefense',{item:'Heavy-Duty Boots'}),quiet()),turn('move 1 1, move 1',idle),['|-enditem|p2a: Mew|Heavy-Duty Boots|[silent]|[from] move: Trick']);
item('trick_stickyhold',team(mon('Mew','trick',{item:'Heavy-Duty Boots'}),quiet()),team(mon('Mew','irondefense',{ability:'stickyhold',item:'Heavy-Duty Boots'}),quiet()),turn('move 1 1, move 1',idle),['|-immune|']);
item('trick_empty_failure',team(mon('Mew','trick'),quiet()),team(quiet(),quiet()),turn('move 1 1, move 1',idle),['|-fail|']);
item('knockoff_remove',team(mon('Mew','knockoff'),quiet()),team(mon('Mew','irondefense',{item:'Heavy-Duty Boots'}),quiet()),turn('move 1 1, move 1',idle),['|-enditem|']);
item('knockoff_stickyhold',team(mon('Mew','knockoff'),quiet()),team(mon('Mew','irondefense',{ability:'stickyhold',item:'Heavy-Duty Boots'}),quiet()),turn('move 1 1, move 1',idle),['ability: Sticky Hold']);
item('magician_take',team(mon('Mew','shadowball',{ability:'magician'}),quiet()),team(mon('Mew','irondefense',{item:'Heavy-Duty Boots'}),quiet()),turn('move 1 1, move 1',idle),['[from] ability: Magician']);
item('magician_spread_tie',team(mon('Mew','surf',{ability:'magician'}),quiet()),team(mon('Mew','irondefense',{item:'Heavy-Duty Boots'}),mon('Mew','irondefense',{item:'Heavy-Duty Boots'})),turn(idle,idle),['[from] ability: Magician']);
item('pickpocket_contact',team(mon('Mew','bodyslam',{item:'Heavy-Duty Boots'}),quiet()),team(mon('Mew','irondefense',{ability:'pickpocket'}),quiet()),turn('move 1 1, move 1',idle),['[from] ability: Pickpocket']);
item('pickpocket_no_contact',team(mon('Mew','shadowball',{item:'Heavy-Duty Boots'}),quiet()),team(mon('Mew','irondefense',{ability:'pickpocket'}),quiet()),turn('move 1 1, move 1',idle));
item('frisk_both_slots',team(mon('Mew','irondefense',{ability:'frisk'}),quiet()),team(mon('Mew','irondefense',{item:'Heavy-Duty Boots'}),mon('Mew','irondefense',{item:'Silk Scarf'})),[],['Heavy-Duty Boots|[from] ability: Frisk','Silk Scarf|[from] ability: Frisk']);
item('poltergeist_success',team(mon('Mew','poltergeist'),quiet()),team(mon('Mew','irondefense',{item:'Heavy-Duty Boots'}),quiet()),turn('move 1 1, move 1',idle),['move: Poltergeist|Heavy-Duty Boots']);
item('poltergeist_empty_failure',team(mon('Mew','poltergeist'),quiet()),team(quiet(),quiet()),turn('move 1 1, move 1',idle),['|-fail|']);
item('recycle_no_lastitem',team(mon('Mew','recycle'),quiet()),team(quiet(),quiet()),turn(idle,idle),['|-fail|']);
for(const [batch,list] of Object.entries(rows)){fs.writeFileSync(new URL(`../../../crates/engine/src/effects/moves/batonpass/${batch}.tsv`,import.meta.url),'# name\tseed\tp1\tp2\tsteps\tfinal seed\tlog\n'+list.join('\n')+'\n');console.log(`${batch}: ${list.length} scenarios`);}
