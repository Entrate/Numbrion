// Pure helper vectors from the real pinned Showdown: BattleActions.combineResults, Math.round and
// Battle.clampIntRange. usage: node tools/probes/moves/pure.mjs [path-to-pokemon-showdown]
import fs from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';

const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const { BattleActions } = createRequire(sim.root + '/package.json')('./dist/sim/battle-actions');
const b = new sim.Battle({ formatid: 'gen9randomdoublesbattle', seed: '1,2,3,4' });
const enc = v => {
	if (v === undefined) return 'U';
	if (v === null) return 'N';
	if (v === '') return 'S';
	if (v === true) return 'T';
	if (v === false) return 'F';
	if (typeof v === 'number') return Number.isNaN(v) ? '#NaN' : Object.is(v, -0) ? '#-0' : `#${v}`;
	throw new Error(`unencodable ${String(v)}`);
};
const values = [undefined, '', null, true, false, 0, 1, 5, 12.5, -3, NaN, 0.5];
const out = ['# kind\ta\tb\tc\tresult'];
for (const l of values) {
	for (const r of values) {
		out.push(['C', enc(l), enc(r), '-', enc(BattleActions.prototype.combineResults.call(b.actions, l, r))].join('\t'));
	}
}
const rounds = [0, 0.4, 0.5, 0.6, 1.5, 2.5, 3.5, 0.49999999999999994, 4503599627370495.5, 4503599627370496, 123456.5, 61.25, 61.5,
	1e21, 7.499999999999999, 30.5, 8.5];
for (const x of rounds) out.push(['J', `#${x}`, '-', '-', `#${Math.round(x)}`].join('\t'));
const clampInputs = [0, 0.5, 1, 1.9, -0.5, -1, -6.5, 6.9, 7, 100, NaN, 3.0000001, -7];
for (const x of clampInputs) {
	for (const [min, max] of [[1, undefined], [-6, 6], [undefined, undefined], [0, 10]]) {
		out.push(['K', `#${x}`, min === undefined ? '-' : min, max === undefined ? '-' : max, enc(b.clampIntRange(x, min, max))].join('\t'));
	}
}
const dest = fileURLToPath(new URL('../../../crates/engine/src/actions/moves/tests/pure-vectors.tsv', import.meta.url));
fs.writeFileSync(dest, out.join('\n') + '\n');
console.log(`${out.length - 1} pure vectors`);
