// Faint ordering, endpoint logs/outcomes, generic Tera, and turn-limit branch vectors.
// Calls the real pinned methods after the initial runSwitch batch, without endTurn or
// request construction. No moves execute and no effect hooks are intercepted.
import fs from 'node:fs';
import path from 'node:path';
import {root, rng, randomTeams, pack, startBattle, seedWords, stateString, monIndex, withoutFormeTera} from './common.mjs';

withoutFormeTera();
const r = rng(0xe0d9);
const scripts = [
	'wipe 0;messages 0 0 1',
	'wipe 1;messages 0 0 1',
	'wipe 0;wipe 1;messages 0 0 1',
	'wipe 1;wipe 0;messages 0 0 1',
	'wipe 0;wipe 1;messages 1 0 1',
	'wipe 1;wipe 0;messages 1 0 1',
	'wipe 0;wipe 1;messages 0 0 0;messages 0 1 1',
	'faint 0;messages 0 0 0;checkfainted',
	'tera 0;faint 0;messages 0 0 0;checkfainted',
	'tera 6;faint 6;messages 0 0 0;checkfainted',
	'cap 0;faint 0;messages 0 0 0',
	'faint 0;faint 0;messages 0 0 0;messages 0 0 1',
	'tie;tie;messages 0 1 1',
	'lose 0;lose 1',
	'tiebreak',
	'fraction 0 0.5;fraction 1 0.25;tiebreak',
	'faint 0;messages 0 0 0;tiebreak',
	'left 0;left 1;messages 0 1 1',
	...[100, 101, 499, 500, 899, 900, 989, 990, 999, 1000, 1001].map(t => `limit ${t}`),
];
const lines = ['# case\tseed\tp1\tp2\tops\tresult\tseedAfter\tstate\tlog\toutcome'];
let caseNo = 0;
for (let repetition = 0; repetition < 10; repetition++) {
	const packed = randomTeams(r).map(t => t.map(pack).join(']'));
	const seed = Array.from({length: 4}, () => Math.floor(r() * 65536)).join(',');
	for (const ops of scripts) {
		const b = startBattle(seed, packed);
		b.log = [];
		b.sentLogPos = 0;
		const mon = idx => b.sides[idx < 6 ? 0 : 1].pokemon.find(p => monIndex(p) === idx);
		const result = [];
		for (const op of ops.split(';')) {
			const [name, x, y, z] = op.split(' ');
			let value;
			switch (name) {
			case 'wipe': for (const p of b.sides[+x].pokemon) p.faint(); break;
			case 'faint': mon(+x).faint(); break;
			case 'messages': value = b.faintMessages(!!+x, !!+y, !!+z); break;
			case 'checkfainted': b.checkFainted(); break;
			case 'tera': b.actions.terastallize(mon(+x)); break;
			case 'cap': b.sides[+x].totalFainted = 100; break;
			case 'left': b.sides[+x].pokemonLeft = 0; break;
			case 'tie': value = b.tie(); break;
			case 'lose': value = b.lose(b.sides[+x]); break;
			case 'fraction': for (const p of b.sides[+x].pokemon) p.hp = Math.floor(p.maxhp * +y); break;
			case 'tiebreak': value = b.tiebreak(); break;
			case 'limit': b.turn = +x; value = !!b.maybeTriggerEndlessBattleClause([], []); break;
			default: throw new Error(op);
			}
			result.push(value === undefined ? '-' : String(value));
		}
		const winner = b.winner === 'A' ? 0 : b.winner === 'B' ? 1 : '-';
		const outcome = b.ended ? `${winner}:${b.turn}:${b.sides.map(s => s.pokemonLeft).join(',')}` : '-';
		lines.push([caseNo++, seed, ...packed, ops, result.join(','), seedWords(b), stateString(b), b.log.join('~'), outcome].join('\t'));
	}
}
fs.writeFileSync(path.join(root, 'crates/engine/src/sim/lifecycle/vectors/boundary.tsv'), lines.join('\n') + '\n');
console.log(`${caseNo} boundary cases`);
