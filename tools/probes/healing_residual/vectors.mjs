// Real pinned callbacks and mutators, with HP/status varied independently of the turn loop.
import fs from 'node:fs';
import {fileURLToPath} from 'node:url';
import {loadSim} from '../../oracle/lib/common.mjs';
const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const packed = 'Pikachu|||dancer|thunderbolt|Serious||M|||100|,,,,,Electric';
const rows = ['# effect\tevent\thp\tstatus\tseed\tfinal_hp\tfinal_status\trelay\tfinal_seed\tlog'];
for (const [kind, id, event] of [
  ['abilities','shedskin','Residual'], ['abilities','regenerator','SwitchOut'],
  ['items','leftovers','Residual'], ['items','flameorb','Residual'], ['items','toxicorb','Residual'],
  ['moves','junglehealing','Hit'], ['moves','lunarblessing','Hit'], ['moves','takeheart','Hit'],
  ['moves','healpulse','Hit'],
]) for (const hp of [0, 1, 100, 211]) for (const status of ['', 'par', 'tox']) for (let n=1;n<=12;n++) {
  const seed = [n,2,3,4];
  const b = new sim.Battle({formatid:'gen9randomdoublesbattle',seed});
  b.setPlayer('p1',{name:'Alice',team:packed+']'+packed}); b.setPlayer('p2',{name:'Bob',team:packed+']'+packed});
  const p = b.sides[0].active[0];
  if (p.maxhp !== 211) throw new Error(`Unexpected maxhp ${p.maxhp}`);
  p.hp = hp; p.status = status;
  b.prng = new sim.PRNG(seed); b.log.length=0;
  const effect = b.dex[kind].get(id);
  const result = b.singleEvent(event,effect,null,p,p,effect);
  rows.push([id,event,hp,status||'-',seed.join(','),p.hp,p.status||'-',
    result===undefined?'undefined':String(result), b.prng.getSeed().replace(/^gen5,/,''),
    b.log.join('\\n')].join('\t'));
  b.destroy();
}
const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/abilities/healingresidual/vectors.tsv', import.meta.url));
fs.writeFileSync(dest, rows.join('\n')+'\n');
console.log(`Wrote ${rows.length-1} real callback vectors to ${dest}`);
