// Oracle vectors for the start action and the initial switch-ins (owner L):
// battle.ts:1909-1973, 2674-2707 and battle-actions.ts:62-189 run for real on the pinned
// Showdown, stopped at the first endTurn. Output: crates/engine/src/sim/lifecycle/vectors/start.tsv
import fs from 'node:fs';
import path from 'node:path';
import {root, rng, randomTeams, pack, startBattle, stateSummary} from './common.mjs';

const CASES = 300;
const r = rng(0x5eed1);
const lines = ['# case\tseed\tp1\tp2\tstep\tchoice\tseedAfter\teffectOrder\tqueue\tactive\tparty\tspeeds\torders\tspeedOrder'];
for (let c = 0; c < CASES; c++) {
	const teams = randomTeams(r);
	const packed = teams.map(t => t.map(pack).join(']'));
	const seed = [1 + Math.floor(r() * 65535), Math.floor(r() * 65536), Math.floor(r() * 65536), Math.floor(r() * 65536)].join(',');
	let step = 0;
	startBattle(seed, packed, {
		afterAction(b, choice) {
			const s = stateSummary(b);
			lines.push([c, seed, packed[0], packed[1], step++, choice, s.seed, s.effectOrder, s.queue, s.active, s.party, s.speeds, s.orders, s.speedOrder].join('\t'));
		},
	});
}
const out = path.join(root, 'crates/engine/src/sim/lifecycle/vectors/start.tsv');
fs.mkdirSync(path.dirname(out), {recursive: true});
fs.writeFileSync(out, lines.join('\n') + '\n');
console.log(`${lines.length - 1} start-sequence rows over ${CASES} battles`);
