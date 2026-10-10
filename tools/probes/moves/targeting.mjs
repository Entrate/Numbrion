// Targeting vectors from the real pinned Showdown (Battle.getTarget / getRandomTarget / validTargetLoc /
// Pokemon.getLocOf / getAtLoc / getSmartTargets / getMoveTargets without redirect handlers).
// The auto-start is suspended exactly like the other query probes; actives are set by hand.
// usage: node tools/probes/moves/targeting.mjs [path-to-pokemon-showdown]
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';

const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const team = 'Pikachu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric]' +
	'Raichu||lightball|static|thunderbolt|Serious||M|||100|,,,,,Electric';
const SEED = '1,2,3,4';
const MOVES = [
	'accelerock', 'swordsdance', 'aurasphere', 'dazzlinggleam', 'tailwind', 'earthquake', 'helpinghand',
	'raindance', 'howl', 'stealthrock', 'struggle', 'dragondarts',
];
const TYPES = ['normal', 'self', 'any', 'allAdjacentFoes', 'allySide', 'allAdjacent', 'adjacentAlly', 'all', 'allies',
	'foeSide', 'randomNormal'];

function fixture(state) {
	const b = new sim.Battle({ formatid: 'gen9randomdoublesbattle', seed: SEED });
	b.start = () => {};
	b.setPlayer('p1', { team });
	b.setPlayer('p2', { team });
	b.p1.foe = b.p2;
	b.p2.foe = b.p1;
	const mons = [b.p1.pokemon[0], b.p1.pokemon[1], b.p2.pokemon[0], b.p2.pokemon[1]];
	b.p1.active = [mons[0], mons[1]];
	b.p2.active = [mons[2], mons[3]];
	mons.forEach((m, i) => {
		m.isActive = true;
		m.position = i % 2;
		m.hp = state[i] === 0 ? m.maxhp : 0;
		m.fainted = state[i] === 2;
	});
	return { b, mons };
}
const idx = (mons, p) => (p ? String(mons.indexOf(p)) : '-');
const reseed = b => { b.prng = new sim.PRNG(SEED); };
const states = [];
for (let s = 0; s < 81; s++) states.push([s % 3, Math.floor(s / 3) % 3, Math.floor(s / 9) % 3, Math.floor(s / 27) % 3]);
const out = ['# kind\tstate\tuser\tmove\tloc\torig\tstalwart\ttwoturn\tresult\tseed'];
const row = (...a) => out.push(a.join('\t'));

for (const state of states) {
	const { b, mons } = fixture(state);
	const key = state.join('');
	// Section A: getTarget, plain.
	for (let u = 0; u < 4; u++) {
		for (const move of MOVES) {
			for (let loc = -2; loc <= 2; loc++) {
				reseed(b);
				const r = b.getTarget(mons[u], move, loc);
				row('T', key, u, move, loc, '-', 0, 0, idx(mons, r), b.prng.getSeed());
			}
			// Section C: getRandomTarget.
			reseed(b);
			const r = b.getRandomTarget(mons[u], move);
			row('R', key, u, move, 0, '-', 0, 0, idx(mons, r), b.prng.getSeed());
		}
		// Section F: getSmartTargets (Dragon Darts) toward each possible target.
		for (let t = 0; t < 4; t++) {
			const mv = b.dex.getActiveMove('dragondarts');
			reseed(b);
			const list = mons[u].getSmartTargets(mons[t], mv);
			row('S', key, u, 'dragondarts', t, '-', 0, 0, list.map(p => mons.indexOf(p)).join(',') + '/' + (mv.smartTarget ? 1 : 0),
				b.prng.getSeed());
		}
		// Section E: location helpers.
		for (let t = 0; t < 4; t++) {
			reseed(b);
			row('L', key, u, '-', t, '-', 0, 0, mons[u].getLocOf(mons[t]), b.prng.getSeed());
		}
		for (let loc = -2; loc <= 2; loc++) {
			reseed(b);
			row('A', key, u, '-', loc, '-', 0, 0, idx(mons, mons[u].getAtLoc(loc)), b.prng.getSeed());
		}
	}
}
// Section B: tracksTarget (Stalwart), the twoturnmove self-location exception and originalTarget.
for (let s = 0; s < 81; s++) {
	const state = states[s];
	if (state.includes(1)) continue;
	const { b, mons } = fixture(state);
	const key = state.join('');
	for (let u = 0; u < 4; u++) {
		for (const stalwart of [0, 1]) {
			for (const twoturn of [0, 1]) {
				mons[u].ability = stalwart ? 'stalwart' : 'pressure';
				if (twoturn) mons[u].volatiles = { twoturnmove: { id: 'twoturnmove' } }; else mons[u].volatiles = {};
				for (const move of ['accelerock', 'aurasphere', 'dragondarts', 'struggle']) {
					for (let loc = -2; loc <= 2; loc++) {
						for (const orig of ['-', '0', '3']) {
							reseed(b);
							const original = orig === '-' ? undefined : mons[Number(orig)];
							const r = b.getTarget(mons[u], move, loc, original);
							row('T', key, u, move, loc, orig, stalwart, twoturn, idx(mons, r), b.prng.getSeed());
						}
					}
				}
			}
		}
		mons[u].volatiles = {};
		mons[u].ability = 'static';
	}
}
// Section D: validTargetLoc for every move target type.
{
	const { b, mons } = fixture([0, 0, 0, 0]);
	for (let u = 0; u < 4; u++) {
		for (const type of TYPES) {
			for (let loc = -3; loc <= 3; loc++) {
				reseed(b);
				row('V', '0000', u, type, loc, '-', 0, 0, b.validTargetLoc(loc, mons[u], type) ? 1 : 0, b.prng.getSeed());
			}
		}
	}
}
const dest = fileURLToPath(new URL('../../../crates/engine/src/actions/moves/tests/targeting-vectors.tsv', import.meta.url));
fs.writeFileSync(dest, out.join('\n') + '\n');
console.log(`${out.length - 1} targeting vectors`);
