// Aligns two traces (docs/design/TRACE.md) and prints the first divergence with context and a short
// classification: extra / missing RNG draw, different handler order, different relay value, different log
// line, ...
//
// Usage:
//   node tools/oracle/trace-diff.mjs expected.jsonl actual.jsonl [--context N] [--level 1|2|3] [--kinds k1,k2]
//        [--skip-kinds k1,k2] [--skip-events E1,E2] [--squash-empty] [--merge-logedits] [--rng-raw] [--ignore-seeds]
//        [--window W] [--full] [--json]
//   expected = the oracle (tools/oracle/trace.mjs), actual = the engine; either may be .gz.
//   --context N      records of context before / after the divergence (default 6)
//   --level L        compare only kinds up to level L (default: the lower of the two headers' levels)
//   --kinds a,b      compare only these record kinds (e.g. log,rng)
//   --skip-kinds a,b compare all kinds but these (e.g. boundary,choose,chosen when the engine does not emit markers)
//   --skip-events    drop these event names (ev/evx pairs and the sort/hc/hcx they own) from both traces,
//                    e.g. the engine does not model ModifySpe/ModifyBoost/Update the way Showdown does
//   --squash-empty   drop ev/evx pairs that contain no other record and leave the relay value unchanged
//                    (Showdown calls runEvent far more often than anything has a handler)
//   --merge-logedits  fold each `logedit` into the `log` record it edits (for an engine that only knows the
//                    final text of a line when it appends it)
//   --rng-raw        compare PRNG draws only as the seed chain (one record per next() call), ignoring which
//                    API call (random / randomChance / sample / shuffle) made them and with which arguments
//   --ignore-seeds   do not compare PRNG seeds (s0/s1 of rng records, seed of boundary records)
//   --window W       how far ahead to look for the point where the traces re-synchronize (default 1000)
//   --full           do not truncate long records when printing
//   --json           machine readable result on stdout
//
// Records are compared by their canonical form: keys sorted, top-level keys starting with `_` (oracle-only
// annotations) and the redundant depth field `d` ignored, `hdr` records ignored. Exit status: 0 no
// divergence, 1 divergence found, 2 usage / input error.
import { readAllLines } from './lib/trace-io.mjs';
import { KIND_LEVEL, canonical, isTrivialPair } from './lib/trace-schema.mjs';

function usage(msg) {
	if (msg) console.error(`trace-diff.mjs: ${msg}`);
	console.error('usage: trace-diff.mjs expected.jsonl actual.jsonl [--context N] [--level 1|2|3] [--kinds k1,k2]\n' +
		'       [--skip-kinds k1,k2] [--skip-events E1,E2] [--squash-empty] [--merge-logedits] [--rng-raw] [--ignore-seeds]\n       [--window W] [--full] [--json]');
	process.exit(2);
}

function parseArgs(argv) {
	const args = {
		files: [], context: 6, level: null, kinds: null, skipKinds: new Set(), skipEvents: new Set(), squashEmpty: false, ignoreSeeds: false,
		window: 1000, full: false, json: false, mergeLogedits: false, rngRaw: false,
	};
	const int = (name, v, min) => {
		const n = Number(v);
		if (v === undefined || !Number.isInteger(n) || n < min) usage(`${name} needs an integer >= ${min}`);
		return n;
	};
	for (let i = 0; i < argv.length; i++) {
		const a = argv[i];
		if (a === '--context') args.context = int(a, argv[++i], 0);
		else if (a === '--level') {
			args.level = int(a, argv[++i], 1);
			if (args.level > 3) usage('--level must be 1, 2 or 3');
		} else if (a === '--window') args.window = int(a, argv[++i], 1);
		else if (a === '--kinds') args.kinds = new Set((argv[++i] || '').split(',').filter(Boolean));
		else if (a === '--skip-kinds') args.skipKinds = new Set((argv[++i] || '').split(',').filter(Boolean));
		else if (a === '--skip-events') args.skipEvents = new Set((argv[++i] || '').split(',').filter(Boolean));
		else if (a === '--squash-empty') args.squashEmpty = true;
		else if (a === '--ignore-seeds') args.ignoreSeeds = true;
		else if (a === '--merge-logedits') args.mergeLogedits = true;
		else if (a === '--rng-raw') args.rngRaw = true;
		else if (a === '--full') args.full = true;
		else if (a === '--json') args.json = true;
		else if (a.startsWith('--')) usage(`unknown option ${a}`);
		else args.files.push(a);
	}
	if (args.files.length !== 2) usage('need exactly two trace files');
	return args;
}

// ---------------------------------------------------------------------------------------
// Loading and normalization
// ---------------------------------------------------------------------------------------

function load(file) {
	let lines;
	try {
		lines = readAllLines(file);
	} catch (err) {
		usage(`cannot read ${file}: ${err.message}`);
	}
	const recs = [];
	let hdr = null;
	lines.forEach((text, n) => {
		let rec;
		try {
			rec = JSON.parse(text);
		} catch {
			usage(`${file}: line ${n + 1} is not valid JSON`);
		}
		if (rec === null || typeof rec !== 'object' || typeof rec.k !== 'string') usage(`${file}: line ${n + 1} is not a trace record`);
		if (rec.k === 'hdr') hdr = hdr || rec;
		else recs.push({ rec, line: n + 1 });
	});
	const level = hdr && Number.isInteger(hdr.level) ? hdr.level : Math.max(1, ...recs.map(r => KIND_LEVEL[r.rec.k] || 1));
	return { file, hdr, recs, level };
}

/** Drops ev/evx pairs of the named events together with the sort/hc/hcx records they own. */
function dropEvents(recs, names) {
	if (!names.size) return recs;
	const out = [];
	const dropped = [];
	for (const item of recs) {
		const r = item.rec;
		if (r.k === 'ev') {
			const drop = names.has(r.e);
			dropped.push(drop);
			if (!drop) out.push(item);
		} else if (r.k === 'evx') {
			if (!dropped.pop()) out.push(item);
		} else if (r.k === 'hc' || r.k === 'hcx' || (r.k === 'sort' && ['event', 'field', 'each'].includes(r.w))) {
			if (!(dropped.length && dropped[dropped.length - 1])) out.push(item);
		} else {
			out.push(item);
		}
	}
	return out;
}

/** Drops trivial ev/evx pairs: nothing between them and the relay value unchanged (see isTrivialPair). */
function squashEmpty(recs) {
	const out = [];
	for (const item of recs) {
		const r = item.rec;
		const top = out[out.length - 1];
		if (r.k === 'evx' && top && top.rec.k === 'ev' && top.rec.e === r.e && isTrivialPair(top.rec, r)) out.pop();
		else out.push(item);
	}
	return out;
}

/** Folds `logedit` records into the `log` record they edit (deletions, line: null, are dropped). */
function mergeLogedits(recs) {
	const out = [];
	const byIndex = new Map();
	for (const item of recs) {
		const r = item.rec;
		if (r.k === 'log') {
			const copy = { ...item, rec: { ...r } };
			byIndex.set(r.i, copy);
			out.push(copy);
		} else if (r.k === 'logedit') {
			const target = byIndex.get(r.i);
			if (target && typeof r.line === 'string') target.rec.line = r.line;
		} else {
			out.push(item);
		}
	}
	return out;
}

const wordsOf = s => [Number(s >> 48n), Number((s >> 32n) & 0xFFFFn), Number((s >> 16n) & 0xFFFFn), Number(s & 0xFFFFn)];

/** One `{k:'rng', s0, s1}` record per underlying next() call. */
function rngRaw(recs) {
	const out = [];
	for (const item of recs) {
		const r = item.rec;
		if (r.k !== 'rng') {
			out.push(item);
			continue;
		}
		let s = seedToBig(r.s0);
		for (let n = 0; n < r.n; n++) {
			const next = stepBig(s);
			out.push({ ...item, rec: { k: 'rng', s0: wordsOf(s), s1: wordsOf(next) } });
			s = next;
		}
	}
	return out;
}

function normalize(trace, args, level) {
	let recs = trace.recs.filter(({ rec }) => (KIND_LEVEL[rec.k] || 1) <= level && (!args.kinds || args.kinds.has(rec.k)) && !args.skipKinds.has(rec.k));
	recs = dropEvents(recs, args.skipEvents);
	if (args.squashEmpty) recs = squashEmpty(recs);
	if (args.mergeLogedits) recs = mergeLogedits(recs);
	if (args.rngRaw) recs = rngRaw(recs);
	return recs.map(item => {
		const c = canonical(item.rec);
		delete c.d;
		if (args.ignoreSeeds) {
			delete c.s0;
			delete c.s1;
			if (c.k === 'boundary') delete c.seed;
		}
		return { ...item, text: JSON.stringify(c), c };
	});
}

// ---------------------------------------------------------------------------------------
// Seed arithmetic (Gen5 LCG) to express a seed mismatch as a number of draws
// ---------------------------------------------------------------------------------------

const MASK64 = (1n << 64n) - 1n;
const MUL = 0x5D588B656C078965n;
const ADD = 0x269EC3n;
const seedToBig = w => (BigInt(w[0]) << 48n) | (BigInt(w[1]) << 32n) | (BigInt(w[2]) << 16n) | BigInt(w[3]);
const stepBig = s => (s * MUL + ADD) & MASK64;

/** n such that advancing `from` by n draws gives `to` (0 <= n <= limit), or null. */
function drawsBetween(from, to, limit = 20000) {
	if (!Array.isArray(from) || !Array.isArray(to) || from.length !== 4 || to.length !== 4) return null;
	let s = seedToBig(from);
	const t = seedToBig(to);
	for (let n = 0; n <= limit; n++) {
		if (s === t) return n;
		s = stepBig(s);
	}
	return null;
}

function seedRelation(expSeed, actSeed) {
	const fwd = drawsBetween(expSeed, actSeed);
	if (fwd !== null) return fwd === 0 ? 'equal' : `actual is ${fwd} draw${fwd === 1 ? '' : 's'} ahead of expected`;
	const back = drawsBetween(actSeed, expSeed);
	if (back !== null) return `actual is ${back} draw${back === 1 ? '' : 's'} behind expected`;
	return 'unrelated (not reachable by LCG steps within 20000 draws)';
}

// ---------------------------------------------------------------------------------------
// Presentation
// ---------------------------------------------------------------------------------------

function short(rec, full) {
	const copy = { ...rec };
	const cs = copy._cs;
	delete copy._cs;
	let s = JSON.stringify(copy);
	if (!full && s.length > 240) s = `${s.slice(0, 237)}...`;
	return cs ? `${s}  [cs ${cs.join(' < ')}]` : s;
}

function describe(rec) {
	switch (rec.k) {
		case 'rng': return rec.m ? `${rec.m}(${(rec.a || []).join(',')})` : `next() from seed ${JSON.stringify(rec.s0)}`;
		case 'log': return `log line ${rec.i} ${JSON.stringify(rec.line)}`;
		case 'logedit': return `log edit ${rec.i}`;
		case 'ev': return `${rec.f} event ${rec.e}${rec.t !== undefined ? ` t=${JSON.stringify(rec.t)}` : ''}${rec.x ? ` x=${rec.x}` : ''}`;
		case 'evx': return `end of event ${rec.e}`;
		case 'sort': return `${rec.w} sort${rec.e ? ` (${rec.e})` : ''}`;
		case 'hc': return `handler ${rec.x} (${rec.cn}) of ${JSON.stringify(rec.h)}`;
		case 'hcx': return 'handler return';
		default: return rec.k;
	}
}

function eventStack(recs, upTo) {
	const frames = [];
	for (let i = 0; i < upTo && i < recs.length; i++) {
		const r = recs[i].rec;
		if (r.k === 'ev') frames.push(`${r.f}:${r.e}${r.t !== undefined && r.t !== null ? `(${Array.isArray(r.t) ? r.t.join('+') : r.t})` : ''}`);
		else if (r.k === 'evx') frames.pop();
		else if (r.k === 'hc') frames.push(`handler:${r.x}[${r.cn}]`);
		else if (r.k === 'hcx') frames.pop();
	}
	return frames;
}

function whereInfo(recs, upTo) {
	let boundary = null;
	let choose = [];
	let lastLog = null;
	for (let i = 0; i < upTo && i < recs.length; i++) {
		const r = recs[i].rec;
		if (r.k === 'boundary') {
			boundary = r;
			choose = [];
		} else if (r.k === 'choose') choose.push(`${r.side}: ${r.input}`);
		else if (r.k === 'log') lastLog = r;
	}
	return { boundary, choose, lastLog };
}

// ---------------------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------------------

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

function diffFields(e, a) {
	const keys = new Set([...Object.keys(e), ...Object.keys(a)]);
	return [...keys].filter(k => !same(e[k], a[k]));
}

function itemKey(it) {
	return JSON.stringify(it);
}

function classifySort(e, a) {
	const parts = [];
	const ek = e.it.map(itemKey);
	const ak = a.it.map(itemKey);
	const label = `${e.w} sort${e.e ? ` of ${e.e}` : ''}`;
	const missing = ek.filter(k => !ak.includes(k));
	const extra = ak.filter(k => !ek.includes(k));
	if (!missing.length && !extra.length && ek.length === ak.length) {
		const orderOf = it => `${it.x ?? it.c ?? ''}@${it.h ?? ''}`;
		if (!same(e.it, a.it)) {
			return {
				cls: 'different handler order', detail: `${label}: expected ${e.it.map(orderOf).join(' > ')} but got ${a.it.map(orderOf).join(' > ')}` +
					` (shuffles: expected ${e.sh}, actual ${a.sh})`,
			};
		}
		return { cls: 'different sort metadata', detail: `${label}: fields ${diffFields(e, a).join(',')} differ` };
	}
	// same effects but different ordering keys?
	const idOf = it => `${it.x ?? it.c ?? ''}|${it.h ?? ''}|${it.cn ?? ''}|${it.m ?? ''}`;
	const eIds = e.it.map(idOf);
	const aIds = a.it.map(idOf);
	if (same([...eIds].sort(), [...aIds].sort())) {
		for (const it of e.it) {
			const other = a.it.find(o => idOf(o) === idOf(it));
			const f = other ? diffFields(it, other) : [];
			if (f.length) {
				return {
					cls: 'different handler ordering keys',
					detail: `${label}: ${idOf(it)} differs in ${f.map(k => `${k} (expected ${JSON.stringify(it[k])}, actual ${JSON.stringify(other[k])})`).join(', ')}`,
				};
			}
		}
	}
	if (missing.length) parts.push(`missing in actual: ${missing.slice(0, 4).join(' ')}`);
	if (extra.length) parts.push(`extra in actual: ${extra.slice(0, 4).join(' ')}`);
	return { cls: 'different handler list', detail: `${label}: ${parts.join('; ')}` };
}

/** Both records have the same kind and differ in content. */
function classifyPair(e, a) {
	const fields = diffFields(e, a);
	switch (e.k) {
		case 'rng': {
			const call = e.m ? `${e.m}(${(e.a || []).join(',')})` : 'next()';
			if (e.m !== a.m || !same(e.a, a.a)) {
				return { cls: 'different RNG call', detail: `expected ${e.m}(${(e.a || []).join(',')}), got ${a.m}(${(a.a || []).join(',')})` };
			}
			if (!same(e.s0, a.s0)) {
				return { cls: 'PRNG state differs before the draw', detail: `${call}: ${seedRelation(e.s0, a.s0)}` };
			}
			return { cls: 'different RNG result', detail: `${call}: fields ${fields.join(',')} differ (expected r=${JSON.stringify(e.r)} n=${e.n}, actual r=${JSON.stringify(a.r)} n=${a.n})` };
		}
		case 'log':
			return { cls: 'different log line', detail: `line ${e.i}${e.i !== a.i ? ` (actual index ${a.i})` : ''}` };
		case 'logedit':
			return { cls: 'different log edit', detail: `line ${e.i}` };
		case 'ev': {
			const ident = fields.filter(f => f !== 'v');
			if (ident.length) return { cls: 'different event call', detail: `fields ${ident.join(',')} differ` };
			return { cls: 'different relay value in', detail: `${e.f} ${e.e}: expected ${JSON.stringify(e.v)}, got ${JSON.stringify(a.v)}` };
		}
		case 'evx':
			return { cls: 'different relay value out', detail: `${e.e}: expected ${JSON.stringify(e.v)}, got ${JSON.stringify(a.v)}` };
		case 'sort':
			if (Array.isArray(e.it) && Array.isArray(a.it)) return classifySort(e, a);
			return { cls: 'different sort', detail: `fields ${fields.join(',')} differ` };
		case 'hc':
			return { cls: 'different handler invoked', detail: `expected ${e.x} (${e.cn}) of ${JSON.stringify(e.h)}, got ${a.x} (${a.cn}) of ${JSON.stringify(a.h)}; fields ${fields.join(',')}` };
		case 'hcx':
			return { cls: 'different handler return value', detail: `expected ${JSON.stringify(e.r)}, got ${JSON.stringify(a.r)}` };
		case 'boundary': {
			const rel = fields.includes('seed') ? `; PRNG: ${seedRelation(e.seed, a.seed)}` : '';
			return { cls: 'different decision boundary', detail: `step ${e.step}: fields ${fields.join(',')} differ${rel}` };
		}
		case 'choose': case 'chosen':
			return { cls: `different ${e.k} marker`, detail: `fields ${fields.join(',')} differ` };
		default:
			return { cls: `different ${e.k} record`, detail: `fields ${fields.join(',')} differ` };
	}
}

function kindSummary(items) {
	const counts = {};
	for (const it of items) counts[it.rec.k] = (counts[it.rec.k] || 0) + 1;
	return Object.entries(counts).map(([k, n]) => `${n} ${k}`).join(', ');
}

function classify(E, A, i, window) {
	const e = E[i];
	const a = A[i];
	if (!e && !a) return null;
	if (!e) {
		return { cls: 'extra records at the end of actual', detail: `${A.length - i} extra, first: ${describe(a.rec)} (${kindSummary(A.slice(i))})`, da: 0, db: A.length - i };
	}
	if (!a) {
		return { cls: 'actual ends early', detail: `${E.length - i} expected records missing, first: ${describe(e.rec)} (${kindSummary(E.slice(i))})`, da: E.length - i, db: 0 };
	}
	// Smallest (da, db) after which the next K records agree again.
	const K = 4;
	const agree = (da, db) => {
		for (let k = 0; k < K; k++) {
			const x = E[i + da + k];
			const y = A[i + db + k];
			if (!x || !y) return k > 0 && !x && !y; // both ended together
			if (x.text !== y.text) return false;
		}
		return true;
	};
	let found = null;
	for (let c = 1; c <= window && !found; c++) {
		// prefer the cheapest explanation; for equal cost prefer a pure insertion / deletion over a substitution
		const order = [];
		for (let da = 0; da <= c; da++) order.push([da, c - da]);
		order.sort((p, q) => Math.min(p[0], p[1]) - Math.min(q[0], q[1]));
		for (const [da, db] of order) {
			if (i + da > E.length || i + db > A.length) continue;
			if (agree(da, db)) {
				found = { da, db };
				break;
			}
		}
	}
	if (found && (found.da === 0 || found.db === 0)) {
		const { da, db } = found;
		if (db > 0) {
			const extra = A.slice(i, i + db);
			return {
				cls: classifyInsertion(extra, 'extra'), da, db, detail: `actual has ${db} record(s) not in expected: ${kindSummary(extra)}; first: ${describe(extra[0].rec)}`,
			};
		}
		const missing = E.slice(i, i + da);
		return {
			cls: classifyInsertion(missing, 'missing'), da, db, detail: `actual lacks ${da} expected record(s): ${kindSummary(missing)}; first: ${describe(missing[0].rec)}`,
		};
	}
	const pairable = e.rec.k === a.rec.k;
	if (pairable) {
		const p = classifyPair(e.rec, a.rec);
		return { ...p, da: found ? found.da : 1, db: found ? found.db : 1, resync: !!found };
	}
	return {
		cls: 'different record kind', da: found ? found.da : 1, db: found ? found.db : 1, resync: !!found,
		detail: `expected ${describe(e.rec)} but got ${describe(a.rec)}`,
	};
}

function classifyInsertion(items, which) {
	const first = items[0].rec;
	switch (first.k) {
		case 'rng': return `${which} RNG draw`;
		case 'log': return `${which} log line`;
		case 'logedit': return `${which} log edit`;
		case 'ev': case 'evx': return `${which} event call`;
		case 'sort': return `${which} handler list / sort`;
		case 'hc': case 'hcx': return `${which} handler invocation`;
		default: return `${which} ${first.k} record`;
	}
}

// ---------------------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------------------

const args = parseArgs(process.argv.slice(2));
const te = load(args.files[0]);
const ta = load(args.files[1]);
const level = args.level ?? Math.min(te.level, ta.level);
const E = normalize(te, args, level);
const A = normalize(ta, args, level);

let i = 0;
while (i < E.length && i < A.length && E[i].text === A[i].text) i++;

const result = {
  identical: i === E.length && i === A.length,
  level,
  compared: i,
  expectedRecords: E.length,
  actualRecords: A.length,
};

if (result.identical) {
	if (args.json) console.log(JSON.stringify(result));
	else console.log(`traces identical: ${E.length} records compared at level ${level}${args.kinds ? ` (kinds ${[...args.kinds]})` : ''}`);
	process.exit(0);
}

const cls = classify(E, A, i, args.window);
const where = whereInfo(E, i);
const stackE = eventStack(E, i);
Object.assign(result, {
  expectedAt: E[i] ? { record: i, line: E[i].line } : null,
  actualAt: A[i] ? { record: i, line: A[i].line } : null,
  classification: cls.cls,
  detail: cls.detail,
  eventStack: stackE,
  boundary: where.boundary,
  lastLog: where.lastLog ? where.lastLog.line : null,
});

if (args.json) {
	console.log(JSON.stringify(result));
	process.exit(1);
}

const out = [];
const hdrInfo = h => (h ? `oracle ${String(h.oracle || '?').slice(0, 7)}, battle ${h.fixture ? `index ${h.fixture.index}` : '?'}, level ${h.level}` : 'no header');
out.push(`expected: ${args.files[0]}  (${E.length} records; ${hdrInfo(te.hdr)})`);
out.push(`actual:   ${args.files[1]}  (${A.length} records; ${hdrInfo(ta.hdr)})`);
out.push(`compared at level ${level}${args.kinds ? `, kinds ${[...args.kinds]}` : ''}${args.skipKinds.size ? `, without kinds ${[...args.skipKinds]}` : ''}${args.skipEvents.size ? `, skipping events ${[...args.skipEvents]}` : ''}${args.squashEmpty ? ', empty events squashed' : ''}${args.mergeLogedits ? ', log edits merged' : ''}${args.rngRaw ? ', raw draws only' : ''}${args.ignoreSeeds ? ', seeds ignored' : ''}`);
out.push('');
out.push(`DIVERGENCE after ${i} identical records`);
out.push(`  class:   ${cls.cls}`);
out.push(`  detail:  ${cls.detail}`);
if (cls.resync === false) out.push(`  note:    the traces do not re-synchronize within ${args.window} records`);
if (where.boundary) {
	out.push(`  where:   after decision boundary step ${where.boundary.step} (turn ${where.boundary.turn}, ${where.boundary.state})` +
		`${where.choose.length ? `, choices: ${where.choose.join(' | ')}` : ''}`);
}
if (where.lastLog) out.push(`  last log line before: [${where.lastLog.i}] ${where.lastLog.line}`);
if (stackE.length) out.push(`  inside:  ${stackE.join(' > ')}`);
const annot = rec => (rec && (rec._cs || rec._fx || rec._ev) ? `call site: ${(rec._cs || []).join(' < ')}${rec._fx ? ` effect ${rec._fx}` : ''}${rec._ev ? ` event ${rec._ev}` : ''}` : null);
for (const [name, items] of [['expected', E], ['actual', A]]) {
	const note = annot(items[i] && items[i].rec);
	if (note) out.push(`  ${name} ${note}`);
}
out.push('');
out.push(`common context (last ${Math.min(args.context, i)} identical records):`);
for (let k = Math.max(0, i - args.context); k < i; k++) out.push(`    #${k} (line ${E[k].line}) ${short(E[k].rec, args.full)}`);
const show = (items, from, n) => items.slice(from, from + n);
const spanE = Math.min(Math.max(cls.da ?? 1, 1), 8) + args.context;
const spanA = Math.min(Math.max(cls.db ?? 1, 1), 8) + args.context;
out.push('');
out.push('- expected (next records):');
for (const [k, it] of show(E, i, spanE).entries()) out.push(`-   #${i + k} (line ${it.line}) ${short(it.rec, args.full)}`);
if (i >= E.length) out.push('-   (end of trace)');
out.push('+ actual (next records):');
for (const [k, it] of show(A, i, spanA).entries()) out.push(`+   #${i + k} (line ${it.line}) ${short(it.rec, args.full)}`);
if (i >= A.length) out.push('+   (end of trace)');
console.log(out.join('\n'));
process.exit(1);
