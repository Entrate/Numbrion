// Real outside-caller execution, including hit/effect/recoil and recursive Dancer.
import fs from 'node:fs';
import {fileURLToPath} from 'node:url';
import {loadSim} from '../../oracle/lib/common.mjs';
const sim=loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const seeds=['1,2,3,4','5,6,7,8','65535,65535,65535,65535'];
const moves=['thunderbolt','aurasphere','dazzlinggleam','closecombat','leafstorm','dragondarts','bulletseed','populationbomb','drainpunch','doubleedge','swordsdance','howl','uturn','struggle'];
const out=['# seed\tmove\tstate\tseedAfter\tlog'];
for(const seed of seeds) for(const move of moves) {
  const team=`Pikachu|||dancer|${move}|Serious||M|||100|,,,,,Electric]Raichu|||dancer|${move}|Serious||M|||100|,,,,,Electric`;
  const b=new sim.Battle({formatid:'gen9randomdoublesbattle',seed}); b.start=()=>{};
  b.setPlayer('p1',{name:'Player 1',team}); b.setPlayer('p2',{name:'Player 2',team});
  b.p1.foe=b.p2;b.p2.foe=b.p1;
  const mons=[...b.p1.pokemon,...b.p2.pokemon];
  b.p1.active=mons.slice(0,2);b.p2.active=mons.slice(2);
  mons.forEach((m,i)=>{m.isActive=true;m.position=i%2;});
  b.prng=new sim.PRNG(seed);b.log=[];b.lastMoveLine=-1;
  b.actions.runMove(move,mons[0],1);
  const enc=m=>[m.hp,Object.values(m.boosts).join(','),m.lastMove?.id||'-',m.activeMoveActions,m.timesAttacked,m.moveSlots[0].pp,m.status||'-',String(m.moveThisTurnResult),m.switchFlag||'-'].join('/');
  out.push([seed,move,mons.map(enc).join(';'),b.prng.getSeed(),JSON.stringify(b.log)].join('\t'));
}
fs.writeFileSync(fileURLToPath(new URL('../../../crates/engine/src/actions/moves/tests/execution-vectors.tsv',import.meta.url)),out.join('\n')+'\n');console.log(`${out.length-1} outside execution vectors`);
