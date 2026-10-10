// Oracle vectors for move actions in the queue (owner L): resolveAction's expansion
// (terastallize + move, Struggle, Recharge), FractionalPriority, random targets, getActionSpeed
// (target draws, ModifyPriority, speed), the commit sort, the gen-8 dynamic re-sort after every
// action (updateSpeed + getActionSpeed of every queued action + sort) and the consequences of
// mid-turn faints (fainted Pokemon's queued moves keep drawing).
//
// BattleActions.runMove is replaced by a no-op (optionally faints a chosen Pokemon "during"
// the move) so the vectors isolate the queue; owner M's move execution is not involved.
// Output: crates/engine/src/sim/lifecycle/vectors/move.tsv
import fs from 'node:fs';
import path from 'node:path';
import {
	root, rng, pick, randomTeams, pack, startBattle, queueSummary, seedWords, monIndex, withoutFormeTera, realEndTurn,
	stateString, Dex,
} from './common.mjs';

withoutFormeTera();
const CASES = 300;
const r = rng(0xa07e5);
const lines = ['# case\tseed\tp1\tp2\tstep\top\tresult\tseedAfter\tqueue\teffectOrder\tstate'];
const ROUNDS = 3;

function mon(b, idx) {
	return b.sides[idx < 6 ? 0 : 1].pokemon.find(p => monIndex(p) === idx);
}

let pendingFaint = null;

function run(b, op) {
	const [name, x, y, z, w] = op.split(' ');
	const q = b.queue;
	switch (name) {
	case 'faint': mon(b, +x).faint(); return '-';
	case 'speed': mon(b, +x).storedStats.spe = +y; return '-';
	case 'updatespeed': b.updateSpeed(); return '-';
	case 'stash': b.heldQueue = q.list; q.list = []; return '-';
	case 'commitsort':
		q.sort();
		q.list.push(...b.heldQueue);
		b.heldQueue = [];
		return '-';
	case 'prelude':
		if (!b.midTurn) {
			q.insertChoice({choice: 'beforeTurn'});
			q.addChoice({choice: 'residual'});
			b.midTurn = true;
		}
		return '-';
	case 'addmove': {
		const p = mon(b, +x);
		const choice = {choice: 'move', pokemon: p, moveid: y, targetLoc: +z};
		if (w === 'tera') choice.terastallize = p.teraType;
		q.addChoice(choice);
		return '-';
	}
	case 'addstruggle': q.addChoice({choice: 'move', pokemon: mon(b, +x), moveid: 'struggle'}); return '-';
	case 'addrecharge': q.addChoice({choice: 'move', pokemon: mon(b, +x), moveid: 'recharge', targetLoc: +y}); return '-';
	case 'addsw': q.addChoice({choice: 'switch', pokemon: mon(b, +x), target: mon(b, +y)}); return '-';
	case 'addinsw': q.addChoice({choice: 'instaswitch', pokemon: mon(b, +x), target: mon(b, +y)}); return '-';
	case 'stepfaint':
		pendingFaint = +x;
	// fall through
	case 'step': {
		const a = q.shift();
		if (!a) return 'none';
		b.runAction(a);
		pendingFaint = null;
		return `${a.choice}${a.pokemon ? ':' + monIndex(a.pokemon) : ''}`;
	}
	case 'clearrequest': b.requestState = ''; return '-';
	case 'endturn':
		realEndTurn.call(b);
		b.midTurn = false;
		q.clear();
		return '-';
	default: throw new Error(op);
	}
}

const MOVES = new Map();
function moveData(id) {
	if (!MOVES.has(id)) MOVES.set(id, Dex.moves.get(id));
	return MOVES.get(id);
}

const CHOOSABLE = new Set(['normal', 'any', 'adjacentAlly', 'adjacentAllyOrSelf', 'adjacentFoe']);

for (let c = 0; c < CASES; c++) {
	const teams = randomTeams(r);
	const packed = teams.map(t => t.map(pack).join(']'));
	const moveLists = teams.map(t => t.map(s => s.moves));
	const seed = [1 + Math.floor(r() * 65535), Math.floor(r() * 65536), Math.floor(r() * 65536), Math.floor(r() * 65536)].join(',');
	startBattle(seed, packed, {
		beforeEndTurn(b) {
			b.makeRequest = function (type) {
				this.requestState = type || this.requestState;
			};
			b.actions.runMove = function () {
				if (pendingFaint !== null) {
					const p = mon(b, pendingFaint);
					pendingFaint = null;
					if (p) p.faint();
				}
			};
			let step = 0;
			const exec = op => {
				const result = run(b, op);
				lines.push([c, seed, packed[0], packed[1], step++, op, result, seedWords(b), queueSummary(b), b.effectOrder, stateString(b)].join('\t'));
				return result;
			};
			exec('endturn');
			for (let round = 0; round < ROUNDS && !b.ended; round++) {
				if (r() < 0.25) {
					const p = pick(r, b.sides.flatMap(s => s.active).filter(p => !p.fainted && !p.faintQueued));
					if (p) exec(`faint ${monIndex(p)}`);
				}
				for (const p of b.sides.flatMap(s => s.active)) {
					if (r() < 0.2) exec(`speed ${monIndex(p)} ${pick(r, [100, 150, 200, 250])}`);
				}
				exec('updatespeed');
				exec('stash');
				const teraUsed = [false, false];
				const taken = [];
				for (let s = 0; s < 2; s++) {
					for (const p of b.sides[s].active) {
						if (p.fainted || p.faintQueued) continue;
						const idx = monIndex(p);
						const roll = r();
						const bench = b.sides[s].pokemon.slice(2).filter(m => !m.fainted && !taken.includes(monIndex(m)));
						if (roll < 0.14 && bench.length) {
							const t = pick(r, bench);
							taken.push(monIndex(t));
							exec(`addsw ${idx} ${monIndex(t)}`);
						} else if (roll < 0.19) {
							exec(`addstruggle ${idx}`);
						} else if (roll < 0.25) {
							exec(`addrecharge ${idx} ${pick(r, [0, -1, 1, 2, -2])}`);
						} else {
							const moves = moveLists[s][monIndex(p) % 6];
							const id = pick(r, moves);
							const md = moveData(id);
							let loc = 0;
							if (CHOOSABLE.has(md.target) && r() < 0.8) {
								const valid = [-2, -1, 1, 2].filter(l => b.validTargetLoc(l, p, md.target));
								if (valid.length) loc = pick(r, valid);
							}
							let tera = '';
							if (!teraUsed[s] && !b.sides[s].pokemon.some(m => m.terastallized) && p.canTerastallize && r() < 0.35) {
								tera = ' tera';
								teraUsed[s] = true;
							}
							exec(`addmove ${idx} ${id} ${loc}${tera}`);
						}
					}
				}
				exec('commitsort');
				exec('prelude');
				let guard = 0;
				while ((b.queue.list.length || b.requestState === "switch") && !b.ended && guard++ < 60) {
					if (b.requestState === 'switch') {
						exec('updatespeed');
						exec('stash');
						for (let s = 0; s < 2; s++) {
							const taken2 = [];
							for (const p of b.sides[s].active) {
								if (!p.switchFlag) continue;
								const bench = b.sides[s].pokemon.slice(2).filter(m => !m.fainted && !taken2.includes(monIndex(m)));
								if (!bench.length) continue;
								const t = pick(r, bench);
								taken2.push(monIndex(t));
								exec(`addinsw ${monIndex(p)} ${monIndex(t)}`);
							}
						}
						exec('commitsort');
						exec('clearrequest');
						continue;
					}
					const next = b.queue.list[0];
					if (next.choice === 'move' && next.pokemon.isActive && !next.pokemon.fainted && r() < 0.15) {
						const victims = b.sides.flatMap(s => s.active).filter(p => !p.fainted && !p.faintQueued);
						if (victims.length) {
							exec(`stepfaint ${monIndex(pick(r, victims))}`);
							continue;
						}
					}
					exec('step');
				}
				if (b.ended) break;
				if (!b.queue.list.length && !b.requestState) exec('endturn');
				else break;
			}
		},
	});
}
const out = path.join(root, 'crates/engine/src/sim/lifecycle/vectors/move.tsv');
fs.mkdirSync(path.dirname(out), {recursive: true});
fs.writeFileSync(out, lines.join('\n') + '\n');
console.log(`${lines.length - 1} move-queue rows over ${CASES} battles`);
