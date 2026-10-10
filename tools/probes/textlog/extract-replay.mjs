// Extracts real Showdown battle logs from the oracle fixtures into the compact TSV that the
// TextLog replay test (crates/engine/src/log/text/tests/replay.rs) feeds through the Rust sink.
//
//   node tools/probes/textlog/extract-replay.mjs --out <file.tsv> [--select N] [--limit N] [--skip N] <fixtures...>
//
//   --select N   greedy set cover: keep at most N battles that together cover the most distinct
//                line shapes / feature flags (smallest battles first on ties). Default: keep all.
//   --limit N    stop reading after N battles per input file (default: all).
//   --skip N     skip the first N battles of each input file.
//
// Inputs are `.jsonl` or `.jsonl.gz` (docs/design/FIXTURES.md). A battle's raw log is the concatenation
// of steps[].log and end.log (entries exactly as battle.log stores them, `|t:|` already normalized).
// Output, UTF-8: per battle a header `#B<TAB>id<TAB>p1 name<TAB>p2 name<TAB>p1 packed team<TAB>p2 packed team`
// followed by one raw log entry per line (an empty public split entry is an empty line).
import fs from 'node:fs';
import zlib from 'node:zlib';
import readline from 'node:readline';

const argv = process.argv.slice(2);
const opts = { out: null, select: Infinity, limit: Infinity, skip: 0, files: [] };
for (let i = 0; i < argv.length; i++) {
	const a = argv[i];
	if (a === '--out') opts.out = argv[++i];
	else if (a === '--select') opts.select = Number(argv[++i]);
	else if (a === '--limit') opts.limit = Number(argv[++i]);
	else if (a === '--skip') opts.skip = Number(argv[++i]);
	else opts.files.push(a);
}
if (!opts.out || !opts.files.length) {
	console.error('usage: extract-replay.mjs --out file.tsv [--select N] [--limit N] [--skip N] fixtures...');
	process.exit(2);
}

async function* battles(file) {
	const raw = fs.createReadStream(file);
	const input = file.endsWith('.gz') ? raw.pipe(zlib.createGunzip()) : raw;
	const rl = readline.createInterface({ input, crlfDelay: Infinity });
	let n = 0;
	for await (const line of rl) {
		if (!line.trim()) continue;
		if (n++ < opts.skip) continue;
		if (n - opts.skip > opts.limit) break;
		yield JSON.parse(line);
	}
	rl.close();
}

// Kinds whose first literal after the ident selects a different sub-grammar (05-log-shapes.txt, section 2).
const VOCAB_KINDS = new Set(['-activate', '-start', '-end', '-singleturn', '-singlemove', '-sidestart', '-sideend', '-fieldstart', '-fieldend',
	'-weather', 'cant', '-fail', '-block']);
const IDENT = /^p[12][ab]?: /;

// Coarse argument class, close to the catalog's shape granularity (05-log-shapes.txt).
function cls(v) {
	if (v === '') return 'E';
	let m;
	if (/^p[12][ab]: /.test(v)) return 'P';
	if (/^p[12]: /.test(v)) return 'N';
	if (/^p[12][ab](,p[12][ab])+$/.test(v)) return 'SS';
	if (/^p[12][ab]$/.test(v)) return 'S';
	if (/^p[12]$/.test(v)) return 'D';
	if (/^-?\d+$/.test(v)) return 'I';
	if (/^\d+\/\d+( [a-z]+)?$/.test(v) || v === '0 fnt') return 'H';
	if ((m = /^\[([A-Za-z]+)\](?: (.*))?$/s.exec(v))) return m[2] === undefined ? `[${m[1]}]` : `[${m[1]}]${cls(m[2])}`;
	if ((m = /^(move|ability|item|pokemon): /.exec(v))) return m[1][0].toUpperCase();
	return 'W';
}

function shapeKeys(log) {
	const keys = new Set();
	for (let i = 0; i < log.length; i++) {
		const line = log[i];
		if (line === '') continue;
		const f = line.split('|');
		const kind = f[1];
		const tags = f.slice(2).filter(x => /^\[[a-z]+\]/.test(x)).map(x => x.split(' ')[0]);
		const plain = f.slice(2).filter(x => !/^\[[a-z]+\]/.test(x));
		let key = `${kind}/${plain.length}/${tags.join('')}`;
		if (VOCAB_KINDS.has(kind) && plain.length >= 2) {
			const lit = plain[1];
			if (!IDENT.test(lit) && !/^-?\d+$/.test(lit)) key += `/${lit}`;
		}
		if (kind === 'move') {
			// the exact tag sequence is the shape of a move line
			key = `move/${plain.length}/${f.slice(5).map(x => x.split(' ')[0]).join('')}`;
		}
		if (kind === 'switch' || kind === 'drag') key += /, tera:/.test(f[3]) ? '/tera' : '';
		keys.add(key);
		keys.add(`S:${kind}|${f.slice(2).map(cls).join('|')}`);
	}
	return keys;
}

function featureKeys(teams, log) {
	const keys = new Set();
	const t = teams.join(']');
	if (/\|[Ii]llusion\|/.test(t)) keys.add('F:illusion');
	if (log.some(l => l.startsWith('|replace|'))) keys.add('F:replace');
	if (log.some(l => /^\|switch\|p[12][ab]: [^|]+\|[^|]+, tera:/.test(l))) keys.add('F:switch-tera');
	if (log.some(l => /^\|detailschange\|p[12]: /.test(l))) keys.add('F:faint-regression');
	if (log.some(l => /^\|detailschange\|/.test(l) && /tera:/.test(l))) keys.add('F:detailschange-tera');
	if (log.some(l => /^\|switch\|[^|]*\|[^|,]+, [MF]\|/.test(l))) keys.add('F:level100');
	if (log.some(l => /, shiny/.test(l))) keys.add('F:shiny');
	if (log.some(l => /^\|switch\|p[12][ab]: ([^|]+)\|(?!\1)[^|,]+(, |\|)/.test(l))) keys.add('F:forme-vs-name');
	if (log.some(l => l.includes('Greninja'))) keys.add('F:greninja');
	if (log.some(l => /^\|-heal\|p[12]: /.test(l))) keys.add('F:heal-noslot');
	if (log.some(l => /^\|-hitcount\|p[12]: /.test(l))) keys.add('F:hitcount-noslot');
	if (log.some(l => /^\|tie/.test(l))) keys.add('F:tie');
	return keys;
}

const kept = [];
let seen = 0;
for (const file of opts.files) {
	let idx = 0;
	for await (const fx of battles(file)) {
		const log = [];
		for (const s of fx.steps) log.push(...s.log);
		log.push(...fx.end.log);
		const names = fx.players.map(p => p.name);
		const keys = new Set([...shapeKeys(log), ...featureKeys(fx.teams, log)]);
		kept.push({ id: `${file.split('/').pop()}#${idx}`, names, teams: fx.teams, log, keys });
		idx++;
		seen++;
	}
}

let chosen = kept;
if (Number.isFinite(opts.select)) {
	chosen = [];
	const covered = new Set();
	const pool = kept.slice();
	while (chosen.length < opts.select && pool.length) {
		let best = -1, bestScore = 0;
		for (let i = 0; i < pool.length; i++) {
			let fresh = 0;
			for (const k of pool[i].keys) if (!covered.has(k)) fresh++;
			const score = fresh / Math.sqrt(pool[i].log.length + 50);
			if (fresh > 0 && score > bestScore) { best = i; bestScore = score; }
		}
		if (best < 0) break;
		const b = pool.splice(best, 1)[0];
		for (const k of b.keys) covered.add(k);
		chosen.push(b);
	}
	const all = new Set();
	for (const b of kept) for (const k of b.keys) all.add(k);
	console.error(`covered ${covered.size}/${all.size} distinct shape/feature keys with ${chosen.length} of ${seen} battles`);
	for (const k of [...all].filter(k => !covered.has(k))) console.error(`  uncovered: ${k}`);
}

let out = '# generated by tools/probes/textlog/extract-replay.mjs from oracle fixtures; do not edit\n';
let entries = 0;
for (const b of chosen) {
	out += `#B\t${b.id}\t${b.names[0]}\t${b.names[1]}\t${b.teams[0]}\t${b.teams[1]}\n`;
	for (const l of b.log) {
		if (l.includes('\n') || l.includes('\t')) throw new Error(`unsupported entry ${JSON.stringify(l)}`);
		out += l + '\n';
		entries++;
	}
}
fs.writeFileSync(opts.out, out);
console.error(`${chosen.length} battles, ${entries} entries, ${out.length} bytes -> ${opts.out}`);
