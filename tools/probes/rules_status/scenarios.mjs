// rules_status batch: directed gen9randomdoublesbattle scenarios replayed on the pinned Showdown.
// Output: crates/engine/src/effects/conditions/rulesstatus/scenarios.txt, consumed by the scenario test in
// rulesstatus/tests.rs (exact log + PRNG seed after every decision boundary). The same file format is used by
// tools/probes/hazards_screens/scenarios.mjs (see its header for the record layout).
//
//   node tools/probes/rules_status/scenarios.mjs [path/to/pokemon-showdown]
//
// What the scenarios reach (the effects the batch ports behaviourally; the six status conditions, flinch and
// the format rules are reached by every scenario):
//   yawn_sleep_clause     Yawn move + volatile: expiry after the second residual, restart/fail on an already
//                         yawned or statused target, Sleep Clause Mod blocking the second Yawn sleep.
//   yawn_blockers         Yawn into Electric Terrain (TryAddVolatile), Insomnia / Vital Spirit / Comatose
//                         (silent onSetStatus refusal at expiry), switching out.
//   rest_branches         Rest: full-HP fail, Insomnia / Vital Spirit / Comatose failures, replacing a burn,
//                         the fixed 2 turns of sleep (time = startTime = 3), Early Bird.
//   status_lotteries      Dire Claw (50%, 100% with Serene Grace) and Tri Attack (20%, 40%) against immune and
//                         vulnerable types, Sleep Clause, Shield Dust.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';

const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const { Dex } = createRequire(sim.root + '/package.json')('./dist/sim');
const scope = JSON.parse(fs.readFileSync(new URL('../../../data/scope.json', import.meta.url)));
const toID = s => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');
const inScope = {
	species: new Set(scope.species.map(s => s.id)),
	moves: new Set(scope.moves.map(s => s.id)),
	abilities: new Set(scope.abilities.map(s => s.id)),
	items: new Set(scope.items.map(s => s.id)),
};

/** A fully specified set (explicit gender so the Pokemon constructor never draws from the PRNG). */
function mon(species, ability, moves, o = {}) {
	const sp = Dex.species.get(species);
	if (!inScope.species.has(sp.id)) throw new Error(`species ${sp.id} outside the generated scope`);
	if (o.item && !inScope.items.has(toID(o.item))) throw new Error(`item ${o.item} outside scope`);
	if (!inScope.abilities.has(toID(ability))) throw new Error(`ability ${ability} outside scope (${sp.name})`);
	for (const m of moves) if (!inScope.moves.has(toID(m))) throw new Error(`move ${m} outside scope`);
	return {
		name: sp.name, species: sp.name, item: o.item ?? '', ability, moves, nature: 'Serious',
		gender: sp.gender || 'M', level: o.level ?? 100,
		evs: o.evs ?? { hp: 0, atk: 0, def: 0, spa: 0, spd: 0, spe: 0 },
		ivs: { hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31 },
	};
}
const bulk = { hp: 252, atk: 0, def: 128, spa: 0, spd: 128, spe: 0 };
const repeat = (n, a, b) => Array.from({ length: n }, () => [a, b]);

const S = [];
const scenario = (name, seed, p1, p2, turns) => S.push({ name, seed, teams: [sim.Teams.pack(p1), sim.Teams.pack(p2)], turns });

// Weak level-50 attackers: nobody faints, but everybody takes damage (so Rest and Soft-Boiled have work to do).
const attackers = () => [
	mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Earth Power', 'Swords Dance'], { level: 50 }),
	mon('Gengar', 'Levitate', ['Shadow Ball', 'Sludge Bomb', 'Thunderbolt', 'Hypnosis'], { level: 50 }),
	mon('Arcanine', 'Intimidate', ['Flare Blitz', 'Will-O-Wisp', 'Toxic', 'Thunder Wave'], { level: 50 }),
	mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'], { level: 50 }),
];

// 1. Yawn on both foes, expiry (sleep for the first, Sleep Clause refusal for the second), Yawn against a
//    sleeping / statused / already yawned target, and Yawn against the foe that Sleep Clause keeps awake.
scenario('yawn_sleep_clause', [11, 22, 33, 44],
	[mon('Latios', 'Levitate', ['Yawn', 'Recover', 'Surf', 'Dazzling Gleam'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Yawn', 'Moonblast', 'Recover', 'Calm Mind'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'], { evs: bulk }),
	 mon('Gengar', 'Levitate', ['Yawn', 'Shadow Ball', 'Sludge Bomb', 'Calm Mind'], { evs: bulk })],
	[mon('Blissey', 'Serene Grace', ['Soft-Boiled', 'Toxic', 'Calm Mind', 'Thunder Wave'], { evs: bulk }),
	 mon('Snorlax', 'Thick Fat', ['Body Slam', 'Calm Mind', 'Swords Dance', 'Toxic'], { level: 50, evs: bulk }),
	 ...attackers().slice(0, 2)],
	[
		['move 1 1, move 1 2', 'move 4 1, move 3'],
		['move 2, move 3', 'move 1, move 1 1'],
		['move 1 1, move 1 2', 'move 1, move 1 1'],
		['move 1 1, move 1 2', 'move 1, move 1 1'],
		['move 2, move 3', 'move 1, move 1 1'],
		['move 1 1, move 1 2', 'move 1, move 1 1'],
		['move 1 1, move 1 2', 'move 1, move 1 1'],
		['move 2, move 3', 'move 1, move 1 1'],
		['move 1 2, move 1 1', 'move 1, move 1 1'],
		['move 1 1, move 1 2', 'move 1, move 1 1'],
	]);

// 2. Yawn blockers. p2 leads Pincurchin (Electric Surge) + Hypno (Insomnia): Yawn is stopped by Electric
//    Terrain's onTryAddVolatile / Insomnia's onTryAddVolatile. After the terrain is gone, Yawn lands on
//    Comatose mons and the expiry sleep is refused silently; a switch-out removes
//    the volatile. All p1 sets share one move order and so do all p2 sets (choices are positional).
const yawners = () => ['Clefable', 'Latios', 'Gengar', 'Scizor'].map(sp =>
	mon(sp, sp === 'Clefable' ? 'Magic Guard' : sp === 'Scizor' ? 'Technician' : 'Levitate', ['Yawn', 'Recover', 'Calm Mind', 'Thunder Wave'], { evs: bulk }));
const sleepers = () => [['Pincurchin', 'Electric Surge'], ['Hypno', 'Insomnia'], ['Komala', 'Comatose'], ['Vigoroth', 'Vital Spirit']].map(([sp, ab]) =>
	mon(sp, ab, ['Calm Mind', 'Recover', 'Swords Dance', 'Thunder Wave'], { evs: bulk }));
scenario('yawn_blockers', [101, 202, 303, 404], yawners(), sleepers(),
	[
		['move 1 1, move 1 2', 'move 3, move 3'],
		['move 1 2, move 1 1', 'move 2, move 2'],
		['move 1 1, move 1 2', 'switch 3, switch 4'],
		['move 1 1, move 1 2', 'move 3, move 3'],
		['move 1 1, move 1 2', 'move 3, move 3'],
		['move 1 1, move 1 2', 'move 3, move 3'],
		['move 3, move 3', 'move 3, move 3'],
		['move 1 1, move 1 2', 'move 3, move 3'],
		['move 3, move 3', 'switch 3, switch 4'],
		['move 4 2, move 1 1', 'move 3, move 3'],
		['move 1 2, move 1 1', 'move 3, move 3'],
		['move 3, move 3', 'move 3, move 3'],
	]);

// 3. Rest. Full HP (heal fail comes before the ability checks), Insomnia / Vital Spirit / Comatose, a burn
//    replaced by sleep, exactly two sleeping turns (time = 3), Early Bird shortening that.
const resters = () => [['Snorlax', 'Thick Fat'], ['Hypno', 'Insomnia'], ['Dodrio', 'Early Bird'], ['Vigoroth', 'Vital Spirit'], ['Komala', 'Comatose']].map(([sp, ab]) =>
	mon(sp, ab, ['Rest', 'Calm Mind', 'Swords Dance', 'Body Slam'], { evs: bulk }));
const hitters = () => [['Garchomp', 'Rough Skin'], ['Gengar', 'Levitate'], ['Arcanine', 'Intimidate'], ['Scizor', 'Technician']].map(([sp, ab]) =>
	mon(sp, ab, ['Body Slam', 'Will-O-Wisp', 'Swords Dance', 'Calm Mind'], { level: 50 }));
scenario('rest_branches', [7, 8, 9, 10], resters(), hitters(),
	[
		['move 1, move 1', 'move 3, move 3'],
		['move 3, move 3', 'move 1 1, move 1 2'],
		['move 1, move 1', 'move 3, move 3'],
		['move 1, switch 3', 'move 1 1, move 1 2'],
		['move 1, move 1', 'move 3, move 3'],
		['move 1, move 1', 'move 1 1, move 1 2'],
		['move 3, move 3', 'move 1 1, move 1 2'],
		['move 3, move 3', 'move 2 1, move 2 2'],
		['move 1, move 1', 'move 3, move 3'],
		['move 1, move 1', 'move 3, move 3'],
		['move 1, move 1', 'move 3, move 3'],
		['switch 5, switch 4', 'move 1 1, move 1 2'],
		['move 1, move 1', 'move 3, move 3'],
		['move 1, move 1', 'move 3, move 3'],
		['switch 3, switch 4', 'move 1 1, move 2 2'],
		['move 1, move 1', 'move 3, move 3'],
	]);

// 4. Dire Claw / Tri Attack. Serene Grace doubles the chances (Dire Claw is then a sure secondary), the
//    foes are immune to some of the three statuses (Poison / Fire / Ice / Electric) and Shield Dust removes
//    the secondary. Sleep Clause refuses a second foe put to sleep.
const lotto = () => [['Blissey', 'Serene Grace'], ['Jirachi', 'Serene Grace'], ['Snorlax', 'Thick Fat'], ['Scizor', 'Technician']].map(([sp, ab]) =>
	mon(sp, ab, ['Dire Claw', 'Tri Attack', 'Calm Mind', 'Recover'], { level: 50 }));
const targets = () => [['Gengar', 'Levitate'], ['Charizard', 'Solar Power'], ['Glalie', 'Inner Focus'], ['Jolteon', 'Volt Absorb'], ['Venomoth', 'Shield Dust'], ['Dugtrio', 'Sturdy']].map(([sp, ab]) =>
	mon(sp, ab, ['Calm Mind', 'Recover', 'Toxic', 'Thunder Wave'], { evs: bulk }));
scenario('status_lotteries', process.env.RS_LOTTO_SEED ? process.env.RS_LOTTO_SEED.split(',').map(Number) : [91, 92, 93, 94], lotto(), targets(),
	[
		['move 2 1, move 2 2', 'move 2, move 2'],
		['move 2 2, move 2 1', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 1 2, move 1 1', 'move 2, move 2'],
		['move 2 1, move 2 2', 'switch 3, switch 4'],
		['move 2 1, move 2 2', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 2 1, move 2 2', 'switch 5, switch 6'],
		['move 2 1, move 2 2', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 2 2, move 2 1', 'switch 3, switch 4'],
		['move 1 2, move 1 1', 'move 2, move 2'],
		['move 2 1, move 2 2', 'switch 5, switch 6'],
		['move 1 1, move 1 2', 'switch 3, switch 4'],
		['move 2 1, move 2 2', 'switch 5, switch 6'],
		['move 1 1, move 1 2', 'move 2, move 2'],
	]);

// ---------------------------------------------------------------------------------------------
function forcedChoice(request) {
	if (!request || !request.forceSwitch) return '';
	const taken = new Set();
	return request.forceSwitch.map(needs => {
		if (!needs) return 'pass';
		const idx = request.side.pokemon.findIndex((p, i) => i >= request.forceSwitch.length && !taken.has(i) && !p.condition.endsWith(' fnt'));
		if (idx < 0) return 'pass';
		taken.add(idx);
		return `switch ${idx + 1}`;
	}).join(', ');
}

const CHECKS = [
	'|brn', '|par', '|frz', '|psn', '|slp', '|tox',
	'move: Yawn|[of]', 'move: Yawn|[silent]', '|-fail|', '|-activate|', '|-message|Sleep Clause Mod activated.',
	'|-status|', '|cant|', '|-curestatus|', '|-immune|', '|-heal|', '[from] ability: Insomnia', '[from] ability: Vital Spirit',
	'|move|', '|-ability|', '|-fieldstart|',
];
function run() {
	const out = [];
	for (const sc of S) {
		const session = new Session(sim, { seed: sc.seed, teams: sc.teams });
		out.push(`scenario ${sc.name}`, `seed ${sc.seed.join(',')}`, `p1 ${sc.teams[0]}`, `p2 ${sc.teams[1]}`);
		let snap = session.snapshot();
		out.push('step start\t\t', `seed_after ${snap.seed.join(',')}`, ...snap.log.map(l => `log ${l}`));
		const all = [];
		let n = 0, forced = 0;
		const play = (label, c1, c2) => {
			for (const [side, c] of [['p1', c1], ['p2', c2]]) {
				if (c === '') continue;
				const r = session.choose(side, c);
				if (!r.ok) throw new Error(`${sc.name} ${label}: ${side} choice "${c}" rejected: ${r.error}`);
			}
			snap = session.snapshot();
			all.push(...snap.log);
			if (process.env.RS_DEBUG) console.log(`--- ${sc.name} ${label}\n${snap.log.filter(l => !l.startsWith('|split|')).join('\n')}`);
			out.push(`step ${label}\t${c1}\t${c2}`, `seed_after ${snap.seed.join(',')}`, ...snap.log.map(l => `log ${l}`));
		};
		for (const [c1, c2] of sc.turns) {
			if (session.ended) throw new Error(`${sc.name}: battle ended early`);
			play(`turn${++n}`, c1, c2);
			for (let guard = 0; guard < 4 && !session.ended; guard++) {
				const f1 = forcedChoice(snap.requests.p1), f2 = forcedChoice(snap.requests.p2);
				if (f1 === '' && f2 === '') break;
				play(`forced${++forced}`, f1, f2);
			}
		}
		const counts = CHECKS.map(c => `${c}=${all.filter(l => l.includes(c)).length}`).join('  ');
		console.log(`${sc.name}: ${n} turns (${forced} forced), faints=${all.filter(l => l.startsWith('|faint|')).length}\n  ${counts}`);
		if (session.anomalies.length) throw new Error(`${sc.name}: anomalies ${session.anomalies.join('; ')}`);
	}
	return out;
}
if (process.argv[1]?.endsWith('rules_status/scenarios.mjs')) {
	const lines = run();
	const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/conditions/rulesstatus/scenarios.txt', import.meta.url));
	fs.mkdirSync(path.dirname(dest), { recursive: true });
	fs.writeFileSync(dest, lines.join('\n') + '\n');
	console.log(`wrote ${dest} (${lines.length} lines)`);
}
export { S, mon, scenario, bulk, repeat, attackers };
