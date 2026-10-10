// Whole start() through turn 1, with real rule hooks and real cached requests.
// Deliberately inert abilities/no items isolate the lifecycle from effect batches.
import fs from 'node:fs';
import path from 'node:path';
import {root, sim, FORMAT_ID, rng, randomTeams, pack, seedWords, stateString, withoutFormeTera} from './common.mjs';

withoutFormeTera();
const r = rng(0x57a27);
const lines = ['# case\tseed\tp1\tp2\tseedAfter\tstate\tlog\trequest1\trequest2'];
for (let c = 0; c < 50; c++) {
	const packed = randomTeams(r).map(t => t.map(pack).join(']'));
	const seed = Array.from({length: 4}, () => Math.floor(r() * 65536)).join(',');
	const b = new sim.Battle({formatid: FORMAT_ID, seed, send: () => {}});
	b.setPlayer('p1', {name: 'A', team: packed[0]});
	b.setPlayer('p2', {name: 'B', team: packed[1]});
	const log = b.log.map(line => line.startsWith('|t:|') ? '|t:|' : line).join('~');
	lines.push([c, seed, ...packed, seedWords(b), stateString(b), log, ...b.sides.map(s => JSON.stringify(s.activeRequest))].join('\t'));
}
fs.writeFileSync(path.join(root, 'crates/engine/src/sim/lifecycle/vectors/startup.tsv'), lines.join('\n') + '\n');
console.log('50 full startup boundaries');
