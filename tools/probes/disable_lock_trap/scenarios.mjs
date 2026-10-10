// disable_lock_trap batch: directed gen9randomdoublesbattle scenarios replayed on the pinned Showdown.
// Output: crates/engine/src/effects/conditions/locktrap/scenarios.txt, consumed by the scenario test in
// locktrap/tests.rs (exact log + PRNG seed after every decision boundary).
//
//   node tools/probes/disable_lock_trap/scenarios.mjs [path/to/pokemon-showdown]
//   DLT_DEBUG=1|<scenario name> node tools/probes/disable_lock_trap/scenarios.mjs   (print the logs, no output file)
//
// File format (one record per line; fields separated by a tab where noted):
//   scenario <name>
//   seed <a,b,c,d>                    battle PRNG seed
//   p1 <packed team>
//   p2 <packed team>
//   step <label>\t<p1 choice>\t<p2 choice>   one decision boundary; `start` has no choices and an empty
//                                     choice means that side had no pending request (wait). A choice may be
//                                     `bad || good`: every attempt before the last one is REJECTED by the sim.
//   seed_after <a,b,c,d>              PRNG state after the step
//   log <line>                        normalized log entries produced by the step, in order
// A forced replacement after a faint is appended as an extra `forced<n>` step (chosen automatically).
// In the source below `a ?? b` tries the alternatives in order and records the first one Showdown accepts.
// Every scenario lists `expect` regexes that must match the joined log, so a scenario that silently stops
// reaching the branch it was written for fails at generation time.
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

const S = [];
const scenario = (name, seed, p1, p2, turns, expect = []) =>
	S.push({ name, seed, teams: [sim.Teams.pack(p1), sim.Teams.pack(p2)], turns, expect });
const repeat = (n, a, b) => Array.from({ length: n }, () => [a, b]);
/** A passive filler mon (no damaging moves) used to pad benches and keep scenarios alive. */
const wall = (species, ability, o = {}) => mon(species, ability, ['Protect', 'Reflect', 'Light Screen', 'Thunder Wave'], { evs: bulk, ...o });

// 1. Choice lock + Dancer + Taunt/Encore/Disable on one Choice Band holder. Oricorio locks into Hurricane;
//    Dragonite's Dragon Dance makes Dancer try to copy it, which BeforeMove rejects (bare `|move|` line,
//    `[still]`, `-fail`). Garchomp (Choice Band) is Taunted, Encored and Disabled until only Struggle is left.
scenario('choicelock_dancer_encore_disable_struggle', [1101, 1102, 1103, 1104],
	[mon('Oricorio', 'Dancer', ['Hurricane', 'Revelation Dance', 'Quiver Dance', 'Roost'], { item: 'Choice Scarf', level: 50 }),
	 mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Swords Dance', 'Rock Slide'], { item: 'Choice Band', level: 50 }),
	 wall('Clefable', 'Magic Guard'),
	 wall('Corviknight', 'Pressure')],
	[mon('Blissey', 'Serene Grace', ['Taunt', 'Encore', 'Disable', 'Soft-Boiled'], { evs: bulk }),
	 mon('Dragonite', 'Multiscale', ['Dragon Dance', 'Extreme Speed', 'Roost', 'Fire Punch'], { evs: bulk }),
	 wall('Toxapex', 'Regenerator'),
	 wall('Snorlax', 'Thick Fat')],
	[
		['move 1 2, move 1 2', 'move 1 2, move 1'],
		['move 1 2, move 1 2', 'move 2 2, move 1'],
		['move 1 2, move 1 2', 'move 3 2, move 1'],
		...repeat(4, 'move 1 2, move 1 ?? move 1 2, move 1 2', 'move 4, move 1'),
	],
	[/^\|move\|p1a: Oricorio\|Dragon Dance\|\|\[still\]\n\|-fail\|p1a: Oricorio$/m,
		/\|-start\|p1b: Garchomp\|Encore$/m, /\|-start\|p1b: Garchomp\|Disable\|Body Slam$/m,
		/\|move\|p1b: Garchomp\|Struggle\|/m]);

// 2. Heal Block (Psychic Noise): duration 2, queued Soft-Boiled blocked in BeforeMove, a third-party Pollen Puff
//    on a damaged Heal-Blocked ally cancelled by TryHeal (`[still]` + `cant`), a Heal-Blocked Pollen Puff user
//    stopped by the move's own onTryMove, a second Psychic Noise silently keeping the block (onRestart),
//    expiry, and healing again afterwards.
scenario('healblock_pollenpuff', [1201, 1202, 1203, 1204],
	[mon('Mesprit', 'Levitate', ['Psychic Noise', 'Dazzling Gleam', 'Protect', 'Recover']),
	 mon('Clefable', 'Magic Guard', ['Moonblast', 'Protect', 'Thunder Wave', 'Calm Mind']),
	 wall('Corviknight', 'Pressure'),
	 wall('Toxapex', 'Regenerator')],
	[mon('Vikavolt', 'Levitate', ['Pollen Puff', 'Giga Drain', 'Protect', 'Thunderbolt'], { level: 70, evs: bulk }),
	 mon('Blissey', 'Serene Grace', ['Soft-Boiled', 'Thunder Wave', 'Protect', 'Reflect'], { evs: bulk }),
	 wall('Skarmory', 'Sturdy'),
	 wall('Snorlax', 'Thick Fat')],
	[
		['move 1 2, move 1 2', 'move 1 -2, move 1'],
		['move 1 2, move 2', 'move 1 -2, move 2 1'],
		['move 2, move 1 2', 'move 1 -2, move 1'],
		['move 1 1, move 2', 'move 1 -2, move 1'],
		['move 2, move 2', 'move 1 -2, move 1'],
		['move 2, move 2', 'move 1 -2, move 1'],
	],
	[/\|cant\|p2b: Blissey\|move: Heal Block\|Soft-Boiled$/m, /\|cant\|p2a: Vikavolt\|move: Heal Block\|Pollen Puff$/m,
		/\|-end\|p2b: Blissey\|move: Heal Block$/m]);


/** Cross product of per-slot options as `a ?? b ?? c` whole-side alternatives. */
const combos = (as, bs) => as.flatMap(a => bs.map(b => `${a}, ${b}`)).join(' ?? ');
// Preferred actions of the passive side in scenario 3 (first accepted wins; Struggle takes no target).
const P2 = combos(['move 1 1', 'move 2', 'move 3', 'move 4', 'move 1'], ['move 2 1', 'move 3', 'move 1 2', 'move 4', 'move 1']);

// 3. Taunt / Encore / Disable / Imprison flow. Blissey is Taunted (every move is Status, so it is left with
//    Struggle), Garchomp is Disabled, Encored and Imprisoned; durations count down (Taunt 3/4, Disable 5/4,
//    Encore 3/4 depending on whether the target still has a move queued) and each condition ends with `-end`.
scenario('taunt_encore_disable_imprison_flow', [1301, 1302, 1303, 1304],
	[mon('Gengar', 'Cursed Body', ['Taunt', 'Encore', 'Disable', 'Imprison']),
	 mon('Clefable', 'Magic Guard', ['Imprison', 'Taunt', 'Thunder Wave', 'Disable'], { evs: bulk }),
	 wall('Corviknight', 'Pressure'),
	 wall('Toxapex', 'Regenerator')],
	[mon('Blissey', 'Serene Grace', ['Thunder Wave', 'Soft-Boiled', 'Reflect', 'Light Screen'], { evs: bulk }),
	 mon('Garchomp', 'Rough Skin', ['Body Slam', 'Dragon Claw', 'Rock Slide', 'Swords Dance'], { level: 50 }),
	 wall('Snorlax', 'Thick Fat'),
	 wall('Skarmory', 'Sturdy')],
	[
		['move 1 1, move 4 2', P2],
		['move 2 2, move 3 1', P2],
		['move 3 2, move 2 2', P2],
		['move 4, move 1', P2],
		['move 1 1, move 4 2', P2],
		['move 2 2, move 3 1', P2],
		['move 3 1, move 4 2', P2],
		['move 4, move 2 1', P2],
	],
	[/\|-start\|p2a: Blissey\|move: Taunt$/m, /\|-end\|p2a: Blissey\|move: Taunt$/m, /\|-start\|p2b: Garchomp\|Disable\|/m,
		/\|-end\|p2b: Garchomp\|Disable$/m, /\|-start\|p2b: Garchomp\|Encore$/m, /\|-end\|p2b: Garchomp\|Encore$/m,
		/\|cant\|p2a: Blissey\|move: Taunt\|/m, /\|cant\|p2a: Blissey\|move: Imprison\|/m]);

// 4. Infestation (partiallytrapped): 1/8 residual damage, hard trap on a grounded non-Ghost (switch rejected with
//    hidden information), a Ghost foe takes the damage but is not trapped, and the volatile expires with
//    `-end|...|Infestation|[partiallytrapped]` after its 5-7 turn duration (one `random(5, 7)` draw).
scenario('infestation_trap_expiry', [1401, 1402, 1403, 1404],
	[mon('Vikavolt', 'Levitate', ['Infestation', 'Protect', 'Thunderbolt', 'U-turn'], { evs: bulk }),
	 wall('Clefable', 'Magic Guard'),
	 wall('Corviknight', 'Pressure'),
	 wall('Toxapex', 'Regenerator')],
	[mon('Blissey', 'Serene Grace', ['Protect', 'Reflect', 'Light Screen', 'Thunder Wave'], { evs: bulk }),
	 mon('Gengar', 'Cursed Body', ['Protect', 'Reflect', 'Light Screen', 'Shadow Ball'], { level: 50, evs: bulk }),
	 wall('Snorlax', 'Thick Fat'),
	 wall('Skarmory', 'Sturdy')],
	[
		['move 1 1, move 2', 'move 2, move 2'],
		['move 1 2, move 2', 'switch 3, switch 4 || move 2, move 2'],
		['move 2, move 2', 'switch 3, switch 4 || move 2, switch 4'],
		...repeat(7, 'move 2, move 2', 'move 2, move 2'),
	],
	[/\|-activate\|p2a: Blissey\|move: Infestation\|\[of\] p1a: Vikavolt$/m, /\|-end\|p2a: Blissey\|Infestation\|\[partiallytrapped\]$/m,
		/\|-activate\|p2b: Gengar\|move: Infestation\|\[of\] p1a: Vikavolt$/m]);

// 5. Infestation where the trapper leaves: the victim's next residual deletes the volatile silently
//    (`-end|...|[partiallytrapped]|[silent]`, no End event) and the victim can switch again.
scenario('infestation_trapper_leaves', [1501, 1502, 1503, 1504],
	[mon('Vikavolt', 'Levitate', ['Infestation', 'Protect', 'Thunderbolt', 'U-turn'], { evs: bulk }),
	 wall('Clefable', 'Magic Guard'),
	 wall('Corviknight', 'Pressure'),
	 wall('Toxapex', 'Regenerator')],
	[mon('Blissey', 'Serene Grace', ['Protect', 'Reflect', 'Light Screen', 'Thunder Wave'], { evs: bulk }),
	 mon('Snorlax', 'Thick Fat', ['Protect', 'Reflect', 'Light Screen', 'Thunder Wave'], { evs: bulk }),
	 wall('Skarmory', 'Sturdy'),
	 wall('Gengar', 'Cursed Body', { level: 50 })],
	[
		['move 1 1, move 2', 'move 2, move 2'],
		['move 2, move 2', 'move 2, move 2'],
		['switch 3, move 2', 'move 2, move 2'],
		['move 2, move 2', 'move 2, switch 4 ?? move 2, move 2'],
		['move 2, move 2', 'switch 3, switch 4 ?? switch 3, move 2 ?? move 2, move 2'],
	],
	[/\|-end\|p2a: Blissey\|Infestation\|\[partiallytrapped\]\|\[silent\]$/m]);

// 6. Spirit Shackle (trapped + linked trapper), No Retreat. Toxapex is trapped by Spirit Shackle, so its own No
//    Retreat boosts without the volatile (`delete move.volatileStatus`); Blissey's No Retreat sets the volatile
//    and a second use fails. When the Spirit Shackle user leaves, the linked trapped volatile is removed.
scenario('spiritshackle_noretreat', [1601, 1602, 1603, 1604],
	[mon('Decidueye', 'Overgrow', ['Spirit Shackle', 'Protect', 'Swords Dance', 'Leaf Blade'], { level: 60 }),
	 wall('Clefable', 'Magic Guard'),
	 wall('Corviknight', 'Pressure'),
	 wall('Mesprit', 'Levitate')],
	[mon('Toxapex', 'Regenerator', ['No Retreat', 'Protect', 'Thunder Wave', 'Reflect'], { evs: bulk }),
	 mon('Blissey', 'Serene Grace', ['No Retreat', 'Soft-Boiled', 'Reflect', 'Light Screen'], { evs: bulk }),
	 wall('Skarmory', 'Sturdy'),
	 wall('Snorlax', 'Thick Fat')],
	[
		['move 1 1, move 2', 'move 1, move 1'],
		['move 1 1, move 2', 'move 1, move 1'],
		['move 2, move 2', 'switch 3, switch 4 || move 1, move 3'],
		['switch 3, move 2', 'switch 3, switch 4 || move 4, move 3'],
		['move 2, move 2', 'switch 3, switch 4 || switch 3, move 3'],
	],
	[/\|-start\|p2b: Blissey\|move: No Retreat$/m, /\|-activate\|p2a: Toxapex\|trapped$/m, /\|-fail\|p2b: Blissey$/m]);

// 7. Trapping abilities: Arena Trap (grounded foes), Magnet Pull (Steel foes) and Shadow Tag (everything but a
//    Shadow Tag holder; Ghost foes are immune to trapping). Rejected switches reveal the trap; once the trappers
//    are gone the switch is accepted.
scenario('trap_abilities_switching', [1701, 1702, 1703, 1704],
	[mon('Dugtrio', 'Arena Trap', ['Protect', 'Reflect', 'Light Screen', 'Thunder Wave'], { evs: bulk }),
	 mon('Magnezone', 'Magnet Pull', ['Protect', 'Reflect', 'Light Screen', 'Thunder Wave'], { evs: bulk }),
	 wall('Gothitelle', 'Shadow Tag'),
	 wall('Clefable', 'Magic Guard')],
	[mon('Corviknight', 'Pressure', ['Protect', 'Reflect', 'Light Screen', 'Thunder Wave'], { evs: bulk }),
	 mon('Blissey', 'Serene Grace', ['Protect', 'Reflect', 'Light Screen', 'Thunder Wave'], { evs: bulk }),
	 wall('Gengar', 'Cursed Body', { level: 50 }),
	 wall('Toxapex', 'Regenerator')],
	[
		['move 1, move 1', 'switch 3, switch 4 || switch 3, move 1 || move 1, switch 4 || move 1, move 1'],
		['move 1, switch 3', 'switch 3, switch 4 || switch 3, move 1 || move 1, switch 4 || move 1, move 1'],
		['move 1, move 1', 'switch 3, switch 4 || switch 3, move 1 || move 1, switch 4 || move 1, move 1'],
		['switch 4, move 1', 'switch 3, switch 4 || switch 3, move 1 || move 1, switch 4 || move 1, move 1'],
		['move 1, switch 3', 'switch 3, switch 4 || switch 3, move 1 || move 1, move 1'],
		['move 1, move 1', 'switch 3, switch 4 || move 1, switch 3'],
	],
	[/\|switch\|p2b: Gengar\|/m]);

export { S, mon, scenario, bulk, repeat, wall, toID, sim };

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

/** Try `a ?? b ?? c` alternatives (each a whole-side choice); returns the first accepted one. */
function chooseFirst(session, side, spec) {
	const options = spec.split(' ?? ');
	for (const o of options.slice(0, -1)) {
		// A rejected choice leaves the side's choice state untouched, so the next option starts clean.
		const r = session.choose(side, o);
		if (r.ok) return o;
	}
	const last = options[options.length - 1];
	const r = session.choose(side, last);
	if (!r.ok) throw new Error(`${side}: no alternative of "${spec}" was accepted (${r.error})`);
	return last;
}

function run() {
	const out = [];
	for (const sc of S) {
		const session = new Session(sim, { seed: sc.seed, teams: sc.teams });
		out.push(`scenario ${sc.name}`, `seed ${sc.seed.join(',')}`, `p1 ${sc.teams[0]}`, `p2 ${sc.teams[1]}`);
		let snap = session.snapshot();
		const all = [...snap.log];
		out.push('step start\t\t', `seed_after ${snap.seed.join(',')}`, ...snap.log.map(l => `log ${l}`));
		let n = 0, forced = 0;
		const play = (label, c1, c2) => {
			const recorded = [c1, c2];
			[['p1', c1], ['p2', c2]].forEach(([side, c], k) => {
				if (c === '') return;
				if (c.includes(' ?? ')) {
					recorded[k] = chooseFirst(session, side, c);
					return;
				}
				const attempts = c.split(' || ');
				attempts.forEach((a, i) => {
					const r = session.choose(side, a);
					const last = i === attempts.length - 1;
					if (r.ok !== last) throw new Error(`${sc.name} ${label}: ${side} choice "${a}" ${r.ok ? 'accepted' : 'rejected: ' + r.error}, expected ${last ? 'accept' : 'reject'}`);
				});
			});
			snap = session.snapshot();
			all.push(...snap.log);
			if (process.env.DLT_DEBUG && (process.env.DLT_DEBUG === '1' || process.env.DLT_DEBUG === sc.name)) {
				console.log(`--- ${sc.name} ${label}: ${recorded[0]} / ${recorded[1]}`);
				console.log(snap.log.filter(l => !l.startsWith('|split|') && l !== '|' && !l.startsWith('|t:|')).join('\n'));
			}
			out.push(`step ${label}\t${recorded[0]}\t${recorded[1]}`, `seed_after ${snap.seed.join(',')}`, ...snap.log.map(l => `log ${l}`));
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
		const text = all.join('\n');
		for (const re of sc.expect) if (!re.test(text)) throw new Error(`${sc.name}: expected ${re} in the log`);
		console.log(`${sc.name}: ${n} turns (${forced} forced), faints=${all.filter(l => l.includes('|faint|')).length}`);
		if (session.anomalies.length) throw new Error(`${sc.name}: anomalies ${session.anomalies.join('; ')}`);
	}
	return out;
}
if (process.argv[1]?.endsWith('disable_lock_trap/scenarios.mjs')) {
	const lines = run();
	if (process.env.DLT_DEBUG) {
		console.log('(debug run: no file written)');
	} else {
		const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/conditions/locktrap/scenarios.txt', import.meta.url));
		fs.writeFileSync(dest, lines.join('\n') + '\n');
		console.log(`wrote ${dest} (${lines.length} lines)`);
	}
}
