// Records hand-built protect_redirection battles on the pinned Showdown build, for the replay tests
// of the protect_redirection effect batch (Protect family and Stall, Wide Guard, Follow Me / Rage
// Powder / Lightning Rod / Storm Drain redirection, Helping Hand, Pollen Puff and the doubles
// support abilities).
//
//   node tools/probes/protect_redirection/scenarios.mjs [PS_PATH] [--out FILE]
//   DEBUG_PROBE=1 node tools/probes/protect_redirection/scenarios.mjs     (prints the log of a failing script)
//
// Each scenario is a pair of constructed teams (scoped species, moves and abilities), a Gen5 battle
// seed and a fixed choice script. The recorder drives the real Showdown simulator exactly like
// tools/oracle/lib/session.mjs and writes the omniscient log, turn and PRNG seed of every decision
// boundary (format documented at the top of the generated scenarios.txt). A choice that no
// alternative can satisfy aborts the run: the scripts must be legal.
//
// Scenarios with random triggers (Stall's randomChance) list `must` substrings that have to appear
// in the final log; the recorder tries consecutive first seed words (`search` of them) until every
// `must` is met so the table always contains the branch the scenario exists for.
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';

const args = process.argv.slice(2);
const outIdx = args.indexOf('--out');
const outPath = outIdx >= 0 ? args.splice(outIdx, 2)[1] : fileURLToPath(
	new URL('../../../crates/engine/src/effects/conditions/protectredirection/scenarios.txt', import.meta.url));
const sim = loadSim(args[0] || `${process.env.HOME}/src/pokemon-showdown`);

/** Packed set: NICK|SPECIES|ITEM|ABILITY|MOVES|NATURE|EVS|GENDER|IVS|SHINY|LEVEL|MISC (12 fields). */
function set(species, ability, moves, { gender = 'M', item = '', nick = '', evs = '252,0,128,0,128,0', level = 100 } = {}) {
	return [nick, species, item, ability, moves.join(','), 'Serious', evs, gender, '', '', String(level), ',,,,,'].join('|');
}
const team = (...sets) => sets.join(']');
/**
 * One turn: the text for p1a, p1b, p2a, p2b. Each side tries "both", then "first only", then
 * "second only" so that the script survives a faint without a bench; a rejection draws nothing.
 */
const both = (x, y) => [`${x}, ${y}`, x, y];
const turn = (a1, b1, a2, b2) => [both(a1, b1), both(a2, b2)];

const SCENARIOS = [
	{
		// Protect chain with four Protect users: the slowest Protect of a turn has an empty queue behind
		// it (willAct() false) and fails with -fail; users that succeeded carry a stall volatile, so
		// their next Protect draws randomChance(1, 3), then (1, 9) and may fail; Protect blocks the
		// attacker of the other turns.
		name: 'protect_stall_chain',
		seed: [1, 2, 3, 4],
		search: 60,
		p1: team(
			set('Corviknight', 'Pressure', ['protect', 'brickbreak']),
			set('Dondozo', 'Unaware', ['protect', 'brickbreak'])),
		p2: team(
			set('Kingambit', 'Defiant', ['protect', 'brickbreak'], { level: 50 }),
			set('Garchomp', 'Rough Skin', ['protect', 'brickbreak'], { level: 50 })),
		script: [
			turn('move 1', 'move 1', 'move 1', 'move 1'),
			turn('move 1', 'move 1', 'move 1', 'move 1'),
			turn('move 1', 'move 1', 'move 1', 'move 1'),
			turn('move 1', 'move 1', 'move 2 1', 'move 2 2'),
			turn('move 1', 'move 1', 'move 1', 'move 1'),
			turn('move 2 1', 'move 2 2', 'move 1', 'move 1'),
		],
		must: ['|-activate|', '|-fail|'],
	},
	{
		// The four shields against contact and non-contact attackers: Spiky Shield damage (1/8 of the
		// attacker's max HP), Baneful Bunker poison, Burning Bulwark burn; status moves are blocked
		// by Baneful Bunker / Protect but not by Burning Bulwark (blockStatus false).
		name: 'shields_contact_and_status',
		seed: [5, 6, 7, 8],
		p1: team(
			set('Corviknight', 'Pressure', ['banefulbunker', 'burningbulwark', 'spikyshield', 'protect']),
			set('Dondozo', 'Unaware', ['protect', 'spikyshield', 'banefulbunker', 'burningbulwark'])),
		p2: team(
			set('Kingambit', 'Defiant', ['brickbreak', 'thunderwave', 'dragonpulse', 'protect'], { level: 50 }),
			set('Garchomp', 'Rough Skin', ['dragonpulse', 'brickbreak', 'thunderwave', 'protect'], { level: 50 })),
		script: [
			turn('move 1', 'move 2', 'move 1 1', 'move 2 2'),
			turn('move 2', 'move 3', 'move 2 1', 'move 1 2'),
			turn('move 3', 'move 1', 'move 3 1', 'move 2 2'),
			turn('move 4', 'move 4', 'move 2 1', 'move 3 2'),
			turn('move 1', 'move 4', 'move 1 1', 'move 1 2'),
			turn('move 2', 'move 3', 'move 1 1', 'move 2 2'),
		],
		must: ['[from] Spiky Shield', '|-status|', '|-activate|'],
	},
	{
		// Wide Guard blocks Earthquake (allAdjacent) and Rock Slide (allAdjacentFoes) but not a
		// single-target move; its user gains a stall volatile, so repeating it draws randomChance.
		name: 'wide_guard',
		seed: [9, 10, 11, 12],
		search: 40,
		p1: team(
			set('Hitmontop', 'Intimidate', ['wideguard', 'protect', 'brickbreak']),
			set('Dondozo', 'Unaware', ['wideguard', 'protect', 'brickbreak'])),
		p2: team(
			set('Garchomp', 'Rough Skin', ['earthquake', 'rockslide', 'brickbreak', 'protect'], { level: 50 }),
			set('Kingambit', 'Defiant', ['rockslide', 'brickbreak', 'protect', 'earthquake'], { level: 50 })),
		script: [
			turn('move 1', 'move 2', 'move 1', 'move 1'),
			turn('move 1', 'move 2', 'move 2', 'move 1'),
			turn('move 1', 'move 3 1', 'move 3 1', 'move 2 2'),
			turn('move 1', 'move 1', 'move 1', 'move 4'),
			turn('move 2', 'move 2', 'move 1', 'move 1'),
		],
		must: ['move: Wide Guard'],
	},
	{
		// Follow Me / Rage Powder redirection: single-target attacks aimed at the partner go to the
		// user; a Grass-type attacker ignores Rage Powder (runStatusImmunity('powder')); with both
		// redirectors active the priorityEvent ordering decides.
		name: 'follow_me_rage_powder',
		seed: [13, 14, 15, 16],
		p1: team(
			set('Clefairy', 'Friend Guard', ['followme', 'calmmind', 'brickbreak'], { gender: 'F' }),
			set('Amoonguss', 'Regenerator', ['ragepowder', 'calmmind', 'brickbreak'], { gender: 'F' })),
		p2: team(
			set('Rillaboom', 'Overgrow', ['brickbreak', 'calmmind'], { level: 50 }),
			set('Kingambit', 'Defiant', ['brickbreak', 'calmmind'], { level: 50 })),
		script: [
			turn('move 1', 'move 2', 'move 1 2', 'move 1 2'),
			turn('move 2', 'move 1', 'move 1 1', 'move 1 1'),
			turn('move 1', 'move 1', 'move 1 1', 'move 1 2'),
			turn('move 2', 'move 1', 'move 1 1', 'move 1 1'),
			turn('move 1', 'move 2', 'move 1 2', 'move 1 2'),
		],
		must: ['move: Follow Me', 'move: Rage Powder'],
	},
	{
		// Stalwart and Propeller Tail attackers (tracksTarget) ignore Follow Me and Rage Powder.
		name: 'redirection_ignored_by_tracking',
		seed: [17, 18, 19, 20],
		p1: team(
			set('Clefairy', 'Friend Guard', ['followme', 'calmmind', 'brickbreak'], { gender: 'F' }),
			set('Amoonguss', 'Regenerator', ['ragepowder', 'calmmind', 'brickbreak'], { gender: 'F' })),
		p2: team(
			set('Duraludon', 'Stalwart', ['brickbreak', 'calmmind'], { level: 50 }),
			set('Barraskewda', 'Propeller Tail', ['brickbreak', 'calmmind'], { level: 50 })),
		script: [
			turn('move 1', 'move 2', 'move 1 2', 'move 1 2'),
			turn('move 2', 'move 1', 'move 1 1', 'move 1 1'),
			turn('move 1', 'move 2', 'move 1 2', 'move 1 2'),
		],
		must: ['move: Follow Me', 'move: Rage Powder'],
	},
	{
		// Lightning Rod / Storm Drain: absorption (+1 SpA), redirection of Electric / Water moves
		// aimed at the partner, and the -immune line once the boost is capped.
		name: 'lightning_rod_storm_drain',
		seed: [21, 22, 23, 24],
		p1: team(
			set('Raichu', 'Lightning Rod', ['swordsdance', 'brickbreak']),
			set('Gastrodon', 'Storm Drain', ['swordsdance', 'brickbreak'], { gender: 'F' })),
		p2: team(
			set('Rotom-Wash', 'Levitate', ['thunderbolt', 'hydropump', 'calmmind'], { level: 50, gender: 'N' }),
			set('Lanturn', 'Volt Absorb', ['thunderbolt', 'hydropump', 'calmmind'], { level: 50 })),
		script: [
			turn('move 1', 'move 1', 'move 1 2', 'move 2 1'),
			turn('move 1', 'move 1', 'move 2 1', 'move 1 2'),
			turn('move 1', 'move 1', 'move 1 1', 'move 2 2'),
			turn('move 1', 'move 1', 'move 2 2', 'move 1 1'),
			turn('move 1', 'move 1', 'move 1 2', 'move 2 1'),
			turn('move 1', 'move 1', 'move 2 1', 'move 1 2'),
			turn('move 1', 'move 1', 'move 1 1', 'move 2 2'),
			turn('move 1', 'move 1', 'move 2 2', 'move 1 1'),
		],
		must: ['ability: Lightning Rod', 'ability: Storm Drain', '|-immune|'],
	},
	{
		// Helping Hand stacks its 1.5x into the ally's damage; Pollen Puff damages foes but heals an
		// ally by half its max HP (infiltrating, basePower 0); a full-HP ally makes the heal fail.
		name: 'helping_hand_pollen_puff',
		seed: [25, 26, 27, 28],
		p1: team(
			set('Ribombee', 'Shield Dust', ['helpinghand', 'pollenpuff', 'calmmind'], { gender: 'F' }),
			set('Garchomp', 'Rough Skin', ['dragonpulse', 'brickbreak', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['brickbreak', 'calmmind'], { level: 70 }),
			set('Dondozo', 'Unaware', ['calmmind', 'brickbreak'], { level: 70 })),
		script: [
			turn('move 1 -2', 'move 1 1', 'move 2', 'move 1'),
			turn('move 2 -2', 'move 3', 'move 1 2', 'move 2 2'),
			turn('move 2 1', 'move 3', 'move 1 2', 'move 2 2'),
			turn('move 2 -2', 'move 3', 'move 2', 'move 2 2'),
			turn('move 1 -2', 'move 2 2', 'move 2', 'move 1'),
		],
		must: ['Helping Hand', '|-heal|'],
	},
	{
		// Friend Guard (0.75 on the holder's ally), Power Spot (5325/4096 on the holder's allies'
		// moves), Steely Spirit (1.5 on Steel moves, the holder's own included).
		name: 'ally_damage_modifiers',
		seed: [29, 30, 31, 32],
		p1: team(
			set('Clefairy', 'Friend Guard', ['dazzlinggleam', 'calmmind'], { gender: 'F' }),
			set('Stonjourner', 'Power Spot', ['rockslide', 'calmmind'])),
		p2: team(
			set('Perrserker', 'Steely Spirit', ['bulletpunch', 'brickbreak', 'calmmind'], { gender: 'F', level: 70 }),
			set('Kingambit', 'Defiant', ['bulletpunch', 'brickbreak', 'calmmind'], { level: 70 })),
		script: [
			turn('move 1', 'move 2', 'move 1 2', 'move 1 1'),
			turn('move 2', 'move 1', 'move 1 1', 'move 1 2'),
			turn('move 1', 'move 2', 'move 1 2', 'move 2 2'),
			turn('move 2', 'move 1', 'move 3', 'move 3'),
		],
		must: ['|-damage|'],
	},
	{
		// Armor Tail: priority moves aimed at the holder or its ally are stopped with a `cant` line;
		// a non-priority move is not.
		name: 'armor_tail',
		seed: [33, 34, 35, 36],
		p1: team(
			set('Farigiraf', 'Armor Tail', ['calmmind', 'brickbreak']),
			set('Dondozo', 'Unaware', ['calmmind', 'brickbreak'])),
		p2: team(
			set('Kingambit', 'Defiant', ['suckerpunch', 'brickbreak', 'calmmind'], { level: 50 }),
			set('Hitmontop', 'Intimidate', ['quickattack', 'brickbreak', 'calmmind'], { level: 50 })),
		script: [
			turn('move 1', 'move 1', 'move 1 1', 'move 1 2'),
			turn('move 1', 'move 1', 'move 2 1', 'move 2 2'),
			turn('move 1', 'move 1', 'move 1 2', 'move 1 1'),
		],
		must: ['ability: Armor Tail'],
	},
	{
		// Queenly Majesty: same shape as Armor Tail, with Fake Out (priority 3, turn one only).
		name: 'queenly_majesty',
		seed: [37, 38, 39, 40],
		p1: team(
			set('Tsareena', 'Queenly Majesty', ['calmmind', 'brickbreak'], { gender: 'F' }),
			set('Dondozo', 'Unaware', ['calmmind', 'brickbreak'])),
		p2: team(
			set('Kingambit', 'Defiant', ['fakeout', 'brickbreak', 'calmmind'], { level: 50 }),
			set('Hitmontop', 'Intimidate', ['quickattack', 'brickbreak', 'calmmind'], { level: 50 })),
		script: [
			turn('move 1', 'move 1', 'move 1 2', 'move 1 1'),
			turn('move 1', 'move 1', 'move 2 1', 'move 1 2'),
			turn('move 1', 'move 1', 'move 3', 'move 2 2'),
		],
		must: ['ability: Queenly Majesty'],
	},
	{
		// Dragon Darts (smartTarget) into Protect: the protecting target silently clears
		// smartTarget (no -activate for that hit), the other hit goes to the second target.
		name: 'dragon_darts_into_protect',
		seed: [41, 42, 43, 44],
		p1: team(
			set('Dragapult', 'Clear Body', ['dragondarts', 'calmmind', 'brickbreak'], { level: 50 }),
			set('Corviknight', 'Pressure', ['calmmind', 'brickbreak'])),
		p2: team(
			set('Kingambit', 'Defiant', ['protect', 'calmmind', 'brickbreak']),
			set('Dondozo', 'Unaware', ['protect', 'calmmind', 'brickbreak'])),
		script: [
			turn('move 1 1', 'move 1', 'move 1', 'move 2'),
			turn('move 1 2', 'move 1', 'move 2', 'move 1'),
			turn('move 1 1', 'move 1', 'move 1', 'move 1'),
		],
		must: ['Dragon Darts', '|-singleturn|'],
	},
];
function record(sc, seed) {
	const s = new Session(sim, { seed, teams: [sc.p1, sc.p2] });
	const out = [];
	out.push(`S ${sc.name}`, `SEED ${seed.join(',')}`, `T1 ${sc.p1}`, `T2 ${sc.p2}`);
	const boundary = () => {
		const snap = s.snapshot();
		out.push(`B ${snap.turn} ${snap.seed.join(',')}`);
		for (const line of snap.log) out.push(`L ${line}`);
		return snap;
	};
	boundary();
	for (const [i, [c1, c2]] of sc.script.entries()) {
		if (s.ended) break;
		for (const [side, alternatives] of [['p1', c1], ['p2', c2]]) {
			// A rejection leaves the PRNG and log untouched, so the first accepted alternative is recorded.
			let choice = null;
			let r = null;
			for (const alt of alternatives) {
				choice = alt;
				r = s.choose(side, alt);
				if (r.ok) break;
				if (process.env.DEBUG_PROBE) console.error(`  ${sc.name} turn ${i + 1} ${side} ${JSON.stringify(alt)}: ${r.error}`);
			}
			if (!r.ok) {
				if (process.env.DEBUG_PROBE) {
					console.error(`--- ${sc.name}`);
					console.error(s.battle.log.filter(l => /^\|(move|faint|turn|-weather|-fieldstart|-fieldend|-activate|-immune|-fail|-heal|-status|-damage|-curestatus|cant|-miss|win|switch|-start|-end|-singleturn|-boost)/.test(l)).join('\n'));
				}
				throw new Error(`${sc.name} turn ${i + 1}: ${side} ${JSON.stringify(alternatives)} rejected: ${r.error}`);
			}
			out.push(`C ${side} ${choice}`);
		}
		boundary();
	}
	if (s.anomalies.length) throw new Error(`${sc.name}: anomalies ${JSON.stringify(s.anomalies)}`);
	out.push(`E ${s.ended ? 'ended' : 'running'}`);
	return { out, fullLog: s.battle.log };
}
function recordWithSearch(sc) {
	const tries = sc.search || 1;
	for (let i = 0; i < tries; i++) {
		const seed = [sc.seed[0] + i, sc.seed[1], sc.seed[2], sc.seed[3]];
		const { out, fullLog } = record(sc, seed);
		const missing = (sc.must || []).filter(m => !fullLog.some(l => l.includes(m)));
		if (!missing.length) return out;
		if (i === tries - 1) {
			if (process.env.DEBUG_PROBE) console.error(fullLog.join('\n'));
			throw new Error(`${sc.name}: no seed produced ${JSON.stringify(missing)}`);
		}
	}
	throw new Error(`${sc.name}: unreachable`);
}
const header = [
	'# protect_redirection scenarios recorded from the pinned Showdown build by',
	'# tools/probes/protect_redirection/scenarios.mjs. Do not edit by hand.',
	'# S name | SEED a,b,c,d | T1/T2 packed team | B turn seed (decision boundary) | L raw log line',
	'# (|t:| normalized) produced to reach that boundary | C side choice | E ended/running',
];
const lines = [...header];
let failed = 0;
for (const sc of SCENARIOS) {
	if (process.env.ONLY && !process.env.ONLY.split(',').includes(sc.name)) continue;
	try {
		lines.push(...recordWithSearch(sc));
	} catch (e) {
		failed++;
		console.error(e.message);
	}
}
if (failed) process.exit(1);
fs.writeFileSync(outPath, lines.join('\n') + '\n');
console.log(`Wrote ${SCENARIOS.length} scenarios (${lines.length} lines) to ${outPath}`);
