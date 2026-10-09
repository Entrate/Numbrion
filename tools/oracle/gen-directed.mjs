// Directed-corpus generator: plays battles between hand-built (constructed) teams on the pinned Showdown build and
// records standard fixtures (docs/design/FIXTURES.md + the additive key `profile`), through the same Session code
// as gen-fixtures.mjs. Profiles and content rules: docs/design/CORPUS.md.
//
// Usage:
//   node tools/oracle/gen-directed.mjs --profile SPEC --count N --out <file.jsonl[.gz]> [--seed S] [--threads K]
//        [--verify] [--start-index I] [--summary-json F] [--ps PATH]
//   node tools/oracle/gen-directed.mjs --verify-file <file.jsonl[.gz]> [--threads K] [--ps PATH]
//   node tools/oracle/gen-directed.mjs --list
//   node tools/oracle/gen-directed.mjs --selftest [--ps PATH]
//
//   SPEC: slice0 | slice1 | slice1x | slice2 | batch:<name> | effect:<kind:id | id>, each optionally +tera
//   --seed S      run seed (default: fixed per profile, see `--list`); battle i is a pure function of (profile, S, i)
//   --verify      after generating, verify every fixture (also what --verify-file does): the teams rebuilt from the profile
//                 and the recorded seeds equal the fixture's, they conform to the profile's allow-lists, and the fixture
//                 passes the oracle's determinism checks (session.mjs replayFixture + replayViaBattleStream)
//   --selftest    playDirected with the random generator's teams and the default policy reproduces playBattle byte for byte
//
// Exit status 2: a battle crashed, a rejection was not explained by hidden information, or a verification failed.
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';
import fs from 'node:fs';
import zlib from 'node:zlib';
import readline from 'node:readline';
import { once } from 'node:events';
import { DEFAULT_KNOBS } from './lib/choices.mjs';
import { FORMAT_ID, deriveSeeds, expandHome, loadSim, readOracleCommit, seedString } from './lib/common.mjs';
import { newBattleStats, playBattle } from './lib/session.mjs';
import { loadDirectedData } from './lib/directed-data.mjs';
import { INERT_FAMILIES, defaultRunSeed, listBatches, makeProfile } from './lib/directed-profiles.mjs';
import { buildTeams } from './lib/directed-teams.mjs';
import { playDirected } from './lib/directed-play.mjs';
import { verifyFixture } from './lib/directed-replay.mjs';

const DEFAULT_PS = '~/src/pokemon-showdown';
const toID = s => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');

// ---------------------------------------------------------------------------------------
// Conformance lint: the teams of a fixture against the allow-lists of its profile
// ---------------------------------------------------------------------------------------

const TERA_TYPES = new Set(['Normal', 'Fighting', 'Flying', 'Poison', 'Ground', 'Rock', 'Bug', 'Ghost', 'Steel', 'Fire', 'Water', 'Grass', 'Electric', 'Psychic', 'Ice', 'Dragon', 'Dark', 'Fairy', 'Stellar']);

function lintTeams(sim, fx, profile, data) {
	const problems = [];
	const abilityOwner = new Map(); // inert ability -> family
	for (const f of profile.families) for (const a of f.abilities) abilityOwner.set(a, f);
	const sets = fx.teams.map(t => sim.Teams.unpack(t));
	const allMoves = new Set();
	for (const team of sets) for (const s of team) for (const m of s.moves) allMoves.add(toID(m));
	sets.forEach((team, side) => {
		if (team.length !== 6) problems.push(`p${side + 1}: ${team.length} Pokemon`);
		const bases = new Set();
		for (const s of team) {
			const sid = toID(s.species);
			const sp = data.species.get(sid) || [...data.species.values()].find(x => x.formes.some(([n]) => toID(n) === sid));
			const tag = `p${side + 1} ${s.species}`;
			if (!sp) { problems.push(`${tag}: species not producible by the generator`); continue; }
			if (bases.has(sp.baseId)) problems.push(`${tag}: duplicate species (Species Clause)`);
			bases.add(sp.baseId);
			if (sp.speciesCondition && !profile.speciesAllow.has(sp.id)) problems.push(`${tag}: species with a species condition outside the world`);
			// Teams.pack omits level 100; Teams.unpack leaves the default undefined.
			if ((s.level ?? 100) !== sp.level) problems.push(`${tag}: level ${s.level ?? 100} != ${sp.level}`);
			if (!s.gender) problems.push(`${tag}: empty gender`);
			if (!TERA_TYPES.has(s.teraType)) problems.push(`${tag}: tera type ${s.teraType}`);
			if (!(s.moves.length >= 1 && s.moves.length <= 4) || new Set(s.moves).size !== s.moves.length) problems.push(`${tag}: moves ${s.moves}`);
			for (const m of s.moves) {
				const id = toID(m);
				if (!data.moves.has(id)) problems.push(`${tag}: move ${id} not in scope`);
				else if (!profile.worldMoves.has(id)) problems.push(`${tag}: move ${id} outside the profile`);
			}
			const ab = toID(s.ability);
			if (!data.abilities.has(ab)) problems.push(`${tag}: ability ${ab} not in scope`);
			else if (!profile.abilities.has(ab)) {
				const fam = abilityOwner.get(ab);
				if (!fam) problems.push(`${tag}: ability ${ab} outside the profile`);
				else for (const id of allMoves) if (fam.trigger(data.moves.get(id))) problems.push(`${tag}: inert ability ${ab} but ${id} triggers it`);
			}
			const it = toID(s.item);
			if (it) {
				if (!data.items.has(it)) problems.push(`${tag}: item ${it} not in scope`);
				else if (!profile.items.has(it) && !profile.fillerItems.has(it)) problems.push(`${tag}: item ${it} outside the profile`);
			}
			if (!sp.abilitySet.has(ab)) problems.push(`${tag}: ability ${ab} is not in the species' generator sets`);
		}
	});
	return problems;
}

function verifyOne(sim, data, fx) {
	const diffs = [];
	let profile;
	try {
		profile = makeProfile(fx.profile, data);
	} catch (err) {
		return [{ where: 'profile', what: String(err.message) }];
	}
	const seeds = deriveSeeds(fx.runSeed, fx.index);
	try {
		const built = buildTeams(sim, profile, data, { runSeed: fx.runSeed, index: fx.index, seeds });
		if (JSON.stringify(built.packed) !== JSON.stringify(fx.teams)) diffs.push({ where: 'teams', what: 'rebuilt teams differ from the fixture' });
	} catch (err) {
		diffs.push({ where: 'teams', what: `rebuild threw ${err.message}` });
	}
	for (const p of lintTeams(sim, fx, profile, data)) diffs.push({ where: 'lint', what: p });
	if (!diffs.length) {
		try {
			diffs.push(...verifyFixture(sim, fx));
		} catch (err) {
			diffs.push({ where: 'replay', what: `threw ${err && err.stack}` });
		}
	}
	return diffs;
}

// ---------------------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------------------

if (!isMainThread) {
	const { psPath, spec, runSeed } = workerData;
	const sim = loadSim(psPath);
	const data = loadDirectedData();
	const profile = spec ? makeProfile(spec, data) : null;
	parentPort.postMessage({ type: 'ready' });
	parentPort.on('message', msg => {
		if (msg.type === 'gen') {
			const t0 = performance.now();
			const r = playDirected(sim, profile, data, { runSeed, index: msg.index });
			const used = { species: [], moves: [], abilities: [], items: [] };
			if (r.built && r.built.sets) {
				for (const team of r.built.sets) {
					for (const s of team) {
						used.species.push(s.speciesId);
						used.moves.push(...s.moves);
						used.abilities.push(toID(s.ability));
						if (s.item) used.items.push(toID(s.item));
					}
				}
			}
			const base = {
				type: 'result', index: msg.index, ms: performance.now() - t0, stats: r.stats, used,
				notes: r.built ? r.built.notes : null, families: r.built ? r.built.active : [],
			};
			if (r.crash) parentPort.postMessage({ ...base, line: null, crashLine: JSON.stringify({ ...r.fixture, crash: r.crash }) });
			else parentPort.postMessage({ ...base, line: JSON.stringify(r.fixture) });
		} else if (msg.type === 'verify') {
			const t0 = performance.now();
			const fx = JSON.parse(msg.line);
			let diffs;
			try {
				diffs = verifyOne(sim, data, fx);
			} catch (err) {
				diffs = [{ where: 'verify', what: `threw ${err && err.stack}` }];
			}
			parentPort.postMessage({ type: 'verified', index: fx.index, diffs, ms: performance.now() - t0 });
		} else if (msg.type === 'exit') {
			process.exit(0);
		}
	});
}

// ---------------------------------------------------------------------------------------
// Main thread
// ---------------------------------------------------------------------------------------

function usage(msg) {
	if (msg) console.error(`gen-directed.mjs: ${msg}`);
	console.error('usage: gen-directed.mjs --profile SPEC --count N --out FILE [--seed S] [--threads K] [--verify] [--start-index I]\n' +
		'                        [--summary-json F] [--ps PATH]\n' +
		'       gen-directed.mjs --verify-file FILE [--threads K]     gen-directed.mjs --list     gen-directed.mjs --selftest');
	process.exit(1);
}

function parseArgs(argv) {
	const a = {
		ps: DEFAULT_PS, profile: null, count: 0, out: null, seed: null, threads: 1, verify: false, startIndex: 0, summaryJson: null,
		verifyFile: null, list: false, selftest: false,
	};
	for (let i = 0; i < argv.length; i++) {
		const x = argv[i];
		const num = () => { const v = Number(argv[++i]); if (!Number.isFinite(v)) usage(`${x} needs a number`); return v; };
		if (x === '--ps') a.ps = argv[++i];
		else if (x === '--profile') a.profile = argv[++i];
		else if (x === '--count') a.count = num();
		else if (x === '--out') a.out = argv[++i];
		else if (x === '--seed') a.seed = num();
		else if (x === '--threads') a.threads = num();
		else if (x === '--verify') a.verify = true;
		else if (x === '--start-index') a.startIndex = num();
		else if (x === '--summary-json') a.summaryJson = argv[++i];
		else if (x === '--verify-file') a.verifyFile = argv[++i];
		else if (x === '--list') a.list = true;
		else if (x === '--selftest') a.selftest = true;
		else usage(`unknown argument ${x}`);
	}
	if (a.threads > 4) usage('--threads is capped at 4 (shared machine)');
	if (!Number.isInteger(a.threads) || a.threads < 1) usage('--threads must be a positive integer');
	if (!Number.isInteger(a.count) || a.count < 0) usage('--count must be a non-negative integer');
	if (!Number.isInteger(a.startIndex) || a.startIndex < 0) usage('--start-index must be a non-negative integer');
	if (!a.list && !a.selftest && !a.verifyFile && !(a.profile && a.out && a.count > 0)) usage();
	return a;
}

function openSink(file) {
	const fileStream = fs.createWriteStream(file);
	if (!file.endsWith('.gz')) return { write: s => fileStream.write(s), end: () => { fileStream.end(); return once(fileStream, 'close'); } };
	const gz = zlib.createGzip({ level: 6 });
	gz.pipe(fileStream);
	return { write: s => gz.write(s), end: () => { gz.end(); return once(fileStream, 'close'); } };
}

function openLines(file) {
	let input = fs.createReadStream(file);
	if (file.endsWith('.gz')) input = input.pipe(zlib.createGunzip());
	return readline.createInterface({ input, crlfDelay: Infinity });
}

function spawnWorkers(workerArgs, n) {
	return Promise.all(Array.from({ length: n }, () => new Promise((resolve, reject) => {
		const w = new Worker(new URL(import.meta.url), { workerData: workerArgs });
		w.once('error', reject);
		w.once('message', m => (m.type === 'ready' ? resolve(w) : reject(new Error('bad worker handshake'))));
	})));
}

const bump = (map, key, n = 1) => { map[key] = (map[key] || 0) + n; };
const sortedEntries = map => Object.entries(map).sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));

async function generate(args, psPath, spec, runSeed) {
	const workers = await spawnWorkers({ psPath, spec, runSeed }, Math.min(args.threads, args.count));
	const sink = openSink(args.out);
	const agg = {
		battles: 0, crashes: 0, bytes: 0, workerMs: 0, sum: newBattleStats(), unexplained: [], unexplainedCount: 0, crashExamples: [],
		anomalyBattles: [], ties: 0, maxTurns: { turns: 0, index: -1 }, content: { species: {}, moves: {}, abilities: {}, items: {} },
		focus: {}, skipped: {}, families: {}, legalMoves: 0, looseMoves: 0, turnsList: [],
	};
	const crashSink = [];
	const results = new Map();
	let next = 0;
	let written = 0;
	const t0 = performance.now();
	let lastProgress = t0;
	await new Promise((resolve, reject) => {
		const dispatch = w => { if (next < args.count) w.postMessage({ type: 'gen', index: args.startIndex + next++ }); };
		const flush = () => {
			while (results.has(written)) {
				const r = results.get(written);
				results.delete(written);
				written++;
				if (r.line !== null) { sink.write(`${r.line}\n`); agg.bytes += r.line.length + 1; } else crashSink.push(r.crashLine);
			}
			if (written === args.count) resolve();
		};
		for (const w of workers) {
			w.on('error', reject);
			w.on('message', m => {
				if (m.type !== 'result') return;
				absorb(agg, m);
				results.set(m.index - args.startIndex, m);
				flush();
				dispatch(w);
				const now = performance.now();
				if (now - lastProgress > 5000) { lastProgress = now; console.error(`  ${agg.battles}/${args.count} battles, ${((now - t0) / 1000).toFixed(0)} s`); }
			});
			dispatch(w);
			dispatch(w);
		}
	});
	await sink.end();
	for (const w of workers) w.postMessage({ type: 'exit' });
	if (crashSink.length) fs.writeFileSync(`${args.out.replace(/\.gz$/, '').replace(/\.jsonl$/, '')}.crashes.jsonl`, `${crashSink.join('\n')}\n`);
	agg.wallMs = performance.now() - t0;
	return agg;
}

function absorb(agg, m) {
	const s = m.stats;
	agg.battles++;
	agg.workerMs += m.ms;
	const t = agg.sum;
	for (const k of ['turns', 'steps', 'decisions', 'choiceCalls', 'logLines', 'teraChosen', 'switchesChosen', 'passesExplicit', 'genderlessSets', 'anomalies']) t[k] += s[k];
	for (const [k, v] of Object.entries(s.rejections)) bump(t.rejections, k, v);
	for (const [k, v] of Object.entries(s.features)) bump(t.features, k, v);
	if (s.tie) agg.ties++;
	if (s.anomalies) agg.anomalyBattles.push(m.index);
	if (s.turns > agg.maxTurns.turns) agg.maxTurns = { turns: s.turns, index: m.index };
	for (const u of s.unexplained) if (agg.unexplained.length < 10) agg.unexplained.push({ battle: m.index, ...u });
	agg.unexplainedCount += s.unexplained.length;
	agg.turnsList.push(s.turns);
	for (const kind of Object.keys(agg.content)) for (const id of m.used[kind]) bump(agg.content[kind], id);
	if (m.notes) {
		for (const k of m.notes.focus) bump(agg.focus, k);
		for (const k of m.notes.skipped) bump(agg.skipped, k);
		agg.legalMoves += m.notes.legal;
		agg.looseMoves += m.notes.loose;
	}
	for (const f of m.families) bump(agg.families, f);
	if (m.line === null) {
		agg.crashes++;
		if (agg.crashExamples.length < 5) {
			const c = JSON.parse(m.crashLine);
			agg.crashExamples.push({ battle: m.index, message: c.crash.message, stack: c.crash.stack.split('\n').slice(0, 8).join('\n') });
		}
	}
}

async function verifyFile(file, threads, psPath) {
	const workers = await spawnWorkers({ psPath, spec: null, runSeed: 0 }, threads);
	const res = { checked: 0, failed: [], wallMs: 0 };
	const t0 = performance.now();
	const idle = [...workers];
	let inflight = 0;
	let wake = null;
	const settle = () => { if (wake) { const w = wake; wake = null; w(); } };
	for (const w of workers) {
		w.on('message', m => {
			if (m.type !== 'verified') return;
			res.checked++;
			if (m.diffs.length) res.failed.push({ index: m.index, diffs: m.diffs.slice(0, 5) });
			inflight--;
			idle.push(w);
			settle();
		});
	}
	for await (const line of openLines(file)) {
		if (!line.trim()) continue;
		while (!idle.length) await new Promise(r => { wake = r; });
		inflight++;
		idle.pop().postMessage({ type: 'verify', line });
	}
	while (inflight > 0) await new Promise(r => { wake = r; });
	for (const w of workers) w.postMessage({ type: 'exit' });
	res.wallMs = performance.now() - t0;
	return res;
}

function printSummary(args, spec, runSeed, profile, agg, ver) {
	const n = agg.battles;
	const ok = n - agg.crashes;
	const sec = agg.wallMs / 1000;
	const out = [];
	const p = s => out.push(s);
	const sorted = agg.turnsList.slice().sort((a, b) => a - b);
	p(`oracle ${readOracleCommit().slice(0, 7)}  profile ${spec}  seed ${runSeed}  battles ${args.startIndex}..${args.startIndex + n - 1}`);
	p(`  ${profile.description}`);
	p(`wall ${sec.toFixed(1)} s with ${Math.min(args.threads, n)} worker thread(s): ${(n / sec).toFixed(1)} battles/s`);
	p(`output ${args.out}: ${(agg.bytes / 1e6).toFixed(1)} MB uncompressed (${(agg.bytes / Math.max(1, ok) / 1024).toFixed(0)} KiB/battle)`);
	p(`battles ok ${ok}, crashed ${agg.crashes}, ties ${agg.ties}, sets without gender ${agg.sum.genderlessSets}`);
	p(`turns/battle avg ${(agg.sum.turns / n).toFixed(1)} (median ${sorted[Math.floor(n / 2)]}, max ${agg.maxTurns.turns} @${agg.maxTurns.index}), ` +
		`decision steps/battle ${(agg.sum.steps / n).toFixed(1)}, log lines/battle ${(agg.sum.logLines / n).toFixed(0)}`);
	p(`choices: tera ${agg.sum.teraChosen}, switches ${agg.sum.switchesChosen}`);
	const rejTotal = Object.values(agg.sum.rejections).reduce((a, b) => a + b, 0);
	p(`rejections: ${rejTotal}, unexplained ${agg.unexplainedCount}`);
	for (const [k, v] of sortedEntries(agg.sum.rejections)) p(`  ${String(v).padStart(7)}  ${k}`);
	p(`content: ${Object.keys(agg.content.species).length} species, ${Object.keys(agg.content.moves).length} moves, ` +
		`${Object.keys(agg.content.abilities).length} abilities, ${Object.keys(agg.content.items).length} items; ` +
		`moves legal for their species ${agg.legalMoves}, loosened ${agg.looseMoves}`);
	if (Object.keys(agg.families).length) p(`inert families used (battles): ${sortedEntries(agg.families).map(([k, v]) => `${k} ${v}`).join(', ')}`);
	if (profile.focus) {
		p(`focus effects placed: ${Object.keys(agg.focus).length}/${profile.focus.keys.length}; skipped placements ${Object.values(agg.skipped).reduce((a, b) => a + b, 0)}` +
			(profile.focus.unreachable.length ? `; unreachable: ${profile.focus.unreachable.join(' ')}` : ''));
	}
	for (const u of agg.unexplained) p(`  UNEXPLAINED battle ${u.battle} turn ${u.turn} ${u.side}: "${u.input}" -> ${u.error}`);
	if (agg.anomalyBattles.length) p(`ANOMALIES in battles: ${agg.anomalyBattles.slice(0, 20).join(', ')}`);
	for (const c of agg.crashExamples) p(`CRASH battle ${c.battle}: ${c.message}\n${c.stack}`);
	if (ver) {
		p(`verification: ${ver.checked} fixtures (rebuild + lint + replay + BattleStream) in ${(ver.wallMs / 1000).toFixed(1)} s, failures ${ver.failed.length}`);
		for (const f of ver.failed.slice(0, 5)) p(`  FAILED battle ${f.index}: ${JSON.stringify(f.diffs.slice(0, 3)).slice(0, 1200)}`);
	}
	console.log(out.join('\n'));
}

async function selftest(psPath) {
	const sim = loadSim(psPath);
	const data = loadDirectedData();
	const fake = { id: 'selftest', knobs: { ...DEFAULT_KNOBS }, focusMoves: new Set(), bias: { focusMoveWeight: 1, stallTurn: 40 } };
	let bad = 0;
	for (let index = 0; index < 8; index++) {
		const ref = playBattle(sim, { runSeed: 1, index, knobs: DEFAULT_KNOBS });
		const seeds = deriveSeeds(1, index);
		const teams = seeds.teamSeeds.map(s => sim.Teams.pack(sim.Teams.getGenerator(FORMAT_ID, seedString(s)).getTeam()));
		const mine = playDirected(sim, fake, data, { runSeed: 1, index, teams });
		const a = JSON.parse(JSON.stringify(ref.fixture));
		const b = JSON.parse(JSON.stringify(mine.fixture));
		delete b.profile;
		const same = JSON.stringify(a) === JSON.stringify(b);
		if (!same) bad++;
		console.log(`selftest index ${index}: playBattle == playDirected(default policy): ${same ? 'identical' : 'DIFFERENT'}`);
	}
	process.exit(bad ? 2 : 0);
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const psPath = expandHome(args.ps);
	if (args.list) {
		const data = loadDirectedData();
		const specs = ['slice0', 'slice1', 'slice1x', 'slice2', ...listBatches(data).map(b => `batch:${b}`)];
		for (const s of specs) {
			const p = makeProfile(s, data);
			console.log(`${s.padEnd(34)} seed ${String(defaultRunSeed(s, data)).padStart(5)}  moves ${String(p.worldMoves.size).padStart(3)} abilities ${String(p.abilities.size).padStart(3)} ` +
				`+inert families ${p.families.map(f => f.name).join('/') || '-'}  items ${p.items.size}` +
				(p.focus ? `  focus ${p.focus.keys.length}${p.focus.unreachable.length ? ` (unreachable ${p.focus.unreachable.length})` : ''}` : ''));
		}
		process.exit(0);
	}
	if (args.selftest) return selftest(psPath);
	loadSim(psPath);
	if (args.verifyFile) {
		const ver = await verifyFile(args.verifyFile, args.threads, psPath);
		console.log(`verification of ${args.verifyFile}: ${ver.checked} fixtures (rebuild + lint + replay + BattleStream) in ${(ver.wallMs / 1000).toFixed(1)} s, failures ${ver.failed.length}`);
		for (const f of ver.failed.slice(0, 5)) console.log(`  FAILED battle ${f.index}: ${JSON.stringify(f.diffs.slice(0, 3)).slice(0, 1200)}`);
		process.exit(ver.failed.length ? 2 : 0);
	}
	const data = loadDirectedData();
	const profile = makeProfile(args.profile, data);
	const spec = profile.id;
	const runSeed = args.seed ?? defaultRunSeed(args.profile, data);
	const agg = await generate(args, psPath, spec, runSeed);
	let ver = null;
	if (args.verify) ver = await verifyFile(args.out, args.threads, psPath);
	printSummary(args, spec, runSeed, profile, agg, ver);
	if (args.summaryJson) {
		const top = m => sortedEntries(m).map(([k, v]) => [k, v]);
		fs.writeFileSync(args.summaryJson, JSON.stringify({
			profile: spec, description: profile.description, runSeed, count: agg.battles, startIndex: args.startIndex, crashes: agg.crashes,
			turnsAvg: +(agg.sum.turns / agg.battles).toFixed(2), stepsAvg: +(agg.sum.steps / agg.battles).toFixed(2), uncompressedMB: +(agg.bytes / 1e6).toFixed(2),
			content: Object.fromEntries(Object.entries(agg.content).map(([k, v]) => [k, Object.fromEntries(top(v))])),
			focusPlaced: agg.focus, focusSkipped: agg.skipped, unreachable: profile.focus ? profile.focus.unreachable : [],
			families: agg.families, legalMoves: agg.legalMoves, looseMoves: agg.looseMoves,
			verification: ver ? { checked: ver.checked, failed: ver.failed.length } : null,
		}, null, 1));
	}
	const bad = agg.crashes || agg.unexplainedCount || agg.anomalyBattles.length || (ver && ver.failed.length);
	process.exit(bad ? 2 : 0);
}

if (isMainThread) await main();
