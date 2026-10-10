// Pinned oracle vectors for accuracy ordering/draws and declarative self/secondary effects.
import fs from 'node:fs';
import {fileURLToPath} from 'node:url';
import {loadSim} from '../../oracle/lib/common.mjs';
const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const team = 'Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric]' +
  'Raichu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric';
function fixture(seed) {
  const b = new sim.Battle({formatid:'gen9randomdoublesbattle', seed});
  b.start = () => {};
  b.setPlayer('p1',{team}); b.setPlayer('p2',{team});
  b.p1.foe=b.p2; b.p2.foe=b.p1;
  const mons = [b.p1.pokemon[0],b.p1.pokemon[1],b.p2.pokemon[0],b.p2.pokemon[1]];
  b.p1.active = mons.slice(0,2); b.p2.active = mons.slice(2);
  mons.forEach((m,i) => {m.isActive=true; m.position=i%2;});
  b.prng = new sim.PRNG(seed);
  return {b,mons};
}
const seeds = ['1,2,3,4','5,6,7,8','65535,65535,65535,65535'];
const out=['# stage\tseed\tmove\taccuracyStage\tevasionStage\ttargets\tresult\tseedAfter'];
for (const seed of seeds) for (const move of ['thunderbolt','focusblast','aurasphere','dragondarts']) {
  for (const a of [-6,-3,0,3,6]) for (const e of [-6,-3,0,3,6]) for (const t of ['2','2,3']) {
    const {b,mons}=fixture(seed);
    mons[0].boosts.accuracy=a;
    mons[2].boosts.evasion=mons[3].boosts.evasion=e;
    const mv=b.dex.getActiveMove(move);
    const result=b.actions.hitStepAccuracy(t.split(',').map(i=>mons[i]),mons[0],mv);
    out.push(['A',seed,move,a,e,t,result.map(Boolean).map(Number).join(',')+'/'+Number(!!mv.smartTarget),b.prng.getSeed()].join('\t'));
  }
}
for (const seed of seeds) for (const move of ['closecombat','leafstorm','scale shot','fierydance','psychic','flamecharge']) {
  for (const target of ['pokemon','null','false']) {
    const {b,mons}=fixture(seed); const mv=b.dex.getActiveMove(move);
    // Exercise the unconditional self and secondary rolls independently of damage/lifecycle.
    const t=target==='pokemon'?mons[2]:target==='null'?null:false;
    b.setActiveMove(mv,mons[0],mons[2]);
    b.actions.selfDrops([t],mons[0],mv,mv,false);
    b.actions.secondaries([t],mons[0],mv,mv,false);
    out.push(['E',seed,mv.id,0,0,target,JSON.stringify([mons[0].boosts,mons[2].boosts,mv.selfDropped||false]),b.prng.getSeed()].join('\t'));
  }
}
fs.writeFileSync(fileURLToPath(new URL('../../../crates/engine/src/actions/moves/tests/hit-stage-vectors.tsv',import.meta.url)),out.join('\n')+'\n');
console.log(`${out.length-1} hit-stage vectors`);
