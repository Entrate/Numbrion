// Regenerates crates/engine/src/effects/abilities/reactivestats/scenarios.tsv: small two-player
// scenarios that exercise the reactive_stats abilities, with the expected raw battle log and
// final PRNG seed taken from the pinned Showdown build (tools/oracle/ORACLE_COMMIT).
//
//   node tools/probes/reactive_stats/scenarios.mjs [showdown checkout]
//
// Columns (tab separated): name, seed, p1 packed team, p2 packed team, steps (';' separated
// "p1:<choice>" / "p2:<choice>", applied in order), final seed, expected log ("\n" escape between
// lines; the wall-clock `|t:|<secs>` lines are normalized to `|t:|`).
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
		// Intimidate drops both foes; Defiant and Competitive answer (AfterEachBoost, self boost).
		name: 'intimidate_defiant_competitive',
		seed: [1, 2, 3, 4],
		p1: team(mon('Incineroar', 'intimidate', ['knockoff', 'protect']), mon('Mudsdale', 'stamina', ['protect'])),
		p2: team(mon('Kingambit', 'defiant', ['protect']), mon('Milotic', 'competitive', ['protect'], { gender: 'F' })),
		steps: [],
		must: ['|-ability|p1a: Incineroar|Intimidate|boost', 'Defiant|boost', 'Competitive|boost'],
	},
	{
		// Rattled's AfterBoost fires only for Intimidate-caused Atk changes; Download reads foes' Def/SpD.
		name: 'intimidate_rattled_download',
		seed: [5, 6, 7, 8],
		p1: team(mon('Incineroar', 'intimidate', ['protect']), mon('Porygon2', 'download', ['protect'], { gender: 'N' })),
		p2: team(mon('Dudunsparce', 'rattled', ['protect'], { gender: 'F' }), mon('Mudsdale', 'stamina', ['protect'])),
		steps: [],
		must: ['Rattled', 'Download'],
	},
	{
		// Moxie after a KO (AfterFaint relay length), Soul-Heart reacting to any faint, Stamina on a hit.
		name: 'moxie_soulheart_stamina',
		seed: [9, 10, 11, 12],
		p1: team(mon('Honchkrow', 'moxie', ['bravebird', 'protect']), mon('Magearna', 'soulheart', ['protect'], { gender: 'N' })),
		p2: team(mon('Wigglytuff', 'competitive', ['playrough'], { level: 5, gender: 'F' }), mon('Mudsdale', 'stamina', ['highhorsepower']), mon('Milotic', 'competitive', ['scald'], { gender: 'F' })),
		steps: ['p1:move 1 1, move 1', 'p2:move 1 1, move 1 1', 'p2:switch 3'],
		must: ['Moxie|boost', 'Soul-Heart|boost'],
	},
	{
		// Contact reactions: Gooey, Weak Armor and Stamina hit by contact moves.
		name: 'contact_weakarmor_gooey_stamina',
		seed: [13, 14, 15, 16],
		p1: team(mon('Wugtrio', 'gooey', ['liquidation']), mon('Ceruledge', 'weakarmor', ['xscissor'], { gender: 'F' })),
		p2: team(mon('Kingambit', 'defiant', ['suckerpunch']), mon('Mudsdale', 'stamina', ['rockslide'])),
		steps: ['p1:move 1 2, move 1 2', 'p2:move 1 1, move 1'],
		must: ['Gooey', 'Weak Armor', 'Stamina'],
	},
	{
		// Wind Rider: onStart/onSideConditionStart with Tailwind, and onTryHit absorbing a wind move.
		name: 'windrider_tailwind_wind_move',
		seed: [17, 18, 19, 20],
		p1: team(mon('Brambleghast', 'windrider', ['seedbomb'], { gender: 'F' }), mon('Incineroar', 'intimidate', ['tailwind'])),
		p2: team(mon('Honchkrow', 'moxie', ['hurricane']), mon('Mudsdale', 'stamina', ['protect'])),
		steps: ['p1:move 1 1, move 1', 'p2:move 1 1, move 1'],
		must: ['Wind Rider'],
	},
	{
		// Well-Baked Body and Thermal Exchange absorbing Fire; Thermal Exchange blocks a burn.
		name: 'fire_absorb_wellbaked_thermal',
		seed: [21, 22, 23, 24],
		p1: team(mon('Dachsbun', 'wellbakedbody', ['playrough'], { gender: 'F' }), mon('Baxcalibur', 'thermalexchange', ['dragonclaw'])),
		p2: team(mon('Incineroar', 'intimidate', ['flareblitz']), mon('Magcargo', 'weakarmor', ['willowisp'])),
		steps: ['p1:move 1 1, move 1 2', 'p2:move 1 1, move 1 2', 'p1:move 1 1, move 1 2', 'p2:move 1 2, move 1 1'],
		must: ['Well-Baked Body', 'Thermal Exchange'],
	},
	{
		// Berserk: the boost happens in AfterMoveSecondary once a single hit crosses half HP.
		name: 'berserk_half_hp',
		seed: [33, 34, 35, 36],
		p1: team(mon('Moltres-Galar', 'berserk', ['flamethrower']), mon('Mudsdale', 'stamina', ['protect'])),
		p2: team(mon('Kingambit', 'defiant', ['ironhead']), mon('Honchkrow', 'moxie', ['bravebird'])),
		steps: ['p1:move 1 1, move 1', 'p2:move 1 1, move 1 1'],
		must: ['Berserk|boost'],
	},
	{
		// Speed Boost after the first full turn (activeTurns truthiness).
		name: 'speedboost_residual',
		seed: [25, 26, 27, 28],
		p1: team(mon('Blaziken', 'speedboost', ['protect']), mon('Espathra', 'speedboost', ['protect'], { gender: 'F' })),
		p2: team(mon('Mudsdale', 'stamina', ['protect']), mon('Kingambit', 'defiant', ['protect'])),
		steps: ['p1:move 1, move 1', 'p2:move 1, move 1', 'p1:move 1, move 1', 'p2:move 1, move 1'],
		must: ['Speed Boost|boost'],
	},
	{
		// Ruin abilities: -ability lines at start and the 0.75 drop applied once per stat.
		name: 'ruin_abilities_damage',
		seed: [29, 30, 31, 32],
		p1: team(mon('Chi-Yu', 'beadsofruin', ['heatwave']), mon('Ting-Lu', 'vesselofruin', ['shadowball'])),
		p2: team(mon('Wo-Chien', 'tabletsofruin', ['leafblade']), mon('Chien-Pao', 'swordofruin', ['iciclecrash'])),
		steps: ['p1:move 1, move 1 1', 'p2:move 1 1, move 1 2'],
		must: ['Beads of Ruin', 'Tablets of Ruin', 'Sword of Ruin', 'Vessel of Ruin'],
	},
];

let out = '# name\tseed\tp1\tp2\tsteps\tfinal seed\tlog\n';
for (const s of SCENARIOS) {
	const session = new Session(sim, { seed: s.seed, teams: [s.p1, s.p2] });
	for (const step of s.steps) {
		const [side, choice] = [step.slice(0, 2), step.slice(3)];
		const r = session.choose(side, choice);
		if (!r.ok) throw new Error(`${s.name}: ${step} rejected: ${r.error}`);
	}
	const log = session.battle.log.map(l => (/^\|t:\|\d+$/.test(l) ? '|t:|' : l));
	for (const m of s.must) {
		if (!log.some(l => l.includes(m))) throw new Error(`${s.name}: expected a log line containing ${JSON.stringify(m)}\n${log.join('\n')}`);
	}
	if (session.anomalies.length) throw new Error(`${s.name}: ${session.anomalies.join('; ')}`);
	out += [
		s.name, s.seed.join(','), s.p1, s.p2, s.steps.join(';'),
		session.currentSeed().join(','), log.join('\\n'),
	].join('\t') + '\n';
}
const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/abilities/reactivestats/scenarios.tsv', import.meta.url));
fs.writeFileSync(dest, out);
console.log(`Wrote ${SCENARIOS.length} scenarios to ${dest}`);
