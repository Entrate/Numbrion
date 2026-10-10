// Generates ground-truth vectors for owner C (requests, choice parsing/validation, legal actions)
// from the pinned Showdown build. One JSON object per line; see
// crates/engine/src/sim/choices/tests.rs for the consumer.
//
// Usage:
//   node tools/probes/choices/gen-vectors.mjs ~/src/pokemon-showdown --out <file.jsonl>
//        [--seed S] [--battles N] [--cap K] [--max-cases M] [--stunt-prob P] [--ops N] [--no-brute]
//
// For each decision boundary of randomly played battles it records:
//   * the internal Showdown state that the Rust test must reconstruct (party order, hp, flags,
//     move slots, hidden trapping/disabling, volatiles, slot conditions),
//   * the two cached requests as JSON.stringify text,
//   * a sequence of probe `choose` calls (valid, mutated, legacy and garbage strings) with the
//     exact rejection text, any re-sent request, the partial choice, cantUndo/forced counters,
//   * a brute-force acceptance bit-vector over a fixed candidate list (see `candidates`), plus
//     a hash of the set of normalized accepted choices (Side.getChoice) for legal-action checks.
// Boundaries are selected by request feature tags (rare features first). With --stunt-prob some
// boundaries get a Showdown-API state edit (hidden disabling, trapping, PP 0, recharge, Tera
// used, Revival Blessing) followed by makeRequest, to reach states random play rarely hits.
import fs from 'node:fs';
import { loadSim, deriveSeeds, seedString, FORMAT_ID, PLAYER_NAMES, SIDE_IDS } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';
import { DEFAULT_KNOBS, pickChoice, requestFeatures, requestKind } from '../../oracle/lib/choices.mjs';

const args = process.argv.slice(2);
const psPath = args.shift();
const opt = (name, def) => {
	const i = args.indexOf(name);
	return i < 0 ? def : args[i + 1];
};
const flag = name => args.includes(name);
const OUT = opt('--out', '/tmp/choices-vectors.jsonl');
const RUN_SEED = Number(opt('--seed', 7));
const BATTLES = Number(opt('--battles', 60));
const CAP = Number(opt('--cap', 5));
const MAX_CASES = Number(opt('--max-cases', 150));
const STUNT_PROB = Number(opt('--stunt-prob', 0.35));
const OPS = Number(opt('--ops', 14));
const BRUTE = !flag('--no-brute');
const START = Number(opt('--start-index', 0));
const PARSER_PROBES = flag('--parser-probes');

const sim = loadSim(psPath);

// ---------------------------------------------------------------------------------------
// Candidate list shared with the Rust test (tests.rs `candidates`).
// ---------------------------------------------------------------------------------------

export function candidateTokens() {
	const tokens = ['pass'];
	for (let m = 1; m <= 5; m++) {
		for (const loc of ['', ' -2', ' -1', ' 1', ' 2', ' 3']) {
			for (const tera of ['', ' terastallize']) tokens.push(`move ${m}${loc}${tera}`);
		}
	}
	for (let s = 1; s <= 7; s++) tokens.push(`switch ${s}`);
	return tokens;
}

export function candidates() {
	const tokens = candidateTokens();
	const out = [...tokens];
	for (const a of tokens) for (const b of tokens) out.push(`${a}, ${b}`);
	return out;
}

const fnv64 = text => {
	let h = 0xcbf29ce484222325n;
	for (const b of Buffer.from(text, 'utf8')) {
		h ^= BigInt(b);
		h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
	}
	return h;
};

// ---------------------------------------------------------------------------------------
// State dump
// ---------------------------------------------------------------------------------------

const idOrNull = v => (v === undefined || v === null ? null : v);

function dumpPokemon(side, p) {
	const volatiles = {};
	for (const [id, st] of Object.entries(p.volatiles)) {
		volatiles[id] = { targetLoc: idOrNull(st.targetLoc), move: idOrNull(st.move) };
	}
	return {
		ti: side.team.indexOf(p.set),
		name: p.name,
		species: p.species.id,
		baseSpecies: p.baseSpecies.id,
		details: p.details,
		hp: p.hp,
		maxhp: p.maxhp,
		status: p.status,
		fainted: p.fainted,
		isActive: p.isActive,
		position: p.position,
		ability: p.ability,
		baseAbility: p.baseAbility,
		item: p.item,
		teraType: p.teraType,
		terastallized: p.terastallized,
		canTerastallize: p.canTerastallize === undefined ? null : p.canTerastallize,
		trapped: p.trapped,
		maybeTrapped: !!p.maybeTrapped,
		maybeDisabled: !!p.maybeDisabled,
		maybeLocked: !!p.maybeLocked,
		switchFlag: p.switchFlag,
		lastMoveTargetLoc: idOrNull(p.lastMoveTargetLoc),
		transformed: !!p.transformed,
		stats: [p.baseStoredStats.atk, p.baseStoredStats.def, p.baseStoredStats.spa, p.baseStoredStats.spd, p.baseStoredStats.spe],
		types: p.types,
		addedType: p.addedType || '',
		moveSlots: p.moveSlots.map(m => [m.id, m.pp, m.maxpp, m.target, m.disabled === true ? 1 : m.disabled === 'hidden' ? 2 : 0]),
		volatiles,
	};
}

function dumpState(battle) {
	return {
		turn: battle.turn,
		requestState: battle.requestState,
		supportCancel: !!battle.supportCancel,
		ended: battle.ended,
		sides: battle.sides.map(side => ({
			name: side.name,
			pokemonLeft: side.pokemonLeft,
			active: side.active.map(a => (a ? side.pokemon.indexOf(a) : null)),
			slotConditions: [0, 1].map(i => Object.keys(side.slotConditions[i] || {})),
			party: side.pokemon.map(p => dumpPokemon(side, p)),
		})),
	};
}

// Compact per-side post state: hypotheses flags per party member.
const flagString = side => side.pokemon.map(p => `${p.trapped === 'hidden' ? 2 : p.trapped ? 1 : 0}${+!!p.maybeTrapped}${+!!p.maybeDisabled}${+!!p.maybeLocked}${p.switchFlag ? 1 : 0}`).join(' ');

const requestText = side => JSON.stringify(side.activeRequest);

// ---------------------------------------------------------------------------------------
// Running choices without committing (Battle.choose minus allChoicesDone/commitChoices)
// ---------------------------------------------------------------------------------------

function chooseNoCommit(session, side, input) {
	session.events = [];
	const battle = session.battle;
	const seedBefore = battle.prng.getSeed();
	const logBefore = JSON.stringify(battle.log);
	let ok = side.choose(input);
	if (!ok) {
		if (!side.choice.error) {
			side.emitChoiceError(`Unknown error for choice: ${input}. If you're not using a custom client, please report this as a bug.`);
		}
	} else if (!side.isChoiceDone()) {
		side.emitChoiceError(`Incomplete choice: ${input} - missing other pokemon`);
		ok = false;
	}
	const errors = [];
	let resent = null;
	for (const [type, data] of session.events) {
		if (type !== 'sideupdate') continue;
		const nl = data.indexOf('\n');
		const payload = data.slice(nl + 1);
		if (payload.startsWith('|error|')) errors.push(payload.slice('|error|'.length));
		else if (payload.startsWith('|request|')) resent = payload.slice('|request|'.length);
	}
	session.events = [];
	if (battle.prng.getSeed() !== seedBefore || JSON.stringify(battle.log) !== logBefore) {
		throw new Error(`noncommitting choice changed PRNG or battle log: ${JSON.stringify(input)}`);
	}
	return { ok, error: errors.length ? errors[errors.length - 1] : null, resent };
}

// ---------------------------------------------------------------------------------------
// Probe input generation
// ---------------------------------------------------------------------------------------

const ATOMS = [
	'pass', 'skip', 'auto', 'default', 'shift', 'shift 1', 'team 1', 'team 2, 3', 'testfight', 'move testfight', 'move', 'switch',
	'move 0', 'move 5', 'move 99999999999999999999999', 'switch 0', 'switch 7', 'switch -1', 'switch 1x', 'switch 0x2', 'switch  2',
	'move 1 0', 'move 1 3', 'move 1 -3', 'move 1 +1', 'move 1 +2', 'move 1 1 1', 'move 1 2 terastallize', 'move 1 terastallize 1',
	'move 1 mega', 'move 1 megax', 'move 1 megay', 'move 1 zmove', 'move 1 ultra', 'move 1 dynamax', 'move 1 gigantamax', 'move 1 max',
	'move 1 terastal', 'move 1 1 terastal', 'move 1 mega terastallize', 'move 1 1 1 1', 'MOVE 1', 'Move 1', 'move  1', 'move 1  ',
	'move 1 1 dynamax', 'move 2 -1 max', 'move 1 -1 zmove', 'move conversion 2', 'move Conversion 2', 'move 1 conversion2',
	',', ',,', '', ' ', 'foo', 'foo, move 1 1', 'move 1 1, foo', 'move 1 1, move 2 1, move 3 1', 'pass pass', 'pass 1', 'skip 2',
	'auto, pass', 'default, default', 'pass, auto', 'move 1 1, auto', 'switch 3, auto', 'testfight, pass', 'move 1 mega, pass',
	'move 1 1 mega', 'move 1 1 terastallize terastallize', 'switch\t3', 'move\t1\t1', 'move 1 1 ', ' move 1 1', 'move 1 1 ', 'move 1 1',
	'move 1 mega  1', 'move 1 zmove\t1', 'move 1 terastallize \t+1',
	'move 1  \uFEFF1', 'move 1 mega\u2028-2', 'move 1  1  2',
	'move 1 1 terastallize, move 1 2 terastallize', 'switch 3, switch 3',
];

function mutate(rng, s) {
	const ops = [
		t => t.replace(/\d/, () => String(rng.random(0, 9))),
		t => t + (rng.random() < 0.5 ? ' terastallize' : ' ' + rng.sample(['mega', 'max', 'dynamax', 'zmove', 'ultra', 'terastal'])),
		t => t.replace(/ terastallize/, ''),
		t => t.split(', ').reverse().join(', '),
		t => t + ' ' + rng.sample(['1', '2', '-1', '-2', '+1', '3']),
		t => t.replace(/ [-+]?\d$/, ''),
		t => t.replace(/move/, 'switch'),
		t => t.replace(/switch/, 'move'),
		t => t.toUpperCase(),
		t => t + ',',
		t => ' ' + t,
		t => t.replace(/, /g, ','),
		t => t + ', pass',
		t => 'pass, ' + t,
		t => t.split(', ')[0],
		t => `${t}, ${t}`,
	];
	let out = s;
	const n = rng.random(1, 3);
	for (let i = 0; i < n; i++) out = rng.sample(ops)(out);
	return out;
}

function probeInputs(rng, req, battleSide) {
	const inputs = PARSER_PROBES ? ATOMS.slice() : [];
	const names = [];
	if (req && req.active) {
		for (const a of req.active) for (const m of a.moves) names.push(m.id, m.move);
	}
	for (const p of battleSide.pokemon) names.push(p.name, p.species.id, p.species.name);
	const legal = () => {
		if (!req || req.wait) return rng.sample(ATOMS);
		try {
			return pickChoice(req, rng, { ...DEFAULT_KNOBS, switchProb: 0.3, teraProb: 0.4 }).input;
		} catch {
			return rng.sample(ATOMS);
		}
	};
	const hasMaybeLocked = !!(req && req.active && req.active.some(a => a.maybeLocked));
	if (hasMaybeLocked) {
		// The client "Fight button" helper has a hidden-information path.
		inputs.push(rng.sample(['testfight', 'move testfight', 'testfight, pass', 'move testfight 1']));
	}
	for (let i = 0; i < OPS; i++) {
		const r = rng.random();
		if (r < 0.34) inputs.push(legal());
		else if (r < 0.58) inputs.push(mutate(rng, legal()));
		else if (r < 0.78) inputs.push(rng.sample(ATOMS));
		else if (r < 0.9) {
			const nm = rng.sample(names.length ? names : ['x']);
			inputs.push(rng.sample([`move ${nm}`, `move ${nm} 1`, `move ${nm} -2`, `switch ${nm}`, `move ${nm} terastallize`, `move ${nm.toUpperCase()}`]));
		} else {
			inputs.push(`${rng.sample(ATOMS)}, ${rng.sample(ATOMS)}`);
		}
	}
	return inputs;
}

// ---------------------------------------------------------------------------------------
// Stunts: edit the Showdown state through its own API, then rebuild the requests.
// ---------------------------------------------------------------------------------------

function pickActiveMon(rng, battle, pred = () => true) {
	const mons = battle.sides.flatMap(s => s.active).filter(p => p && !p.fainted && pred(p));
	return mons.length ? rng.sample(mons) : null;
}

function applyStunt(rng, session) {
	const battle = session.battle;
	const kind = rng.sample(['disable', 'disableAll', 'hiddenAll', 'hiddenOne', 'pp0', 'ppAll', 'trap', 'trapHidden', 'recharge', 'tera', 'revival', 'healblock', 'curse', 'curseGhost', 'maybeAll']);
	let tag = `stunt:${kind}`;
	const mon = pickActiveMon(rng, battle);
	if (!mon) return null;
	switch (kind) {
	case 'disable': {
		const slot = rng.sample(mon.moveSlots);
		mon.disableMove(slot.id, false, battle.dex.moves.get('taunt'));
		break;
	}
	case 'disableAll':
		for (const slot of mon.moveSlots) mon.disableMove(slot.id, false, battle.dex.moves.get('encore'));
		break;
	case 'hiddenAll':
		for (const slot of mon.moveSlots) mon.disableMove(slot.id, 'hidden', battle.dex.moves.get('imprison'));
		mon.maybeDisabled = true;
		break;
	case 'hiddenOne': {
		const slot = rng.sample(mon.moveSlots);
		mon.disableMove(slot.id, 'hidden', battle.dex.moves.get('imprison'));
		mon.maybeDisabled = true;
		break;
	}
	case 'pp0':
		rng.sample(mon.moveSlots).pp = 0;
		break;
	case 'ppAll':
		for (const slot of mon.moveSlots) slot.pp = 0;
		break;
	case 'trap':
		mon.trapped = true;
		break;
	case 'trapHidden':
		mon.trapped = 'hidden';
		mon.maybeTrapped = true;
		break;
	case 'recharge':
		mon.addVolatile('mustrecharge');
		break;
	case 'curse':
	case 'curseGhost': {
		const slot = rng.sample(mon.moveSlots);
		Object.assign(slot, { move: 'Curse', id: 'curse', pp: 16, maxpp: 16, target: battle.dex.moves.get('curse').target, disabled: false });
		if (kind === 'curseGhost') mon.types = ['Ghost'];
		break;
	}
	case 'maybeAll':
		mon.maybeDisabled = true;
		mon.maybeLocked = true;
		mon.maybeTrapped = true;
		break;
	case 'healblock':
		if (!mon.moveSlots.some(m => m.id === 'pollenpuff')) return null;
		mon.addVolatile('healblock');
		break;
	case 'tera': {
		const side = mon.side;
		for (const ally of side.pokemon) ally.canTerastallize = null;
		mon.terastallized = mon.teraType;
		break;
	}
	case 'revival': {
		const side = mon.side;
		if (!side.pokemon.some(p => p.fainted)) return null;
		side.addSlotCondition(mon, 'revivalblessing', mon, battle.dex.moves.get('revivalblessing'));
		mon.switchFlag = 'revivalblessing';
		battle.requestState = 'switch';
		battle.makeRequest('switch');
		return tag;
	}
	}
	battle.makeRequest();
	return tag;
}

// ---------------------------------------------------------------------------------------
// One case
// ---------------------------------------------------------------------------------------

function bitsToBase64(bits) {
	const bytes = Buffer.alloc(Math.ceil(bits.length / 8));
	bits.forEach((b, i) => { if (b) bytes[i >> 3] |= 1 << (i & 7); });
	return bytes.toString('base64');
}

function bruteForce(session, side, list) {
	const bits = [];
	const accepted = new Set();
	for (const input of list) {
		side.clearChoice();
		const r = chooseNoCommit(session, side, input);
		bits.push(r.ok);
		if (r.ok) accepted.add(side.getChoice());
	}
	side.clearChoice();
	const sorted = [...accepted].sort();
	return {
		bits: bitsToBase64(bits),
		count: accepted.size,
		hash: fnv64(sorted.join('\n')).toString(16),
	};
}

function buildCase(session, id, seeds, teams, tags, stuntTag, rng) {
	const battle = session.battle;
	const state = dumpState(battle);
	const requests = battle.sides.map(requestText);
	const parsed = battle.sides.map(s => (s.activeRequest ? JSON.parse(requestText(s)) : null));
	const out = {
		id, seed: seeds.battleSeed, boundarySeed: battle.prng.getSeed().split(',').map(Number),
		teams, names: PLAYER_NAMES, tags, stunt: stuntTag, state, requests,
		initialFlags: battle.sides.map(flagString),
		ops: [], afterOps: null, brute: null, afterBrute: null,
	};
	// Probe sequences.
	battle.sides.forEach((side, n) => {
		const req = parsed[n];
		const acting = side.requestState !== '';
		const inputs = acting ? probeInputs(rng, req, side) : ['move 1', 'pass', 'default'];
		for (const input of inputs) {
			const r = chooseNoCommit(session, side, input);
			out.ops.push({
				side: n, input, ok: r.ok, error: r.error, resent: r.resent,
				choice: side.getChoice(), cantUndo: side.choice.cantUndo,
				fsl: side.choice.forcedSwitchesLeft, fpl: side.choice.forcedPassesLeft,
			});
		}
	});
	out.afterOps = battle.sides.map(s => ({ request: requestText(s) === out.requests[s.n] ? null : requestText(s), flags: flagString(s) }));
	for (const s of battle.sides) s.clearChoice();
	if (BRUTE) {
		const list = candidates();
		out.brute = battle.sides.map(side => (side.requestState === '' ? null : bruteForce(session, side, list)));
		out.afterBrute = battle.sides.map(s => {
			const before = out.afterOps[s.n].request ?? out.requests[s.n];
			return { request: requestText(s) === before ? null : requestText(s), flags: flagString(s) };
		});
	}
	for (const s of battle.sides) s.clearChoice();
	session.events = [];
	return out;
}

// ---------------------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------------------

const tagCounts = new Map();
const wantCase = tags => {
	let rare = false;
	for (const t of tags) if ((tagCounts.get(t) || 0) < CAP) rare = true;
	return rare;
};
const noteTags = tags => { for (const t of tags) tagCounts.set(t, (tagCounts.get(t) || 0) + 1); };

function featureTags(parsedRequests) {
	const tags = new Set();
	for (const r of parsedRequests) if (r) for (const t of requestFeatures(r)) tags.add(t);
	return [...tags];
}

const out = fs.openSync(OUT, 'w');
let emitted = 0;
let skipped = 0;
const t0 = Date.now();
for (let b = 0; b < BATTLES && emitted < MAX_CASES; b++) {
	const index = START + b;
	const seeds = deriveSeeds(RUN_SEED, index);
	const teams = seeds.teamSeeds.map(s => sim.Teams.pack(sim.Teams.getGenerator(FORMAT_ID, seedString(s)).getTeam()));
	const rng = new sim.PRNG(seedString(seeds.choiceSeed));
	let session;
	try {
		session = new Session(sim, { seed: seeds.battleSeed, teams });
		for (let step = 0; step < 400 && emitted < MAX_CASES; step++) {
			const snap = session.snapshot();
			if (session.ended) break;
			const battle = session.battle;
			let stuntTag = null;
			if (rng.random() < STUNT_PROB) {
				try {
					stuntTag = applyStunt(rng, session);
				} catch (err) {
					stuntTag = null;
					throw new Error(`stunt failed: ${err.message}`);
				}
			}
			const parsed = battle.sides.map(s => (s.activeRequest ? JSON.parse(requestText(s)) : null));
			const tags = featureTags(parsed);
			if (stuntTag) tags.push(stuntTag);
			const transformed = battle.sides.some(s => s.pokemon.some(p => p.transformed));
			if (!transformed && wantCase(tags)) {
				noteTags(tags);
				const c = buildCase(session, `b${index}s${step}`, seeds, teams, tags, stuntTag, rng);
				fs.writeSync(out, JSON.stringify(c) + '\n');
				emitted++;
				// Continue the game from the (possibly updated) cached requests.
			} else if (!transformed && !wantCase(tags)) {
				skipped++;
			}
			// Play on: pick with the oracle policy from the current cached requests.
			const acting = battle.sides.filter(s => s.requestState !== '');
			if (acting.length === 2 && rng.random() < 0.5) acting.reverse();
			for (const side of acting) {
				side.clearChoice();
				let req = JSON.parse(requestText(side));
				for (let attempt = 0; ; attempt++) {
					const input = attempt < 25 ? pickChoice(req, rng, DEFAULT_KNOBS).input : 'default';
					const r = session.choose(SIDE_IDS[side.n], input);
					if (r.ok) break;
					if (input === 'default') throw new Error(`default rejected: ${r.error}`);
					if (r.request) req = r.request;
				}
			}
			void snap;
		}
	} catch (err) {
		console.error(`battle ${index}: ${err.message}`);
	}
}
fs.closeSync(out);
console.error(`wrote ${emitted} cases (skipped ${skipped} boundaries) in ${((Date.now() - t0) / 1000).toFixed(1)}s`);
console.error([...tagCounts.entries()].sort().map(([k, v]) => `${k}=${v}`).join(' '));
