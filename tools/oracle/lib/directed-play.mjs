// Plays one directed battle through the oracle's Session (the same driver gen-fixtures.mjs uses) and records a
// standard fixture (docs/design/FIXTURES.md) plus the single additive key `profile`.
//
// The loop below mirrors session.mjs `playBattle` step for step (Session, snapshot, choose, classifyRejection are
// imported, not copied); the only differences are the teams (built by directed-teams.mjs from the fixture's own
// teamSeeds) and the choice picker. With an unbiased profile the picker IS session.mjs' `pickChoice`; with a biased
// one it samples from `enumerateAll(request)`, i.e. every choice is still one the legal-choice enumerator lists.
// playBattle-equivalence (same teams, default policy => identical fixture) is checked by `gen-directed.mjs --selftest`.
import {
	FIXTURE_VERSION, FORMAT_ID, PLAYER_NAMES, SIDE_IDS, deriveSeeds, readOracleCommit, seedString,
} from './common.mjs';
import {
	buildInput, canOmitPass, enumerateAll, isFainted, pickChoice, requestFeatures, requestKind,
} from './choices.mjs';
import { Session, classifyRejection, newBattleStats } from './session.mjs';
import { buildTeams } from './directed-teams.mjs';

const MAX_PICK_ATTEMPTS = 25;
const MAX_STEPS = 4000;

// ---------------------------------------------------------------------------------------
// Biased picker
// ---------------------------------------------------------------------------------------

const optKey = o => (o.kind === 'move' ? `m${o.move}:${o.target ?? '-'}:${o.tera ? 'T' : '-'}` : o.kind === 'switch' ? `s${o.to}` : 'pass');

/** Per-slot option weights: moves uniform over (weighted) moves then targets, switches share `switchProb`. */
function slotWeights(req, slot, opts, moveWeight, knobs) {
	const w = new Map();
	const moveOpts = opts.filter(o => o.kind === 'move' && !o.tera);
	const idxs = [...new Set(moveOpts.map(o => o.move))];
	const weightOf = j => moveWeight(req.active[slot].moves[j - 1]?.id ?? 'struggle');
	let wm = 0;
	for (const j of idxs) wm += weightOf(j);
	const switches = opts.filter(o => o.kind === 'switch');
	const s = knobs.switchProb;
	const swTotal = switches.length && moveOpts.length ? wm * (s / (1 - s)) : 1;
	const t = knobs.teraProb;
	const teraOdds = t >= 1 ? 1e6 : t / (1 - t);
	for (const o of opts) {
		let x;
		if (o.kind === 'switch') x = swTotal / switches.length;
		else if (o.kind === 'move') {
			const nTargets = moveOpts.filter(m => m.move === o.move).length;
			x = weightOf(o.move) / nTargets * (o.tera ? teraOdds : 1);
		} else x = 1;
		w.set(optKey(o), Math.max(x, 1e-12));
	}
	return w;
}

const slotActionable = (req, i) => {
	if (requestKind(req) === 'move') {
		const mon = req.side.pokemon[i];
		return !isFainted(mon) && !mon.commanding;
	}
	return !!req.forceSwitch[i];
};

export function pickBiased(req, rng, knobs, moveWeight) {
	if (requestKind(req) === 'wait') return null;
	const all = enumerateAll(req);
	if (!all.length) throw new Error(`no legal choice for ${JSON.stringify(req).slice(0, 300)}`);
	const tables = [0, 1].map(i => {
		const seen = new Map();
		for (const e of all) if (e.parts[i]) seen.set(optKey(e.parts[i]), e.parts[i]);
		return slotWeights(req, i, [...seen.values()], moveWeight, knobs);
	});
	let total = 0;
	const ws = all.map(e => {
		const x = tables[0].get(optKey(e.parts[0])) * tables[1].get(optKey(e.parts[1]));
		total += x;
		return x;
	});
	let r = rng.random() * total;
	let pick = all[all.length - 1];
	for (let k = 0; k < all.length; k++) {
		r -= ws[k];
		if (r < 0) { pick = all[k]; break; }
	}
	const parts = pick.parts.map(p => ({ ...p }));
	for (let i = 1; i >= 0; i--) {
		if (slotActionable(req, i)) continue;
		const omit = rng.random() < knobs.implicitPassProb;
		if (omit && canOmitPass(parts, i)) parts[i].implicit = true;
	}
	return { input: buildInput(parts), parts };
}

/** Returns pick(req, rng, knobs, turn) for a profile: stock pickChoice unless the profile biases moves. */
export function makePicker(profile, data) {
	const b = profile.bias;
	const biased = profile.focusMoves.size > 0 && b.focusMoveWeight !== 1;
	if (!biased) return (req, rng, knobs) => pickChoice(req, rng, knobs);
	return (req, rng, knobs, turn) => pickBiased(req, rng, knobs, id => {
		let w = profile.focusMoves.has(id) ? b.focusMoveWeight : 1;
		const m = data.moves.get(id);
		if (m && !m.damaging && turn > b.stallTurn) w *= 0.25; // anti-stall: long battles drift towards damage
		return w;
	});
}

// ---------------------------------------------------------------------------------------
// The battle
// ---------------------------------------------------------------------------------------

export function playDirected(sim, profile, data, { runSeed, index, pick = makePicker(profile, data), maxTurns = 300, teams = null }) {
	const seeds = deriveSeeds(runSeed, index);
	const stats = newBattleStats();
	let built = null;
	const fixture = {
		v: FIXTURE_VERSION, format: FORMAT_ID, oracle: readOracleCommit(), runSeed, index,
		battleSeed: seeds.battleSeed, teamSeeds: seeds.teamSeeds, choiceSeed: seeds.choiceSeed,
		choicePolicy: { ...profile.knobs }, profile: profile.id,
		players: PLAYER_NAMES.map((name, n) => ({ id: SIDE_IDS[n], name })), teams: null, steps: [], end: null,
	};
	let session = null;
	try {
		// `teams` (selftest only) replaces the builder, e.g. by the random generator's teams.
		built = teams ? { packed: teams, notes: {}, active: [] } : buildTeams(sim, profile, data, { runSeed, index, seeds });
		fixture.teams = built.packed;
		const rng = new sim.PRNG(seedString(seeds.choiceSeed));
		stats.genderlessSets = built.packed.join(']').split(']').filter(mon => mon.split('|')[7] === '').length;
		session = new Session(sim, { seed: seeds.battleSeed, teams: built.packed });
		for (;;) {
			const snap = session.snapshot();
			if (snap.log.length > stats.maxStepLines) {
				stats.maxStepLines = snap.log.length;
				stats.maxStepAt = fixture.steps.length;
			}
			stats.logLines += snap.log.length;
			if (session.ended) {
				fixture.end = session.endRecord(snap);
				break;
			}
			if (fixture.steps.length >= MAX_STEPS) throw new Error(`battle exceeded ${MAX_STEPS} decision steps`);
			if (snap.turn > maxTurns) throw new Error(`battle exceeded ${maxTurns} turns`);
			const step = {
				log: snap.log, turn: snap.turn, state: snap.state, seed: snap.seed, requests: snap.requests,
				choices: [], submitted: { p1: null, p2: null },
			};
			if (fixture.steps.length === 0 && snap.seed.join() !== seeds.battleSeed.join()) stats.startupDraws++;
			for (const id of SIDE_IDS) for (const tag of requestFeatures(snap.requests[id])) stats.features[tag] = (stats.features[tag] || 0) + 1;
			const acting = SIDE_IDS.filter(id => requestKind(snap.requests[id]) !== 'wait');
			if (acting.length === 2 && rng.random() < 0.5) acting.reverse();
			for (const id of acting) chooseForSide(session, rng, profile.knobs, pick, id, snap.requests[id], step, stats);
			if (session.battle.log.length === session.logCursor && !session.ended) {
				throw new Error('all choices were accepted but the battle did not advance');
			}
			fixture.steps.push(step);
		}
		stats.turns = fixture.end.turns;
		stats.steps = fixture.steps.length;
		stats.tie = fixture.end.tie;
		if (session.anomalies.length) {
			fixture.anomalies = session.anomalies;
			stats.anomalies += session.anomalies.length;
		}
		return { fixture, stats, crash: null, built };
	} catch (err) {
		if (session && session.anomalies.length) fixture.anomalies = session.anomalies;
		return { fixture, stats, crash: { message: String(err && err.message), stack: String(err && err.stack) }, built };
	}
}

function chooseForSide(session, rng, knobs, pick, id, request, step, stats) {
	let req = request;
	for (let attempt = 0; ; attempt++) {
		const input = attempt < MAX_PICK_ATTEMPTS ? pick(req, rng, knobs, step.turn).input : 'default';
		const r = session.choose(id, input);
		stats.choiceCalls++;
		if (r.ok) {
			step.choices.push({ side: id, input, ok: true });
			step.submitted[id] = input;
			stats.decisions++;
			if (/terastallize/.test(input)) stats.teraChosen++;
			stats.switchesChosen += (input.match(/switch /g) || []).length;
			stats.passesExplicit += (input.match(/pass/g) || []).length;
			return;
		}
		const entry = { side: id, input, ok: false, error: r.error };
		if (r.request) entry.request = r.request;
		step.choices.push(entry);
		const verdict = classifyRejection(req, r);
		stats.rejections[verdict.key] = (stats.rejections[verdict.key] || 0) + 1;
		if (!verdict.explained) stats.unexplained.push({ turn: step.turn, side: id, input, error: r.error, request: req });
		if (input === 'default') throw new Error(`Side.autoChoose was rejected for ${id}: ${r.error}`);
		if (r.request) req = r.request;
	}
}
