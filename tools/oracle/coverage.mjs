// Handler coverage of fixture corpora: replays fixtures on the pinned Showdown build with every in-scope effect
// callback wrapped by a counter (tools/oracle/lib/coverage-instrument.mjs) and reports, per effect and per handler
// (`abilities:intimidate onStart`, `moves:direclaw secondary.onHit`), how often it was invoked and by how many battles.
//
// Usage:
//   node tools/oracle/coverage.mjs <fixtures.jsonl[.gz]>... [--ps PATH] [--threads K] [--json OUT.json]
//        [--batches] [--top N] [--limit N] [--quiet]
//        [--by-fixture[=OUT.jsonl[.gz]]] [--find KEY[,KEY...]]
//
//   --json OUT        full result: handlers, effects, never-invoked lists, per-batch table, declarative move usage
//   --batches         per-batch table in the text summary (default on; --no-batches to drop it)
//   --by-fixture[=F]  one JSON line per battle {file,pos,index,turns,steps,effects:[..],handlers:[..]} to F (stdout if
//                     F is missing); the compact way to pick small fixtures that exercise a given effect
//   --find KEYS       print the smallest battles (fewest turns) that invoke each KEY, an effect (`abilities:intimidate`)
//                     or a handler (`abilities:intimidate onStart`); `--top N` of them per key (default 5)
//   --limit N         only the first N battles of every input file
//   --ps PATH         Showdown checkout (default ~/src/pokemon-showdown)
//
// Every replay is the full session.mjs `replayFixture` check (log, PRNG seeds, requests, results), so a clean run
// also proves that the instrumentation does not change the battle. Directed fixtures (key `profile`) are replayed
// with their recorded packed teams (see lib/directed-replay.mjs). Exit status 2 when a replay differs.
//
// Handler counting is exact for function hooks (every call of the callback). Constant hooks (4 in the whole scope)
// are counted when the dispatcher resolves them ("collected"). Declarative effects (no callbacks) cannot be
// observed through handlers; for moves the log's `|move|` lines are counted instead.
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';
import fs from 'node:fs';
import zlib from 'node:zlib';
import readline from 'node:readline';
import { once } from 'node:events';
import { expandHome, readOracleCommit } from './lib/common.mjs';
import {
	buildUniverse, countsByLabel, instrument, loadBatches, loadScope,
} from './lib/coverage-instrument.mjs';
import { replayFixture, replaySim } from './lib/directed-replay.mjs';

const DEFAULT_PS = '~/src/pokemon-showdown';
const toID = s => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');

// ---------------------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------------------

function movesUsed(fx) {
	const out = {};
	const scan = log => {
		for (const l of log) {
			if (!l.startsWith('|move|')) continue;
			const p = l.split('|');
			const id = toID(p[3] || '');
			if (id) out[id] = (out[id] || 0) + 1;
		}
	};
	for (const s of fx.steps) scan(s.log);
	scan(fx.end.log);
	return out;
}

if (!isMainThread) {
	const { psPath } = workerData;
	const inst = instrument(psPath);
	parentPort.postMessage({ type: 'ready', wrappers: inst.wrapperCount() });
	parentPort.on('message', msg => {
		if (msg.type === 'cov') {
			const t0 = performance.now();
			const fx = JSON.parse(msg.line);
			inst.reset();
			let diffs;
			try {
				diffs = replayFixture(replaySim(inst.sim, fx, { always: true }), fx);
			} catch (err) {
				diffs = [{ where: 'replay', what: `threw ${err && err.stack}` }];
			}
			const snap = inst.snapshot();
			parentPort.postMessage({
				type: 'result', file: msg.file, pos: msg.pos, index: fx.index, turns: fx.end.turns, steps: fx.steps.length,
				profile: fx.profile ?? null, diffs, snap, moves: movesUsed(fx), ms: performance.now() - t0,
			});
		} else if (msg.type === 'exit') {
			process.exit(0);
		}
	});
}

// ---------------------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------------------

function usage(msg) {
	if (msg) console.error(`coverage.mjs: ${msg}`);
	console.error('usage: coverage.mjs <fixtures.jsonl[.gz]>... [--ps PATH] [--threads K] [--json OUT] [--no-batches] [--top N]\n' +
		'       [--limit N] [--by-fixture [OUT]] [--find KEY[,KEY...]] [--quiet]');
	process.exit(1);
}

function parseArgs(argv) {
	const a = {
		files: [], ps: DEFAULT_PS, threads: 2, json: null, batches: true, top: 5, limit: Infinity, byFixture: undefined,
		find: [], quiet: false,
	};
	for (let i = 0; i < argv.length; i++) {
		const x = argv[i];
		if (x === '--ps') a.ps = argv[++i];
		else if (x === '--threads') a.threads = Number(argv[++i]);
		else if (x === '--json') a.json = argv[++i];
		else if (x === '--batches') a.batches = true;
		else if (x === '--no-batches') a.batches = false;
		else if (x === '--top') a.top = Number(argv[++i]);
		else if (x === '--limit') a.limit = Number(argv[++i]);
		else if (x === '--quiet') a.quiet = true;
		else if (x === '--find') a.find.push(...argv[++i].split(',').map(s => s.trim()).filter(Boolean));
		else if (x === '--by-fixture') a.byFixture = '-'; // stdout; a file is given as --by-fixture=FILE
		else if (x.startsWith('--by-fixture=')) a.byFixture = x.slice('--by-fixture='.length);
		else if (x.startsWith('--')) usage(`unknown option ${x}`);
		else a.files.push(x);
	}
	if (!a.files.length) usage('no input files');
	if (!(a.threads >= 1) || !Number.isInteger(a.threads)) usage('--threads must be a positive integer');
	return a;
}

function openLines(file) {
	let input = fs.createReadStream(file);
	if (file.endsWith('.gz')) input = input.pipe(zlib.createGunzip());
	return readline.createInterface({ input, crlfDelay: Infinity });
}

function openSink(file) {
	if (file === '-') return { write: s => process.stdout.write(s), end: async () => {} };
	const fileStream = fs.createWriteStream(file);
	if (!file.endsWith('.gz')) return { write: s => fileStream.write(s), end: () => { fileStream.end(); return once(fileStream, 'close'); } };
	const gz = zlib.createGzip({ level: 6 });
	gz.pipe(fileStream);
	return { write: s => gz.write(s), end: () => { gz.end(); return once(fileStream, 'close'); } };
}

async function main() {
	const args = parseArgs(process.argv.slice(2));
	const psPath = expandHome(args.ps);
	const scope = loadScope();
	const batches = loadBatches();
	const inst = instrument(psPath, scope);
	const universe = buildUniverse(inst, scope);
	const hooks = universe.hooks;
	const fnHooks = hooks.filter(h => h.kind === 'fn');
	const constHooks = hooks.filter(h => h.kind === 'const');
	const uninstrumented = fnHooks.filter(h => h.cid === null).map(h => h.label);
	if (uninstrumented.length) console.error(`WARNING: ${uninstrumented.length} function hooks are not instrumented: ${uninstrumented.slice(0, 10).join(', ')}`);

	// --- workers
	const workers = await Promise.all(Array.from({ length: args.threads }, () => new Promise((resolve, reject) => {
		const w = new Worker(new URL(import.meta.url), { workerData: { psPath } });
		w.once('error', reject);
		w.once('message', m => {
			if (m.type !== 'ready') return reject(new Error('bad worker handshake'));
			if (m.wrappers !== inst.wrapperCount()) return reject(new Error(`worker wrapped ${m.wrappers} callbacks, main ${inst.wrapperCount()}`));
			resolve(w);
		});
	})));

	const total = new Map(); // label -> { calls, battles }
	const moveUses = {};
	const failures = [];
	const perBattle = []; // { file, pos, index, turns, steps, labels: [] }
	const sink = args.byFixture !== undefined ? openSink(args.byFixture) : null;
	const byFixtureOrder = new Map();
	let battles = 0;
	const t0 = performance.now();
	let lastProgress = t0;
	const idle = [...workers];
	let inflight = 0;
	let wake = null;
	const settle = () => { if (wake) { const w = wake; wake = null; w(); } };
	const perFile = [];

	for (const w of workers) {
		w.on('error', err => { console.error(err); process.exit(1); });
		w.on('message', m => {
			if (m.type !== 'result') return;
			battles++;
			if (m.diffs.length) failures.push({ file: m.file, pos: m.pos, index: m.index, diffs: m.diffs.slice(0, 3) });
			const counts = countsByLabel(universe, m.snap);
			for (const [label, n] of counts) {
				const t = total.get(label) || { calls: 0, battles: 0 };
				t.calls += n;
				t.battles++;
				total.set(label, t);
			}
			for (const [id, n] of Object.entries(m.moves)) moveUses[id] = (moveUses[id] || 0) + n;
			const labels = [...counts.keys()];
			const rec = { file: m.file, pos: m.pos, index: m.index, turns: m.turns, steps: m.steps, labels };
			if (args.find.length) perBattle.push(rec);
			if (sink) byFixtureOrder.set(`${m.file}\u0000${m.pos}`, rec);
			inflight--;
			idle.push(w);
			if (!args.quiet && performance.now() - lastProgress > 5000) {
				lastProgress = performance.now();
				console.error(`  ${battles} battles, ${((lastProgress - t0) / 1000).toFixed(0)} s`);
			}
			settle();
		});
	}

	for (const file of args.files) {
		let pos = 0;
		for await (const line of openLines(file)) {
			if (!line.trim()) continue;
			if (pos >= args.limit) break;
			while (!idle.length) await new Promise(r => { wake = r; });
			inflight++;
			idle.pop().postMessage({ type: 'cov', file, pos, line });
			pos++;
		}
		perFile.push({ file, battles: pos });
	}
	while (inflight > 0) await new Promise(r => { wake = r; });
	for (const w of workers) w.postMessage({ type: 'exit' });

	if (sink) {
		const recs = [...byFixtureOrder.values()].sort((a, b) => (a.file < b.file ? -1 : a.file > b.file ? 1 : a.pos - b.pos));
		for (const r of recs) {
			const effects = [...new Set(r.labels.map(l => l.slice(0, l.indexOf(' '))))];
			sink.write(`${JSON.stringify({ file: r.file, pos: r.pos, index: r.index, turns: r.turns, steps: r.steps, effects, handlers: r.labels })}\n`);
		}
		await sink.end();
	}

	// --- aggregate
	const effectsOf = new Map(); // effect -> hooks
	for (const h of hooks) {
		if (!effectsOf.has(h.effect)) effectsOf.set(h.effect, []);
		effectsOf.get(h.effect).push(h);
	}
	const effectRows = {};
	const neverEffects = [];
	for (const [effect, hs] of effectsOf) {
		const fns = hs.filter(h => h.kind === 'fn' || h.kind === 'const');
		if (!fns.length) continue;
		let calls = 0;
		let invoked = 0;
		for (const h of fns) {
			const t = total.get(h.label);
			if (t) { calls += t.calls; invoked++; }
		}
		effectRows[effect] = { hooks: fns.length, invoked, calls };
		if (!invoked) neverEffects.push(effect);
	}
	const handlers = {};
	const never = [];
	for (const h of [...fnHooks, ...constHooks]) {
		const t = total.get(h.label);
		if (t) handlers[h.label] = { kind: h.kind, calls: t.calls, battles: t.battles };
		else never.push(h.label);
	}
	const batchRows = batches.groups.map(g => {
		const keys = new Set(g.keys);
		const fns = fnHooks.filter(h => keys.has(h.effect));
		const consts = constHooks.filter(h => keys.has(h.effect));
		const missing = fns.filter(h => !total.has(h.label)).map(h => h.label);
		const missingConst = consts.filter(h => !total.has(h.label)).map(h => h.label);
		const effMissing = g.keys.filter(k => neverEffects.includes(k));
		return {
			batch: g.batch, effects: g.effects, handlers: fns.length, invoked: fns.length - missing.length, missing,
			constants: consts.length, constantsInvoked: consts.length - missingConst.length, missingConstants: missingConst,
			effectsNeverInvoked: effMissing,
		};
	});
	const hookedMoves = new Set(hooks.filter(h => h.effect.startsWith('moves:')).map(h => h.effect.slice(6)));
	const declMoves = scope.moves.filter(m => !hookedMoves.has(m.id)).map(m => m.id);
	const declUsed = declMoves.filter(id => moveUses[id]);
	const result = {
		tool: 'tools/oracle/coverage.mjs', oracle: readOracleCommit(), inputs: perFile, battles, wallSeconds: +((performance.now() - t0) / 1000).toFixed(1),
		replayFailures: failures,
		universe: { functionHooks: fnHooks.length, constantHooks: constHooks.length, orderingOnly: hooks.length - fnHooks.length - constHooks.length, uninstrumented },
		invokedHandlers: Object.keys(handlers).length,
		handlers, neverInvokedHandlers: never,
		effects: effectRows, neverInvokedEffects: neverEffects,
		batches: batchRows,
		declarativeMoves: { total: declMoves.length, used: declUsed.length, usage: Object.fromEntries(declUsed.map(id => [id, moveUses[id]])), neverUsed: declMoves.filter(id => !moveUses[id]) },
	};
	if (args.json) fs.writeFileSync(args.json, JSON.stringify(result, null, 1));

	// --- text summary (stderr when per-fixture JSON goes to stdout)
	const out = args.byFixture === '-' ? console.error : console.log;
	out(`coverage: ${battles} battles from ${perFile.map(f => `${f.file} (${f.battles})`).join(', ')}  ${result.wallSeconds}s with ${args.threads} thread(s)`);
	out(`replay check: ${failures.length ? `${failures.length} FAILED` : 'all battles identical to their fixture'}`);
	for (const f of failures.slice(0, 5)) out(`  ${f.file} #${f.pos} (index ${f.index}): ${JSON.stringify(f.diffs).slice(0, 600)}`);
	out(`function handlers invoked: ${fnHooks.filter(h => total.has(h.label)).length}/${fnHooks.length}   ` +
		`constant hooks collected: ${constHooks.filter(h => total.has(h.label)).length}/${constHooks.length}   ` +
		`effects with callbacks invoked: ${Object.keys(effectRows).length - neverEffects.length}/${Object.keys(effectRows).length}`);
	out(`declarative moves (no callbacks) used in a battle: ${declUsed.length}/${declMoves.length}`);
	if (args.batches) {
		out('');
		out(`${'batch'.padEnd(26)} ${'handlers'.padStart(8)} ${'invoked'.padStart(8)} ${'missing'.padStart(8)}  consts  never-invoked effects`);
		for (const b of batchRows) {
			out(`${b.batch.padEnd(26)} ${String(b.handlers).padStart(8)} ${String(b.invoked).padStart(8)} ${String(b.handlers - b.invoked).padStart(8)}  ${b.constantsInvoked}/${b.constants}`.padEnd(66) +
				` ${b.effectsNeverInvoked.length}`);
		}
	}
	if (neverEffects.length) {
		out('');
		out(`effects never invoked (${neverEffects.length}):`);
		const by = {};
		for (const e of neverEffects) (by[e.slice(0, e.indexOf(':'))] ||= []).push(e.slice(e.indexOf(':') + 1));
		for (const [k, v] of Object.entries(by)) out(`  ${k}: ${v.join(' ')}`);
	}
	if (args.find.length) {
		out('');
		for (const key of args.find) {
			const hits = perBattle.filter(r => r.labels.some(l => l === key || l.startsWith(`${key} `)))
				.sort((a, b) => a.turns - b.turns || a.pos - b.pos).slice(0, args.top);
			out(`${key}: ${perBattle.filter(r => r.labels.some(l => l === key || l.startsWith(`${key} `))).length} battles; smallest:`);
			for (const r of hits) out(`  ${r.file} position ${r.pos} (index ${r.index}): ${r.turns} turns, ${r.steps} steps`);
		}
	}
	process.exit(failures.length ? 2 : 0);
}

if (isMainThread) await main();
