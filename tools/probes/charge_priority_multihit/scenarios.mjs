// charge_priority_multihit batch: directed gen9randomdoublesbattle scenarios replayed on the pinned Showdown.
// Output: crates/engine/src/effects/moves/chargepriority/scenarios.txt, consumed by the scenario test in
// chargepriority/tests.rs (exact log + PRNG seed per decision boundary).
//
//   node tools/probes/charge_priority_multihit/scenarios.mjs [path/to/pokemon-showdown] [--out FILE]
//
// File format (one record per line; fields separated by a tab where noted):
//   scenario <name>
//   seed <a,b,c,d>                    battle PRNG seed
//   p1 <packed team>
//   p2 <packed team>
//   pre <op>\t<args...>               a direct state edit applied before the next step's choices
//                                     (only `addvolatile\t<p1|p2>\t<slot 0|1>\t<volatile>\t<move>`: the real
//                                     `pokemon.addVolatile(volatile, null, dex.moves.get(move))`, used to put a
//                                     lockedmove on a Pokemon because no scoped move creates one)
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

const args = process.argv.slice(2);
const outIndex = args.indexOf('--out');
const outFile = outIndex >= 0 ? args.splice(outIndex, 2)[1] : null;
const sim = loadSim(args[0] || `${process.env.HOME}/src/pokemon-showdown`);
const { Dex } = createRequire(sim.root + '/package.json')('./dist/sim');
const scope = JSON.parse(fs.readFileSync(new URL('../../../data/scope.json', import.meta.url)));
const toID = s => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');
const inScope = {
	species: new Set(scope.species.map(s => s.id)),
	moves: new Set(scope.moves.map(s => s.id)),
	abilities: new Set(scope.abilities.map(s => s.id)),
	items: new Set(scope.items.map(s => s.id)),
};

// The callbacks of these moves belong to other batches (protect_redirection, disable_lock_trap, move_callbacks, ...) and
// may still be unported when this probe is replayed; the scenarios use declarative stand-ins in the same move slot so every
// choice index stays valid. Re-run the probe without the substitution once those batches land to also cover Protect
// interactions (Phantom/Shadow Force breaking Protect, Fake Out into Protect).
const OTHER_BATCHES = new Set(['Protect', 'Taunt', 'Helping Hand', 'Follow Me', 'Leech Seed', 'Rest', 'Roost', 'Spikes', 'Tailwind', 'Knock Off', 'Super Fang']);
const STAND_INS = ['Agility', 'Iron Defense', 'Bulk Up', 'Nasty Plot', 'Calm Mind', 'Swords Dance', 'Cosmic Power', 'Rock Polish', 'Dragon Dance', 'Quiver Dance', 'Coil', 'Howl'];
function withStandIns(moves) {
	if (process.env.SC_KEEP_OTHER_BATCH_MOVES) return moves;
	const out = [...moves];
	out.forEach((m, i) => {
		if (!OTHER_BATCHES.has(m)) return;
		const wanted = m === 'Knock Off' || m === 'Super Fang' ? ['Crunch', ...STAND_INS] : STAND_INS;
		out[i] = wanted.find(c => !out.includes(c));
	});
	return out;
}

/** A fully specified set (explicit gender so the Pokemon constructor never draws from the PRNG). */
function mon(species, ability, moves, o = {}) {
	moves = withStandIns(moves);
	const sp = Dex.species.get(species);
	if (!inScope.species.has(sp.id)) throw new Error(`species ${sp.id} outside the generated scope`);
	if (o.item && !inScope.items.has(toID(o.item))) throw new Error(`item ${o.item} outside scope`);
	if (!inScope.abilities.has(toID(ability))) throw new Error(`ability ${ability} outside scope (${sp.name})`);
	for (const m of moves) if (!inScope.moves.has(toID(m))) throw new Error(`move ${m} outside scope`);
	return {
		name: sp.name, species: sp.name, item: o.item ?? '', ability, moves, nature: o.nature ?? 'Serious',
		gender: sp.gender || 'M', level: o.level ?? 100,
		evs: o.evs ?? { hp: 0, atk: 0, def: 0, spa: 0, spd: 0, spe: 0 },
		ivs: { hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31 },
	};
}
const bulk = { hp: 252, atk: 0, def: 128, spa: 0, spd: 128, spe: 0 };
const fast = { hp: 0, atk: 0, def: 0, spa: 0, spd: 0, spe: 252 };

const S = [];
const scenario = (name, seed, p1, p2, turns) => S.push({ name, seed, teams: [sim.Teams.pack(p1), sim.Teams.pack(p2)], turns });
const repeat = (n, a, b) => Array.from({ length: n }, () => [a, b]);
/** A turn with a state edit first. */
const withPre = (pre, c1, c2) => ({ pre, c1, c2 });

// Level 50 keeps the damage low enough that nobody faints unexpectedly.
const L50 = { level: 50, evs: bulk };
const snorlax = () => mon('Snorlax', 'Thick Fat', ['Body Slam', 'Protect', 'Rest', 'Earthquake'], L50);
const blissey = () => mon('Blissey', 'Serene Grace', ['Soft-Boiled', 'Protect', 'Thunder Wave', 'Helping Hand'], L50);
const clefable = () => mon('Clefable', 'Magic Guard', ['Moonblast', 'Protect', 'Calm Mind', 'Thunder Wave'], L50);
const dondozo = () => mon('Dondozo', 'Oblivious', ['Wave Crash', 'Protect', 'Rest', 'Earthquake'], L50);
const incineroar = () => mon('Incineroar', 'Intimidate', ['Fake Out', 'Flare Blitz', 'Knock Off', 'Protect'], L50);
const garchomp = (ev = bulk) => mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Earth Power', 'Protect'], { level: 50, evs: ev });
const kingambit = () => mon('Kingambit', 'Pressure', ['Sucker Punch', 'Knock Off', 'Protect', 'Swords Dance'], L50);

// 1. Truant (Slaking): Giga Impact recharge (mustrecharge Start/BeforeMove, -mustrecharge, truant removal),
//    the loafing cycle afterwards, Sucker Punch into a recharging target, switching Slaking out and back in.
scenario('truant_recharge_loafing', [11, 22, 33, 44],
	[mon('Slaking', 'Truant', ['Giga Impact', 'Body Slam', 'Protect', 'Earthquake'], L50),
	 mon('Sableye', 'Prankster', ['Fake Out', 'Thunder Wave', 'Taunt', 'Protect'], L50),
	 snorlax()],
	[kingambit(), incineroar(), clefable()],
	[
		['move 1 1, move 1 2', 'move 1 1, move 1 2'],
		['move 1, move 2 1', 'move 1 1, move 2 1'],
		['move 2 1, move 3 1', 'move 3, move 4'],
		['move 2 1, move 4', 'move 2 1, move 2 2'],
		['move 2 1, move 4', 'move 2 1, move 2 2'],
		['switch 3, move 4', 'move 3, move 4'],
		['switch 3, move 4', 'move 2 1, move 2 2'],
	]);

// 2. Electromorphosis (Bellibolt): Charge Start/Restart, Electric Base Power x2 consumed after the move,
//    non-Electric moves keep it, a flinched Electric move aborts it (MoveAborted).
scenario('electromorphosis_charge', [101, 202, 303, 404],
	[mon('Bellibolt', 'Electromorphosis', ['Thunderbolt', 'Protect', 'Muddy Water', 'Body Slam'], L50), clefable(), snorlax()],
	[incineroar(), garchomp({ ...bulk, spe: 252 }), dondozo()],
	[
		['move 1 1, move 2', 'move 1 1, move 1 1'],
		['move 1 1, move 2', 'move 4, move 1 1'],
		['move 3, move 2', 'move 4, move 1 1'],
		['move 1 1, move 2', 'move 4, move 4'],
		['move 1 1, move 2', 'move 2 1, move 1 1'],
	]);

// 3. Solar Beam under rain / sun / sand / snow (halved Base Power on the attack turn, sun skips the charge turn).
scenario('solar_beam_weathers', [5, 6, 7, 8],
	[mon('Venusaur', 'Chlorophyll', ['Solar Beam', 'Protect', 'Sludge Bomb', 'Sleep Powder'], L50),
	 mon('Rillaboom', 'Overgrow', ['Solar Beam', 'Protect', 'Wood Hammer', 'Grassy Glide'], L50),
	 mon('Torkoal', 'Drought', ['Solar Beam', 'Protect', 'Will-O-Wisp', 'Helping Hand'], L50)],
	[mon('Pelipper', 'Drizzle', ['Hurricane', 'Protect', 'Tailwind', 'Scald'], L50),
	 snorlax(),
	 mon('Tyranitar', 'Sand Stream', ['Rock Slide', 'Protect', 'Crunch', 'Earthquake'], L50),
	 mon('Abomasnow', 'Snow Warning', ['Blizzard', 'Protect', 'Wood Hammer', 'Ice Shard'], L50)],
	[
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 1, move 1', 'move 2, move 2'],
		['switch 3, move 2', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 1 1, move 1 2', 'switch 3, move 2'],
		['move 1, move 1', 'move 2, move 2'],
		['move 1 1, move 1 2', 'switch 4, move 2'],
		['move 1, move 1', 'move 2, move 2'],
	]);

// 4. Power Herb on every charge move: the charge turn is skipped (-enditem, -anim with [still]), SpA boosts first.
scenario('power_herb_charge_moves', [15, 25, 35, 45],
	[mon('Sableye', 'Prankster', ['Meteor Beam', 'Protect', 'Fake Out', 'Taunt'], { ...L50, item: 'Power Herb' }),
	 mon('Rillaboom', 'Overgrow', ['Solar Beam', 'Protect', 'Wood Hammer', 'Grassy Glide'], { ...L50, item: 'Power Herb' }),
	 mon('Gengar', 'Pressure', ['Shadow Force', 'Phantom Force', 'Protect', 'Shadow Ball'], { ...L50, item: 'Power Herb' }),
	 mon('Pincurchin', 'Electric Surge', ['Electro Shot', 'Protect', 'Recover', 'Spikes'], { ...L50, item: 'Power Herb' })],
	[snorlax(), blissey(), clefable()],
	[
		['move 1 1, move 1 2', 'move 1 1, move 1'],
		['move 1 1, move 1 2', 'move 1 1, move 1'],
		['move 1, move 1', 'move 2, move 2'],
		['switch 3, switch 4', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 2 1, move 1 2', 'move 2, move 2'],
		['move 1, move 1', 'move 2, move 2'],
	]);

// 5. Electro Shot: +1 SpA each time, rain short-cut (no charge turn, -anim with [still]); the rain runs out later.
scenario('electro_shot_rain', [31, 32, 33, 34],
	[mon('Pincurchin', 'Electric Surge', ['Electro Shot', 'Protect', 'Recover', 'Spikes'], L50),
	 mon('Pelipper', 'Drizzle', ['Hurricane', 'Protect', 'Tailwind', 'Scald'], L50),
	 clefable()],
	[blissey(), dondozo(), snorlax()],
	[
		['move 1 1, move 2', 'move 1, move 2'],
		['move 1 1, move 2', 'move 1, move 2'],
		['move 1 1, move 2', 'move 1, move 2'],
		['move 1 1, move 2', 'move 1, move 2'],
		['move 1 1, move 2', 'move 1, move 2'],
		['move 1 1, move 2', 'move 1, move 2'],
		['move 1, move 2', 'move 1, move 2'],
		['move 1 1, move 2', 'move 1, move 2'],
	]);

// 6. Phantom Force / Shadow Force: invulnerable charge turn (constant Invulnerability), breaksProtect on the attack turn.
scenario('force_moves_invulnerable', [51, 52, 53, 54],
	[mon('Gengar', 'Pressure', ['Shadow Force', 'Phantom Force', 'Protect', 'Shadow Ball'], L50),
	 mon('Sableye', 'Prankster', ['Phantom Force', 'Protect', 'Fake Out', 'Taunt'], L50),
	 snorlax()],
	[incineroar(), garchomp({ ...bulk, spe: 252 }), dondozo()],
	[
		['move 1 1, move 1 2', 'move 3 1, move 1 1'],
		['move 1, move 1', 'move 4, move 4'],
		['move 2 1, move 1 2', 'move 2 1, move 1 1'],
		['move 1, move 1', 'move 4, move 2 1'],
		['move 1 1, move 2', 'move 4, move 1 1'],
		['move 1, move 2', 'move 4, move 4'],
	]);

// 6b. A flinch on the second turn of Solar Beam aborts it (MoveAborted -> twoturnmove removed, move volatile cleared).
scenario('solar_beam_flinched', [57, 58, 59, 60],
	[mon('Venusaur', 'Chlorophyll', ['Solar Beam', 'Protect', 'Sludge Bomb', 'Sleep Powder'], L50),
	 mon('Rillaboom', 'Overgrow', ['Solar Beam', 'Protect', 'Wood Hammer', 'Grassy Glide'], L50),
	 snorlax()],
	[snorlax(), blissey(), incineroar()],
	[
		['move 1 1, move 1 2', 'switch 3, move 2'],
		['move 1, move 1', 'move 1 1, move 2'],
		['move 1 1, move 1 2', 'move 4, move 2'],
		['move 1, move 1', 'move 4, move 2'],
	]);

// 7. Priority abilities: Gale Wings (full HP only), Triage (heal +3), Mycelium Might (status last, ignores Insomnia).
scenario('priority_abilities', [61, 62, 63, 64],
	[mon('Talonflame', 'Gale Wings', ['Brave Bird', 'Protect', 'Flare Blitz', 'Tailwind'], { level: 50, evs: fast }),
	 mon('Comfey', 'Triage', ['Floral Healing', 'Draining Kiss', 'Taunt', 'Protect'], L50),
	 mon('Toedscruel', 'Mycelium Might', ['Spore', 'Leech Seed', 'Protect', 'Knock Off'], L50),
	 mon('Sableye', 'Prankster', ['Thunder Wave', 'Taunt', 'Fake Out', 'Protect'], L50)],
	[snorlax(), blissey(),
	 mon('Hypno', 'Insomnia', ['Psychic', 'Protect', 'Thunder Wave', 'Hypnosis'], L50),
	 kingambit()],
	[
		['move 1 1, move 1 -1', 'move 1 1, move 2'],
		['move 1 1, move 1 -1', 'move 1 1, move 2'],
		['move 2, move 3 1', 'move 2, move 2'],
		['switch 3, move 4', 'switch 3, move 2'],
		['move 1 1, move 4', 'move 1 2, move 2'],
		['move 2 1, move 2 1', 'move 1 2, move 2'],
		['move 4 1, move 2 1', 'move 1 2, move 2'],
	]);

// 8. Quash and the Sucker Punch family: Quash makes the (faster) target act last and fails when it already moved;
//    Sucker Punch / Thunderclap work only against a queued attack and fail against status moves.
scenario('quash_sucker_punch_thunderclap', [71, 72, 73, 74],
	[mon('Grimmsnarl', 'Prankster', ['Quash', 'Spirit Break', 'Taunt', 'Thunder Wave'], L50),
	 kingambit(),
	 mon('Thundurus', 'Prankster', ['Thunderclap', 'Thunder Wave', 'Taunt', 'Protect'], L50),
	 snorlax()],
	[garchomp({ ...bulk, spe: 252 }), incineroar(), blissey(), clefable()],
	[
		['move 1 1, move 1 2', 'move 1 1, move 1 1'],
		['move 1 1, move 1 2', 'move 4, move 1 1'],
		['move 1 1, move 1 2', 'move 1 1, move 4'],
		['switch 3, move 1 1', 'move 1 1, move 3 1'],
		['move 1 1, move 2 1', 'move 4, move 4'],
		['move 4, move 1 1', 'move 4, move 1 1'],
		['move 1 1, move 3', 'switch 3, move 1 1'],
	]);

// 9. Fake Out / First Impression: first turn out only (activeMoveActions), hint on later turns, works again after a switch.
scenario('fake_out_first_impression', [81, 82, 83, 84],
	[incineroar(),
	 mon('Kingambit', 'Pressure', ['First Impression', 'Sucker Punch', 'Protect', 'Swords Dance'], L50),
	 mon('Ambipom', 'Skill Link', ['Fake Out', 'Knock Off', 'Protect', 'Taunt'], L50),
	 snorlax()],
	[garchomp(), blissey(), clefable()],
	[
		['move 1 1, move 1 2', 'move 4, move 2'],
		['move 1 1, move 1 2', 'move 4, move 2'],
		['switch 3, switch 4', 'move 4, move 2'],
		['move 1 1, move 3', 'move 4, move 2'],
		['move 1 1, move 3', 'move 4, move 2'],
		['switch 3, move 3', 'move 4, move 2'],
		['move 1 1, move 1 2', 'move 4, move 2'],
	]);

// 10. Multihit: Skill Link (range -> maximum, no multiaccuracy), Loaded Dice (4-5 hits / ten hits, no multiaccuracy),
//     plain ranges, Triple Axel (20/40/60 base power, accuracy per hit) and Population Bomb.
scenario('multihit_skill_link_loaded_dice', [91, 92, 93, 94],
	[mon('Cloyster', 'Skill Link', ['Icicle Spear', 'Rock Blast', 'Protect', 'Shell Smash'], L50),
	 mon('Maushold', 'Technician', ['Population Bomb', 'Protect', 'Super Fang', 'Follow Me'], { ...L50, item: 'Loaded Dice' }),
	 mon('Cinccino', 'Skill Link', ['Bullet Seed', 'Rock Blast', 'Population Bomb', 'Protect'], L50),
	 mon('Weavile', 'Pressure', ['Triple Axel', 'Ice Shard', 'Protect', 'Knock Off'], { ...L50, item: 'Loaded Dice' })],
	[snorlax(), blissey(), dondozo(), clefable()],
	[
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 2 1, move 1 2', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['switch 3, switch 4', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 3 1, move 2 2', 'move 2, move 2'],
		['move 2 1, move 4 2', 'move 2, move 2'],
		['move 3 1, move 1 2', 'move 2, move 2'],
		['move 3 2, move 1 1', 'move 2, move 2'],
	]);

scenario('triple_axel_accuracy', [95, 96, 97, 98],
	[mon('Weavile', 'Pressure', ['Triple Axel', 'Ice Shard', 'Protect', 'Knock Off'], L50),
	 mon('Weavile', 'Pressure', ['Triple Axel', 'Ice Shard', 'Protect', 'Knock Off'], { ...L50, item: 'Loaded Dice' }),
	 snorlax()],
	[snorlax(), dondozo(), blissey()],
	repeat(8, 'move 1 1, move 1 2', 'move 2, move 2'));

// 11. High Jump Kick: crash damage on a miss, into Protect and into a Ghost (immunity); `[from] highjumpkick`.
scenario('high_jump_kick_crash', [111, 112, 113, 114],
	[mon('Hawlucha', 'Mold Breaker', ['High Jump Kick', 'Protect', 'Swords Dance', 'Close Combat'], L50),
	 mon('Lucario', 'Inner Focus', ['High Jump Kick', 'Protect', 'Swords Dance', 'Close Combat'], L50),
	 snorlax()],
	[mon('Gengar', 'Pressure', ['Shadow Ball', 'Protect', 'Sludge Bomb', 'Hypnosis'], L50), blissey(), dondozo()],
	[
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 2, move 2'],
		['move 1 1, move 1 2', 'move 3 1, move 2'],
		['move 1 1, move 1 2', 'move 3 1, move 2'],
		...repeat(1, 'move 1 1, move 1 2', 'move 2, move 2'),
	]);

// 12. lockedmove (Outrage/Thrash/Petal Dance cannot be built because no scoped move creates it; the real
//     `addVolatile('lockedmove', null, move)` is applied to both engines instead, locking the user into a harmless
//     status move): Start draw random(2, 4), forced move with [from] lockedmove, Residual decrement, AfterMove removal,
//     End -> confusion (`[fatigue]`) only when trueDuration ended at 1. Four seeds cover both draw results.
for (const [i, seed] of [[0, [121, 122, 123, 124]], [1, [221, 222, 223, 224]], [2, [321, 322, 323, 324]], [3, [421, 422, 423, 424]]]) {
	scenario(`lockedmove_expiry_confusion_${i}`, seed,
		[mon('Garchomp', 'Rough Skin', ['Earthquake', 'Protect', 'Dragon Claw', 'Swords Dance'], L50), snorlax(), clefable()],
		[dondozo(), blissey(), mon('Gengar', 'Pressure', ['Shadow Ball', 'Protect', 'Sludge Bomb', 'Hypnosis'], L50)],
		[
			withPre([['addvolatile', 'p1', 0, 'lockedmove', 'swordsdance']], 'move 1, move 2', 'move 2, move 2'),
			['move 1, move 2', 'move 2, move 2'],
			['move 1, move 2', 'move 2, move 2'],
			['move 1, move 2', 'move 2, move 2'],
		]);
}

// 12b. lockedmove restarts while trueDuration >= 2 (duration back to 2); a sleeping user loses the volatile in
//      Residual without confusion.
scenario('lockedmove_restart_and_sleep', [131, 132, 133, 134],
	[mon('Garchomp', 'Rough Skin', ['Earthquake', 'Protect', 'Dragon Claw', 'Swords Dance'], L50),
	 mon('Dragonite', 'Inner Focus', ['Recover', 'Protect', 'Dragon Claw', 'Extreme Speed'], L50),
	 clefable()],
	[mon('Vileplume', 'Chlorophyll', ['Spore', 'Protect', 'Sludge Bomb', 'Giga Drain'], { level: 50, evs: { ...bulk, spe: 252 } }), blissey(), dondozo()],
	[
		withPre([['addvolatile', 'p1', 0, 'lockedmove', 'swordsdance'], ['addvolatile', 'p1', 1, 'lockedmove', 'recover']], 'move 1, move 1', 'move 2, move 2'),
		withPre([['addvolatile', 'p1', 0, 'lockedmove', 'swordsdance'], ['addvolatile', 'p1', 1, 'lockedmove', 'recover']], 'move 1, move 1', 'move 2, move 2'),
		withPre([['addvolatile', 'p1', 0, 'lockedmove', 'swordsdance']], 'move 1, move 1', 'move 1 1, move 2'),
		['move 1, move 1', 'move 1 1, move 2'],
		['move 1, move 1', 'move 1 1, move 2'],
		['move 1, move 1', 'move 1 1, move 2'],
	]);

export { S, mon };

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

/**
 * Stand-in moves change whether a slot needs a target; repair the choice the way a player would: drop the target of a
 * move that cannot take one, add one to a move that needs one. The repaired choice is what gets recorded.
 */
function fixTargets(choice, request, error) {
	const parts = choice.split(',').map(p => p.trim());
	let changed = false;
	parts.forEach((part, i) => {
		const m = /^move (\d+)(?: (-?\d+))?$/.exec(part);
		const name = request?.active?.[i]?.moves?.[m ? Number(m[1]) - 1 : -1]?.move;
		if (!m || !name) return;
		if (error.includes(`You can't choose a target for ${name}`) && m[2] !== undefined) { parts[i] = `move ${m[1]}`; changed = true; }
		else if (error.includes(`${name} needs a target`) && m[2] === undefined) { parts[i] = `move ${m[1]} 1`; changed = true; }
	});
	return changed ? parts.join(', ') : null;
}

function applyPre(session, op) {
	const [kind, side, slot, volatile, moveId] = op;
	if (kind !== 'addvolatile') throw new Error(`unknown op ${kind}`);
	const battle = session.battle;
	const pokemon = battle.sides[side === 'p1' ? 0 : 1].active[slot];
	const result = pokemon.addVolatile(volatile, null, battle.dex.moves.get(moveId));
	if (!result) throw new Error(`addVolatile ${volatile} on ${side} ${slot} failed`);
}

function run() {
	const out = [];
	for (const sc of S) {
		const session = new Session(sim, { seed: sc.seed, teams: sc.teams });
		out.push(`scenario ${sc.name}`, `seed ${sc.seed.join(',')}`, `p1 ${sc.teams[0]}`, `p2 ${sc.teams[1]}`);
		let snap = session.snapshot();
		out.push('step start\t\t', `seed_after ${snap.seed.join(',')}`, ...snap.log.map(l => `log ${l}`));
		let n = 0, forced = 0;
		const play = (label, c1, c2, pre = []) => {
			const rec = [];
			for (const op of pre) {
				applyPre(session, op);
				rec.push(`pre ${op.join('\t')}`);
			}
			const final = { p1: c1, p2: c2 };
			for (const [side, c] of [['p1', c1], ['p2', c2]]) {
				if (c === '') continue;
				let choice = c;
				for (let attempt = 0; ; attempt++) {
					const r = session.choose(side, choice);
					if (r.ok) break;
					const req = snap.requests[side];
					const fixed = attempt < 4 ? fixTargets(choice, req, r.error) : null;
					if (fixed && fixed !== choice) { choice = fixed; continue; }
					const moves = req?.active?.map(a => a.moves.map(m => `${m.move}${m.disabled ? '(x)' : ''}`).join('/')).join(' | ');
					throw new Error(`${sc.name} ${label}: ${side} choice "${choice}" rejected: ${r.error}; request moves: ${moves}`);
				}
				final[side] = choice;
			}
			snap = session.snapshot();
			if (process.env.SC_DEBUG) console.log(sc.name, label, final.p1, '||', final.p2, '::', snap.log.filter(l => /\|(faint|switch|drag|cant|-fail|-miss|-prepare|-mustrecharge)\|/.test(l)).join(' ; '));
			rec.push(`step ${label}\t${final.p1}\t${final.p2}`, `seed_after ${snap.seed.join(',')}`, ...snap.log.map(l => `log ${l}`));
			out.push(...rec);
		};
		for (const turn of sc.turns) {
			const [c1, c2, pre] = Array.isArray(turn) ? [turn[0], turn[1], []] : [turn.c1, turn.c2, turn.pre];
			if (session.ended) { if (process.env.SC_STRICT) throw new Error(`${sc.name}: battle ended early`); break; }
			try {
				play(`turn${n + 1}`, c1, c2, pre);
				n++;
				for (let guard = 0; guard < 4 && !session.ended; guard++) {
					const f1 = forcedChoice(snap.requests.p1), f2 = forcedChoice(snap.requests.p2);
					if (f1 === '' && f2 === '') break;
					play(`forced${++forced}`, f1, f2);
				}
			} catch (e) {
				if (process.env.SC_STRICT || n < 3) throw e;
				console.log(`  ${sc.name}: stopped after ${n} turns: ${e.message.slice(0, 160)}`);
				break;
			}
		}
		const count = txt => out.filter(l => l.startsWith('log ') && l.includes(txt)).length;
		console.log(`${sc.name}: ${n} turns (${forced} forced), faints=${count('|faint|')}`);
		if (session.anomalies.length) throw new Error(`${sc.name}: anomalies ${session.anomalies.join('; ')}`);
	}
	return out;
}
if (process.argv[1]?.endsWith('scenarios.mjs')) {
	const lines = run();
	const dest = outFile ?? fileURLToPath(new URL('../../../crates/engine/src/effects/moves/chargepriority/scenarios.txt', import.meta.url));
	fs.writeFileSync(dest, lines.join('\n') + '\n');
	console.log(`wrote ${dest} (${lines.length} lines)`);
}
