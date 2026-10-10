// Regenerates crates/engine/src/effects/abilities/healingresidual/scenarios.tsv: small two-player
// scenarios that exercise the healing_residual batch (residual HP, cures, passive recovery and
// draining conditions), with the expected raw battle log and final PRNG seed taken from the pinned
// Showdown build (tools/oracle/ORACLE_COMMIT).
//
//   node tools/probes/healing_residual/scenarios.mjs [showdown checkout]
//   DEBUG=<scenario name> node tools/probes/healing_residual/scenarios.mjs   (prints that log)
//
// Columns (tab separated): name, seed, p1 packed team, p2 packed team, steps (';' separated
// "p1:<choice>" / "p2:<choice>", applied in order), final seed, expected log ("\n" escape between
// lines; the wall-clock `|t:|<secs>` lines are normalized to `|t:|`).
//
// A scenario lists `must` substrings that have to appear in the log. Scenarios with random
// triggers (Healer, Shed Skin) try consecutive seeds starting at `seed` until every `must` is met,
// so the table always contains the activating branch; the first draw that did not trigger is
// still covered by the final-seed comparison of the other scenarios.
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';

const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);

// name|species|item|ability|moves|nature|evs|gender|ivs|shiny|level|happiness
function mon(name, ability, moves, { level = 100, gender = 'M', item = '' } = {}) {
	return `${name}||${item}|${ability}|${moves.join(',')}|Serious||${gender}|||${level}|`;
}
const team = (...mons) => mons.join(']');

const SCENARIOS = [
	{
		// Toxic Orb / Flame Orb residual statuses, Leftovers and Poison Heal in the same residual pass.
		name: 'orbs_leftovers_poisonheal',
		seed: [1, 2, 3, 4],
		p1: team(
			mon('Gliscor', 'poisonheal', ['earthquake', 'protect'], { item: 'toxicorb' }),
			mon('Mudsdale', 'stamina', ['highhorsepower', 'protect'], { item: 'leftovers' }),
		),
		p2: team(
			mon('Incineroar', 'intimidate', ['flamethrower', 'protect']),
			mon('Kingambit', 'defiant', ['ironhead', 'protect'], { item: 'flameorb' }),
		),
		steps: ['p1:move 1, move 1 2', 'p2:move 1 1, move 1 2', 'p1:move 2, move 2', 'p2:move 2, move 2', 'p1:move 2, move 2', 'p2:move 2, move 2'],
		must: ['[from] item: Toxic Orb', '[from] item: Flame Orb', '[from] item: Leftovers', '[from] ability: Poison Heal'],
	},
	{
		// Leech Seed: start line, drain+silent heal at residual, Grass immunity (TryImmunity false).
		name: 'leech_seed_drain_and_grass_immune',
		seed: [5, 6, 7, 8],
		p1: team(
			mon('Rillaboom', 'overgrow', ['leechseed', 'protect']),
			mon('Incineroar', 'intimidate', ['leechseed', 'protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['protect']),
			mon('Amoonguss', 'regenerator', ['protect'], { gender: 'F' }),
		),
		steps: ['p1:move 1 1, move 1 2', 'p2:move 1, move 1', 'p1:move 2, move 2', 'p2:move 1, move 1'],
		must: ['move: Leech Seed', '|-immune|'],
	},
	{
		// Leech Seed reads the seeder's slot: after the seeder switches out the replacement gets the HP.
		name: 'leech_seed_slot_after_switch',
		seed: [9, 10, 11, 12],
		p1: team(
			mon('Rillaboom', 'overgrow', ['leechseed', 'protect']),
			mon('Incineroar', 'intimidate', ['protect']),
			mon('Mudsdale', 'stamina', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['protect']),
			mon('Garchomp', 'roughskin', ['protect']),
		),
		steps: ['p1:move 1 1, move 2', 'p2:move 1, move 1', 'p1:switch 3, move 1', 'p2:move 1, move 1'],
		must: ['move: Leech Seed'],
	},
	{
		// Wish: hint-free resolution on the next turn, healing whoever occupies the slot ([wisher] name).
		name: 'wish_heals_next_turn',
		seed: [13, 14, 15, 16],
		p1: team(
			mon('Blissey', 'healer', ['wish', 'protect'], { gender: 'F' }),
			mon('Mudsdale', 'stamina', ['protect']),
			mon('Incineroar', 'intimidate', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['ironhead', 'protect']),
			mon('Garchomp', 'roughskin', ['earthquake', 'protect']),
		),
		steps: ['p1:move 1, move 1', 'p2:move 1 2, move 1 2', 'p1:switch 3, move 1', 'p2:move 2, move 2'],
		must: ['[from] move: Wish|[wisher] Blissey'],
	},
	{
		// Natural Cure: both mons that could have it switch out, only one has it -> ambiguous message.
		name: 'natural_cure_ambiguous_switch',
		seed: [17, 18, 19, 20],
		p1: team(
			mon('Altaria', 'naturalcure', ['protect'], { gender: 'F' }),
			mon('Blissey', 'healer', ['protect'], { gender: 'F' }),
			mon('Mudsdale', 'stamina', ['protect']),
			mon('Incineroar', 'intimidate', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['thunderwave', 'protect']),
			mon('Garchomp', 'roughskin', ['thunderwave', 'protect']),
		),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1 2', 'p1:switch 3, switch 4', 'p2:move 2, move 2'],
		must: ['cured by Natural Cure.)'],
	},
	{
		// Natural Cure on a single switch (the cure is revealed) and Regenerator healing one third.
		name: 'natural_cure_and_regenerator_switch',
		seed: [21, 22, 23, 24],
		p1: team(
			mon('Altaria', 'naturalcure', ['protect'], { gender: 'F' }),
			mon('Slowbro', 'regenerator', ['protect']),
			mon('Mudsdale', 'stamina', ['protect']),
			mon('Incineroar', 'intimidate', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['thunderwave', 'ironhead', 'protect']),
			mon('Garchomp', 'roughskin', ['earthquake', 'protect']),
		),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 2 2', 'p1:switch 3, switch 4', 'p2:move 3, move 2'],
		must: ['[from] ability: Natural Cure|[silent]'],
	},
	{
		// Healer (randomChance(3, 10) per statused adjacent ally).
		name: 'healer_ally_status',
		seed: [25, 26, 27, 28],
		search: 60,
		p1: team(
			mon('Blissey', 'healer', ['protect'], { gender: 'F' }),
			mon('Mudsdale', 'stamina', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['thunderwave', 'protect']),
			mon('Garchomp', 'roughskin', ['protect']),
		),
		steps: ['p1:move 1, move 1', 'p2:move 1 2, move 2', 'p1:move 1, move 1', 'p2:move 2, move 2', 'p1:move 1, move 1', 'p2:move 2, move 2'],
		must: ['ability: Healer', '|-curestatus|'],
	},
	{
		// Shed Skin (randomChance(33, 100) once statused).
		name: 'shed_skin_statused',
		seed: [29, 30, 31, 32],
		search: 60,
		p1: team(
			mon('Sandaconda', 'shedskin', ['protect']),
			mon('Mudsdale', 'stamina', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['thunderwave', 'protect']),
			mon('Garchomp', 'roughskin', ['protect']),
		),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 2', 'p1:move 1, move 1', 'p2:move 2, move 2', 'p1:move 1, move 1', 'p2:move 2, move 2'],
		must: ['ability: Shed Skin', '|-curestatus|'],
	},
	{
		// Bad Dreams damages sleeping foes (and Comatose holders) at residual.
		name: 'bad_dreams_sleeping_foe',
		seed: [33, 34, 35, 36],
		search: 60,
		p1: team(
			mon('Darkrai', 'baddreams', ['spore', 'protect'], { gender: 'N' }),
			mon('Mudsdale', 'stamina', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['protect']),
			mon('Komala', 'comatose', ['protect']),
		),
		steps: ['p1:move 1 1, move 2', 'p2:move 1, move 1', 'p1:move 2, move 1', 'p2:move 1, move 1'],
		must: ['[from] ability: Bad Dreams'],
	},
	{
		// Dry Skin: Water absorption (heal or -immune at full HP), rain heal, Fire damage boost.
		name: 'dry_skin_water_rain_fire',
		seed: [37, 38, 39, 40],
		p1: team(
			mon('Toxicroak', 'dryskin', ['protect', 'earthquake']),
			mon('Mudsdale', 'stamina', ['protect']),
		),
		p2: team(
			mon('Incineroar', 'intimidate', ['flamethrower', 'protect']),
			mon('Milotic', 'competitive', ['surf', 'raindance', 'protect'], { gender: 'F' }),
		),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1 1', 'p1:move 1, move 1', 'p2:move 2, move 3', 'p1:move 1, move 1', 'p2:move 3, move 3'],
		must: ['[from] ability: Dry Skin'],
	},
	{
		// Dry Skin in sun takes 1/8 each residual; Drought sets the weather.
		name: 'dry_skin_sun_damage',
		seed: [41, 42, 43, 44],
		p1: team(
			mon('Toxicroak', 'dryskin', ['protect']),
			mon('Mudsdale', 'stamina', ['protect']),
		),
		p2: team(
			mon('Torkoal', 'drought', ['protect']),
			mon('Kingambit', 'defiant', ['protect']),
		),
		steps: ['p1:move 1, move 1', 'p2:move 1, move 1'],
		must: ['[from] ability: Dry Skin'],
	},
	{
		// Salt Cure: 1/8 per residual, 1/4 on a Water or Steel target, -end on switch-out.
		name: 'salt_cure_water_steel',
		seed: [45, 46, 47, 48],
		p1: team(
			mon('Garganacl', 'purifyingsalt', ['saltcure', 'protect']),
			mon('Mudsdale', 'stamina', ['protect']),
			mon('Incineroar', 'intimidate', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['protect']),
			mon('Milotic', 'competitive', ['protect'], { gender: 'F' }),
			mon('Garchomp', 'roughskin', ['protect']),
		),
		steps: ['p1:move 1 1, move 1 2', 'p2:move 1, move 1', 'p1:move 2, move 2', 'p2:move 1, switch 3'],
		must: ['|-start|', 'Salt Cure'],
	},
	{
		// Syrup Bomb: speed drop each residual for the volatile's four turns, silent -end when it wears off.
		name: 'syrup_bomb_speed_drops',
		seed: [49, 50, 51, 52],
		p1: team(
			mon('Dipplin', 'stickyhold', ['syrupbomb', 'protect'], { gender: 'F' }),
			mon('Mudsdale', 'stamina', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['protect']),
			mon('Garchomp', 'roughskin', ['protect']),
		),
		steps: ['p1:move 1 1, move 2', 'p2:move 1, move 1', 'p1:move 2, move 1', 'p2:move 1, move 1', 'p1:move 2, move 1', 'p2:move 1, move 1', 'p1:move 2, move 1', 'p2:move 1, move 1'],
		must: ['Syrup Bomb'],
	},
	{
		// Syrup Bomb's onUpdate removes the volatile when its source leaves the field.
		name: 'syrup_bomb_source_leaves',
		seed: [53, 54, 55, 56],
		p1: team(
			mon('Dipplin', 'stickyhold', ['syrupbomb', 'protect'], { gender: 'F' }),
			mon('Mudsdale', 'stamina', ['protect']),
			mon('Incineroar', 'intimidate', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['protect']),
			mon('Garchomp', 'roughskin', ['protect']),
		),
		steps: ['p1:move 1 1, move 2', 'p2:move 1, move 1', 'p1:switch 3, move 1', 'p2:move 1, move 1'],
		must: ['Syrup Bomb|[silent]'],
	},
	{
		// Heal Pulse (Mega Launcher 3/4 vs half), Jungle Healing and Lunar Blessing heal + cure,
		// Take Heart boosts and cures.
		name: 'heal_moves_cure_and_boost',
		seed: [57, 58, 59, 60],
		p1: team(
			mon('Clawitzer', 'megalauncher', ['healpulse', 'protect']),
			mon('Lilligant', 'chlorophyll', ['junglehealing', 'protect'], { gender: 'F' }),
			mon('Mudsdale', 'stamina', ['protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['ironhead', 'thunderwave', 'protect']),
			mon('Garchomp', 'roughskin', ['earthquake', 'protect']),
		),
		steps: ['p1:move 2, move 2', 'p2:move 2 1, move 1', 'p1:move 1 -2, move 1', 'p2:move 3, move 2'],
		must: ['|-heal|'],
	},
	{
		// Lunar Blessing and Take Heart cure a statused user and apply their boosts; Strength Sap
		// heals by the target's Atk and drops it (and fails at -6 Atk).
		name: 'lunar_take_heart_strength_sap',
		seed: [61, 62, 63, 64],
		p1: team(
			mon('Cresselia', 'levitate', ['lunarblessing', 'protect'], { gender: 'F' }),
			mon('Meowscarada', 'protean', ['takeheart', 'protect']),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['thunderwave', 'protect']),
			mon('Amoonguss', 'regenerator', ['strengthsap', 'protect'], { gender: 'F' }),
		),
		steps: ['p1:move 2, move 2', 'p2:move 1 1, move 1 2', 'p1:move 1, move 1', 'p2:move 1 2, move 1 1'],
		must: ['|-heal|'],
	},
	{
		// Hospitality heals the adjacent ally by a quarter when the holder enters.
		name: 'hospitality_on_entry',
		seed: [65, 66, 67, 68],
		p1: team(
			mon('Mudsdale', 'stamina', ['protect']),
			mon('Incineroar', 'intimidate', ['protect']),
			mon('Sinistcha', 'hospitality', ['protect'], { gender: 'F' }),
		),
		p2: team(
			mon('Kingambit', 'defiant', ['ironhead', 'protect']),
			mon('Garchomp', 'roughskin', ['earthquake', 'protect']),
		),
		steps: ['p1:move 1, move 1', 'p2:move 1 2, move 1 2', 'p1:move 1, switch 3', 'p2:move 2, move 2'],
		must: ['[from] ability: Hospitality'],
	},
];

function play(s, seed) {
	const session = new Session(sim, { seed, teams: [s.p1, s.p2] });
	for (const step of s.steps) {
		const [side, choice] = [step.slice(0, 2), step.slice(3)];
		const r = session.choose(side, choice);
		if (!r.ok) throw new Error(`${s.name}: ${step} rejected: ${r.error}`);
	}
	const log = session.battle.log.map(l => (/^\|t:\|\d+$/.test(l) ? '|t:|' : l));
	return { session, log };
}

let out = '# name\tseed\tp1\tp2\tsteps\tfinal seed\tlog\n';
const wantDebug = process.env.DEBUG;
let written = 0;
let failed = 0;
for (const s of SCENARIOS) {
	let result = null;
	let usedSeed = null;
	const tries = s.search || 1;
	let lastLog = null;
	for (let i = 0; i < tries && !result; i++) {
		const seed = [s.seed[0] + i, s.seed[1], s.seed[2], s.seed[3]];
		let r;
		try {
			r = play(s, seed);
		} catch (e) {
			console.error(`FAIL ${s.name}: ${e.message}`);
			break;
		}
		lastLog = r.log;
		if (s.must.every(m => r.log.some(l => l.includes(m)))) {
			result = r;
			usedSeed = seed;
		}
	}
	if (wantDebug && wantDebug === s.name && (result || lastLog)) console.log((result ? result.log : lastLog).join('\n'));
	if (!result) {
		const missing = lastLog ? s.must.filter(m => !lastLog.some(l => l.includes(m))) : ['<rejected>'];
		console.error(`FAIL ${s.name}: no seed produced ${JSON.stringify(missing)}`);
		failed++;
		continue;
	}
	if (result.session.anomalies.length) throw new Error(`${s.name}: ${result.session.anomalies.join('; ')}`);
	out += [
		s.name, usedSeed.join(','), s.p1, s.p2, s.steps.join(';'),
		result.session.currentSeed().join(','), result.log.join('\\n'),
	].join('\t') + '\n';
	written++;
}
if (failed) {
	console.error(`${failed} scenario(s) failed; table not written`);
	process.exit(1);
}
const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/abilities/healingresidual/scenarios.tsv', import.meta.url));
fs.writeFileSync(dest, out);
console.log(`Wrote ${written} scenarios to ${dest}`);
