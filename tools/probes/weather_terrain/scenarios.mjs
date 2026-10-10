// Records hand-built weather/terrain battles on the pinned Showdown build, for the
// `#[ignore = "needs core"]` replay tests of the weather_terrain effect batch.
//
//   node tools/probes/weather_terrain/scenarios.mjs [PS_PATH] [--out FILE]
//
// Each scenario is a pair of constructed teams (scoped species, moves and abilities;
// abilities are assigned freely, as in the directed corpora), a Gen5 battle seed and a
// fixed choice script. The recorder drives the real Showdown simulator exactly like
// tools/oracle/lib/session.mjs and writes the omniscient log, turn and PRNG seed of every
// decision boundary to scenarios.txt (format documented at the top of that file).
// A rejected choice aborts the run: the scripts must be legal.
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';

const args = process.argv.slice(2);
const outIdx = args.indexOf('--out');
const outPath = outIdx >= 0 ? args.splice(outIdx, 2)[1] : fileURLToPath(
	new URL('../../../crates/engine/src/effects/abilities/weatherterrain/scenarios.txt', import.meta.url));
const sim = loadSim(args[0] || `${process.env.HOME}/src/pokemon-showdown`);

/** Packed set: NICK|SPECIES|ITEM|ABILITY|MOVES|NATURE|EVS|GENDER|IVS|SHINY|LEVEL|MISC (12 fields). */
function set(species, ability, moves, { gender = 'M', item = '' } = {}) {
	// 252 HP / 128 Def / 128 SpD keeps the constructed Pokemon alive through the short scripts.
	return ['', species, item, ability, moves.join(','), 'Serious', '252,0,128,0,128,0', gender, '', '', '100', ',,,,,'].join('|');
}
const team = (...sets) => sets.join(']');

const SCENARIOS = [
	{
		// All four weather setters switch in together (speed order, replacing one another), then the
		// surviving weather upkeeps and expires: Drizzle/Drought/Sand Stream/Snow Warning, durations,
		// -weather lines with [from]/[of], upkeep ticks and the -weather|none end.
		name: 'weather_leads',
		seed: [1, 2, 3, 4],
		p1: team(
			set('Pelipper', 'Drizzle', ['swordsdance', 'protect', 'roost', 'tailwind']),
			set('Tyranitar', 'Sand Stream', ['swordsdance', 'protect', 'rockslide', 'crunch'])),
		p2: team(
			set('Torkoal', 'Drought', ['swordsdance', 'protect', 'eruption', 'willowisp']),
			set('Abomasnow', 'Snow Warning', ['swordsdance', 'protect', 'blizzard', 'woodhammer'])),
		script: [
			['move 1, move 1', 'move 1, move 1'], ['move 1, move 1', 'move 1, move 1'],
			['move 1, move 1', 'move 1, move 1'], ['move 1, move 1', 'move 1, move 1'],
			['move 1, move 1', 'move 1, move 1'], ['move 1, move 1', 'move 1, move 1'],
		],
	},
	{
		// Sun: Drought, Weather Ball (Fire, doubled), Thunder/Hurricane 50% accuracy, Solar Power
		// (Sp. Atk and residual damage), Chlorophyll, Leaf Guard (status move -immune, secondary
		// paralysis silent, Yawn blocked), sun-scaled healing moves, Fire/Water damage scaling.
		name: 'sun',
		seed: [11, 22, 33, 44],
		p1: team(
			set('Torkoal', 'Drought', ['weatherball', 'thunder', 'synthesis', 'swordsdance']),
			set('Charizard', 'Solar Power', ['flamethrower', 'willowisp', 'yawn', 'moonlight'])),
		p2: team(
			set('Snorlax', 'Chlorophyll', ['synthesis', 'swordsdance', 'weatherball', 'hurricane']),
			set('Blissey', 'Leaf Guard', ['swordsdance', 'seedbomb', 'synthesis', 'surf'], { gender: 'F' })),
		script: [
			['move 1 1, move 2 2', 'move 1, move 1'],
			['move 2 2, move 3 2', 'move 4 1, move 2 1'],
			['move 3, move 4', 'move 1, move 1'],
			['move 1 2, move 1 1', 'move 3 1, move 4'],
			['move 4, move 4', 'move 2, move 3'],
			['move 4, move 4', 'move 2, move 3'],
		],
	},
	{
		// Rain: Drizzle, Swift Swim, Hydration curing a burn at residual (before burn damage), Hurricane
		// and Thunder never miss, Bleakwind/Wildbolt Storm accuracy, Water/Fire scaling, Weather Ball
		// (Water), reduced healing.
		name: 'rain',
		seed: [101, 202, 303, 404],
		p1: team(
			set('Pelipper', 'Drizzle', ['hurricane', 'thunder', 'weatherball', 'roost']),
			set('Kingdra', 'Swift Swim', ['willowisp', 'bleakwindstorm', 'wildboltstorm', 'swordsdance'])),
		p2: team(
			set('Vaporeon', 'Hydration', ['swordsdance', 'protect', 'scald', 'recover']),
			set('Venusaur', 'Chlorophyll', ['synthesis', 'swordsdance', 'flamethrower', 'weatherball'])),
		script: [
			['move 1 1, move 1 1', 'move 1, move 1'],
			['move 2 2, move 2', 'move 3 1, move 3 1'],
			['move 3 1, move 3', 'move 1, move 1'],
			['move 4, move 1 2', 'move 3 2, move 1'],
			['move 4, move 4', 'move 1, move 1'],
			['move 4, move 4', 'move 1, move 1'],
		],
	},
	{
		// Sandstorm: Sand Stream, residual damage with immunities, Sand Rush, Sand Force and its
		// immunity, Rock-type Sp. Def boost, Weather Ball (Rock), Shore Up (0.667).
		name: 'sand',
		seed: [5, 6, 7, 8],
		p1: team(
			set('Tyranitar', 'Sand Stream', ['weatherball', 'rockslide', 'shoreup', 'swordsdance']),
			set('Excadrill', 'Sand Rush', ['earthquake', 'swordsdance', 'shoreup', 'ironhead'])),
		p2: team(
			set('Garchomp', 'Sand Force', ['rockslide', 'swordsdance', 'weatherball', 'shoreup']),
			set('Clefable', 'Unaware', ['moonblast', 'swordsdance', 'recover', 'psychic'])),
		script: [
			['move 1 2, move 4 1', 'move 1, move 1 1'],
			['move 2, move 1', 'move 3 2, move 4'],
			['move 3, move 3', 'move 4, move 1 2'],
			['move 4, move 2', 'move 1, move 3'],
			['move 4, move 2', 'move 2, move 3'],
			['move 4, move 2', 'move 2, move 3'],
		],
	},
	{
		// Snowscape: Snow Warning, Slush Rush, Ice Body healing, Ice-type Defense boost, Blizzard
		// always hits (and its freeze secondary), Weather Ball (Ice), reduced healing.
		name: 'snow',
		seed: [9, 19, 29, 39],
		p1: team(
			set('Abomasnow', 'Snow Warning', ['blizzard', 'weatherball', 'bodyslam', 'swordsdance']),
			set('Beartic', 'Slush Rush', ['icepunch', 'swordsdance', 'bodyslam', 'protect'])),
		p2: team(
			set('Glalie', 'Ice Body', ['bodyslam', 'weatherball', 'swordsdance', 'protect']),
			set('Venusaur', 'Chlorophyll', ['synthesis', 'swordsdance', 'sludgebomb', 'protect'])),
		script: [
			['move 1, move 3 1', 'move 1 1, move 3 1'],
			['move 2 1, move 1', 'move 1 1, move 3 2'],
			['move 4, move 3 2', 'move 2 1, move 3 1'],
			['move 4, move 2', 'move 3, move 1'],
			['move 4, move 2', 'move 3, move 1'],
			['move 4, move 2', 'move 3, move 1'],
		],
	},
	{
		// Electric Terrain: Hadron Engine (set, then -activate when a second holder finds it already
		// up, with a speed-tie shuffle), Psyblade boost, Surge Surfer, sleep immunity (Spore, Yawn
		// -activate), Electric move boost, terrain expiry.
		name: 'electric_terrain',
		seed: [21, 31, 41, 51],
		p1: team(
			set('Miraidon', 'Hadron Engine', ['psyblade', 'thunderbolt', 'protect', 'swordsdance'], { gender: 'N' }),
			set('Raichu-Alola', 'Surge Surfer', ['thunderbolt', 'swordsdance', 'protect', 'psychic'])),
		p2: team(
			set('Miraidon', 'Hadron Engine', ['psyblade', 'thunderbolt', 'protect', 'swordsdance'], { gender: 'N' }),
			set('Amoonguss', 'Unaware', ['spore', 'yawn', 'swordsdance', 'protect'])),
		script: [
			['move 1 1, move 1 1', 'move 1 1, move 1 2'],
			['move 2 1, move 1 1', 'move 4, move 2 2'],
			['move 4, move 4', 'move 4, move 3'],
			['move 4, move 4', 'move 4, move 3'],
			['move 4, move 4', 'move 4, move 3'],
			['move 4, move 4', 'move 4, move 3'],
		],
	},
	{
		// Grassy Terrain: Grassy Surge, Grassy Glide priority (+1 only while grounded in the
		// terrain), Earthquake halved against grounded targets, Grass boost, residual healing,
		// Floral Healing (2/3 in the terrain), Seed Sower replacing the terrain (needs a second
		// terrain first), terrain expiry.
		name: 'grassy_terrain',
		seed: [61, 71, 81, 91],
		p1: team(
			set('Rillaboom', 'Grassy Surge', ['grassyglide', 'woodhammer', 'swordsdance', 'protect']),
			set('Arboliva', 'Seed Sower', ['floralhealing', 'energyball', 'swordsdance', 'protect'])),
		p2: team(
			set('Garchomp', 'Unaware', ['earthquake', 'rockslide', 'swordsdance', 'protect']),
			set('Pincurchin', 'Electric Surge', ['thunderbolt', 'swordsdance', 'protect', 'spikes'])),
		script: [
			['move 1 1, move 2 1', 'move 1, move 1 1'],
			['move 1 1, move 1 -1', 'move 1, move 2'],
			['move 3, move 1 -1', 'move 3, move 1 2'],
			['move 3, move 3', 'move 3, move 3'],
			['move 3, move 3', 'move 3, move 3'],
			['move 3, move 3', 'move 3, move 3'],
			['move 3, move 3', 'move 3, move 3'],
		],
	},
	{
		// Psychic Terrain: Psychic Surge, Expanding Force (1.5x and spread retarget), priority moves
		// blocked for grounded targets (-activate), airborne target hint, ally and self-target
		// exceptions, Psychic boost, terrain expiry.
		name: 'psychic_terrain',
		seed: [3, 1, 4, 1],
		p1: team(
			set('Indeedee', 'Psychic Surge', ['expandingforce', 'swordsdance', 'protect', 'extremespeed'], { gender: 'M' }),
			set('Corviknight', 'Unaware', ['swordsdance', 'protect', 'quickattack', 'ironhead'])),
		p2: team(
			set('Dragonite', 'Unaware', ['extremespeed', 'swordsdance', 'protect', 'quickattack']),
			set('Snorlax', 'Unaware', ['quickattack', 'swordsdance', 'protect', 'bodyslam'])),
		script: [
			['move 1 1, move 2', 'move 1 1, move 1 2'],
			['move 1 2, move 3 2', 'move 4 1, move 1 2'],
			['move 4 1, move 4 1', 'move 4 2, move 4 1'],
			['move 2, move 2', 'move 2, move 2'],
			['move 2, move 2', 'move 2, move 2'],
			['move 2, move 2', 'move 2, move 2'],
		],
	},
];

function record(sc) {
	const s = new Session(sim, { seed: sc.seed, teams: [sc.p1, sc.p2] });
	const out = [];
	out.push(`S ${sc.name}`, `SEED ${sc.seed.join(',')}`, `T1 ${sc.p1}`, `T2 ${sc.p2}`);
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
			// A choice may be a list of alternatives (e.g. for a fainted active slot); a rejection
			// leaves the PRNG and log untouched, so the first accepted alternative is recorded.
			let choice = null;
			let r = null;
			for (const alt of [].concat(alternatives)) {
				choice = alt;
				r = s.choose(side, alt);
				if (r.ok) break;
			}
			if (!r.ok) {
				if (process.env.DEBUG_PROBE) {
					console.error(`--- ${sc.name}`);
					console.error(s.battle.log.filter(l => /^\|(move|faint|turn|-weather|-fieldstart|-fieldend|-activate|-immune|-fail|-heal|-status|cant|-miss|win)/.test(l)).join('\n'));
				}
				throw new Error(`${sc.name} turn ${i + 1}: ${side} "${choice}" rejected: ${r.error}`);
			}
			out.push(`C ${side} ${choice}`);
		}
		boundary();
	}
	if (s.anomalies.length) throw new Error(`${sc.name}: anomalies ${JSON.stringify(s.anomalies)}`);
	out.push(`E ${s.ended ? 'ended' : 'running'}`);
	return out;
}

const header = [
	'# Weather/terrain scenarios recorded from the pinned Showdown build by',
	'# tools/probes/weather_terrain/scenarios.mjs. Do not edit by hand.',
	'# S name | SEED a,b,c,d | T1/T2 packed team | B turn seed (decision boundary) | L raw log line',
	'# (|t:| normalized) produced to reach that boundary | C side choice | E ended/running',
];
const lines = [...header];
let failed = 0;
for (const sc of SCENARIOS) {
	try {
		lines.push(...record(sc));
	} catch (e) {
		failed++;
		console.error(e.message);
	}
}
if (failed) process.exit(1);
fs.writeFileSync(outPath, lines.join('\n') + '\n');
console.log(`Wrote ${SCENARIOS.length} scenarios (${lines.length} lines) to ${outPath}`);
