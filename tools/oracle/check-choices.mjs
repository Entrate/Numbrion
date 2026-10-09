// Validates tools/oracle/lib/choices.mjs against Showdown itself.
//
// Usage: node tools/oracle/check-choices.mjs <path-to-pokemon-showdown> [--count N] [--seed S] [--stride K] [--no-scenarios]
//
// 1. Plays N random battles with the normal picker. At every K-th decision boundary and for every acting
//    side whose request carries no hidden-information flags (maybeDisabled / maybeLocked / maybeTrapped),
//    it runs a brute-force universe of ~6000 choice strings straight into `Side.choose()` (the battle is not
//    advanced by this), canonicalizes what Showdown accepted, and compares with `enumerateAll(request)`:
//      * soundness:    every enumerated joint choice is accepted by Showdown and completes the choice;
//      * completeness: every accepted string of the universe is enumerated;
//      * implicit passes: dropping the `pass` of a non-acting slot yields the same actions;
//      * the random picker only produces enumerated choices.
// 2. Runs hand-built scenarios for request shapes that random play rarely reaches (Revival Blessing with
//    and without a fainted ally slot, double faint with one bench Pokemon, locked Outrage, Recharge,
//    Struggle, Mean Look, Commander) through the same check.
//
// Known Showdown quirk handled by canonicalization: `terastallize` on a locked move or Struggle is accepted
// and silently dropped, so such strings canonicalize to the plain `move 1`.
import {
	DEFAULT_KNOBS, buildInput, canOmitPass, enumerateAll, optionKey, pickChoice, requestFeatures, requestKind,
} from './lib/choices.mjs';
import { FORMAT_ID, SIDE_IDS, deriveSeeds, loadSim, seedString } from './lib/common.mjs';
import { Session } from './lib/session.mjs';

function parseArgs(argv) {
	const a = { psPath: null, count: 10, seed: 1000, stride: 1, scenarios: true };
	for (let i = 0; i < argv.length; i++) {
		if (argv[i] === '--count') a.count = Number(argv[++i]);
		else if (argv[i] === '--seed') a.seed = Number(argv[++i]);
		else if (argv[i] === '--stride') a.stride = Number(argv[++i]);
		else if (argv[i] === '--no-scenarios') a.scenarios = false;
		else if (!a.psPath) a.psPath = argv[i];
		else throw new Error(`unexpected argument ${argv[i]}`);
	}
	if (!a.psPath) throw new Error('usage: check-choices.mjs <path-to-pokemon-showdown> [--count N] [--seed S] [--stride K] [--no-scenarios]');
	return a;
}

function actionKey(action) {
	switch (action.choice) {
	case 'pass': return 'pass';
	case 'switch':
	case 'instaswitch':
	case 'revivalblessing': return `s${action.target.position + 1}`;
	case 'move':
		if (action.moveSlot === undefined) return 'm1:-:-'; // locked move / Struggle
		return `m${action.moveSlot + 1}:${action.targetLoc || '-'}:${action.terastallize ? 'T' : '-'}`;
	}
	throw new Error(`unexpected action ${action.choice}`);
}

/** Runs one input through Side.choose without committing. Returns the canonical joint key or null. */
function tryChoice(session, side, input) {
	const flags = side.active.map(p => p.switchFlag);
	const accepted = side.choose(input) && side.isChoiceDone() && side.choice.actions.length === 2;
	const key = accepted ? side.choice.actions.map(actionKey).join(' | ') : null;
	// Side.chooseSwitch clears switchFlag at choice time for Revival Blessing; undo that.
	side.active.forEach((p, i) => { p.switchFlag = flags[i]; });
	session.events = [];
	return key;
}

function universe() {
	const tokens = ['pass'];
	for (let m = 1; m <= 5; m++) {
		for (const t of ['', ' 1', ' 2', ' 3', ' -1', ' -2', ' -3']) {
			tokens.push(`move ${m}${t}`, `move ${m}${t} terastallize`);
		}
	}
	for (let k = 1; k <= 7; k++) tokens.push(`switch ${k}`);
	const joint = [];
	for (const a of tokens) for (const b of tokens) joint.push(`${a}, ${b}`);
	return joint;
}

const args = parseArgs(process.argv.slice(2));
const sim = loadSim(args.psPath);
const UNIVERSE = universe();
const stats = {
	battles: 0, scenarios: 0, boundaries: 0, sideBoundaries: 0, skippedHidden: 0, trials: 0, enumerated: 0, accepted: 0,
	implicitChecked: 0, implicitForbiddenChecked: 0, features: {}, failures: [],
};
const fail = (msg, extra) => {
	if (stats.failures.length < 15) stats.failures.push({ msg, ...extra, request: extra.request && extra.request.slice(0, 1500) });
};
const t0 = performance.now();

/** Compares Showdown and the enumerator for every acting side at the current boundary. */
function checkBoundary(session, snap, rng, label) {
	stats.boundaries++;
	for (const id of SIDE_IDS) {
		const req = snap.requests[id];
		if (requestKind(req) === 'wait') continue;
		const side = session.battle.getSide(id);
		if (req.active && req.active.some(a => a.maybeDisabled || a.maybeLocked || a.maybeTrapped)) {
			stats.skippedHidden++;
			continue;
		}
		stats.sideBoundaries++;
		for (const tag of requestFeatures(req)) stats.features[tag] = (stats.features[tag] || 0) + 1;
		const before = JSON.stringify(side.activeRequest);
		const enumerated = new Map();
		for (const c of enumerateAll(req)) enumerated.set(c.parts.map(optionKey).join(' | '), c);
		stats.enumerated += enumerated.size;
		const acceptedKeys = new Map();
		for (const input of UNIVERSE) {
			stats.trials++;
			const key = tryChoice(session, side, input);
			if (key !== null && !acceptedKeys.has(key)) acceptedKeys.set(key, input);
		}
		stats.accepted += acceptedKeys.size;
		const ctx = { where: label, turn: snap.turn, side: id, request: JSON.stringify(req).slice(0, 2500) };
		for (const [key, c] of enumerated) {
			if (!acceptedKeys.has(key)) {
				const got = tryChoice(session, side, c.input);
				fail('enumerated choice not accepted by Showdown', { ...ctx, input: c.input, key, got, error: JSON.stringify(side.choice.error) });
			}
		}
		for (const [key, input] of acceptedKeys) {
			if (!enumerated.has(key)) fail('Showdown accepts a choice the enumerator lacks', { ...ctx, input, key });
		}
		// implicit passes: the same joint choice with the pass of one auto-passing slot omitted
		for (const c of enumerated.values()) {
			const droppable = [0, 1].filter(i => c.parts[i].kind === 'pass' && !(req.forceSwitch && req.forceSwitch[i]));
			for (const drop of droppable) {
				const parts = c.parts.map((p, i) => (i === drop ? { ...p, implicit: true } : p));
				const input = buildInput(parts);
				const key = tryChoice(session, side, input);
				const same = key === c.parts.map(optionKey).join(' | ');
				if (canOmitPass(c.parts, drop)) {
					stats.implicitChecked++;
					if (!same) fail('implicit-pass variant differs', { ...ctx, input, key, want: c.input });
				} else {
					// Documents why canOmitPass is needed: omitting this pass changes the meaning of the input.
					stats.implicitForbiddenChecked++;
					if (same) fail('canOmitPass forbids an omission that is actually equivalent', { ...ctx, input, want: c.input });
				}
			}
		}
		// Rejecting a request-disabled move makes Showdown add `disabledSource: ""` to the live request (and
		// re-send it). Restore the request so the real battle is not perturbed.
		side.activeRequest = JSON.parse(before);
		for (let k = 0; k < 20; k++) {
			const picked = pickChoice(req, rng, DEFAULT_KNOBS);
			if (!enumerated.has(picked.parts.map(optionKey).join(' | '))) {
				fail('picker produced a choice outside the enumerated set', { ...ctx, input: picked.input });
			}
		}
	}
}

function advance(session, snap, rng) {
	for (const id of SIDE_IDS.filter(s => requestKind(snap.requests[s]) !== 'wait')) {
		let req = snap.requests[id];
		for (let attempt = 0; attempt < 20; attempt++) {
			const r = session.choose(id, pickChoice(req, rng, DEFAULT_KNOBS).input);
			if (r.ok) break;
			if (r.request) req = r.request;
		}
	}
}

// ---------------------------------------------------------------------------------------
// random battles
// ---------------------------------------------------------------------------------------

for (let b = 0; b < args.count; b++) {
	const seeds = deriveSeeds(args.seed, b);
	const teams = seeds.teamSeeds.map(s => sim.Teams.pack(sim.Teams.getGenerator(FORMAT_ID, seedString(s)).getTeam()));
	const rng = new sim.PRNG(seedString(seeds.choiceSeed));
	const session = new Session(sim, { seed: seeds.battleSeed, teams });
	stats.battles++;
	for (let boundary = 0; ; boundary++) {
		const snap = session.snapshot();
		if (session.ended) break;
		if (boundary % args.stride === 0) checkBoundary(session, snap, rng, `battle ${b}`);
		advance(session, snap, rng);
	}
}

// ---------------------------------------------------------------------------------------
// hand-built scenarios
// ---------------------------------------------------------------------------------------

const mk = (species, moves, extra = {}) => ({
	name: species, species, moves, ability: 'No Ability', item: '', nature: 'Serious', gender: 'M', level: 50, evs: {}, ivs: {}, ...extra,
});
const faint = (session, sideIdx, idx) => {
	const side = session.battle.sides[sideIdx];
	const p = side.pokemon[idx];
	p.hp = 0;
	p.fainted = true;
	p.status = 'fnt';
	side.pokemonLeft--;
};
const filler = ['Zubat', 'Rattata', 'Pidgey', 'Weedle', 'Caterpie'].map(s => mk(s, ['tackle']));
const splashers = [mk('Magikarp', ['splash']), mk('Feebas', ['splash']), mk('Ditto', ['transform'])];
const blissey = (moves = ['splash']) => mk('Blissey', moves, { level: 100 });
const tanks = [blissey(), blissey(), mk('Ditto', ['transform'])];
const doubleFaintTeams = [
	[mk('Magikarp', ['splash'], { level: 1 }), mk('Feebas', ['splash'], { level: 1 }), ...filler.slice(1)],
	[mk('Exploud', ['hypervoice'], { level: 100 }), mk('Loudred', ['hypervoice'], { level: 100 }), mk('Whismur', ['splash'])],
];

const scenarios = [
	{
		name: 'revival blessing: fainted ally slot and fainted bench',
		teams: [[mk('Pawmot', ['revivalblessing', 'protect']), ...filler], splashers],
		setup: s => { faint(s, 0, 1); faint(s, 0, 3); },
		script: [['p1', 'move 1'], ['p2', 'move 1, move 1']],
	},
	{
		name: 'revival blessing: no unfainted bench',
		teams: [[mk('Pawmot', ['revivalblessing', 'protect']), ...filler], splashers],
		setup: s => { for (const i of [2, 3, 4, 5]) faint(s, 0, i); },
		script: [['p1', 'move 1, move 1 1'], ['p2', 'move 1, move 1']],
	},
	{
		name: 'revival blessing from slot b: no unfainted bench (a pass-only slot after an auto-pass slot)',
		teams: [[mk('Zubat', ['tackle']), mk('Pawmot', ['revivalblessing', 'protect']), ...filler.slice(1)], splashers],
		setup: s => { for (const i of [2, 3, 4, 5]) faint(s, 0, i); },
		script: [['p1', 'move 1 1, move 1'], ['p2', 'move 1, move 1']],
	},
	{
		name: 'both actives faint, one bench Pokemon left',
		teams: doubleFaintTeams,
		setup: s => { for (const i of [3, 4, 5]) faint(s, 0, i); },
		script: [['p1', 'move 1, move 1'], ['p2', 'move 1, move 1']],
	},
	{
		name: 'both actives faint, two bench Pokemon left',
		teams: doubleFaintTeams,
		setup: s => { for (const i of [4, 5]) faint(s, 0, i); },
		script: [['p1', 'move 1, move 1'], ['p2', 'move 1, move 1']],
	},
	{
		name: 'locked Outrage',
		teams: [[mk('Dragonite', ['outrage', 'tackle']), mk('Zubat', ['tackle', 'protect']), ...filler.slice(1)], tanks],
		setup: () => {},
		script: [['p1', 'move 1, move 1 1'], ['p2', 'move 1, move 1']],
	},
	{
		name: 'recharge (Hyper Beam)',
		teams: [[mk('Snorlax', ['hyperbeam', 'tackle'], { level: 50 }), mk('Zubat', ['tackle', 'protect']), ...filler.slice(1)], tanks],
		setup: () => {},
		script: [['p1', 'move 1 2, move 1 1'], ['p2', 'move 1, move 1']],
	},
	{
		name: 'Struggle (no PP)',
		teams: [[mk('Rattata', ['tackle']), mk('Zubat', ['tackle', 'protect']), ...filler.slice(1)], tanks],
		setup: s => { for (const m of s.battle.sides[0].pokemon[0].moveSlots) m.pp = 0; },
		script: [['p1', 'move 1, move 1 1'], ['p2', 'move 1, move 1']],
	},
	{
		name: 'Mean Look trapped slot',
		teams: [[mk('Magikarp', ['splash']), mk('Feebas', ['splash']), ...filler.slice(1)],
			[mk('Gengar', ['meanlook']), mk('Ditto', ['splash']), mk('Pidgey', ['tackle'])]],
		setup: () => {},
		script: [['p1', 'move 1, move 1'], ['p2', 'move 1 1, move 1']],
	},
	{
		name: 'Commander (Tatsugiri in Dondozo)',
		teams: [[mk('Tatsugiri', ['icebeam', 'protect'], { ability: 'Commander' }),
			mk('Dondozo', ['waterfall', 'protect'], { ability: 'Unaware' }), ...filler.slice(1)], splashers],
		setup: () => {},
		script: [],
	},
];

if (args.scenarios) {
	for (const sc of scenarios) {
		stats.scenarios++;
		const rng = new sim.PRNG('5,6,7,8');
		const session = new Session(sim, { seed: [11, 22, 33, 44], teams: sc.teams.map(t => sim.Teams.pack(t)) });
		sc.setup(session);
		let snap = session.snapshot();
		if (!sc.script.length) checkBoundary(session, snap, rng, `scenario "${sc.name}" (start)`);
		for (const [id, input] of sc.script) {
			const r = session.choose(id, input);
			if (!r.ok) fail('scenario script choice rejected', { where: sc.name, id, input, error: r.error });
		}
		if (sc.script.length) {
			snap = session.snapshot();
			if (session.ended) {
				fail('scenario battle ended', { where: sc.name });
				continue;
			}
			checkBoundary(session, snap, rng, `scenario "${sc.name}"`);
			console.error(`scenario "${sc.name}": ${snap.state} ${SIDE_IDS.map(id => requestFeatures(snap.requests[id]).join('+')).join(' / ')}`);
		}
	}
}

console.log(JSON.stringify({ ...stats, seconds: +((performance.now() - t0) / 1000).toFixed(1) }, null, 1));
process.exit(stats.failures.length ? 2 : 0);
