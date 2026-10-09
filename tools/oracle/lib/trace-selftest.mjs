// End-to-end test of the trace tools on one fixture battle:
//   node tools/oracle/lib/trace-selftest.mjs <path-to-pokemon-showdown> <fixtures.jsonl[.gz]> [--battle POSITION]
//
// 1. traces the battle at levels 1, 2, 3 (+ --skip-empty, + a turn window) and checks that every traced replay
//    still equals the fixture (log, seeds, requests, choices, end record), i.e. tracing does not change the battle;
// 2. runs trace-diff on a trace against itself, against other levels/projections of itself (no divergence);
// 3. perturbs a level-3 trace in 14 controlled ways and checks that trace-diff reports the right class at the
//    right record.
// Exit status 0 when everything passes, 1 otherwise.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadSim } from './common.mjs';
import { findFixture } from './trace-io.mjs';
import { traceFixture } from './trace-replay.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const diffCli = path.join(here, '..', 'trace-diff.mjs');

const argv = process.argv.slice(2);
let position = 0;
const positional = [];
for (let i = 0; i < argv.length; i++) {
	if (argv[i] === '--battle') position = Number(argv[++i]);
	else positional.push(argv[i]);
}
if (positional.length !== 2 || !Number.isInteger(position)) {
	console.error('usage: trace-selftest.mjs <path-to-pokemon-showdown> <fixtures.jsonl[.gz]> [--battle POSITION]');
	process.exit(2);
}
const [psPath, fixtures] = positional;
const found = await findFixture(fixtures, { position });
if (!found) {
	console.error(`no fixture at position ${position}`);
	process.exit(2);
}
const fx = JSON.parse(found.line);
const sim = loadSim(psPath);
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'trace-selftest-'));

let failures = 0;
const check = (name, ok, detail = '') => {
	if (!ok) failures++;
	console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? `  ${detail}` : ''}`);
};

const write = (name, lines) => {
	const file = path.join(dir, `${name}.jsonl`);
	fs.writeFileSync(file, lines.map(l => (typeof l === 'string' ? l : JSON.stringify(l))).join('\n') + '\n');
	return file;
};

const diff = (a, b, ...extra) => {
	const r = spawnSync('node', [diffCli, a, b, '--json', ...extra], { encoding: 'utf8' });
	let result = null;
	try {
		result = JSON.parse(r.stdout);
	} catch { /* leave null */ }
	return { status: r.status, result, stderr: r.stderr };
};

// ---- 1. tracing does not change the battle ------------------------------------------------
const traces = {};
for (const [name, opts] of [
	['L1', { level: 1 }], ['L2', { level: 2 }], ['L3', { level: 3 }], ['L3-skip', { level: 3, skipEmpty: true }],
	['L2-skip', { level: 2, skipEmpty: true }], ['L3-window', { level: 3, fromTurn: 3, toTurn: 4 }],
]) {
	const r = traceFixture(sim, fx, { ...opts, position });
	check(`${name}: traced replay equals the fixture`, r.diffs.length === 0, r.diffs.length ? JSON.stringify(r.diffs[0]).slice(0, 300) : `${r.records} records`);
	traces[name] = { ...r, file: write(name, r.lines) };
}
const parsed = traces.L3.lines.map(l => JSON.parse(l));
check('L3 has all kinds', ['hdr', 'boundary', 'choose', 'chosen', 'log', 'logedit', 'rng', 'ev', 'evx', 'sort', 'hc', 'hcx']
	.every(k => parsed.some(r => r.k === k)));
check('ev/evx are balanced', parsed.filter(r => r.k === 'ev').length === parsed.filter(r => r.k === 'evx').length);
check('hc/hcx are balanced', parsed.filter(r => r.k === 'hc').length === parsed.filter(r => r.k === 'hcx').length);
check('the traced PRNG chain is gapless', (() => {
	let last = null;
	for (const r of parsed) {
		if (r.k !== 'rng') continue;
		if (last && JSON.stringify(last) !== JSON.stringify(r.s0)) return false;
		last = r.s1;
	}
	return true;
})());
check('last rng seed equals the end seed', (() => {
	const rngs = parsed.filter(r => r.k === 'rng');
	return JSON.stringify(rngs[rngs.length - 1].s1) === JSON.stringify(fx.end.seed);
})());

// ---- 2. no divergence ---------------------------------------------------------------------
let d = diff(traces.L3.file, traces.L3.file);
check('trace against itself', d.status === 0 && d.result.identical, `${d.result && d.result.expectedRecords} records`);
d = diff(traces.L3.file, traces.L1.file);
check('level 3 against level 1 (compared at level 1)', d.status === 0 && d.result.level === 1);
d = diff(traces.L3.file, traces['L3-skip'].file, '--squash-empty');
check('full against --skip-empty with --squash-empty', d.status === 0);
d = diff(traces.L2.file, traces['L2-skip'].file, '--squash-empty');
check('level 2 full against --skip-empty with --squash-empty', d.status === 0);
d = diff(traces.L3.file, traces.L3.file, '--rng-raw', '--merge-logedits', '--skip-events', 'Update,ModifyBoost,ModifySpe');
check('self diff with --rng-raw --merge-logedits --skip-events', d.status === 0);
d = diff(traces.L3.file, traces['L3-skip'].file);
check('full against --skip-empty without squashing is a divergence', d.status === 1);

// ---- 3. perturbations ---------------------------------------------------------------------
const clone = () => structuredClone(parsed);
const find = (pred, n, recs) => {
	let c = 0;
	for (let i = 0; i < recs.length; i++) if (pred(recs[i]) && ++c === n) return i;
	throw new Error('no such record for a perturbation');
};
const cases = [
	['missing rng draw', 'missing RNG draw', r => { const i = find(x => x.k === 'rng', 3, r); r.splice(i, 1); return i; }],
	['extra rng draw', 'extra RNG draw', r => {
		const i = find(x => x.k === 'rng', 5, r);
		r.splice(i, 0, { k: 'rng', m: 'random', a: [100], r: 7, n: 1, s0: r[i].s0, s1: r[i].s1 });
		return i;
	}],
	['different rng call', 'different RNG call', r => { const i = find(x => x.k === 'rng' && x.m === 'randomChance', 2, r); r[i].a = [1, 2]; return i; }],
	['seed drift', 'PRNG state differs before the draw', r => { const i = find(x => x.k === 'rng', 4, r); r[i].s0 = [1, 2, 3, 4]; return i; }],
	['relay value out', 'different relay value out', r => { const i = find(x => x.k === 'evx' && typeof x.v === 'number' && x.e === 'ModifyDamage', 2, r); r[i].v += 1; return i; }],
	['relay value in', 'different relay value in', r => { const i = find(x => x.k === 'ev' && typeof x.v === 'number', 7, r); r[i].v += 1; return i; }],
	['handler order', 'different handler order', r => { const i = find(x => x.k === 'sort' && x.w === 'event' && x.it.length >= 2, 1, r); r[i].it.reverse(); return i; }],
	['handler ordering key', 'different handler ordering keys', r => { const i = find(x => x.k === 'sort' && x.w === 'event' && x.it.length >= 2, 3, r); r[i].it[0].sp += 5; return i; }],
	['handler missing', 'different handler list', r => { const i = find(x => x.k === 'sort' && x.w === 'event' && x.it.length >= 2, 2, r); r[i].it.pop(); return i; }],
	['handler return value', 'different handler return value', r => { const i = find(x => x.k === 'hcx' && x.r !== '@undefined', 3, r); r[i].r = 'bogus'; return i; }],
	['event target', 'different event call', r => { const i = find(x => x.k === 'ev' && x.f === 'single', 3, r); r[i].t = 'p9z'; return i; }],
	['extra event', 'extra event call', r => {
		const i = find(x => x.k === 'ev' && x.f === 'run', 20, r);
		r.splice(i, 0, { k: 'ev', d: 0, f: 'run', e: 'Bogus', t: 'battle', s: null, x: null, v: '@undefined' }, { k: 'evx', d: 0, e: 'Bogus', v: true });
		return i;
	}],
	['log line', 'different log line', r => { const i = find(x => x.k === 'log' && x.line.startsWith('|-damage|'), 4, r); r[i].line += '|[from] x'; return i; }],
	['log edit', 'different log edit', r => { const i = find(x => x.k === 'logedit', 2, r); r[i].line += '|[x]'; return i; }],
	['truncated', 'actual ends early', r => { const i = Math.floor(r.length / 2); r.length = i; return i; }],
];
for (const [name, cls, mutate] of cases) {
	const recs = clone();
	const at = mutate(recs);
	const file = write(`perturbed-${name.replace(/ /g, '-')}`, recs);
	const r = diff(traces.L3.file, file, '--context', '1');
	// hdr is not compared, so the first differing comparable record has index `at - 1`
	const ok = r.status === 1 && r.result && r.result.classification === cls && r.result.compared === at - 1;
	check(`diff finds: ${name}`, ok, ok ? `at record ${r.result.compared}` : `got ${r.result && r.result.classification} at ${r.result && r.result.compared}, wanted ${cls} at ${at - 1}`);
}
// the same perturbed relay value is invisible when only logs/rng are compared
{
	const recs = clone();
	const i = find(x => x.k === 'evx' && typeof x.v === 'number' && x.e === 'ModifyDamage', 2, recs);
	recs[i].v += 1;
	const file = write('perturbed-hidden', recs);
	d = diff(traces.L3.file, file, '--kinds', 'log,rng');
	check('--kinds log,rng ignores an event-only difference', d.status === 0);
}

fs.rmSync(dir, { recursive: true, force: true });
console.log(failures ? `\n${failures} check(s) FAILED` : '\nall checks passed');
process.exit(failures ? 1 : 0);
