// Oracle vectors for the BattleQueue operations that do not need move execution (owner L):
// insertChoice (inclusive tie roll), addChoice, sort (speedSort of the whole list),
// prioritizeAction, changeAction, cancelAction, willAct/willSwitch, shift, for every action
// kind that resolves without a move: beforeTurn, residual, runSwitch, terastallize,
// instaswitch, switch. Each case first runs the real start sequence (four leads in), then a
// random operation list. Output: crates/engine/src/sim/lifecycle/vectors/queue.tsv
import fs from 'node:fs';
import path from 'node:path';
import {root, rng, pick, randomTeams, pack, startBattle, queueSummary, seedWords, monIndex} from './common.mjs';

const CASES = 400;
const r = rng(0x0a11ce);
const ACTIVE = [0, 1, 6, 7];
const lines = ['# case\tseed\tp1\tp2\tstep\top\tresult\tseedAfter\tqueue'];

function mon(b, idx) {
	return b.sides[idx < 6 ? 0 : 1].pokemon.find(p => monIndex(p) === idx);
}

function benchOf(b, idx) {
	const side = b.sides[idx < 6 ? 0 : 1];
	return side.pokemon.slice(2).filter(p => !p.fainted).map(monIndex);
}

function genOps(b) {
	const ops = [];
	const n = 5 + Math.floor(r() * 10);
	for (let i = 0; i < n; i++) {
		const roll = r();
		const a = pick(r, ACTIVE);
		if (roll < 0.10) ops.push('ibt');
		else if (roll < 0.17) ops.push('ares');
		else if (roll < 0.34) ops.push(`irs ${a}`);
		else if (roll < 0.44) ops.push(`itera ${a}`);
		else if (roll < 0.50) ops.push(`atera ${a}`);
		else if (roll < 0.58) ops.push(`iis ${a} ${pick(r, benchOf(b, a))}`);
		else if (roll < 0.64) ops.push(`isw ${a} ${pick(r, benchOf(b, a))}`);
		else if (roll < 0.70) ops.push(`cancel ${a}`);
		else if (roll < 0.76) ops.push('sort');
		else if (roll < 0.82) ops.push(`prio ${Math.floor(r() * 4)}`);
		else if (roll < 0.86) ops.push('shift');
		else if (roll < 0.90) ops.push(`chg ${a}`);
		else if (roll < 0.95) ops.push(`boost ${a} ${Math.floor(r() * 5) - 2}`);
		else ops.push(`stat ${a} ${pick(r, [100, 150, 200, 200, 250])}`);
	}
	return ops;
}

function will(b) {
	const act = b.queue.willAct();
	const idx = act ? b.queue.list.indexOf(act) : -1;
	const sw = ACTIVE.map(a => {
		const w = b.queue.willSwitch(mon(b, a));
		return w ? b.queue.list.indexOf(w) : -1;
	});
	return `act=${idx} sw=${sw.join(',')}`;
}

function run(b, op) {
	const [name, x, y] = op.split(' ');
	const q = b.queue;
	switch (name) {
	case 'ibt': q.insertChoice({choice: 'beforeTurn'}); return '';
	case 'ares': q.addChoice({choice: 'residual'}); return '';
	case 'irs': q.insertChoice({choice: 'runSwitch', pokemon: mon(b, +x)}); return '';
	case 'itera': q.insertChoice({choice: 'terastallize', pokemon: mon(b, +x)}); return '';
	case 'atera': q.addChoice({choice: 'terastallize', pokemon: mon(b, +x)}); return '';
	case 'iis': q.insertChoice({choice: 'instaswitch', pokemon: mon(b, +x), target: mon(b, +y)}); return '';
	case 'isw': q.insertChoice({choice: 'switch', pokemon: mon(b, +x), target: mon(b, +y)}); return '';
	case 'cancel': return String(q.cancelAction(mon(b, +x)));
	case 'sort': q.sort(); return '';
	case 'prio': {
		if (+x >= q.list.length) return 'skip';
		q.prioritizeAction(q.list[+x]);
		return '';
	}
	case 'shift': {
		const a = q.shift();
		return a ? a.choice : 'none';
	}
	case 'chg': q.changeAction(mon(b, +x), {choice: 'runSwitch'}); return '';
	case 'boost': mon(b, +x).boosts.spe = +y; return '';
	case 'stat': mon(b, +x).storedStats.spe = +y; return '';
	default: throw new Error(op);
	}
}

for (let c = 0; c < CASES; c++) {
	const teams = randomTeams(r);
	const packed = teams.map(t => t.map(pack).join(']'));
	const seed = [1 + Math.floor(r() * 65535), Math.floor(r() * 65536), Math.floor(r() * 65536), Math.floor(r() * 65536)].join(',');
	let ops = null;
	startBattle(seed, packed, {
		beforeEndTurn(b) {
			ops = genOps(b);
			ops.forEach((op, step) => {
				const result = run(b, op);
				lines.push([c, seed, packed[0], packed[1], step, op, `${result}${result ? ' ' : ''}${will(b)}`, seedWords(b), queueSummary(b)].join('\t'));
			});
		},
	});
}
const out = path.join(root, 'crates/engine/src/sim/lifecycle/vectors/queue.tsv');
fs.mkdirSync(path.dirname(out), {recursive: true});
fs.writeFileSync(out, lines.join('\n') + '\n');
console.log(`${lines.length - 1} queue-operation rows over ${CASES} battles`);
