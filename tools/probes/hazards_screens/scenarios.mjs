// hazards_screens batch: directed gen9randomdoublesbattle scenarios replayed on the pinned Showdown.
// Output: crates/engine/src/effects/conditions/hazardsscreens/scenarios.txt, consumed by the
// `#[ignore = "needs core"]` scenario test in hazardsscreens/tests.rs (exact log + PRNG seed per step).
//
//   node tools/probes/hazards_screens/scenarios.mjs [path/to/pokemon-showdown]
//
// File format (one record per line; fields separated by a tab where noted):
//   scenario <name>
//   seed <a,b,c,d>                    battle PRNG seed
//   p1 <packed team>
//   p2 <packed team>
//   step <label>\t<p1 choice>\t<p2 choice>   one decision boundary; `start` has no choices and an empty
//                                     choice means that side had no pending request (wait)
//   seed_after <a,b,c,d>              PRNG state after the step
//   log <line>                        normalized log entries produced by the step, in order
// A forced replacement after a faint is appended as an extra `forced<n>` step (chosen automatically).
import fs from 'node:fs';
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

// Weak attackers (level 50) so nobody faints during a directed scenario.
const attackers = () => [
	mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Earth Power', 'Swords Dance'], { level: 50 }),
	mon('Gengar', 'Cursed Body', ['Shadow Ball', 'Sludge Bomb', 'Thunderbolt', 'Hypnosis'], { level: 50 }),
	mon('Incineroar', 'Intimidate', ['Flare Blitz', 'Close Combat', 'Thunder Wave', 'Will-O-Wisp'], { level: 50 }),
	mon('Arcanine', 'Intimidate', ['Flare Blitz', 'Extreme Speed', 'Howl', 'Will-O-Wisp'], { level: 50 }),
];
const hit = 'move 1 1, move 1 2'; // Body Slam on p1a, Shadow Ball on p1b
const repeat = (n, a, b) => Array.from({ length: n }, () => [a, b]);

const S = [];
const scenario = (name, seed, p1, p2, turns) => S.push({ name, seed, teams: [sim.Teams.pack(p1), sim.Teams.pack(p2)], turns });

// 1. Reflect / Light Screen: Light Clay (8 turns) vs none (5), failed restarts, weakened hits, expiry.
scenario('screens_clay_expiry', [11, 22, 33, 44],
	[mon('Blissey', 'Serene Grace', ['Reflect', 'Light Screen', 'Soft-Boiled', 'Thunder Wave'], { item: 'Light Clay', evs: bulk }),
	 mon('Corviknight', 'Pressure', ['Light Screen', 'Reflect', 'Recover', 'Brave Bird'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Reflect', 'Moonblast', 'Thunder Wave', 'Calm Mind']),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	attackers(),
	[['move 1, move 1', hit], ['move 1, move 2', hit], ...repeat(7, 'move 3, move 3', hit)]);

// 2. Aurora Veil under Snow Warning with Light Clay (8 turns), restarts failing, expiry.
scenario('aurora_veil_snow_clay', [101, 202, 303, 404],
	[mon('Abomasnow', 'Snow Warning', ['Aurora Veil', 'Blizzard', 'Wood Hammer', 'Ice Shard'], { item: 'Light Clay', evs: bulk }),
	 mon('Blissey', 'Serene Grace', ['Aurora Veil', 'Soft-Boiled', 'Thunder Wave', 'Reflect'], { evs: bulk }),
	 mon('Froslass', 'Cursed Body', ['Aurora Veil', 'Shadow Ball', 'Ice Beam', 'Will-O-Wisp'])],
	attackers(),
	repeat(9, 'move 1, move 1', hit));

// 3. Aurora Veil with no weather fails (`-fail`); Reflect still works beside it.
scenario('aurora_veil_no_snow', [5, 6, 7, 8],
	[mon('Blissey', 'Serene Grace', ['Aurora Veil', 'Soft-Boiled', 'Thunder Wave', 'Reflect'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Moonblast', 'Thunder Wave', 'Calm Mind', 'Reflect'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	attackers(),
	[['move 1, move 4', hit], ['move 1, move 4', hit]]);

// 4. Aurora Veil next to Reflect and Light Screen: an already covered category is not weakened twice.
scenario('aurora_veil_with_screens', [31, 41, 59, 26],
	[mon('Abomasnow', 'Snow Warning', ['Aurora Veil', 'Blizzard', 'Wood Hammer', 'Ice Shard'], { evs: bulk }),
	 mon('Blissey', 'Serene Grace', ['Reflect', 'Light Screen', 'Thunder Wave', 'Soft-Boiled'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	attackers(),
	[['move 1, move 1', hit], ['move 1, move 2', hit], ['move 1, move 4', hit], ['move 1, move 4', hit]]);

// 5. Entry hazards: layers (3 Spikes, 2 Toxic Spikes, 4th/3rd layer fails), Stealth Rock, Sticky Web,
//    then switch-ins of Flying, Poison (absorbs Toxic Spikes), Steel, Heavy-Duty Boots and plain mons.
scenario('hazards_layers_and_switchins', [71, 72, 73, 74],
	[mon('Forretress', 'Sturdy', ['Spikes', 'Toxic Spikes', 'Stealth Rock', 'Sticky Web'], { evs: bulk }),
	 mon('Skarmory', 'Sturdy', ['Spikes', 'Stealth Rock', 'Toxic Spikes', 'Sticky Web'], { evs: bulk }),
	 mon('Corviknight', 'Pressure', ['Spikes', 'Stealth Rock', 'Toxic Spikes', 'Sticky Web'], { evs: bulk })],
	[mon('Pelipper', 'Keen Eye', ['Scald', 'Hurricane', 'Tailwind', 'Hydro Pump'], { level: 50, evs: bulk }),
	 mon('Gengar', 'Cursed Body', ['Shadow Ball', 'Sludge Bomb', 'Thunderbolt', 'Hypnosis'], { level: 50, evs: bulk }),
	 mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Earth Power', 'Swords Dance'], { item: 'Heavy-Duty Boots', level: 50, evs: bulk }),
	 mon('Toxapex', 'Regenerator', ['Scald', 'Recover', 'Toxic', 'Ice Beam'], { level: 50, evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'], { level: 50, evs: bulk }),
	 mon('Landorus-Therian', 'Intimidate', ['Earthquake', 'U-turn', 'Rock Slide', 'Stealth Rock'], { level: 50, evs: bulk })],
	[
		['move 1, move 1', 'move 1 1, move 1 2'],
		['move 1, move 1', 'move 1 1, move 1 2'],
		['move 2, move 3', 'move 1 1, move 1 2'],
		['move 3, move 2', 'move 1 1, move 1 2'],
		['move 4, move 4', 'move 1 1, move 1 2'],
		['move 1, move 1', 'switch 3, switch 4'],
		['move 1, move 1', 'switch 5, switch 6'],
		['move 1, move 1', 'switch 3, switch 4'],
	]);

// 6. Rapid Spin / Mortal Spin remove the user's own side hazards, Leech Seed and trapping.
scenario('spin_clears_hazards_leechseed_trap', [81, 82, 83, 84],
	[mon('Forretress', 'Sturdy', ['Rapid Spin', 'Spikes', 'Toxic Spikes', 'Stealth Rock'], { evs: bulk }),
	 mon('Toxapex', 'Regenerator', ['Mortal Spin', 'Recover', 'Toxic', 'Scald'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[mon('Skarmory', 'Sturdy', ['Spikes', 'Stealth Rock', 'Toxic Spikes', 'Sticky Web'], { evs: bulk }),
	 mon('Incineroar', 'Intimidate', ['Leech Seed', 'Infestation', 'Flare Blitz', 'Will-O-Wisp'], { level: 50, evs: bulk }),
	 mon('Corviknight', 'Pressure', ['Spikes', 'Stealth Rock', 'Toxic Spikes', 'Sticky Web'], { evs: bulk }),
	 mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Earth Power', 'Swords Dance'], { level: 50, evs: bulk })],
	[
		['move 2, move 2', 'move 1, move 1 1'],
		['move 4, move 2', 'move 2, move 2 1'],
		['move 3, move 2', 'move 3, move 3 2'],
		['move 2, move 2', 'move 4, move 4 2'],
		['move 1 1, move 1', 'move 1, move 1 1'],
		['move 1 1, move 1', 'move 1, move 1 1'],
	]);

// 7. Sheer Force sets move.hasSheerForce: neither Mortal Spin nor Rapid Spin clears anything.
scenario('spin_sheer_force_keeps_hazards', [91, 92, 93, 94],
	[mon('Forretress', 'Sturdy', ['Spikes', 'Toxic Spikes', 'Stealth Rock', 'Sticky Web'], { evs: bulk }),
	 mon('Toxapex', 'Sheer Force', ['Mortal Spin', 'Rapid Spin', 'Toxic', 'Scald'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[mon('Skarmory', 'Sturdy', ['Spikes', 'Stealth Rock', 'Toxic Spikes', 'Sticky Web'], { evs: bulk }),
	 mon('Corviknight', 'Pressure', ['Spikes', 'Stealth Rock', 'Toxic Spikes', 'Sticky Web'], { evs: bulk }),
	 mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Earth Power', 'Swords Dance'], { level: 50, evs: bulk })],
	[
		['move 1, move 3 1', 'move 1, move 1'],
		['move 1, move 3 1', 'move 2, move 3'],
		['move 1, move 3 1', 'move 3, move 4'],
		['move 1, move 1', 'move 1, move 1'],
		['move 1, move 2 1', 'move 1, move 1'],
	]);

// 8. Brick Break / Psychic Fangs / Raging Bull shatter Reflect, Light Screen and Aurora Veil (also
//    through a Substitute); Raging Bull's type follows the Paldean forme.
const tauros = (forme, ability) => mon(`Tauros-Paldea-${forme}`, ability, ['Raging Bull', 'Brick Break', 'Psychic Fangs', 'Swords Dance'], { level: 40, evs: bulk });
scenario('shatter_screens_ragingbull_formes', [201, 202, 203, 204],
	[tauros('Combat', 'Intimidate'), tauros('Blaze', 'Intimidate'), tauros('Aqua', 'Intimidate'),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[mon('Abomasnow', 'Snow Warning', ['Aurora Veil', 'Blizzard', 'Wood Hammer', 'Ice Shard'], { evs: bulk }),
	 mon('Blissey', 'Serene Grace', ['Reflect', 'Light Screen', 'Substitute', 'Soft-Boiled'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Moonblast', 'Thunder Wave', 'Calm Mind', 'Reflect'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[
		['move 4, move 4', 'move 1, move 1'],
		['move 4, move 4', 'move 1, move 2'],
		['move 2 2, move 4', 'move 1, move 3'],
		['move 3 2, move 4', 'move 1, move 1'],
		['move 1 1, move 1 2', 'move 1, move 2'],
		['move 2 2, move 3 1', 'move 1, move 4'],
		['switch 3, move 1 1', 'move 1, move 1'],
		['move 1 1, move 1 2', 'move 1 1, move 4'],
	]);

// 9. Ceaseless Edge (Spikes) / Stone Axe (Stealth Rock): AfterHit, AfterSubDamage behind Substitute,
//    third layer then failure, source defaulting.
scenario('ceaseless_stoneaxe_substitute', [301, 302, 303, 304],
	[mon('Kleavor', 'Sharpness', ['Stone Axe', 'Ceaseless Edge', 'Swords Dance', 'Protect'], { level: 50, evs: bulk }),
	 mon('Samurott-Hisui', 'Sharpness', ['Ceaseless Edge', 'Stone Axe', 'Swords Dance', 'Protect'], { level: 50, evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[mon('Blissey', 'Serene Grace', ['Substitute', 'Soft-Boiled', 'Thunder Wave', 'Reflect'], { evs: bulk }),
	 mon('Corviknight', 'Pressure', ['Substitute', 'Recover', 'Brave Bird', 'Roost'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Moonblast', 'Thunder Wave', 'Calm Mind', 'Reflect'], { evs: bulk })],
	[
		['move 3, move 3', 'move 1, move 1'],
		['move 2 1, move 1 2', 'move 2, move 2'],
		['move 1 1, move 2 2', 'move 2, move 2'],
		['move 2 1, move 1 2', 'move 2, move 2'],
		['move 1 1, move 2 2', 'move 2, move 2'],
		['move 2 1, move 1 2', 'move 2, move 2'],
	]);

// 10. Ice Spinner clears terrain (AfterHit and behind a Substitute).
scenario('icespinner_clears_terrain', [401, 402, 403, 404],
	[mon('Baxcalibur', 'Thermal Exchange', ['Ice Spinner', 'Swords Dance', 'Protect', 'Icicle Crash'], { level: 50, evs: bulk }),
	 mon('Weavile', 'Pressure', ['Ice Spinner', 'Swords Dance', 'Protect', 'Icicle Crash'], { level: 50, evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[mon('Rillaboom', 'Grassy Surge', ['Substitute', 'Swords Dance', 'Wood Hammer', 'Protect'], { evs: bulk }),
	 mon('Blissey', 'Serene Grace', ['Substitute', 'Soft-Boiled', 'Thunder Wave', 'Reflect'], { evs: bulk }),
	 mon('Pincurchin', 'Electric Surge', ['Substitute', 'Recover', 'Thunderbolt', 'Spikes'], { level: 50, evs: bulk })],
	[
		['move 2, move 2', 'move 1, move 1'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 2, move 2', 'switch 3, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
	]);

// 11. Court Change swaps screens / Tailwind / hazards between the sides (twice), and fails when none exist.
scenario('court_change_swap', [501, 502, 503, 504],
	[mon('Forretress', 'Sturdy', ['Spikes', 'Court Change', 'Toxic Spikes', 'Sticky Web'], { evs: bulk }),
	 mon('Pelipper', 'Keen Eye', ['Tailwind', 'Court Change', 'Reflect', 'Light Screen'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[mon('Corviknight', 'Pressure', ['Stealth Rock', 'Reflect', 'Court Change', 'Tailwind'], { evs: bulk }),
	 mon('Blissey', 'Serene Grace', ['Reflect', 'Light Screen', 'Court Change', 'Soft-Boiled'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Moonblast', 'Thunder Wave', 'Calm Mind', 'Reflect'], { evs: bulk })],
	[
		['move 2, move 2', 'move 3, move 3'],
		['move 1, move 1', 'move 1, move 1'],
		['move 2, move 3', 'move 2, move 2'],
		['move 3, move 4', 'move 3, move 2'],
		['move 2, move 2', 'move 3, move 3'],
		['move 2, move 2', 'move 3, move 3'],
	]);

// 12. Tidy Up: removes Substitutes and both sides' hazards, +1 Atk/+1 Spe (also with nothing to clear).
scenario('tidy_up_clears_all', [601, 602, 603, 604],
	[mon('Scizor', 'Technician', ['Tidy Up', 'Substitute', 'Swords Dance', 'Bullet Punch'], { evs: bulk }),
	 mon('Forretress', 'Sturdy', ['Tidy Up', 'Spikes', 'Toxic Spikes', 'Stealth Rock'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Moonblast', 'Thunder Wave', 'Calm Mind', 'Reflect'], { evs: bulk })],
	[mon('Skarmory', 'Sturdy', ['Spikes', 'Stealth Rock', 'Substitute', 'Sticky Web'], { evs: bulk }),
	 mon('Corviknight', 'Pressure', ['Spikes', 'Stealth Rock', 'Toxic Spikes', 'Substitute'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Moonblast', 'Thunder Wave', 'Calm Mind', 'Reflect'], { evs: bulk })],
	[
		['move 2, move 3', 'move 1, move 1'],
		['move 3, move 4', 'move 3, move 4'],
		['move 1, move 1', 'move 2, move 3'],
		['move 1, move 1', 'move 2, move 2'],
		['move 1, move 1', 'move 3, move 4'],
	]);

// 13. Trick Room (5 turns, speed order reversal) and Tailwind (4 turns, recast after expiry).
const speedTeams = () => [
	[mon('Torkoal', 'Drought', ['Trick Room', 'Protect', 'Helping Hand', 'Will-O-Wisp'], { evs: bulk }),
	 mon('Pelipper', 'Keen Eye', ['Tailwind', 'Hurricane', 'Scald', 'Protect'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[mon('Talonflame', 'Flame Body', ['Tailwind', 'Brave Bird', 'Protect', 'Swords Dance'], { level: 50 }),
	 mon('Weavile', 'Pressure', ['Swords Dance', 'Protect', 'Ice Shard', 'Icicle Crash'], { level: 50 }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
];
scenario('trickroom_tailwind_expiry', [701, 702, 703, 704], ...speedTeams(),
	[
		['move 1, move 1', 'move 1, move 1'],
		...repeat(7, 'move 3 -2, move 1', 'move 4, move 1'),
	]);

// 13b. Using Trick Room again ends it (onFieldRestart removes the pseudo-weather), a third use restarts it.
scenario('trickroom_toggle', [711, 712, 713, 714], ...speedTeams(),
	[
		['move 1, move 1', 'move 4, move 1'],
		['move 1, move 1', 'move 4, move 1'],
		['move 1, move 1', 'move 4, move 1'],
	]);

// 14. Toxic Spikes status: one layer poisons (psn), two layers badly poison (tox); the status source is
//     the foe's first active.
scenario('toxic_spikes_status', [801, 802, 803, 804],
	[mon('Forretress', 'Sturdy', ['Toxic Spikes', 'Spikes', 'Stealth Rock', 'Sticky Web'], { evs: bulk }),
	 mon('Skarmory', 'Sturdy', ['Toxic Spikes', 'Spikes', 'Stealth Rock', 'Sticky Web'], { evs: bulk }),
	 mon('Scizor', 'Technician', ['Bullet Punch', 'Swords Dance', 'Close Combat', 'Quick Attack'])],
	[mon('Blissey', 'Serene Grace', ['Reflect', 'Light Screen', 'Soft-Boiled', 'Thunder Wave'], { evs: bulk }),
	 mon('Clefable', 'Magic Guard', ['Moonblast', 'Thunder Wave', 'Calm Mind', 'Reflect'], { evs: bulk }),
	 mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Earth Power', 'Swords Dance'], { level: 50, evs: bulk }),
	 mon('Arcanine', 'Intimidate', ['Flare Blitz', 'Extreme Speed', 'Howl', 'Will-O-Wisp'], { level: 50, evs: bulk })],
	[
		['move 1, move 4', 'move 3, move 3'],
		['move 2, move 2', 'switch 3, move 3'],
		['move 1, move 1', 'move 4, move 3'],
		['move 3, move 3', 'switch 4, move 3'],
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

function run() {
	const out = [];
	for (const sc of S) {
		const base = out.length;
		const session = new Session(sim, { seed: sc.seed, teams: sc.teams });
		out.push(`scenario ${sc.name}`, `seed ${sc.seed.join(',')}`, `p1 ${sc.teams[0]}`, `p2 ${sc.teams[1]}`);
		let snap = session.snapshot();
		out.push('step start\t\t', `seed_after ${snap.seed.join(',')}`, ...snap.log.map(l => `log ${l}`));
		let n = 0, forced = 0;
		const play = (label, c1, c2) => {
			for (const [side, c] of [['p1', c1], ['p2', c2]]) {
				if (c === '') continue;
				const r = session.choose(side, c);
				if (!r.ok) throw new Error(`${sc.name} ${label}: ${side} choice "${c}" rejected: ${r.error}`);
			}
			snap = session.snapshot();
			if (process.env.HZ_DEBUG) console.log(label, snap.log.filter(l => /\|(faint|switch|drag)\|/.test(l)).join(' ; '));
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
		const kinds = (txt) => out.slice(base).filter(l => l.startsWith('log ') && l.includes(txt)).length;
		console.log(`${sc.name}: ${n} turns (${forced} forced), faints=${kinds('|faint|')}`);
		if (session.anomalies.length) throw new Error(`${sc.name}: anomalies ${session.anomalies.join('; ')}`);
	}
	return out;
}
if (process.argv[1]?.endsWith('scenarios.mjs')) {
	const lines = run();
	const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/conditions/hazardsscreens/scenarios.txt', import.meta.url));
	fs.writeFileSync(dest, lines.join('\n') + '\n');
	console.log(`wrote ${dest} (${lines.length} lines)`);
}
