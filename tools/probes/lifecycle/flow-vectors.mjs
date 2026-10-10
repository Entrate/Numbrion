// Oracle vectors for lifecycle flows that need no move execution (owner L): switch and
// instaswitch actions, terastallize, forced replacement requests after faints, random drags
// (forceSwitchFlag -> dragIn), Revival Blessing, residual, endTurn and the win/tie checks.
// Each case starts with the real start sequence, then runs scripted rounds. The script is
// generated against the live oracle battle (it reads requestState and switch flags like a
// player would) and every operation is recorded so that the Rust test replays it verbatim.
//
// makeRequest is replaced by a recorder that only sets requestState: request construction
// belongs to owner C and has no effect on queue/PRNG state. Output:
// crates/engine/src/sim/lifecycle/vectors/flow.tsv
import fs from 'node:fs';
import path from 'node:path';
import {root, rng, pick, randomTeams, pack, startBattle, queueSummary, seedWords, monIndex, withoutFormeTera, realEndTurn, stateString} from './common.mjs';

withoutFormeTera();

const CASES = 300;
const r = rng(0xf10e);
const lines = ['# case\tseed\tp1\tp2\tstep\top\tresult\tseedAfter\tqueue\teffectOrder\tstate'];
const ROUNDS = 3;

function mon(b, idx) {
	return b.sides[idx < 6 ? 0 : 1].pokemon.find(p => monIndex(p) === idx);
}

/** battle.turnLoop() minus the final endTurn (Rust replays the same decomposition). */
function loopNoEnd(b) {
	b.add('');
	b.add('t:', 0);
	if (b.requestState) b.requestState = '';
	if (!b.midTurn) {
		b.queue.insertChoice({choice: 'beforeTurn'});
		b.queue.addChoice({choice: 'residual'});
		b.midTurn = true;
	}
	let action;
	while ((action = b.queue.shift())) {
		b.runAction(action);
		if (b.requestState || b.ended) return 'stop';
	}
	return 'empty';
}

function run(b, op) {
	const [name, x, y] = op.split(' ');
	const q = b.queue;
	switch (name) {
	case 'faint': mon(b, +x).faint(); return '';
	case 'force': mon(b, +x).forceSwitchFlag = true; return '';
	case 'swflag': mon(b, +x).switchFlag = true; return '';
	case 'addsw': q.addChoice({choice: 'switch', pokemon: mon(b, +x), target: mon(b, +y)}); return '';
	case 'addinsw': q.addChoice({choice: 'instaswitch', pokemon: mon(b, +x), target: mon(b, +y)}); return '';
	case 'addtera': q.addChoice({choice: 'terastallize', pokemon: mon(b, +x)}); return '';
	case 'addrev': q.addChoice({choice: 'revivalblessing', pokemon: mon(b, +x), target: mon(b, +y)}); return '';
	case 'updatespeed': b.updateSpeed(); return '';
	case 'stash': b.heldQueue = q.list; q.list = []; return '';
	case 'commitsort':
		q.sort();
		q.list.push(...b.heldQueue);
		b.heldQueue = [];
		return '';
	case 'speed': mon(b, +x).storedStats.spe = +y; return '';
	case 'loop': return loopNoEnd(b);
	case 'endturn':
		realEndTurn.call(b);
		b.midTurn = false;
		q.clear();
		return '';
	default: throw new Error(op);
	}
}

const ACTIVE = b => b.sides.flatMap(s => s.active.filter(p => p).map(monIndex));
const ALIVE_BENCH = (b, side, taken) => b.sides[side].pokemon.slice(2).filter(p => !p.fainted && !taken.includes(monIndex(p))).map(monIndex);

for (let c = 0; c < CASES; c++) {
	const teams = randomTeams(r);
	const packed = teams.map(t => t.map(pack).join(']'));
	const seed = [1 + Math.floor(r() * 65535), Math.floor(r() * 65536), Math.floor(r() * 65536), Math.floor(r() * 65536)].join(',');
	startBattle(seed, packed, {
		beforeEndTurn(b) {
			b.makeRequest = function (type) {
				this.requestState = type || this.requestState;
			};
			let step = 0;
			const exec = op => {
				const result = run(b, op);
				lines.push([c, seed, packed[0], packed[1], step++, op, result || '-', seedWords(b), queueSummary(b), b.effectOrder, stateString(b)].join('\t'));
				return result;
			};
			exec('endturn'); // the real endTurn that closes the start action's pseudo-turn
			for (let round = 0; round < ROUNDS && !b.ended; round++) {
				// 1. things that happen "during" the turn
				if (r() < 0.10) {
					// a wipe: every living Pokemon of one side (sometimes both) faints, active ones first
					const sides = r() < 0.4 ? (r() < 0.5 ? [0, 1] : [1, 0]) : [r() < 0.5 ? 0 : 1];
					for (const s of sides) {
						for (const p of [...b.sides[s].pokemon]) if (!p.fainted && !p.faintQueued) exec(`faint ${monIndex(p)}`);
					}
				}
				for (const idx of ACTIVE(b)) {
					const p = mon(b, idx);
					if (p.fainted || p.faintQueued) continue;
					const roll = r();
					if (roll < 0.12) exec(`faint ${idx}`);
					else if (roll < 0.20) exec(`force ${idx}`);
					else if (roll < 0.26) exec(`swflag ${idx}`);
					else if (roll < 0.34) exec(`speed ${idx} ${pick(r, [100, 150, 200, 250])}`);
				}
				// 2. the chosen actions of the turn (switches / tera), then commitChoices' sort
				exec('updatespeed');
				exec('stash');
				const taken = [];
				for (let s = 0; s < 2; s++) {
					const chosen = [];
					for (const p of b.sides[s].active) {
						const idx = monIndex(p);
						if (p.fainted || r() < 0.55) continue;
						const roll = r();
						const bench = ALIVE_BENCH(b, s, taken);
						if (roll < 0.6 && bench.length) {
							const t = pick(r, bench);
							taken.push(t);
							exec(`addsw ${idx} ${t}`);
						} else if (!b.sides[s].pokemon.some(m => m.terastallized) && !chosen.includes('tera')) {
							chosen.push('tera');
							exec(`addtera ${idx}`);
						}
					}
				}
				if (r() < 0.2) {
					// Revival Blessing on a fainted benched mon (the slot condition is absent, as in Node-only runs).
					for (const s of [0, 1]) {
						const user = b.sides[s].active.find(p => p && !p.fainted);
						const fainted = b.sides[s].pokemon.find(p => p.fainted);
						if (user && fainted) { exec(`addrev ${monIndex(user)} ${monIndex(fainted)}`); break; }
					}
				}
				exec('commitsort');
				// 3. run the turn; answer switch requests like a player would
				let state = exec('loop');
				let guard = 0;
				while (state === 'stop' && !b.ended && guard++ < 6) {
					if (b.requestState === 'switch') {
						exec('updatespeed');
						exec('stash');
						for (let s = 0; s < 2; s++) {
							const taken2 = [];
							for (const p of b.sides[s].active) {
								if (!p.switchFlag) continue;
								const bench = ALIVE_BENCH(b, s, taken2);
								if (!bench.length) continue;
								const t = pick(r, bench);
								taken2.push(t);
								exec(`addinsw ${monIndex(p)} ${t}`);
							}
						}
						exec('commitsort');
					}
					state = exec('loop');
				}
				if (b.ended) break;
				if (state === 'empty') exec('endturn');
				else break;
			}
		},
	});
}
const out = path.join(root, 'crates/engine/src/sim/lifecycle/vectors/flow.tsv');
fs.mkdirSync(path.dirname(out), {recursive: true});
fs.writeFileSync(out, lines.join('\n') + '\n');
console.log(`${lines.length - 1} flow rows over ${CASES} battles`);
