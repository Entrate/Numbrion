// Differential-testing oracle: plays random-legal-choice battles of [Gen 9] Random Doubles Battle
// on the pinned Pokemon Showdown build and records ground-truth fixtures (JSON lines).
//
// Usage:
//   node tools/oracle/gen-fixtures.mjs <path-to-pokemon-showdown> --count N --seed S --out <file.jsonl[.gz]>
//        [--threads K] [--verify-replay FRACTION] [--start-index I]
//        [--switch-prob P] [--tera-prob P] [--implicit-pass-prob P] [--revive-pass-prob P]
//        [--summary-json <file>]
//   node tools/oracle/gen-fixtures.mjs <path-to-pokemon-showdown> --verify-file <file.jsonl[.gz]> [--verify-replay FRACTION] [--threads K]
//        (only replay-verifies an existing file; FRACTION defaults to 1)
//
// Fixture format: docs/design/FIXTURES.md. Everything is a pure function of (S, battle index), so the
// output is byte-identical for any --threads. Exit status 2 if any battle crashed Showdown, any
// rejection was not explained by hidden information, or any replay check failed.
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';
import fs from 'node:fs';
import zlib from 'node:zlib';
import readline from 'node:readline';
import { once } from 'node:events';
import { DEFAULT_KNOBS } from './lib/choices.mjs';
import { loadSim, readOracleCommit, selectForVerify } from './lib/common.mjs';
import { newBattleStats, playBattle, replayFixture, replayViaBattleStream } from './lib/session.mjs';

// ---------------------------------------------------------------------------------------
// Worker side
// ---------------------------------------------------------------------------------------

if (!isMainThread) {
	const { psPath, runSeed, knobs } = workerData;
	const sim = loadSim(psPath);
	parentPort.postMessage({ type: 'ready' });
	parentPort.on('message', msg => {
		if (msg.type === 'gen') {
			const t0 = performance.now();
			const r = playBattle(sim, { runSeed, index: msg.index, knobs });
			const ms = performance.now() - t0;
			if (r.crash) {
				parentPort.postMessage({
					type: 'result', index: msg.index, line: null, ms, stats: r.stats,
					crashLine: JSON.stringify({ ...r.fixture, crash: r.crash }),
				});
			} else {
				parentPort.postMessage({ type: 'result', index: msg.index, line: JSON.stringify(r.fixture), ms, stats: r.stats });
			}
		} else if (msg.type === 'verify') {
			const t0 = performance.now();
			const fx = JSON.parse(msg.line);
			let diffs;
			try {
				diffs = replayFixture(sim, fx);
				if (!diffs.length) diffs = replayViaBattleStream(sim, fx);
			} catch (err) {
				diffs = [{ where: 'replay', what: `threw ${err && err.stack}` }];
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

function parseArgs(argv) {
	const args = {
		psPath: null, count: 0, seed: 1, out: null, threads: 1, verifyReplay: 0, startIndex: 0, summaryJson: null, verifyFile: null,
		knobs: { ...DEFAULT_KNOBS },
	};
	const numeric = {
		'--count': v => { args.count = v; },
		'--seed': v => { args.seed = v; },
		'--threads': v => { args.threads = v; },
		'--verify-replay': v => { args.verifyReplay = v; },
		'--start-index': v => { args.startIndex = v; },
		'--switch-prob': v => { args.knobs.switchProb = v; },
		'--tera-prob': v => { args.knobs.teraProb = v; },
		'--implicit-pass-prob': v => { args.knobs.implicitPassProb = v; },
		'--revive-pass-prob': v => { args.knobs.revivePassProb = v; },
	};
	for (let i = 0; i < argv.length; i++) {
		const a = argv[i];
		if (numeric[a]) {
			const v = Number(argv[++i]);
			if (!Number.isFinite(v)) throw new Error(`${a} needs a number`);
			numeric[a](v);
		} else if (a === '--out') {
			args.out = argv[++i];
		} else if (a === '--summary-json') {
			args.summaryJson = argv[++i];
		} else if (a === '--verify-file') {
			args.verifyFile = argv[++i];
		} else if (a.startsWith('--')) {
			throw new Error(`unknown option ${a}`);
		} else if (!args.psPath) {
			args.psPath = a;
		} else {
			throw new Error(`unexpected argument ${a}`);
		}
	}
	if (args.verifyFile) {
		args.out = args.verifyFile;
		if (!args.verifyReplay) args.verifyReplay = 1;
	} else if (!args.psPath || !args.out || !(args.count > 0)) {
		throw new Error('usage: gen-fixtures.mjs <path-to-pokemon-showdown> --count N --seed S --out <file.jsonl> ' +
			'[--threads K] [--verify-replay FRACTION]   or   --verify-file <file> [--verify-replay FRACTION]');
	}
	if (!args.psPath) throw new Error('missing path to the pokemon-showdown checkout');
	if (!Number.isInteger(args.seed) || args.seed < 0) throw new Error('--seed must be a non-negative integer');
	if (!Number.isInteger(args.threads) || args.threads < 1) throw new Error('--threads must be >= 1');
	return args;
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

function spawnWorkers(args, n) {
	return Promise.all(Array.from({ length: n }, () => new Promise((resolve, reject) => {
		const w = new Worker(new URL(import.meta.url), {
			workerData: { psPath: args.psPath, runSeed: args.seed, knobs: args.knobs },
		});
		w.once('error', reject);
		w.once('message', m => (m.type === 'ready' ? resolve(w) : reject(new Error('bad worker handshake'))));
	})));
}

const bump = (map, key, n = 1) => { map[key] = (map[key] || 0) + n; };
const sortedEntries = map => Object.entries(map).sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));

async function generate(args) {
	const workers = await spawnWorkers(args, Math.min(args.threads, args.count));
	const sink = openSink(args.out);
	const crashSink = [];
	const agg = {
		battles: 0, crashes: 0, bytes: 0, workerMs: 0,
		sum: newBattleStats(), maxStep: { lines: 0, index: -1, step: -1 }, maxSteps: { steps: 0, index: -1 },
		maxTurns: { turns: 0, index: -1 }, startupDrawBattles: 0, ties: 0,
		unexplained: [], unexplainedCount: 0, crashExamples: [], anomalyBattles: [],
	};
	const results = new Map();
	let next = 0; // next offset to dispatch
	let written = 0; // offset of the next result to write
	const t0 = performance.now();
	let lastProgress = t0;

	await new Promise((resolve, reject) => {
		const dispatch = w => {
			if (next < args.count) w.postMessage({ type: 'gen', index: args.startIndex + next++ });
		};
		const flush = () => {
			while (results.has(written)) {
				const r = results.get(written);
				results.delete(written);
				written++;
				if (r.line !== null) {
					sink.write(r.line + '\n');
					agg.bytes += r.line.length + 1;
				} else {
					crashSink.push(r.crashLine);
				}
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
				if (now - lastProgress > 5000) {
					lastProgress = now;
					console.error(`  ${agg.battles}/${args.count} battles, ${((now - t0) / 1000).toFixed(0)} s`);
				}
			});
			dispatch(w);
			dispatch(w); // keep one extra battle queued per worker
		}
	});
	await sink.end();
	for (const w of workers) w.postMessage({ type: 'exit' });
	if (crashSink.length) {
		fs.writeFileSync(`${args.out.replace(/\.gz$/, '').replace(/\.jsonl$/, '')}.crashes.jsonl`, crashSink.join('\n') + '\n');
	}
	agg.wallMs = performance.now() - t0;
	return agg;
}

function absorb(agg, m) {
	const s = m.stats;
	agg.battles++;
	agg.workerMs += m.ms;
	const t = agg.sum;
	for (const k of ['turns', 'steps', 'decisions', 'choiceCalls', 'logLines', 'teraChosen', 'switchesChosen', 'passesExplicit', 'genderlessSets', 'anomalies']) {
		t[k] += s[k];
	}
	for (const [k, v] of Object.entries(s.rejections)) bump(t.rejections, k, v);
	for (const [k, v] of Object.entries(s.features)) bump(t.features, k, v);
	if (s.startupDraws) agg.startupDrawBattles++;
	if (s.tie) agg.ties++;
	if (s.anomalies) agg.anomalyBattles.push(m.index);
	if (s.maxStepLines > agg.maxStep.lines) agg.maxStep = { lines: s.maxStepLines, index: m.index, step: s.maxStepAt };
	if (s.steps > agg.maxSteps.steps) agg.maxSteps = { steps: s.steps, index: m.index };
	if (s.turns > agg.maxTurns.turns) agg.maxTurns = { turns: s.turns, index: m.index };
	for (const u of s.unexplained) if (agg.unexplained.length < 10) agg.unexplained.push({ battle: m.index, ...u });
	agg.unexplainedCount += s.unexplained.length;
	if (m.line === null) {
		agg.crashes++;
		if (agg.crashExamples.length < 5) {
			const c = JSON.parse(m.crashLine);
			agg.crashExamples.push({ battle: m.index, message: c.crash.message, stack: c.crash.stack.split('\n').slice(0, 8).join('\n') });
		}
	}
}

async function verify(args) {
	const workers = await spawnWorkers(args, args.threads);
	const res = { checked: 0, failed: [], ms: 0, wallMs: 0 };
	const t0 = performance.now();
	const lines = openLines(args.out);
	const idle = [...workers];
	let inflight = 0;
	let wake = null;
	const settle = () => { if (wake) { const w = wake; wake = null; w(); } };
	for (const w of workers) {
		w.on('message', m => {
			if (m.type !== 'verified') return;
			res.checked++;
			res.ms += m.ms;
			if (m.diffs.length) res.failed.push({ index: m.index, diffs: m.diffs });
			inflight--;
			idle.push(w);
			settle();
		});
	}
	for await (const line of lines) {
		const head = line.slice(0, 400);
		const idx = Number(/"index":(\d+)/.exec(head)[1]);
		const runSeed = Number(/"runSeed":(\d+)/.exec(head)[1]);
		if (!selectForVerify(runSeed, idx, args.verifyReplay)) continue;
		while (!idle.length) await new Promise(r => { wake = r; });
		inflight++;
		idle.pop().postMessage({ type: 'verify', line });
	}
	while (inflight > 0) await new Promise(r => { wake = r; });
	for (const w of workers) w.postMessage({ type: 'exit' });
	res.wallMs = performance.now() - t0;
	return res;
}

function printSummary(args, agg, ver) {
	const n = agg.battles;
	const ok = n - agg.crashes;
	const sec = agg.wallMs / 1000;
	const out = [];
	const p = s => out.push(s);
	p(`oracle ${readOracleCommit().slice(0, 7)}  format gen9randomdoublesbattle  seed ${args.seed}  battles ${args.startIndex}..${args.startIndex + n - 1}`);
	p(`wall ${sec.toFixed(1)} s with ${Math.min(args.threads, n)} worker thread(s): ${(n / sec).toFixed(1)} battles/s, ` +
		`${(agg.sum.steps / sec).toFixed(0)} decision steps/s (sim+JSON, worker busy ${(agg.workerMs / 1000).toFixed(1)} s)`);
	p(`output ${args.out}: ${(agg.bytes / 1e6).toFixed(1)} MB uncompressed (${(agg.bytes / Math.max(1, ok) / 1024).toFixed(0)} KiB/battle)`);
	p(`battles ok ${ok}, crashed ${agg.crashes}, ties ${agg.ties}, battles whose PRNG moved during setup/start ${agg.startupDrawBattles}, ` +
		`sets without gender (would draw in setPlayer) ${agg.sum.genderlessSets}`);
	p(`turns/battle avg ${(agg.sum.turns / n).toFixed(1)} (max ${agg.maxTurns.turns} @${agg.maxTurns.index}), ` +
		`decision steps/battle avg ${(agg.sum.steps / n).toFixed(1)} (max ${agg.maxSteps.steps} @${agg.maxSteps.index}), ` +
		`choose() calls/battle ${(agg.sum.choiceCalls / n).toFixed(1)}, accepted side choices/battle ${(agg.sum.decisions / n).toFixed(1)}`);
	p(`log lines/battle avg ${(agg.sum.logLines / n).toFixed(0)}; largest single step: ${agg.maxStep.lines} lines (battle ${agg.maxStep.index}, step ${agg.maxStep.step})`);
	p(`choices: tera ${agg.sum.teraChosen}, switches ${agg.sum.switchesChosen}, explicit passes ${agg.sum.passesExplicit}`);
	const rejTotal = Object.values(agg.sum.rejections).reduce((a, b) => a + b, 0);
	p(`rejections: ${rejTotal} (${(100 * rejTotal / Math.max(1, agg.sum.choiceCalls)).toFixed(2)}% of choose() calls), unexplained ${agg.unexplainedCount}`);
	for (const [k, v] of sortedEntries(agg.sum.rejections)) p(`  ${String(v).padStart(7)}  ${k}`);
	for (const u of agg.unexplained) {
		p(`  UNEXPLAINED battle ${u.battle} turn ${u.turn} ${u.side}: "${u.input}" -> ${u.error}`);
		p(`    request: ${JSON.stringify(u.request).slice(0, 1200)}`);
	}
	if (agg.anomalyBattles.length) p(`ANOMALIES in battles: ${agg.anomalyBattles.slice(0, 20).join(', ')} (see fixture "anomalies")`);
	for (const c of agg.crashExamples) p(`CRASH battle ${c.battle}: ${c.message}\n${c.stack}`);
	p('request/choice coverage (boundary-side occurrences):');
	for (const [k, v] of sortedEntries(agg.sum.features)) p(`  ${String(v).padStart(7)}  ${k}`);
	if (ver) {
		p(`replay verification: ${ver.checked} fixtures replayed in fresh worker modules (${(ver.wallMs / 1000).toFixed(1)} s), failures ${ver.failed.length}`);
		for (const f of ver.failed.slice(0, 5)) p(`  REPLAY MISMATCH battle ${f.index}: ${JSON.stringify(f.diffs.slice(0, 3)).slice(0, 1500)}`);
	}
	console.log(out.join('\n'));
}

if (isMainThread) {
	const args = parseArgs(process.argv.slice(2));
	loadSim(args.psPath); // fail fast on a wrong checkout
	const agg = args.verifyFile ? null : await generate(args);
	let ver = null;
	if (args.verifyReplay > 0) ver = await verify(args);
	if (agg) printSummary(args, agg, ver);
	else {
		console.log(`replay verification of ${args.verifyFile}: ${ver.checked} fixtures replayed in fresh worker modules ` +
			`(${(ver.wallMs / 1000).toFixed(1)} s), failures ${ver.failed.length}`);
		for (const f of ver.failed.slice(0, 5)) console.log(`  REPLAY MISMATCH battle ${f.index}: ${JSON.stringify(f.diffs.slice(0, 3)).slice(0, 1500)}`);
	}
	if (args.summaryJson) {
		fs.writeFileSync(args.summaryJson, JSON.stringify({ args, agg, ver }, null, 1));
	}
	const bad = (agg && (agg.crashes || agg.unexplainedCount || agg.anomalyBattles.length)) || (ver && ver.failed.length);
	process.exit(bad ? 2 : 0);
}
