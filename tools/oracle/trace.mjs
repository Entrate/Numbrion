// Event / RNG tracer for the Showdown oracle: replays fixture battles on the pinned Showdown build and
// writes a JSONL trace of every PRNG draw, log line, event call and handler list. The Rust engine emits the
// same format (behind a debug feature) and tools/oracle/trace-diff.mjs finds the first difference.
//
// Usage:
//   node tools/oracle/trace.mjs <path-to-pokemon-showdown> <fixtures.jsonl[.gz]> --battle I --out trace.jsonl[.gz]
//        [--from-turn T] [--to-turn U] [--level 1|2|3] [--stack | --no-stack] [--skip-empty] [--index] [--time-plain]
//   --battle SPEC  0-based position among the fixture lines (what `difftest --battle` uses): `3`, `3,7,10-20` or
//                  `all`; with --index the fixture's `index` field instead. One battle: --out is the trace file.
//                  Several battles: --out is a directory (battle-<position>.jsonl[.gz] inside), or omit --out to
//                  only run the checks and print one statistics row per battle.
//   --level L      1: markers + log + rng   2: + event calls   3 (default): + sorted handler lists, handler calls
//   --from-turn/--to-turn   only records emitted while battle.turn is in [T, U] (the replay still runs from the start)
//   --stack        oracle-only `_cs`/`_fx`/`_ev` call-site annotations on rng records (default: on at level 3)
//   --skip-empty   drop ev/evx pairs that contain no other record
//   --gz           (several battles) gzip the files in the --out directory
//   --time-plain   also time the untraced replay (after a warm-up replay) for comparison
//   --out FILE     `.gz` is compressed, `-` is stdout (statistics always go to stderr)
//
// Every traced replay is asserted against the fixture (log, PRNG seeds, requests, accept/reject results,
// end record: tools/oracle/lib/session.mjs replayFixture), so a clean run also proves that tracing does not
// change the battle. Format: docs/design/TRACE.md. Exit status: 0 ok, 1 usage error, 2 a traced replay
// differs from its fixture (the trace is still written).
import fs from 'node:fs';
import path from 'node:path';
import { loadSim } from './lib/common.mjs';
import { readLines, writeLines } from './lib/trace-io.mjs';
import { plainReplay, traceFixture } from './lib/trace-replay.mjs';

function usage(msg) {
	if (msg) console.error(`trace.mjs: ${msg}`);
	console.error('usage: trace.mjs <path-to-pokemon-showdown> <fixtures.jsonl[.gz]> --battle SPEC [--out FILE|DIR]\n' +
		'       [--from-turn T] [--to-turn U] [--level 1|2|3] [--stack | --no-stack] [--skip-empty] [--index] [--time-plain] [--gz]');
	process.exit(1);
}

/** `3`, `3,7,10-20`, `all` -> predicate over numbers, plus the largest wanted number (Infinity for all). */
function parseSpec(spec) {
	if (spec === 'all') return { has: () => true, max: Infinity, single: false };
	const ranges = [];
	for (const part of spec.split(',')) {
		const m = /^(\d+)(?:-(\d+))?$/.exec(part.trim());
		if (!m) usage(`bad --battle spec ${spec}`);
		const lo = Number(m[1]);
		const hi = m[2] === undefined ? lo : Number(m[2]);
		if (hi < lo) usage(`bad --battle range ${part}`);
		ranges.push([lo, hi]);
	}
	return {
		has: n => ranges.some(([lo, hi]) => n >= lo && n <= hi),
		max: Math.max(...ranges.map(r => r[1])),
		single: ranges.length === 1 && ranges[0][0] === ranges[0][1],
	};
}

function parseArgs(argv) {
	const args = {
		psPath: null, file: null, battle: null, out: null, level: 3, stack: undefined, fromTurn: undefined,
		toTurn: undefined, skipEmpty: false, byIndex: false, timePlain: false, gz: false,
	};
	const positional = [];
	const int = (name, v) => {
		const n = Number(v);
		if (v === undefined || !Number.isInteger(n) || n < 0) usage(`${name} needs a non-negative integer`);
		return n;
	};
	for (let i = 0; i < argv.length; i++) {
		const a = argv[i];
		if (a === '--battle') args.battle = argv[++i];
		else if (a === '--level') {
			args.level = int(a, argv[++i]);
			if (![1, 2, 3].includes(args.level)) usage('--level must be 1, 2 or 3');
		} else if (a === '--from-turn') args.fromTurn = int(a, argv[++i]);
		else if (a === '--to-turn') args.toTurn = int(a, argv[++i]);
		else if (a === '--out') args.out = argv[++i];
		else if (a === '--stack') args.stack = true;
		else if (a === '--no-stack') args.stack = false;
		else if (a === '--skip-empty') args.skipEmpty = true;
		else if (a === '--index') args.byIndex = true;
		else if (a === '--time-plain') args.timePlain = true;
		else if (a === '--gz') args.gz = true;
		else if (a.startsWith('--')) usage(`unknown option ${a}`);
		else positional.push(a);
	}
	if (positional.length !== 2) usage('need <path-to-pokemon-showdown> and <fixtures file>');
	[args.psPath, args.file] = positional;
	if (args.battle === null || args.battle === undefined) usage('--battle is required');
	args.spec = parseSpec(args.battle);
	return args;
}

const args = parseArgs(process.argv.slice(2));
const sim = loadSim(args.psPath);
const multi = !args.spec.single;
if (multi && args.out && args.out !== '-') fs.mkdirSync(args.out, { recursive: true });
if (!multi && !args.out) usage('--out is required for a single battle');
if (multi && args.out === '-') usage('--out - needs a single battle');

const fmtKinds = counts => Object.entries(counts).map(([k, n]) => `${k}=${n}`).join(' ');
let position = -1;
let ran = 0;
let failed = 0;
const total = { records: 0, bytes: 0, ms: 0, plainMs: 0 };

for await (const line of readLines(args.file)) {
	position++;
	if (!args.byIndex && position > args.spec.max) break;
	const key = args.byIndex ? Number((/"index":(\d+),/.exec(line.slice(0, 600)) || [])[1]) : position;
	if (!args.spec.has(key)) continue;
	const fx = JSON.parse(line);
	let plainMs = null;
	if (args.timePlain) {
		plainReplay(sim, fx); // warm-up (dex data is loaded lazily by the first replay in a process)
		plainMs = plainReplay(sim, fx).ms;
	}
	const r = traceFixture(sim, fx, {
		level: args.level, stack: args.stack, fromTurn: args.fromTurn, toTurn: args.toTurn, skipEmpty: args.skipEmpty, position,
	});
	ran++;
	total.records += r.records;
	total.bytes += r.bytes;
	total.ms += r.ms;
	total.plainMs += plainMs ?? 0;
	if (args.out) {
		const file = multi ? path.join(args.out, `battle-${position}.jsonl${args.gz ? '.gz' : ''}`) : args.out;
		writeLines(file, r.lines);
	}
	const head = `battle position ${position} (index ${fx.index}, ${fx.steps.length} steps, ${fx.end.turns} turns) level ${args.level}`;
	const stats = `${r.records} records, ${(r.bytes / 1024).toFixed(0)} KiB, ${r.ms.toFixed(0)} ms` +
		`${plainMs === null ? '' : ` (untraced replay ${plainMs.toFixed(0)} ms)`}`;
	if (r.diffs.length) {
		failed++;
		console.error(`${head}: ${stats}\n  REPLAY DIFFERS FROM THE FIXTURE (${r.diffs.length} differences); first:\n${JSON.stringify(r.diffs[0], null, 1)}`);
	} else if (multi) {
		console.error(`${head}: ${stats}; replay matches the fixture`);
	} else {
		console.error(`${head}: ${stats}\n  ${fmtKinds(r.counts)}\n  replay matches the fixture (log, seeds, requests, choices, end record)`);
	}
	if (ran === 1 && !multi) break;
}

if (ran === 0) {
	console.error(`trace.mjs: no fixture matched --battle ${args.battle} in ${args.file}`);
	process.exit(1);
}
if (multi) {
	console.error(`${ran} battles traced at level ${args.level}: ${total.records} records, ${(total.bytes / 1048576).toFixed(1)} MiB, ` +
		`${total.ms.toFixed(0)} ms${args.timePlain ? ` (untraced ${total.plainMs.toFixed(0)} ms)` : ''}; ${failed} differ from their fixture`);
}
process.exit(failed ? 2 : 0);
