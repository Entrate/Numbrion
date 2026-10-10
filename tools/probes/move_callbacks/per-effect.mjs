// move_callbacks batch: replay every effect of the batch as its own directed single-effect corpus.
//
//   cargo build --release -j 2 --manifest-path crates/difftest/Cargo.toml
//   node tools/probes/move_callbacks/per-effect.mjs [count=60] [seed=5] [spec-suffix=+tera]
//
// For each effect it runs `gen-directed.mjs --profile effect:moves:<id><suffix> --verify` (Showdown plays and
// re-verifies the battles) and then `difftest replay --sim engine`; the last line counts effects with failures.
// Panics in other batches' unported hooks show up as "(panics N)" and are expected until the batches merge.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const [count = '60', seed = '5', suffix = '+tera'] = process.argv.slice(2);
const names = `acrobatics alluringvoice avalanche bellydrum burningjealousy clangoroussoul clearsmog collisioncourse
doubleshock dragonenergy electrodrift endeavor eruption facade ficklebeam finalgambit freezedry fusionbolt fusionflare
grassknot gravapple haze heatcrash heavyslam hyperspacefury lashout lastrespects lowkick photongeyser ragefist ruination
shellsidearm stompingtantrum struggle superfang waterspout glaiverush roost`.split(/\s+/);
let failed = 0;
for (const n of names) {
	const out = `/tmp/move-callbacks-${n}.jsonl.gz`;
	const g = spawnSync('node', ['tools/oracle/gen-directed.mjs', '--profile', `effect:moves:${n}${suffix}`, '--count', count,
		'--threads', '2', '--seed', seed, '--out', out, '--verify'], { cwd: root, encoding: 'utf8' });
	if (g.status !== 0) { failed++; console.log(n.padEnd(16), 'GENERATION FAILED', (g.stdout + g.stderr).slice(-300)); continue; }
	const r = spawnSync(`${root}crates/difftest/target/release/difftest`,
		['replay', out, '--sim', 'engine', '--jobs', '2', '--top', '5'], { cwd: root, encoding: 'utf8' });
	const text = r.stdout + r.stderr;
	const m = text.match(/battles : (\d+) total, (\d+) passed, (\d+) failed/);
	const panics = (text.match(/unimplemented effect hook/g) || []).length;
	if (!m || m[3] !== '0') failed++;
	console.log(n.padEnd(16), m ? `${m[2]}/${m[1]} passed` : 'NO RESULT', panics ? `(panics ${panics})` : '');
}
console.log('effects with failures:', failed);
