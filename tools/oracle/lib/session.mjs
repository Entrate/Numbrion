// Drives one Showdown Battle the way a real server does, and records / replays fixtures.
//
// Flush semantics (spec 02, open question 5): Showdown's BattleStream calls `battle.sendUpdates()`
// after EVERY input line it handles (`>start`, each `>player`, each `>pN <choice>`). `Session`
// does exactly that after the constructor, each setPlayer, and each `Battle.choose`. This matters
// for two reasons:
//  * requests are only (re)sent by sendUpdates, and only when the log has grown;
//  * `singleEvent` aborts with "LINE LIMIT EXCEEDED" if more than 1000 log lines are unsent
//    (`log.length - sentLogPos`), and sentLogPos only advances in sendUpdates.
// A choice is rejected by `Battle.choose` returning false; the `|error|` text and any request
// re-emission arrive as `sideupdate` messages.
import {
	FIXTURE_VERSION, FORMAT_ID, PLAYER_NAMES, SIDE_IDS, deriveSeeds, normalizeLog, parseSeed, readOracleCommit,
	seedString,
} from './common.mjs';
import { DEFAULT_KNOBS, hiddenFlags, pickChoice, requestFeatures, requestKind } from './choices.mjs';

export class Session {
	constructor(sim, { seed, teams, names = PLAYER_NAMES }) {
		this.sim = sim;
		this.events = [];
		this.logCursor = 0; // battle.log entries already handed out by snapshot()
		this.streamLines = []; // lines delivered through 'update' messages since the last snapshot
		this.boundaryRequests = { p1: null, p2: null }; // JSON text of the requests of the latest boundary
		this.recorded = []; // every raw log entry handed out, for the late-edit check
		this.anomalies = [];
		this.endMessage = null;

		const startSeed = seedString(seed);
		this.battle = new sim.Battle({
			formatid: FORMAT_ID,
			// "a,b,c,d" with numeric a selects the Gen5 LCG (PRNG.setSeed); anything starting with
			// 'sodium,' would select the ChaCha based generator.
			seed: startSeed,
			send: (type, data) => this.events.push([type, data]),
		});
		if (!(this.battle.prng.rng instanceof sim.Gen5RNG)) throw new Error('battle PRNG is not Gen5RNG');
		if (this.battle.prng.getSeed() !== startSeed) throw new Error('battle PRNG seed was not applied');
		this.battle.sendUpdates(); // >start
		this.ingest();
		this.battle.setPlayer('p1', { name: names[0], team: teams[0] });
		this.battle.sendUpdates(); // >player p1
		this.ingest();
		this.battle.setPlayer('p2', { name: names[1], team: teams[1] });
		this.battle.sendUpdates(); // >player p2: the battle starts here
		this.noteBoundaryRequests(this.ingest());
	}

	noteBoundaryRequests(digest) {
		for (const id of SIDE_IDS) if (digest.requests[id] !== null) this.boundaryRequests[id] = digest.requests[id];
	}

	get ended() {
		return this.battle.ended;
	}

	currentSeed() {
		return parseSeed(this.battle.prng.getSeed());
	}

	/** Digest the messages Showdown sent since the last call. */
	ingest() {
		const events = this.events;
		this.events = [];
		const out = { requests: { p1: null, p2: null }, errors: { p1: null, p2: null } };
		for (const [type, data] of events) {
			if (type === 'update') {
				this.streamLines.push(...data);
			} else if (type === 'sideupdate') {
				const nl = data.indexOf('\n');
				const side = data.slice(0, nl);
				const payload = data.slice(nl + 1);
				if (payload.startsWith('|request|')) out.requests[side] = payload.slice('|request|'.length);
				else if (payload.startsWith('|error|')) out.errors[side] = payload.slice('|error|'.length);
				else throw new Error(`unexpected sideupdate for ${side}: ${payload.slice(0, 100)}`);
			} else if (type === 'end') {
				this.endMessage = JSON.parse(data);
			} else {
				throw new Error(`unexpected send type ${type}`);
			}
		}
		return out;
	}

	/**
	 * Decision-boundary state. Consumes the log lines appended since the previous snapshot.
	 * Requests are the JSON texts the server sent (null for both sides once the battle ended).
	 */
	snapshot() {
		const b = this.battle;
		const raw = b.log.slice(this.logCursor);
		this.logCursor = b.log.length;
		this.recorded.push(...raw);
		const stream = this.streamLines;
		this.streamLines = [];
		if (stream.length !== raw.length || stream.some((l, i) => l !== raw[i])) {
			this.anomalies.push(`update stream differs from battle.log delta (${stream.length} vs ${raw.length} lines)`);
		}
		const requests = { p1: null, p2: null };
		if (!b.ended) {
			b.sides.forEach((side, n) => {
				const id = SIDE_IDS[n];
				const text = this.boundaryRequests[id];
				if (text === null) throw new Error(`no request was sent to ${id}`);
				if (text !== JSON.stringify(side.activeRequest)) {
					this.anomalies.push(`${id}: sent request differs from side.activeRequest`);
				}
				requests[id] = JSON.parse(text);
			});
		}
		return {
			log: normalizeLog(raw),
			turn: b.turn,
			state: b.requestState,
			seed: this.currentSeed(),
			requests,
		};
	}

	/**
	 * `Battle.choose(side, input)` followed by sendUpdates(). Returns
	 * { ok, error, request }: for a rejection `error` is the `|error|` text and `request` the
	 * re-emitted request object if Showdown sent one (hidden-information rejections), else null.
	 */
	choose(side, input) {
		this.events = [];
		const ok = this.battle.choose(side, input);
		this.battle.sendUpdates();
		const d = this.ingest();
		if (ok) {
			if (d.errors.p1 !== null || d.errors.p2 !== null) this.anomalies.push(`error sent for accepted choice ${side}: ${input}`);
			this.noteBoundaryRequests(d);
			return { ok: true, error: null, request: null };
		}
		const errors = SIDE_IDS.filter(id => d.errors[id] !== null);
		if (errors.length !== 1 || errors[0] !== side) this.anomalies.push(`rejected ${side}: ${input} gave errors for [${errors}]`);
		for (const id of SIDE_IDS) {
			if (d.requests[id] !== null && id !== side) this.anomalies.push(`request sent to ${id} during rejection of ${side}`);
		}
		return {
			ok: false,
			error: d.errors[side],
			request: d.requests[side] === null ? null : JSON.parse(d.requests[side]),
		};
	}

	/** End-of-battle record; call right after the final snapshot() of an ended battle. */
	endRecord(finalSnapshot) {
		const b = this.battle;
		const names = b.sides.map(s => s.name);
		const winnerIdx = b.winner === '' ? -1 : names.indexOf(b.winner);
		if (b.winner !== '' && winnerIdx < 0) throw new Error(`unknown winner ${b.winner}`);
		if (this.recorded.length !== b.log.length || this.recorded.some((l, i) => l !== b.log[i])) {
			this.anomalies.push('battle.log was edited after lines were handed out');
		}
		return {
			winner: winnerIdx < 0 ? null : SIDE_IDS[winnerIdx],
			tie: winnerIdx < 0,
			turns: b.turn,
			seed: finalSnapshot.seed,
			pokemonLeft: b.sides.map(s => s.pokemonLeft),
			log: finalSnapshot.log,
		};
	}
}

// ---------------------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------------------

const MAX_PICK_ATTEMPTS = 25;
const MAX_STEPS = 4000;

/** Plays one battle with random legal choices and returns { fixture, stats, crash }. */
export function playBattle(sim, { runSeed, index, knobs = DEFAULT_KNOBS }) {
	const seeds = deriveSeeds(runSeed, index);
	const teams = seeds.teamSeeds.map(s => sim.Teams.pack(sim.Teams.getGenerator(FORMAT_ID, seedString(s)).getTeam()));
	const rng = new sim.PRNG(seedString(seeds.choiceSeed));
	const stats = newBattleStats();
	// Sets without a gender make the Pokemon constructor draw from the battle PRNG during setPlayer.
	stats.genderlessSets = teams.join(']').split(']').filter(mon => mon.split('|')[7] === '').length;
	const fixture = {
		v: FIXTURE_VERSION,
		format: FORMAT_ID,
		oracle: readOracleCommit(),
		runSeed,
		index,
		battleSeed: seeds.battleSeed,
		teamSeeds: seeds.teamSeeds,
		choiceSeed: seeds.choiceSeed,
		choicePolicy: { ...knobs },
		players: PLAYER_NAMES.map((name, n) => ({ id: SIDE_IDS[n], name })),
		teams,
		steps: [],
		end: null,
	};
	let session = null;
	try {
		session = new Session(sim, { seed: seeds.battleSeed, teams });
		for (;;) {
			const snap = session.snapshot();
			if (snap.log.length > stats.maxStepLines) {
				stats.maxStepLines = snap.log.length;
				stats.maxStepAt = fixture.steps.length; // index of the step this log delta leads up to ("end" if last)
			}
			stats.logLines += snap.log.length;
			if (session.ended) {
				fixture.end = session.endRecord(snap);
				break;
			}
			if (fixture.steps.length >= MAX_STEPS) throw new Error(`battle exceeded ${MAX_STEPS} decision steps`);
			const step = {
				log: snap.log, turn: snap.turn, state: snap.state, seed: snap.seed, requests: snap.requests,
				choices: [], submitted: { p1: null, p2: null },
			};
			if (fixture.steps.length === 0 && snap.seed.join() !== seeds.battleSeed.join()) stats.startupDraws++;
			for (const id of SIDE_IDS) for (const tag of requestFeatures(snap.requests[id])) bump(stats.features, tag);

			const acting = SIDE_IDS.filter(id => requestKind(snap.requests[id]) !== 'wait');
			if (acting.length === 2 && rng.random() < 0.5) acting.reverse();
			for (const id of acting) chooseForSide(session, rng, knobs, id, snap.requests[id], step, stats);
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
		return { fixture, stats, crash: null };
	} catch (err) {
		if (session && session.anomalies.length) fixture.anomalies = session.anomalies;
		return { fixture, stats, crash: { message: String(err && err.message), stack: String(err && err.stack) } };
	}
}

function chooseForSide(session, rng, knobs, id, request, step, stats) {
	let req = request;
	for (let attempt = 0; ; attempt++) {
		const input = attempt < MAX_PICK_ATTEMPTS ? pickChoice(req, rng, knobs).input : 'default';
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
		bump(stats.rejections, verdict.key);
		if (!verdict.explained) {
			stats.unexplained.push({ turn: step.turn, side: id, input, error: r.error, request: req });
		}
		if (input === 'default') throw new Error(`Side.autoChoose was rejected for ${id}: ${r.error}`);
		if (r.request) req = r.request;
	}
}

/**
 * A rejection is "explained" if it is a hidden-information rejection: Showdown answered with
 * `[Unavailable choice]`, re-sent the request, and the slot whose request changed carried the
 * matching maybeDisabled/maybeLocked (disabled move) or maybeTrapped (switch) flag.
 */
export function classifyRejection(reqBefore, r) {
	const m = /^\[(Unavailable|Invalid) choice\] (.*)$/s.exec(r.error || '');
	const type = m ? m[1] : '?';
	const msg = m ? m[2] : String(r.error);
	let reason = msg;
	let flags = null;
	if (/^Can't move: .+ is disabled$/.test(msg)) {
		reason = "Can't move: <move> is disabled";
		flags = ['maybeDisabled', 'maybeLocked'];
	} else if (/^Can't switch: The active Pok.mon is trapped$/.test(msg)) {
		reason = msg;
		flags = ['maybeTrapped'];
	}
	let explained = false;
	if (flags && type === 'Unavailable' && r.request && requestKind(reqBefore) === 'move' && r.request.active) {
		const before = hiddenFlags(reqBefore);
		explained = reqBefore.active.some((a, i) => (
			JSON.stringify(a) !== JSON.stringify(r.request.active[i]) && flags.some(f => before[f].includes(i))
		));
	}
	return { key: `[${type}] ${reason}${explained ? '' : ' (UNEXPLAINED)'}`, explained };
}

export function newBattleStats() {
	return {
		turns: 0, steps: 0, decisions: 0, choiceCalls: 0, logLines: 0, maxStepLines: 0, maxStepAt: -1, tie: false,
		teraChosen: 0, switchesChosen: 0, passesExplicit: 0, startupDraws: 0, genderlessSets: 0, anomalies: 0,
		rejections: {}, features: {}, unexplained: [],
	};
}

const bump = (map, key, n = 1) => { map[key] = (map[key] || 0) + n; };

// ---------------------------------------------------------------------------------------
// Replay / determinism check
// ---------------------------------------------------------------------------------------

/**
 * Replays a fixture from its recorded inputs in a fresh Battle and compares everything:
 * team regeneration, log deltas, PRNG seeds, requests (as JSON text, so key order counts),
 * accept/reject results, error texts, request updates and the end record. Returns a list of
 * differences (empty when identical).
 */
export function replayFixture(sim, fx) {
	const diffs = [];
	const note = (where, what, expected, actual) => {
		if (diffs.length < 25) diffs.push({ where, what, expected, actual });
	};
	const same = (where, what, a, b) => {
		const ja = JSON.stringify(a);
		const jb = JSON.stringify(b);
		if (ja === jb) return true;
		note(where, what, ja.length > 600 ? `${ja.slice(0, 600)}...` : ja, jb.length > 600 ? `${jb.slice(0, 600)}...` : jb);
		return false;
	};
	const sameLog = (where, a, b) => {
		if (a.length === b.length && a.every((l, i) => l === b[i])) return;
		let k = 0;
		while (k < a.length && k < b.length && a[k] === b[k]) k++;
		note(where, `log differs at line ${k} (expected ${a.length} lines, got ${b.length})`, a[k], b[k]);
	};

	if (fx.v !== FIXTURE_VERSION) return [{ where: 'header', what: `unknown fixture version ${fx.v}` }];
	const seeds = deriveSeeds(fx.runSeed, fx.index);
	same('header', 'derived seeds', { b: seeds.battleSeed, t: seeds.teamSeeds, c: seeds.choiceSeed },
		{ b: fx.battleSeed, t: fx.teamSeeds, c: fx.choiceSeed });
	const teams = fx.teamSeeds.map(s => sim.Teams.pack(sim.Teams.getGenerator(FORMAT_ID, seedString(s)).getTeam()));
	same('header', 'regenerated teams', fx.teams, teams);

	const session = new Session(sim, { seed: fx.battleSeed, teams: fx.teams, names: fx.players.map(p => p.name) });
	for (let k = 0; k < fx.steps.length; k++) {
		const step = fx.steps[k];
		const where = `step ${k}`;
		const snap = session.snapshot();
		if (session.ended) {
			note(where, 'battle ended early');
			return diffs;
		}
		sameLog(where, step.log, snap.log);
		same(where, 'turn', step.turn, snap.turn);
		same(where, 'request state', step.state, snap.state);
		same(where, 'prng seed', step.seed, snap.seed);
		same(where, 'requests', step.requests, snap.requests);
		for (const [c, ci] of step.choices.map((c, ci) => [c, ci])) {
			const r = session.choose(c.side, c.input);
			const w = `${where} choice ${ci} (${c.side}: ${c.input})`;
			same(w, 'ok', c.ok, r.ok);
			same(w, 'error', c.error ?? null, r.error);
			same(w, 'request update', c.request ?? null, r.request);
		}
		if (diffs.length) return diffs;
	}
	const snap = session.snapshot();
	if (!session.ended) {
		note('end', 'battle did not end');
		return diffs;
	}
	same('end', 'end record', fx.end, session.endRecord(snap));
	if (session.anomalies.length) note('end', 'anomalies', [], session.anomalies);
	return diffs;
}

/**
 * Independent cross-check through Showdown's real `BattleStream` (the exact code a server runs):
 * feeds `>start`, `>player`, and every recorded `>pN <choice>` line (rejected ones included, in
 * order), then compares the streamed `update` lines, the per-side `sideupdate` messages (requests and
 * `|error|` lines) and the final `end` message with the fixture. This proves that `Session`
 * (direct Battle API + sendUpdates after every input) behaves exactly like BattleStream.
 */
export function replayViaBattleStream(sim, fx) {
	const diffs = [];
	const stream = new sim.BattleStream();
	const take = () => stream.buf.splice(0);
	const updates = [];
	const side = { p1: [], p2: [] };
	let end = null;
	const ingest = () => {
		for (const msg of take()) {
			const nl = msg.indexOf('\n');
			const type = msg.slice(0, nl);
			const data = msg.slice(nl + 1);
			if (type === 'update') updates.push(data);
			else if (type === 'sideupdate') {
				const nl2 = data.indexOf('\n');
				side[data.slice(0, nl2)].push(data.slice(nl2 + 1));
			} else if (type === 'end') end = JSON.parse(data);
			else diffs.push({ where: 'stream', what: `unexpected message type ${type}: ${data.slice(0, 200)}` });
		}
	};
	const write = line => {
		stream.write(line);
		ingest();
	};
	write(`>start ${JSON.stringify({ formatid: fx.format, seed: seedString(fx.battleSeed) })}`);
	write(`>player p1 ${JSON.stringify({ name: fx.players[0].name, team: fx.teams[0] })}`);
	write(`>player p2 ${JSON.stringify({ name: fx.players[1].name, team: fx.teams[1] })}`);
	const expectSide = { p1: [], p2: [] };
	const expectLog = [];
	for (const step of fx.steps) {
		expectLog.push(...step.log);
		for (const id of SIDE_IDS) expectSide[id].push(`|request|${JSON.stringify(step.requests[id])}`);
		for (const c of step.choices) {
			write(`>${c.side} ${c.input}`);
			if (!c.ok) {
				expectSide[c.side].push(`|error|${c.error}`);
				if (c.request) expectSide[c.side].push(`|request|${JSON.stringify(c.request)}`);
			}
		}
	}
	expectLog.push(...fx.end.log);
	const gotLog = normalizeLog(updates.join('\n').split('\n'));
	const wantLog = expectLog.join('\n').split('\n');
	if (gotLog.length !== wantLog.length || gotLog.some((l, i) => l !== wantLog[i])) {
		let k = 0;
		while (k < gotLog.length && k < wantLog.length && gotLog[k] === wantLog[k]) k++;
		diffs.push({ where: 'stream', what: `update lines differ at ${k}`, expected: wantLog[k], actual: gotLog[k] });
	}
	for (const id of SIDE_IDS) {
		const a = expectSide[id];
		const b = side[id];
		if (a.length !== b.length || a.some((m, i) => m !== b[i])) {
			let k = 0;
			while (k < a.length && k < b.length && a[k] === b[k]) k++;
			diffs.push({ where: 'stream', what: `${id} sideupdates differ at message ${k}`, expected: String(a[k]).slice(0, 300), actual: String(b[k]).slice(0, 300) });
		}
	}
	if (!end) diffs.push({ where: 'stream', what: 'no end message' });
	else {
		const winner = end.winner === '' || end.winner === undefined ? null : SIDE_IDS[fx.players.findIndex(p => p.name === end.winner)];
		const got = { winner: winner ?? null, turns: end.turns, seed: end.seed, pokemonLeft: end.score };
		const want = { winner: fx.end.winner, turns: fx.end.turns, seed: seedString(fx.battleSeed), pokemonLeft: fx.end.pokemonLeft };
		if (JSON.stringify(got) !== JSON.stringify(want)) diffs.push({ where: 'stream', what: 'end message', expected: want, actual: got });
		// Showdown's own input log of the battle must contain our accepted choices, in commit order.
		const accepted = [];
		for (const step of fx.steps) for (const id of SIDE_IDS) if (step.submitted[id] !== null) accepted.push(id);
		const chosen = end.inputLog.filter(l => /^>p[12] /.test(l));
		if (chosen.length !== accepted.length) {
			diffs.push({ where: 'stream', what: 'inputLog choice count', expected: accepted.length, actual: chosen.length });
		}
	}
	return diffs;
}
