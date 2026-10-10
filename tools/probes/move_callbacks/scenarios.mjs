// move_callbacks batch: directed gen9randomdoublesbattle scenarios replayed on the pinned Showdown.
// Output: crates/engine/src/effects/moves/movecallbacks/scenarios.txt, consumed by the scenario test in
// acrobatics.rs -> movecallbacks/tests.rs (exact log + PRNG seed per decision boundary).
//
//   node tools/probes/move_callbacks/scenarios.mjs [path/to/pokemon-showdown]
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
//
// Every scenario targets a branch that the random directed corpus (tools/oracle/gen-directed.mjs
// --profile batch:move_callbacks) reaches rarely or never: that world is Levitate-heavy, has no Hoopa,
// Substitute or boosting setup next to the callbacks, and rarely Terastallizes.
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
		gender: sp.gender || 'M', level: o.level ?? 100, teraType: o.teraType ?? sp.types[0],
		evs: o.evs ?? { hp: 0, atk: 0, def: 0, spa: 0, spd: 0, spe: 0 },
		ivs: { hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31 },
	};
}
const bulk = { hp: 252, atk: 0, def: 128, spa: 0, spd: 128, spe: 0 };
const fast = { hp: 0, atk: 0, def: 0, spa: 0, spd: 0, spe: 252 };

const S = [];
const scenario = (name, seed, p1, p2, turns) => S.push({ name, seed, teams: [sim.Teams.pack(p1), sim.Teams.pack(p2)], turns });

// A passive wall used as filler: Recover (fails quietly at full HP) / Reflect / Light Screen / Tailwind,
// so it never faints during the directed turns. The scenarios avoid every effect that other effect
// batches port (Protect, Substitute, Knock Off, Prankster, Magician, Lightning Rod, ...) so that they
// run on a tree holding only this batch; the Substitute interactions are the `_substitute` scenarios.
const wall = (species = 'Blissey', ability = 'Serene Grace', o = {}) =>
	mon(species, ability, ['Recover', 'Reflect', 'Light Screen', 'Tailwind'], { evs: bulk, ...o });
const walls = () => [wall('Blissey', 'Serene Grace'), wall('Corviknight', 'Pressure'), wall('Clefable', 'Magic Guard')];
// Ghost / Poison friendly walls (Blissey is Normal, Corviknight Flying/Steel).
const wallsA = () => [wall('Clefable', 'Magic Guard'), wall('Blissey', 'Serene Grace'), wall('Corviknight', 'Pressure')];

// 1. Acrobatics (holding Leftovers vs no item) and Facade (healthy, paralyzed, burned).
scenario('acrobatics_facade', [101, 102, 103, 104],
	[mon('Garchomp', 'Rough Skin', ['Acrobatics', 'Facade', 'Swords Dance', 'Recover'], { item: 'Leftovers', evs: fast }),
	 mon('Snorlax', 'Thick Fat', ['Facade', 'Acrobatics', 'Recover', 'Swords Dance'], { evs: bulk }),
	 wall()],
	[mon('Incineroar', 'Intimidate', ['Thunder Wave', 'Will-O-Wisp', 'Recover', 'Reflect'], { evs: bulk }),
	 mon('Amoonguss', 'Regenerator', ['Spore', 'Recover', 'Reflect', 'Light Screen'], { evs: bulk }),
	 wall('Clefable', 'Magic Guard')],
	[
		['move 1 1, move 1 2', 'move 1 2, move 2'],    // Acrobatics holding Leftovers; Facade healthy; Thunder Wave on Snorlax
		['move 2 1, move 2 2', 'move 2 1, move 2'],    // Garchomp Facade healthy; Snorlax Acrobatics with no item; Will-O-Wisp
		['move 2 1, move 1 2', 'move 3, move 3'],      // Facade burned (Garchomp) and paralyzed (Snorlax): doubled
		['move 2 1, move 1 2', 'move 3, move 3'],
	]);

// 2. Stomping Tantrum: doubled right after a failed move (Recover at full HP), not after a success.
scenario('stomping_tantrum_after_failure', [111, 112, 113, 114],
	[mon('Garchomp', 'Rough Skin', ['Recover', 'Stomping Tantrum', 'Swords Dance', 'Earthquake'], { level: 50 }),
	 mon('Dragonite', 'Multiscale', ['Recover', 'Stomping Tantrum', 'Dragon Dance', 'Extreme Speed'], { level: 50 }),
	 wall()],
	[wall('Blissey', 'Serene Grace'), wall('Clefable', 'Magic Guard'), wall('Corviknight', 'Pressure')],
	[
		['move 1, move 1', 'move 2, move 2'],       // Recover at full HP fails: moveLastTurnResult = false
		['move 2 2, move 2 1', 'move 2, move 2'],   // Stomping Tantrum doubled
		['move 2 2, move 2 1', 'move 2, move 2'],   // not doubled (the last move succeeded)
		['move 1, move 1', 'move 2, move 2'],
		['move 2 2, move 2 1', 'move 2, move 2'],   // doubled again
	]);

// 3. Avalanche (damaged by the target this turn vs not), Rage Fist (times attacked), Lash Out (stat
//    drop earlier in the turn).
scenario('avalanche_ragefist_lashout', [121, 122, 123, 124],
	[mon('Kingambit', 'Defiant', ['Avalanche', 'Rage Fist', 'Lash Out', 'Recover'], { level: 50, evs: bulk }),
	 mon('Ursaluna', 'Guts', ['Rage Fist', 'Avalanche', 'Lash Out', 'Recover'], { level: 50, evs: bulk }),
	 wall()],
	[mon('Incineroar', 'Intimidate', ['Body Slam', 'Snarl', 'Recover', 'Reflect'], { level: 50, evs: bulk }),
	 mon('Grimmsnarl', 'Pressure', ['Snarl', 'Body Slam', 'Recover', 'Thunder Wave'], { level: 50, evs: bulk }),
	 wall('Corviknight', 'Pressure')],
	[
		['move 1 1, move 1 2', 'move 1 1, move 2 2'],   // hit by both foes: Avalanche doubled, Rage Fist counts hits
		['move 1 1, move 1 2', 'move 3, move 3'],       // foes do not attack: Avalanche not doubled
		['move 3 1, move 3 2', 'move 2, move 1'],       // Snarl drops attack first -> Lash Out doubled
		['move 3 1, move 3 2', 'move 3, move 3'],       // no drop this turn: Lash Out normal
	]);

// 3b. Last Respects counts the side's fainted Pokemon; Explosion faints the user at once.
scenario('last_respects_after_faints', [131, 132, 133, 134],
	[mon('Gengar', 'Cursed Body', ['Explosion', 'Shadow Ball', 'Recover', 'Last Respects'], { level: 50 }),
	 mon('Chandelure', 'Flame Body', ['Last Respects', 'Explosion', 'Shadow Ball', 'Recover'], { level: 50 }),
	 mon('Dragapult', 'Clear Body', ['Last Respects', 'Shadow Ball', 'Recover', 'Explosion'], { level: 50 }),
	 mon('Mimikyu', 'Disguise', ['Last Respects', 'Shadow Ball', 'Recover', 'Explosion'], { level: 50 })],
	wallsA(),
	[
		['move 1, move 4', 'move 1, move 1'],       // Gengar Explosion (faints), Chandelure Recover
		['move 1 1, move 1 2', 'move 1, move 1'],   // Dragapult + Chandelure Last Respects (1 fainted)
		['move 1 1, move 2', 'move 1, move 1'],     // Chandelure Explosion
		['move 1 1, move 1 2', 'move 1, move 1'],   // Mimikyu + Dragapult Last Respects (2 fainted)
	]);

// 4. HP-scaled power: Eruption / Water Spout / Dragon Energy at full HP and after Belly Drum halves HP.
scenario('hp_scaled_power_after_bellydrum', [141, 142, 143, 144],
	[mon('Typhlosion', 'Blaze', ['Eruption', 'Belly Drum', 'Reflect', 'Recover'], { evs: bulk }),
	 mon('Kyogre', 'Drizzle', ['Water Spout', 'Belly Drum', 'Reflect', 'Recover'], { evs: bulk }),
	 wall()],
	walls(),
	[
		['move 1, move 1', 'move 1, move 1'],       // full HP power
		['move 2, move 2', 'move 1, move 1'],       // Belly Drum at full HP: +6 Atk, half HP
		['move 2, move 2', 'move 1, move 1'],       // Belly Drum again: HP <= 50% fails
		['move 1, move 1', 'move 1, move 1'],       // powers at half HP
	]);
scenario('dragonenergy_after_clangoroussoul', [145, 146, 147, 148],
	[mon('Rayquaza', 'Air Lock', ['Dragon Energy', 'Clangorous Soul', 'Reflect', 'Recover'], { evs: bulk }),
	 mon('Kommo-o', 'Soundproof', ['Dragon Energy', 'Clangorous Soul', 'Reflect', 'Recover'], { evs: bulk }),
	 wall()],
	walls(),
	[
		['move 1, move 1', 'move 1, move 1'],       // full HP
		['move 2, move 2', 'move 1, move 1'],       // Clangorous Soul: +1 all, HP -33%
		['move 2, move 2', 'move 1, move 1'],       // second Clangorous Soul
		['move 2, move 2', 'move 1, move 1'],       // third use: HP is at 34% -> still allowed, ends near 1%
		['move 2, move 2', 'move 1, move 1'],       // fourth use fails the onTry threshold
		['move 1, move 1', 'move 1, move 1'],       // Dragon Energy at very low HP
	]);

// 5. Belly Drum gates: already +6 Atk (fails at high HP), Contrary (boost of +12 becomes -12).
scenario('bellydrum_gates', [151, 152, 153, 154],
	[mon('Snorlax', 'Thick Fat', ['Belly Drum', 'Swords Dance', 'Reflect', 'Recover'], { evs: bulk }),
	 mon('Serperior', 'Contrary', ['Belly Drum', 'Clangorous Soul', 'Reflect', 'Recover'], { evs: bulk }),
	 wall()],
	walls(),
	[
		['move 2, move 1', 'move 1, move 1'],   // Snorlax +2 Atk; Serperior Belly Drum under Contrary
		['move 2, move 2', 'move 1, move 1'],   // Snorlax +4; Serperior Clangorous Soul under Contrary
		['move 2, move 2', 'move 1, move 1'],   // Snorlax +6
		['move 1, move 2', 'move 1, move 1'],   // Belly Drum at +6 Atk and full HP fails
		['move 1, move 2', 'move 1, move 1'],
	]);

// 6. Hyperspace Fury: Hoopa-Unbound succeeds (-1 Def), Hoopa fails with [forme], others plain fail.
scenario('hyperspacefury_forms', [161, 162, 163, 164],
	[mon('Hoopa-Unbound', 'Pressure', ['Hyperspace Fury', 'Recover', 'Reflect', 'Psychic'], { level: 50 }),
	 mon('Hoopa', 'Pressure', ['Hyperspace Fury', 'Recover', 'Reflect', 'Psychic'], { level: 50 }),
	 mon('Mew', 'Synchronize', ['Hyperspace Fury', 'Recover', 'Psychic', 'Transform'], { level: 50 }),
	 wall()],
	wallsA(),
	[
		['move 1 1, move 1 1', 'move 1, move 1'],
		['move 1 1, switch 3', 'move 1, move 1'],
		['move 2, move 1 1', 'move 1, move 1'],
	]);

// 7. Double Shock: Electric user loses the type, repeated use fails; Electric/Flying keeps Flying;
//    Fighting/Electric keeps Fighting.
scenario('doubleshock_types', [171, 172, 173, 174],
	[mon('Raichu', 'Static', ['Double Shock', 'Recover', 'Thunderbolt', 'Volt Switch'], { level: 50 }),
	 mon('Zapdos', 'Pressure', ['Double Shock', 'Recover', 'Roost', 'Hurricane'], { level: 50 }),
	 mon('Iron Hands', 'Quark Drive', ['Double Shock', 'Recover', 'Drain Punch', 'Wild Charge'], { level: 50 }),
	 wall()],
	walls(),
	[
		['move 1 1, move 1 2', 'move 1, move 1'],
		['move 1 1, move 1 2', 'move 1, move 1'],   // both lost Electric: -fail
		['move 2, switch 3', 'move 1, move 1'],
		['move 2, move 1 1', 'move 1, move 1'],
	]);
scenario('doubleshock_tera', [181, 182, 183, 184],
	[mon('Iron Hands', 'Quark Drive', ['Double Shock', 'Recover', 'Drain Punch', 'Wild Charge'], { level: 50, teraType: 'Electric' }),
	 mon('Zapdos', 'Pressure', ['Double Shock', 'Recover', 'Roost', 'Hurricane'], { level: 50 }),
	 wall()],
	walls(),
	[
		['move 1 1 terastallize, move 2', 'move 1, move 1'],   // Terastallized Electric keeps its type after Double Shock
		['move 1 1, move 2', 'move 1, move 1'],                // and can use it again
	]);

// 8. Roost: Flying removed for the turn (Ground hits Flying/Steel), pure Flying becomes Normal, a
//    Terastallized Flying Roost-er stays Flying (hint), a Terastallized non-Flying one just heals;
//    Transform copies the pre-Roost types (typeWas). Roost fails at full HP without setting the
//    volatile, so every scenario first takes damage on turn 1 from 100%-accurate moves.
const attackers = () => [
	mon('Garchomp', 'Rough Skin', ['Earthquake', 'Recover', 'Rock Slide', 'Dragon Claw'], { level: 50 }),
	mon('Weavile', 'Pressure', ['Icicle Crash', 'Recover', 'Reflect', 'Body Slam'], { level: 50, evs: bulk }),
	wall('Clefable', 'Magic Guard'),
];
const roosters = (o1 = {}, o2 = {}) => [
	mon('Corviknight', 'Pressure', ['Roost', 'Recover', 'Brave Bird', 'Light Screen'], { evs: bulk, ...o1 }),
	mon('Tornadus', 'Defiant', ['Roost', 'Recover', 'Hurricane', 'Tailwind'], { evs: bulk, ...o2 }),
	wall(),
];
scenario('roost_types', [191, 192, 193, 194],
	roosters(),
	attackers(),
	[
		['move 4, move 4', 'move 4 1, move 4 2'],   // Dragon Claw / Body Slam: both roosters take damage
		['move 1, move 1', 'move 1, move 1 1'],     // Roost: Earthquake / Icicle Crash into the roosting mons
		['move 2, move 2', 'move 3, move 1 2'],     // Recover: the Roost volatile expired, types are back to normal
		['move 1, move 1', 'move 1, move 1 2'],
	]);
scenario('roost_tera_flying', [201, 202, 203, 204],
	roosters({ teraType: 'Flying' }),
	attackers(),
	[
		['move 4 terastallize, move 4', 'move 4 1, move 4 2'],   // Terastallize into Flying while taking damage
		['move 1, move 1', 'move 1, move 1 1'],                  // Roost keeps Flying (hint)
		['move 1, move 1', 'move 1, move 1 2'],
	]);
scenario('roost_tera_other', [205, 206, 207, 208],
	roosters({}, { teraType: 'Fighting' }),
	attackers(),
	[
		['move 4, move 4 terastallize', 'move 4 1, move 4 2'],   // Tornadus Terastallizes into Fighting
		['move 1, move 1', 'move 1, move 1 1'],                  // Terastallized Roost heals without the volatile
		['move 1, move 1', 'move 1, move 1 2'],
	]);
scenario('roost_transform', [211, 212, 213, 214],
	[mon('Corviknight', 'Pressure', ['Roost', 'Recover', 'Rock Slide', 'Light Screen'], { evs: bulk }),
	 wall(),
	 wall('Clefable', 'Magic Guard')],
	[mon('Garchomp', 'Rough Skin', ['Earthquake', 'Recover', 'Rock Slide', 'Dragon Claw'], { level: 50 }),
	 mon('Mew', 'Synchronize', ['Transform', 'Recover', 'Reflect', 'Light Screen'], { level: 50 }),
	 wall('Clefable', 'Magic Guard')],
	[
		['move 4, move 1', 'move 4 1, move 2'],       // Dragon Claw damages the Corviknight
		['move 1, move 1', 'move 2, move 1 1'],       // Roost first, then Mew Transforms into the roosting Corviknight
		['move 3, move 1', 'move 2, move 4'],         // Rock Slide into the transformed Mew (Flying is weak to it)
		['move 3, move 1', 'move 2, move 4'],
	]);

// 9. Shell Side Arm: foe targets (anim + hint) and ally targets (neither).
scenario('shellsidearm_targets', [221, 222, 223, 224],
	[mon('Slowbro', 'Regenerator', ['Shell Side Arm', 'Recover', 'Reflect', 'Slack Off'], { evs: bulk }),
	 mon('Toxapex', 'Regenerator', ['Shell Side Arm', 'Recover', 'Reflect', 'Toxic'], { evs: bulk }),
	 wall()],
	wallsA(),
	[
		['move 1 1, move 1 2', 'move 1, move 1'],        // foes
		['move 1 -2, move 1 -1', 'move 1, move 1'],      // Shell Side Arm at each other (ally targets)
		['move 1 1, move 1 2', 'move 1, move 1'],
	]);

// 10. Alluring Voice / Burning Jealousy: only when the target raised its stats this turn.
scenario('alluring_burning_after_boost', [231, 232, 233, 234],
	[mon('Primarina', 'Liquid Voice', ['Alluring Voice', 'Recover', 'Reflect', 'Moonblast'], { level: 50 }),
	 mon('Heatran', 'Flash Fire', ['Burning Jealousy', 'Recover', 'Reflect', 'Flamethrower'], { level: 50 }),
	 wall()],
	[mon('Dragonite', 'Multiscale', ['Dragon Dance', 'Recover', 'Reflect', 'Extreme Speed'], { evs: fast }),
	 mon('Volcarona', 'Flame Body', ['Quiver Dance', 'Recover', 'Reflect', 'Bug Buzz'], { evs: fast }),
	 wall('Clefable', 'Magic Guard')],
	[
		['move 1 1, move 1', 'move 1, move 1'],   // foes boost first -> confusion / burn
		['move 1 1, move 1', 'move 2, move 2'],   // no boost this turn: nothing
		['move 1 2, move 1', 'move 1, move 1'],
	]);

// 11. Weight moves: Heavy Slam / Heat Crash (weight ratio) and Grass Knot / Low Kick (target weight),
//     including Heavy Metal doubling the weight and a Transformed Ditto.
scenario('weight_moves', [241, 242, 243, 244],
	[mon('Metagross', 'Clear Body', ['Heavy Slam', 'Heat Crash', 'Recover', 'Grass Knot'], { level: 50, evs: bulk }),
	 mon('Snorlax', 'Thick Fat', ['Heavy Slam', 'Low Kick', 'Recover', 'Grass Knot'], { level: 50, evs: bulk }),
	 wall()],
	[mon('Pikachu', 'Static', ['Recover', 'Reflect', 'Light Screen', 'Tailwind'], { level: 50 }),
	 mon('Registeel', 'Clear Body', ['Recover', 'Reflect', 'Light Screen', 'Tailwind'], { level: 50, evs: bulk }),
	 mon('Copperajah', 'Heavy Metal', ['Recover', 'Reflect', 'Light Screen', 'Tailwind'], { level: 50, evs: bulk }),
	 mon('Ditto', 'Imposter', ['Transform', 'Recover', 'Reflect', 'Light Screen'], { level: 50 })],
	[
		['move 1 1, move 2 2', 'move 2, move 2'],
		['move 2 2, move 4 1', 'move 2, move 2'],
		['move 3, move 3', 'switch 3, switch 4'],
		['move 1 1, move 2 2', 'move 2, move 2 1'],
		['move 2 2, move 4 1', 'move 2, move 2 1'],
	]);

// 12. Photon Geyser category follows boosted Atk vs SpA; Fusion Bolt / Fusion Flare in either order.
scenario('photon_geyser_boosts', [251, 252, 253, 254],
	[mon('Necrozma', 'Prism Armor', ['Photon Geyser', 'Swords Dance', 'Nasty Plot', 'Recover'], { level: 50 }),
	 mon('Lunala', 'Shadow Shield', ['Photon Geyser', 'Swords Dance', 'Nasty Plot', 'Recover'], { level: 50 }),
	 wall()],
	walls(),
	[
		['move 1 1, move 1 2', 'move 1, move 1'],
		['move 2, move 3', 'move 1, move 1'],
		['move 2, move 3', 'move 1, move 1'],
		['move 1 1, move 1 2', 'move 1, move 1'],
	]);
scenario('fusion_pair_order', [255, 256, 257, 258],
	[mon('Reshiram', 'Turboblaze', ['Fusion Flare', 'Fusion Bolt', 'Recover', 'Nasty Plot'], { level: 50, evs: fast }),
	 mon('Zekrom', 'Teravolt', ['Fusion Bolt', 'Fusion Flare', 'Recover', 'Swords Dance'], { level: 50 }),
	 wall()],
	walls(),
	[
		['move 1 1, move 1 2', 'move 1, move 1'],   // Flare then Bolt: Bolt doubled
		['move 2 1, move 2 2', 'move 1, move 1'],   // Bolt then Flare: Flare doubled
		['move 1 1, move 3', 'move 1, move 1'],     // Flare alone
		['move 3, move 1 2', 'move 1, move 1'],     // Bolt alone
	]);

// 13. Freeze-Dry into Water, Water/Flying, Ground/Water, and a Terastallized Water target.
scenario('freezedry_water_targets', [261, 262, 263, 264],
	[mon('Glaceon', 'Pressure', ['Freeze-Dry', 'Recover', 'Reflect', 'Ice Beam'], { level: 50, evs: fast }),
	 mon('Weavile', 'Pressure', ['Freeze-Dry', 'Recover', 'Reflect', 'Ice Beam'], { level: 50, evs: fast }),
	 wall()],
	[mon('Gyarados', 'Intimidate', ['Recover', 'Reflect', 'Light Screen', 'Tailwind'], { level: 50, evs: bulk }),
	 mon('Quagsire', 'Unaware', ['Recover', 'Reflect', 'Light Screen', 'Tailwind'], { level: 50, evs: bulk }),
	 mon('Pelipper', 'Drizzle', ['Recover', 'Reflect', 'Light Screen', 'Tailwind'], { level: 50, evs: bulk, teraType: 'Water' }),
	 mon('Toxapex', 'Regenerator', ['Recover', 'Reflect', 'Light Screen', 'Tailwind'], { level: 50, evs: bulk, teraType: 'Water' })],
	[
		['move 1 1, move 1 2', 'move 1, move 1'],
		['move 1 1, move 1 2', 'switch 3, switch 4'],
		['move 1 1, move 1 2', 'move 1 terastallize, move 1'],
		['move 1 1, move 1 2', 'move 1, move 1'],
	]);

// 14. Endeavor (user HP below / not below target HP), Final Gambit, Super Fang and Ruination.
scenario('endeavor_gambit_fang_ruination', [271, 272, 273, 274],
	[mon('Pikachu', 'Static', ['Endeavor', 'Super Fang', 'Reflect', 'Recover'], { level: 50 }),
	 mon('Kingambit', 'Defiant', ['Final Gambit', 'Super Fang', 'Reflect', 'Recover'], { level: 50 }),
	 mon('Chien-Pao', 'Sword of Ruin', ['Ruination', 'Super Fang', 'Reflect', 'Recover'], { level: 50 }),
	 wall()],
	[wall('Blissey', 'Serene Grace'), wall('Corviknight', 'Pressure'), wall('Clefable', 'Magic Guard'), wall('Skarmory', 'Sturdy')],
	[
		['move 1 1, move 2 2', 'move 1, move 1'],   // Endeavor works (lower HP); Super Fang halves
		['move 1 1, move 1 2', 'move 2, move 2'],   // Endeavor fails (HP not below the target's); Final Gambit
		['move 1 1, move 2 2', 'move 1, move 1'],   // Ruination, Super Fang
		['move 1 1, move 1 2', 'move 2, move 2'],
	]);

// 15. Glaive Rush: the user takes double damage and cannot be missed until its next move starts.
scenario('glaive_rush_drawback', [281, 282, 283, 284],
	[mon('Dragonite', 'Multiscale', ['Glaive Rush', 'Recover', 'Reflect', 'Extreme Speed'], { evs: bulk }),
	 mon('Gyarados', 'Intimidate', ['Glaive Rush', 'Recover', 'Reflect', 'Waterfall'], { evs: bulk }),
	 wall()],
	[mon('Thundurus', 'Defiant', ['Focus Blast', 'Thunder', 'Hurricane', 'Recover']),
	 mon('Zapdos', 'Pressure', ['Thunder', 'Hurricane', 'Focus Blast', 'Recover']),
	 wall('Clefable', 'Magic Guard')],
	[
		['move 1 1, move 1 2', 'move 1 1, move 2 2'],
		['move 2, move 2', 'move 1 1, move 2 2'],
		['move 1 1, move 2', 'move 3 1, move 3 1'],
		['move 3, move 2', 'move 2, move 2 1'],
	]);

// 16. Haze / Clear Smog clear boosts.
scenario('haze_clearsmog', [291, 292, 293, 294],
	[mon('Dragonite', 'Multiscale', ['Dragon Dance', 'Recover', 'Reflect', 'Haze'], { evs: bulk }),
	 mon('Toxapex', 'Regenerator', ['Clear Smog', 'Haze', 'Recover', 'Reflect'], { evs: bulk }),
	 wall()],
	[mon('Volcarona', 'Flame Body', ['Quiver Dance', 'Recover', 'Reflect', 'Light Screen'], { evs: bulk }),
	 mon('Snorlax', 'Thick Fat', ['Belly Drum', 'Recover', 'Reflect', 'Light Screen'], { evs: bulk }),
	 wall('Clefable', 'Magic Guard')],
	[
		['move 1, move 3', 'move 1, move 1'],      // boosts everywhere
		['move 1, move 1 1', 'move 1, move 2'],    // Clear Smog clears Volcarona
		['move 4, move 1 2', 'move 2, move 2'],    // Haze; Clear Smog on the Snorlax
		['move 1, move 2', 'move 1, move 1'],
		['move 4, move 3', 'move 2, move 3'],
	]);

// 17. Substitute interactions (need `moves:substitute` from another batch: the Rust tests for these two
//     scenarios are #[ignore]d until that batch is merged). Shell Side Arm into a Substitute fires
//     onAfterSubDamage (hint only); Clear Smog's onHit is skipped by a Substitute so boosts stay.
const subWall = (species, ability) => mon(species, ability, ['Substitute', 'Recover', 'Reflect', 'Light Screen'], { evs: bulk });
scenario('shellsidearm_substitute', [301, 302, 303, 304],
	[mon('Slowbro', 'Regenerator', ['Shell Side Arm', 'Recover', 'Reflect', 'Slack Off'], { evs: bulk }),
	 mon('Toxapex', 'Regenerator', ['Shell Side Arm', 'Recover', 'Reflect', 'Toxic'], { evs: bulk }),
	 wall()],
	[subWall('Clefable', 'Magic Guard'), subWall('Blissey', 'Serene Grace'), wall('Corviknight', 'Pressure')],
	[
		['move 2, move 2', 'move 1, move 1'],        // foes put up Substitutes
		['move 1 1, move 1 2', 'move 2, move 2'],    // Shell Side Arm into the Substitutes (AfterSubDamage hint)
		['move 1 1, move 1 2', 'move 2, move 2'],    // and again until they break
		['move 1 1, move 1 2', 'move 2, move 2'],
	]);
scenario('clearsmog_substitute', [311, 312, 313, 314],
	[mon('Toxapex', 'Regenerator', ['Clear Smog', 'Haze', 'Recover', 'Reflect'], { evs: bulk }),
	 mon('Dragonite', 'Multiscale', ['Dragon Dance', 'Recover', 'Reflect', 'Haze'], { evs: bulk }),
	 wall()],
	[mon('Volcarona', 'Flame Body', ['Quiver Dance', 'Substitute', 'Recover', 'Reflect'], { evs: bulk }),
	 mon('Snorlax', 'Thick Fat', ['Belly Drum', 'Substitute', 'Recover', 'Reflect'], { evs: bulk }),
	 wall('Clefable', 'Magic Guard')],
	[
		['move 3, move 2', 'move 1, move 3'],        // Volcarona boosts
		['move 3, move 2', 'move 2, move 2'],        // both foes put up Substitutes
		['move 1 1, move 1', 'move 3, move 3'],      // Clear Smog hits the Substitute: Volcarona keeps its boosts
		['move 2, move 4', 'move 3, move 3'],        // Haze clears everything
	]);

export { S, mon, scenario, bulk, toID, sim };

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
			out.push(`step ${label}\t${c1}\t${c2}`, `seed_after ${snap.seed.join(',')}`, ...snap.log.map(l => `log ${l}`));
			if (process.env.MC_DEBUG === sc.name) console.log(`## ${label} ${c1} / ${c2}\n${snap.log.join('\n')}`);
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
		const kinds = (txt) => out.filter(l => l.startsWith('log ') && l.includes(txt)).length;
		console.log(`${sc.name}: ${n} turns (${forced} forced), faints=${kinds('|faint|')}`);
		if (session.anomalies.length) throw new Error(`${sc.name}: anomalies ${session.anomalies.join('; ')}`);
	}
	return out;
}
if (process.argv[1]?.endsWith('scenarios.mjs')) {
	const lines = run();
	const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/moves/movecallbacks/scenarios.txt', import.meta.url));
	fs.writeFileSync(dest, lines.join('\n') + '\n');
	console.log(`wrote ${dest} (${lines.length} lines)`);
}
