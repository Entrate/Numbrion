// Checks that the committed replay vectors (crates/engine/src/log/text/tests/replay-vectors.tsv)
// contain at least one real line of every shape in docs/showdown/05-log-shapes.txt (section 1),
// by running tools/oracle/log-shapes.mjs over them and comparing the shape sets.
//
//   node tools/probes/textlog/shape-coverage.mjs [replay-vectors.tsv]
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '..', '..', '..');
const tsv = process.argv[2] || path.join(repo, 'crates/engine/src/log/text/tests/replay-vectors.tsv');

const battles = [];
for (const line of fs.readFileSync(tsv, 'utf8').split('\n')) {
	if (line.startsWith('# ')) continue;
	if (line.startsWith('#B\t')) {
		const [, , p1, p2, t1, t2] = line.split('\t');
		battles.push({ players: [{ id: 'p1', name: p1 }, { id: 'p2', name: p2 }], teams: [t1, t2], steps: [{ log: [] }], end: { log: [] } });
	} else if (battles.length) battles[battles.length - 1].steps[0].log.push(line);
}
const last = battles[battles.length - 1];
if (last.steps[0].log.at(-1) === '') last.steps[0].log.pop();

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'shape-coverage-'));
const jsonl = path.join(tmp, 'replay.jsonl');
fs.writeFileSync(jsonl, battles.map(b => JSON.stringify(b)).join('\n') + '\n');
const mine = path.join(tmp, 'shapes.txt');
execFileSync('node', [path.join(repo, 'tools/oracle/log-shapes.mjs'), '--out', mine, jsonl], { stdio: ['ignore', 'ignore', 'inherit'] });

const shapesOf = (file) => {
	const text = fs.readFileSync(file, 'utf8');
	const section1 = text.slice(text.indexOf('## 1. SHAPES'), text.indexOf('## 2.'));
	const out = new Set();
	let kind = null;
	for (const l of section1.split('\n')) {
		if (l.startsWith('=== ')) kind = l.slice(4).split('   ')[0];
		const m = /^ {2}\[\d+\] (.*)$/.exec(l);
		if (m) out.add(`${kind} :: ${m[1]}`);
	}
	return out;
};
const want = shapesOf(path.join(repo, 'docs/showdown/05-log-shapes.txt'));
const have = shapesOf(mine);
const missing = [...want].filter(s => !have.has(s));
console.log(`${want.size} catalog shapes, ${have.size} in the replay vectors, ${missing.length} missing`);
for (const s of missing) console.log(`  missing: ${s}`);
process.exit(missing.length ? 1 : 0);
