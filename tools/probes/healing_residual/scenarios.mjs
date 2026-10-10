// Records hand-built healing_residual battles on the pinned Showdown build, for the replay tests
// of the healing_residual effect batch (residual HP, cures, passive recovery and draining
// conditions).
//
//   node tools/probes/healing_residual/scenarios.mjs [PS_PATH] [--out FILE]
//   DEBUG_PROBE=1 node tools/probes/healing_residual/scenarios.mjs     (prints the log of a failing script)
//
// Each scenario is a pair of constructed teams (scoped species, moves and abilities; abilities are
// assigned freely, as in the directed corpora), a Gen5 battle seed and a fixed choice script. The
// recorder drives the real Showdown simulator exactly like tools/oracle/lib/session.mjs and writes
// the omniscient log, turn and PRNG seed of every decision boundary (format documented at the top
// of scenarios.txt, identical to the weather_terrain batch). A choice that no alternative can
// satisfy aborts the run: the scripts must be legal.
//
// Scenarios with random triggers (Healer 30%, Shed Skin 33%) list `must` substrings that have to
// appear in the final log; the recorder tries consecutive first seed words (`search` of them) until
// every `must` is met so the table always contains the activating branch.
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';

const args = process.argv.slice(2);
const outIdx = args.indexOf('--out');
const outPath = outIdx >= 0 ? args.splice(outIdx, 2)[1] : fileURLToPath(
	new URL('../../../crates/engine/src/effects/abilities/healingresidual/scenarios.txt', import.meta.url));
const sim = loadSim(args[0] || `${process.env.HOME}/src/pokemon-showdown`);

/** Packed set: NICK|SPECIES|ITEM|ABILITY|MOVES|NATURE|EVS|GENDER|IVS|SHINY|LEVEL|MISC (12 fields). */
function set(species, ability, moves, { gender = 'M', item = '', nick = '', evs = '252,0,128,0,128,0' } = {}) {
	return [nick, species, item, ability, moves.join(','), 'Serious', evs, gender, '', '', '100', ',,,,,'].join('|');
}
const team = (...sets) => sets.join(']');
/**
 * One turn: the text for p1a, p1b, p2a, p2b. Each side tries "both", then "first only", then
 * "second only" so that the script survives a faint without a bench; a rejection draws nothing.
 */
const both = (x, y) => [`${x}, ${y}`, x, y];
const turn = (a1, b1, a2, b2) => [both(a1, b1), both(a2, b2)];
const repeat = (n, t) => Array.from({ length: n }, () => t);
// Switch-only turn for one side (the other side moves) uses a plain `[ [text] ]` alternative list.
const only = text => [text];

const SCENARIOS = [
	{
		// Toxic Orb / Flame Orb residual statuses, Leftovers and Poison Heal in the same residual pass
		// (Poison Heal cancels the poison damage with a heal; Magic Guard-less Kingambit just burns).
		name: 'orbs_leftovers_poisonheal',
		seed: [1, 2, 3, 4],
		p1: team(
			set('Gliscor', 'Poison Heal', ['calmmind', 'swordsdance'], { item: 'Toxic Orb' }),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance'], { item: 'Leftovers' })),
		p2: team(
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind']),
			set('Kingambit', 'Defiant', ['ironhead', 'calmmind', 'swordsdance'], { item: 'Flame Orb' })),
		script: [
			turn('move 2', 'move 2', 'move 1 2', 'move 1 1'),
			turn('move 2', 'move 2', 'move 1 2', 'move 1 1'),
			turn('move 2', 'move 2', 'move 2', 'move 3'),
			turn('move 2', 'move 2', 'move 2', 'move 3'),
		],
		must: ['[from] item: Toxic Orb', '[from] item: Flame Orb', '[from] item: Leftovers', '[from] ability: Poison Heal'],
	},
	{
		// Leech Seed: start line, drain + silent heal at residual, Grass immunity (TryImmunity false).
		name: 'leech_seed_drain_and_grass_immune',
		seed: [5, 6, 7, 8],
		search: 30,
		p1: team(
			set('Rillaboom', 'Overgrow', ['leechseed', 'calmmind']),
			set('Incineroar', 'Intimidate', ['leechseed', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['calmmind', 'swordsdance']),
			set('Amoonguss', 'Regenerator', ['calmmind', 'swordsdance'], { gender: 'F' })),
		script: [
			turn('move 1 1', 'move 1 2', 'move 2', 'move 2'),
			turn('move 2', 'move 2', 'move 2', 'move 2'),
			turn('move 2', 'move 2', 'move 2', 'move 2'),
		],
		must: ['move: Leech Seed', '|-immune|'],
	},
	{
		// Leech Seed reads the seeder's slot: after the seeder switches out its replacement gets the HP.
		name: 'leech_seed_slot_after_switch',
		seed: [9, 10, 11, 12],
		p1: team(
			set('Rillaboom', 'Overgrow', ['leechseed', 'calmmind']),
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind']),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance'])),
		p2: team(
			set('Kingambit', 'Defiant', ['ironhead', 'calmmind', 'swordsdance']),
			set('Garchomp', 'Rough Skin', ['calmmind', 'swordsdance'])),
		script: [
			turn('move 1 1', 'move 2', 'move 3', 'move 2'),
			[only('switch 3, move 2'), only('move 1 1, move 2')],
			turn('move 2', 'move 2', 'move 2', 'move 2'),
			turn('move 2', 'move 2', 'move 2', 'move 2'),
		],
		must: ['move: Leech Seed', '|-heal|'],
	},
	{
		// Wish: the next turn's residual heals whoever occupies the slot ([wisher] name); the wisher
		// switching out first hands the heal to the replacement. Blissey is damaged first.
		name: 'wish_heals_replacement',
		seed: [13, 14, 15, 16],
		p1: team(
			set('Blissey', 'Healer', ['wish', 'calmmind'], { gender: 'F', nick: 'Wishy' }),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance']),
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['ironhead', 'calmmind', 'swordsdance']),
			set('Garchomp', 'Rough Skin', ['earthquake', 'calmmind', 'swordsdance'])),
		script: [
			turn('move 1', 'move 2', 'move 1 1', 'move 2'),
			[only('switch 3, move 1'), only('move 1 1, move 2')],
			turn('move 2', 'move 2', 'move 2', 'move 2'),
			turn('move 2', 'move 2', 'move 1 1', 'move 3'),
			turn('move 2', 'move 2', 'move 2', 'move 2'),
		],
		must: ['[from] move: Wish|[wisher] Wishy'],
	},
	{
		// Wish heal on the wisher itself in place (the damaged Blissey stays), and a second Wish used
		// while one is pending (fails: no onRestart).
		name: 'wish_in_place_and_repeat',
		seed: [17, 18, 19, 20],
		p1: team(
			set('Blissey', 'Natural Cure', ['wish', 'calmmind'], { gender: 'F' }),
			set('Clefable', 'Natural Cure', ['wish', 'calmmind'], { gender: 'F' })),
		p2: team(
			set('Kingambit', 'Defiant', ['ironhead', 'calmmind']),
			set('Garchomp', 'Rough Skin', ['earthquake', 'calmmind'])),
		script: [
			turn('move 1', 'move 2', 'move 1 1', 'move 2'),
			turn('move 1', 'move 2', 'move 2', 'move 2'),
			turn('move 2', 'move 2', 'move 1 1', 'move 2'),
			turn('move 1', 'move 2', 'move 2', 'move 2'),
		],
		must: ['[from] move: Wish|[wisher]'],
	},
	{
		// Natural Cure: both mons that could have it switch out, only one has it -> the ambiguous
		// "(N of Alice's pokemon was cured by Natural Cure.)" message and showCure = false.
		name: 'natural_cure_ambiguous_switch',
		seed: [21, 22, 23, 24],
		p1: team(
			set('Altaria', 'Natural Cure', ['calmmind', 'swordsdance'], { gender: 'F' }),
			set('Blissey', 'Serene Grace', ['calmmind', 'swordsdance'], { gender: 'F' }),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance']),
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['thunderwave', 'calmmind']),
			set('Garchomp', 'Rough Skin', ['thunderwave', 'calmmind'])),
		script: [
			turn('move 2', 'move 2', 'move 1 1', 'move 1 2'),
			[only('switch 3, switch 4'), only('move 2, move 2')],
			turn('move 2', 'move 2', 'move 2', 'move 2'),
		],
		must: ['cured by Natural Cure.)'],
	},
	{
		// Natural Cure on a single switch (the cure is revealed with a silent -curestatus), then the
		// ability's holder returns statused again; Regenerator heals a third on switch-out.
		name: 'natural_cure_and_regenerator',
		seed: [25, 26, 27, 28],
		p1: team(
			set('Altaria', 'Natural Cure', ['calmmind', 'swordsdance'], { gender: 'F' }),
			set('Slowbro', 'Regenerator', ['calmmind', 'swordsdance']),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance']),
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['thunderwave', 'ironhead', 'calmmind']),
			set('Garchomp', 'Rough Skin', ['earthquake', 'thunderwave', 'calmmind'])),
		script: [
			turn('move 2', 'move 2', 'move 1 1', 'move 2 2'),
			[only('switch 3, move 2'), only('move 3, move 3')],
			[only('switch 4, move 2'), only('move 3, move 3')],
			turn('move 2', 'move 2', 'move 1 1', 'move 2 2'),
			[only('switch 3, move 2'), only('move 3, move 3')],
		],
		must: ['[from] ability: Natural Cure|[silent]'],
	},
	{
		// Healer (randomChance(3, 10) per statused adjacent ally).
		name: 'healer_ally_status',
		seed: [29, 30, 31, 32],
		search: 80,
		p1: team(
			set('Blissey', 'Healer', ['calmmind', 'swordsdance'], { gender: 'F' }),
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['thunderwave', 'calmmind']),
			set('Garchomp', 'Rough Skin', ['calmmind', 'swordsdance'])),
		script: [
			turn('move 2', 'move 1 1', 'move 1 2', 'move 2'),
			...repeat(5, turn('move 2', 'move 2', 'move 2', 'move 2')),
		],
		must: ['ability: Healer', '|-curestatus|'],
	},
	{
		// Shed Skin (randomChance(33, 100) once statused).
		name: 'shed_skin_statused',
		seed: [33, 34, 35, 36],
		search: 80,
		p1: team(
			set('Sandaconda', 'Shed Skin', ['calmmind', 'swordsdance']),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance'])),
		p2: team(
			set('Kingambit', 'Defiant', ['willowisp', 'calmmind']),
			set('Garchomp', 'Rough Skin', ['calmmind', 'swordsdance'])),
		script: [
			turn('move 2', 'move 2', 'move 1 1', 'move 2'),
			...repeat(5, turn('move 2', 'move 2', 'move 2', 'move 2')),
		],
		must: ['ability: Shed Skin', '|-curestatus|'],
	},
	{
		// Bad Dreams damages sleeping foes (and Comatose holders) at residual.
		name: 'bad_dreams_sleeping_and_comatose',
		seed: [37, 38, 39, 40],
		search: 80,
		p1: team(
			set('Darkrai', 'Bad Dreams', ['spore', 'calmmind'], { gender: 'N' }),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance'])),
		p2: team(
			set('Kingambit', 'Defiant', ['calmmind', 'swordsdance']),
			set('Komala', 'Comatose', ['calmmind', 'swordsdance'])),
		script: [
			turn('move 1 1', 'move 2', 'move 2', 'move 2'),
			...repeat(3, turn('move 2', 'move 2', 'move 2', 'move 2')),
		],
		must: ['[from] ability: Bad Dreams'],
	},
	{
		// Dry Skin: Water absorption (heal, or -immune at full HP), rain heal, Fire damage boost.
		name: 'dry_skin_water_rain_fire',
		seed: [41, 42, 43, 44],
		p1: team(
			set('Toxicroak', 'Dry Skin', ['calmmind', 'swordsdance']),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance'])),
		p2: team(
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind']),
			set('Milotic', 'Competitive', ['surf', 'raindance', 'calmmind'], { gender: 'F' })),
		script: [
			turn('move 2', 'move 2', 'move 1 1', 'move 1'),
			turn('move 2', 'move 2', 'move 1 1', 'move 2'),
			turn('move 2', 'move 2', 'move 1 1', 'move 1'),
			turn('move 2', 'move 2', 'move 2', 'move 3'),
		],
		must: ['[from] ability: Dry Skin', '|-immune|'],
	},
	{
		// Dry Skin in sun takes 1/8 each residual; Drought sets the weather.
		name: 'dry_skin_sun_damage',
		seed: [45, 46, 47, 48],
		p1: team(
			set('Toxicroak', 'Dry Skin', ['calmmind', 'swordsdance']),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance'])),
		p2: team(
			set('Torkoal', 'Drought', ['calmmind', 'swordsdance']),
			set('Kingambit', 'Defiant', ['calmmind', 'swordsdance'])),
		script: repeat(3, turn('move 2', 'move 2', 'move 2', 'move 2')),
		must: ['[from] ability: Dry Skin'],
	},
	{
		// Salt Cure: 1/8 per residual, 1/4 on a Water or Steel target, volatile lost on switch-out.
		name: 'salt_cure_water_steel',
		seed: [49, 50, 51, 52],
		p1: team(
			set('Garganacl', 'Purifying Salt', ['saltcure', 'calmmind']),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance']),
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['calmmind', 'swordsdance']),
			set('Milotic', 'Competitive', ['calmmind', 'swordsdance'], { gender: 'F' }),
			set('Garchomp', 'Rough Skin', ['calmmind', 'swordsdance'])),
		script: [
			turn('move 1 1', 'move 2', 'move 2', 'move 2'),
			turn('move 1 2', 'move 2', 'move 2', 'move 2'),
			[only('move 2, move 2'), only('switch 3, move 2')],
			turn('move 2', 'move 2', 'move 2', 'move 2'),
		],
		must: ['Salt Cure'],
	},
	{
		// Syrup Bomb: -1 Spe at each residual for the volatile's four turns, silent -end when it wears off.
		name: 'syrup_bomb_speed_drops',
		seed: [53, 54, 55, 56],
		p1: team(
			set('Dipplin', 'Sticky Hold', ['syrupbomb', 'calmmind'], { gender: 'F' }),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance'])),
		p2: team(
			set('Kingambit', 'Defiant', ['calmmind', 'swordsdance']),
			set('Garchomp', 'Rough Skin', ['calmmind', 'swordsdance'])),
		script: [
			turn('move 1 1', 'move 2', 'move 2', 'move 2'),
			...repeat(5, turn('move 2', 'move 2', 'move 2', 'move 2')),
		],
		must: ['Syrup Bomb', '|-end|'],
	},
	{
		// Syrup Bomb's onUpdate removes the volatile when its source leaves the field.
		name: 'syrup_bomb_source_leaves',
		seed: [57, 58, 59, 60],
		p1: team(
			set('Dipplin', 'Sticky Hold', ['syrupbomb', 'calmmind'], { gender: 'F' }),
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance']),
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['calmmind', 'swordsdance']),
			set('Garchomp', 'Rough Skin', ['calmmind', 'swordsdance'])),
		script: [
			turn('move 1 1', 'move 2', 'move 2', 'move 2'),
			[only('switch 3, move 2'), only('move 2, move 2')],
			turn('move 2', 'move 2', 'move 2', 'move 2'),
		],
		must: ['Syrup Bomb|[silent]'],
	},
	{
		// Heal Pulse (Mega Launcher 3/4 vs half, on an ally and on a foe), Jungle Healing (heals and
		// cures the user and its ally), at full HP (-fail) and damaged.
		name: 'heal_pulse_and_jungle_healing',
		seed: [61, 62, 63, 64],
		p1: team(
			set('Clawitzer', 'Mega Launcher', ['healpulse', 'calmmind']),
			set('Lilligant', 'Chlorophyll', ['junglehealing', 'calmmind'], { gender: 'F' })),
		p2: team(
			set('Kingambit', 'Defiant', ['ironhead', 'thunderwave', 'calmmind']),
			set('Garchomp', 'Rough Skin', ['earthquake', 'calmmind'])),
		script: [
			turn('move 1 -2', 'move 1', 'move 1 2', 'move 1'),
			turn('move 1 -2', 'move 1', 'move 2 2', 'move 2'),
			turn('move 1 -2', 'move 1', 'move 2 1', 'move 2'),
			turn('move 1 1', 'move 1', 'move 3', 'move 2'),
		],
		must: ['|-heal|', '|-fail|'],
	},
	{
		// Lunar Blessing and Take Heart cure a statused user and apply their boosts; Strength Sap heals
		// by the target's Atk and drops it.
		name: 'lunar_blessing_take_heart_strength_sap',
		seed: [65, 66, 67, 68],
		search: 30,
		p1: team(
			set('Cresselia', 'Levitate', ['lunarblessing', 'calmmind'], { gender: 'F' }),
			set('Meowscarada', 'Overgrow', ['takeheart', 'calmmind'])),
		p2: team(
			set('Kingambit', 'Defiant', ['thunderwave', 'ironhead', 'calmmind']),
			set('Amoonguss', 'Regenerator', ['strengthsap', 'calmmind'], { gender: 'F' })),
		script: [
			turn('move 1', 'move 1', 'move 1 1', 'move 1 1'),
			turn('move 1', 'move 1', 'move 2 1', 'move 1 1'),
			turn('move 1', 'move 1', 'move 2 1', 'move 1 1'),
			turn('move 2', 'move 2', 'move 3', 'move 1 1'),
			turn('move 2', 'move 2', 'move 3', 'move 1 1'),
			turn('move 2', 'move 2', 'move 3', 'move 1 1'),
			turn('move 2', 'move 2', 'move 3', 'move 1 1'),
		],
		must: ['|-heal|', '|-boost|', '|-curestatus|'],
	},
	{
		// Hospitality heals the adjacent ally by a quarter when the holder enters.
		name: 'hospitality_on_entry',
		seed: [69, 70, 71, 72],
		p1: team(
			set('Mudsdale', 'Stamina', ['calmmind', 'swordsdance']),
			set('Incineroar', 'Intimidate', ['flamethrower', 'calmmind']),
			set('Sinistcha', 'Hospitality', ['calmmind', 'swordsdance'], { gender: 'F' })),
		p2: team(
			set('Kingambit', 'Defiant', ['ironhead', 'calmmind', 'swordsdance']),
			set('Garchomp', 'Rough Skin', ['earthquake', 'calmmind', 'swordsdance'])),
		script: [
			turn('move 2', 'move 2', 'move 1 1', 'move 2'),
			[only('move 2, switch 3'), only('move 2, move 2')],
			turn('move 1', 'move 1', 'move 2', 'move 2'),
		],
		must: ['[from] ability: Hospitality'],
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
	const lines = [];
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
					console.error(s.battle.log.filter(l => /^\|(move|faint|turn|-weather|-fieldstart|-fieldend|-activate|-immune|-fail|-heal|-status|-damage|-curestatus|cant|-miss|win|switch|-start|-end)/.test(l)).join('\n'));
				}
				throw new Error(`${sc.name} turn ${i + 1}: ${side} ${JSON.stringify(alternatives)} rejected: ${r.error}`);
			}
			lines.push(`C ${side} ${choice}`);
			out.push(`C ${side} ${choice}`);
		}
		boundary();
	}
	if (s.anomalies.length) throw new Error(`${sc.name}: anomalies ${JSON.stringify(s.anomalies)}`);
	out.push(`E ${s.ended ? 'ended' : 'running'}`);
	const fullLog = s.battle.log;
	return { out, fullLog };
}

function recordWithSearch(sc) {
	const tries = sc.search || 1;
	let last = null;
	for (let i = 0; i < tries; i++) {
		const seed = [sc.seed[0] + i, sc.seed[1], sc.seed[2], sc.seed[3]];
		const { out, fullLog } = record(sc, seed);
		last = fullLog;
		const missing = (sc.must || []).filter(m => !fullLog.some(l => l.includes(m)));
		if (!missing.length) return out;
		if (i === tries - 1) {
			if (process.env.DEBUG_PROBE) console.error(fullLog.join('\n'));
			throw new Error(`${sc.name}: no seed produced ${JSON.stringify(missing)}`);
		}
	}
	throw new Error(`${sc.name}: unreachable ${last}`);
}

const header = [
	'# healing_residual scenarios recorded from the pinned Showdown build by',
	'# tools/probes/healing_residual/scenarios.mjs. Do not edit by hand.',
	'# S name | SEED a,b,c,d | T1/T2 packed team | B turn seed (decision boundary) | L raw log line',
	'# (|t:| normalized) produced to reach that boundary | C side choice | E ended/running',
];
const lines = [...header];
let failed = 0;
for (const sc of SCENARIOS) {
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
