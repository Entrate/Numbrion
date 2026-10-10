// Regenerates crates/engine/src/effects/items/consumables/scenarios.tsv: small two-player scenarios
// that exercise the consumables batch (berries, Focus Sash, Throat Spray, Weakness Policy, White
// Herb, Cheek Pouch, Gluttony, Harvest, Ripen, Unnerve), with the expected raw battle log and the
// final PRNG seed taken from the pinned Showdown build (tools/oracle/ORACLE_COMMIT).
//
//   node tools/probes/consumables/scenarios.mjs [showdown checkout] [--print NAME]
//
// Columns (tab separated): name, seed, p1 packed team, p2 packed team, steps (';' separated
// "p1:<choice>" / "p2:<choice>", applied in order), final seed, expected log ("\n" escape between
// lines; the wall-clock `|t:|<secs>` lines are normalized to `|t:|`).
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';

const args = process.argv.slice(2);
const printIdx = args.indexOf('--print');
const printName = printIdx >= 0 ? args.splice(printIdx, 2)[1] : null;
const sim = loadSim(args[0] || `${process.env.HOME}/src/pokemon-showdown`);

// name|species|item|ability|moves|nature|evs|gender|ivs|shiny|level|happiness
function mon(name, ability, moves, { level = 100, gender = 'M', item = '', nature = 'Serious', evs = '' } = {}) {
	return `${name}||${item}|${ability}|${moves.join(',')}|${nature}|${evs}|${gender}|||${level}|`;
}
const team = (...mons) => mons.join(']');
// A filler that never acts in a way that matters.
const wall = (name = 'Mudsdale', ability = 'stamina', extra = {}) => mon(name, ability, ['swordsdance'], extra);

const SCENARIOS = [
	{
		// Sitrus Berry: eaten by onUpdate once HP <= 50% (TryHeal + heal(baseMaxhp / 4)); the
		// holder survives the hit. Unnerve on the other side keeps the berry uneaten until it faints.
		name: 'sitrus_pinch',
		seed: [1, 2, 3, 4],
		p1: team(mon('Snorlax', 'levitate', ['swordsdance'], { item: 'sitrusberry', level: 60 }), wall()),
		p2: team(mon('Incineroar', 'intimidate', ['flareblitz'], { level: 80 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: ['Sitrus Berry|[eat]', '[from] item: Sitrus Berry'],
	},
	{
		// Unnerve (onStart -ability line once, onFoeTryEatItem veto): the foe's Sitrus Berry stays
		// uneaten while the Unnerve holder is alive and active.
		name: 'unnerve_blocks_berry',
		seed: [5, 6, 7, 8],
		p1: team(mon('Snorlax', 'levitate', ['swordsdance'], { item: 'sitrusberry', level: 100 }), wall()),
		p2: team(mon('Mewtwo', 'unnerve', ['swordsdance'], { gender: 'N' }), mon('Kingambit', 'defiant', ['ironhead'], { level: 90 })),
		steps: ['p1:move 1, move 1', 'p2:move 1, move 1 1', 'p1:move 1, move 1', 'p2:move 1, move 1 1'],
		must: ['-ability|p2a: Mewtwo|Unnerve'],
	},
	{
		// Knocking out the Unnerve holder: its hp is already 0 when the next Update runs (before the
		// faint is announced), so Unnerve no longer vetoes and Snorlax eats its Sitrus Berry *before*
		// `|faint|` (the foes() lists drop hp 0 Pokemon, side.ts:392-397). Needs core event collection
		// to filter Ally/Foe/Any listeners on hp, see the report.
		name: 'unnerve_holder_ko_frees_berry',
		seed: [65, 66, 67, 68],
		p1: team(
			mon('Snorlax', 'levitate', ['swordsdance'], { item: 'sitrusberry', level: 100 }),
			mon('Kingambit', 'defiant', ['ironhead', 'swordsdance'], { level: 100 }),
		),
		p2: team(
			mon('Mewtwo', 'unnerve', ['swordsdance'], { gender: 'N', level: 5 }),
			mon('Garchomp', 'roughskin', ['rockslide'], { level: 90 }),
		),
		steps: [
			'p1:move 1, move 2', 'p2:move 1, move 1',
			'p1:move 1, move 2', 'p2:move 1, move 1',
			'p1:move 1, move 1 1', 'p2:move 1, move 1',
		],
		must: ['-ability|p2a: Mewtwo|Unnerve', 'Sitrus Berry|[eat]'],
	},
	{
		// Figy and Aguav with Gluttony (<= 50%): heal 1/3 and confusion because the natures' minus
		// stats match (Modest = -atk, Naughty = -spd). Rock Slide is a spread hit on both holders.
		name: 'pinch_berries_confusion_a',
		seed: [9, 10, 11, 12],
		p1: team(
			mon('Snorlax', 'gluttony', ['swordsdance'], { item: 'figyberry', level: 60, nature: 'Modest' }),
			mon('Swalot', 'gluttony', ['swordsdance'], { item: 'aguavberry', level: 60, nature: 'Naughty' }),
		),
		p2: team(mon('Garchomp', 'roughskin', ['rockslide'], { level: 80 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1, move 1', 'p1:move 1, move 1', 'p2:move 1, move 1'],
		must: ['Figy Berry|[eat]', 'Aguav Berry|[eat]', '-start|p1a: Snorlax|confusion', '-start|p1b: Swalot|confusion'],
	},
	{
		// Iapapa (Lonely = -def) and Wiki (Adamant = -spa) without Gluttony: the first Rock Slide only
		// brings them under 50%, the second under 25%, which is where they are eaten.
		name: 'pinch_berries_confusion_b',
		seed: [57, 58, 59, 60],
		p1: team(
			mon('Mudsdale', 'stamina', ['swordsdance'], { item: 'iapapaberry', level: 40, nature: 'Lonely' }),
			mon('Kingambit', 'defiant', ['swordsdance'], { item: 'wikiberry', level: 40, nature: 'Adamant' }),
		),
		p2: team(mon('Garchomp', 'roughskin', ['rockslide'], { level: 60 }), wall('Snorlax', 'levitate')),
		steps: ['p1:move 1, move 1', 'p2:move 1, move 1', 'p1:move 1, move 1', 'p2:move 1, move 1'],
		must: [],
	},
	{
		// Mago (Brave = -spe) next to a neutral-nature holder: only the matching nature is confused.
		name: 'mago_neutral_nature',
		seed: [61, 62, 63, 64],
		p1: team(
			mon('Snorlax', 'gluttony', ['swordsdance'], { item: 'magoberry', level: 60, nature: 'Brave' }),
			mon('Swalot', 'gluttony', ['swordsdance'], { item: 'figyberry', level: 60, nature: 'Serious' }),
		),
		p2: team(mon('Garchomp', 'roughskin', ['rockslide'], { level: 80 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1, move 1', 'p1:move 1, move 1', 'p2:move 1, move 1'],
		must: ['Mago Berry|[eat]', 'Figy Berry|[eat]', '-start|p1a: Snorlax|confusion'],
	},
	{
		// Gluttony: abilityState.gluttony (set at start and by every Damage event) lets Aguav fire at
		// <= 50% instead of <= 25%.
		name: 'gluttony_half_hp',
		seed: [13, 14, 15, 16],
		p1: team(mon('Swalot', 'gluttony', ['swordsdance'], { item: 'aguavberry', level: 70 }), wall()),
		p2: team(mon('Incineroar', 'intimidate', ['flareblitz'], { level: 60 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: ['Aguav Berry|[eat]'],
	},
	{
		// Chesto Berry: onUpdate eats it while asleep and onEat cures sleep.
		name: 'chesto_cures_sleep',
		seed: [17, 18, 19, 20],
		p1: team(mon('Snorlax', 'levitate', ['swordsdance'], { item: 'chestoberry', level: 80 }), wall()),
		p2: team(mon('Breloom', 'technician', ['spore'], { level: 80 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: ['Chesto Berry|[eat]', '-curestatus|p1a: Snorlax|slp|[msg]'],
	},
	{
		// Focus Sash: survives a would-be KO from full HP at 1 HP, item consumed.
		name: 'focus_sash',
		seed: [21, 22, 23, 24],
		p1: team(mon('Dedenne', 'cheekpouch', ['swordsdance'], { item: 'focussash', level: 50 }), wall()),
		p2: team(mon('Kingambit', 'defiant', ['ironhead'], { level: 100 }), wall('Incineroar', 'intimidate')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: ['Focus Sash'],
	},
	{
		// Weakness Policy: a super effective hit uses the item (atk +2, spa +2); a resisted hit and a
		// fixed-damage hit would not.
		name: 'weakness_policy',
		seed: [25, 26, 27, 28],
		p1: team(mon('Snorlax', 'levitate', ['swordsdance'], { item: 'weaknesspolicy', level: 100 }), wall()),
		p2: team(mon('Hariyama', 'guts', ['closecombat'], { level: 60 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: ['-enditem|p1a: Snorlax|Weakness Policy', '-boost|p1a: Snorlax|atk|2|[from] item: Weakness Policy', '-boost|p1a: Snorlax|spa|2|[from] item: Weakness Policy'],
	},
	{
		// Throat Spray: a sound move used by the holder consumes it and raises Sp. Atk.
		name: 'throat_spray',
		seed: [29, 30, 31, 32],
		p1: team(mon('Dragonite', 'soundproof', ['hypervoice'], { item: 'throatspray', level: 70 }), wall()),
		p2: team(mon('Snorlax', 'levitate', ['swordsdance']), mon('Kingambit', 'defiant', ['swordsdance'])),
		steps: ['p1:move 1, move 1', 'p2:move 1, move 1'],
		must: ['Throat Spray', '-boost|p1a: Dragonite|spa|1'],
	},
	{
		// White Herb: Intimidate lowers Atk, the herb answers on the AnySwitchIn pass (priority -2)
		// and resets the stage (-clearnegativeboost).
		name: 'white_herb_intimidate',
		seed: [33, 34, 35, 36],
		p1: team(mon('Snorlax', 'levitate', ['swordsdance'], { item: 'whiteherb' }), wall()),
		p2: team(mon('Incineroar', 'intimidate', ['swordsdance']), wall('Mudsdale', 'stamina')),
		steps: ['p1:move 1, move 1', 'p2:move 1, move 1'],
		must: ['White Herb', '-clearnegativeboost'],
	},
	{
		// Cheek Pouch heals another 1/3 whenever a berry is eaten (EatItem), on top of Sitrus.
		name: 'cheek_pouch_sitrus',
		seed: [37, 38, 39, 40],
		p1: team(mon('Greedent', 'cheekpouch', ['swordsdance'], { item: 'sitrusberry', level: 60, gender: 'F' }), wall()),
		p2: team(mon('Incineroar', 'intimidate', ['flareblitz'], { level: 70 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: ['Sitrus Berry|[eat]'],
	},
	{
		// Ripen: -activate on TryEatItem, doubled healing through TryHeal's chainModify(2).
		name: 'ripen_sitrus',
		seed: [41, 42, 43, 44],
		p1: team(mon('Appletun', 'ripen', ['swordsdance'], { item: 'sitrusberry', level: 60, gender: 'F' }), wall()),
		p2: team(mon('Incineroar', 'intimidate', ['flareblitz'], { level: 70 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: ['ability: Ripen'],
	},
	{
		// Harvest in Sun (Drought, no draw) and without (randomChance(1, 2)): the eaten berry returns
		// at residual, `-item|mon|Sitrus Berry|[from] ability: Harvest`.
		name: 'harvest_sun',
		seed: [45, 46, 47, 48],
		p1: team(mon('Exeggutor', 'harvest', ['swordsdance'], { item: 'sitrusberry', level: 100 }), mon('Torkoal', 'drought', ['swordsdance'], { level: 50 })),
		p2: team(mon('Incineroar', 'intimidate', ['flareblitz'], { level: 50 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1', 'p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: ['Harvest'],
	},
	{
		// Harvest with no sun: one randomChance(1, 2) per residual after the berry is eaten.
		name: 'harvest_random',
		seed: [49, 50, 51, 52],
		p1: team(mon('Exeggutor', 'harvest', ['swordsdance'], { item: 'sitrusberry', level: 100 }), wall()),
		p2: team(mon('Incineroar', 'intimidate', ['flareblitz'], { level: 50 }), wall('Kingambit', 'defiant')),
		steps: ['p1:move 1, move 1', 'p2:move 1 1, move 1', 'p1:move 1, move 1', 'p2:move 1 1, move 1', 'p1:move 1, move 1', 'p2:move 1 1, move 1'],
		must: [],
	},
];

// Runs one scenario; returns { log, session } or the first missing `must` string.
function run(s, seed) {
	const session = new Session(sim, { seed, teams: [s.p1, s.p2] });
	for (const step of s.steps) {
		const [side, choice] = [step.slice(0, 2), step.slice(3)];
		const r = session.choose(side, choice);
		if (!r.ok) throw new Error(`${s.name}: ${step} rejected: ${r.error}`);
	}
	const log = session.battle.log.map(l => (/^\|t:\|\d+$/.test(l) ? '|t:|' : l));
	const missing = s.must.find(m => !log.some(l => l.includes(m)));
	return { log, session, missing };
}

let out = '# name\tseed\tp1\tp2\tsteps\tfinal seed\tlog\n';
for (const s of SCENARIOS) {
	// Rolls (accuracy, flinch, sleep duration) are seed dependent: take the first seed, starting at
	// the declared one, for which every `must` line is present.
	let seed = s.seed;
	let r = run(s, seed);
	for (let k = 1; r.missing !== undefined && k <= 200; k++) {
		seed = [s.seed[0] + k, s.seed[1], s.seed[2], s.seed[3]];
		r = run(s, seed);
	}
	if (printName === s.name) console.log(r.log.join('\n'));
	if (r.missing !== undefined) throw new Error(`${s.name}: expected a log line containing ${JSON.stringify(r.missing)}\n${r.log.join('\n')}`);
	if (r.session.anomalies.length) throw new Error(`${s.name}: ${r.session.anomalies.join('; ')}`);
	out += [
		s.name, seed.join(','), s.p1, s.p2, s.steps.join(';'),
		r.session.currentSeed().join(','), r.log.join('\\n'),
	].join('\t') + '\n';
}
if (!printName) {
	const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/items/consumables/scenarios.tsv', import.meta.url));
	fs.writeFileSync(dest, out);
	console.log(`Wrote ${SCENARIOS.length} scenarios to ${dest}`);
}
