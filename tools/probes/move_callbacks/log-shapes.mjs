// move_callbacks batch: count the protocol lines that identify each branch of the batch's handlers in a
// fixture file, to see which branches a corpus reaches (the callbacks' logs are the only direct evidence).
//
//   node tools/oracle/gen-directed.mjs --profile batch:move_callbacks+tera --count 300 --out /tmp/mc.jsonl.gz --verify
//   node tools/probes/move_callbacks/log-shapes.mjs /tmp/mc.jsonl.gz [more files]
import fs from 'node:fs';
import zlib from 'node:zlib';
import readline from 'node:readline';

const shapes = {
	'Belly Drum used': /^\|move\|[^|]+\|Belly Drum\|/,
	'Belly Drum/Clangorous fail': /^\|-fail\|[^|]+$/,
	'setboost atk 6 (Belly Drum)': /^\|-setboost\|[^|]+\|atk\|6/,
	'Clangorous Soul used': /^\|move\|[^|]+\|Clangorous Soul\|/,
	'Hyperspace Fury fail [forme]': /^\|-fail\|[^|]+\|move: Hyperspace Fury\|\[forme\]/,
	'Hyperspace Fury fail': /^\|-fail\|[^|]+\|move: Hyperspace Fury$/,
	'Double Shock fail': /^\|-fail\|[^|]+\|move: Double Shock/,
	'Double Shock typechange': /^\|-start\|[^|]+\|typechange\|.*\[from\] move: Double Shock/,
	'Fickle Beam doubled': /^\|-activate\|[^|]+\|move: Fickle Beam/,
	'Shell Side Arm hint (Physical)': /^\|-hint\|Physical Shell Side Arm/,
	'Shell Side Arm hint (Special)': /^\|-hint\|Special Shell Side Arm/,
	'Shell Side Arm [anim]': /Shell Side Arm (Physical|Special)/,
	'Haze': /^\|-clearallboost/,
	'Clear Smog': /^\|-clearboost\|/,
	'Roost -singleturn': /^\|-singleturn\|[^|]+\|move: Roost/,
	'Roost Tera hint': /^\|-hint\|If a Terastallized Pokemon uses Roost/,
	'Glaive Rush start': /^\|-singlemove\|[^|]+\|Glaive Rush\|\[silent\]/,
	'confusion start': /^\|-start\|[^|]+\|confusion/,
	'burn': /^\|-status\|[^|]+\|brn/,
	'Endeavor -immune (any)': /^\|-immune\|/,
	'Stomping Tantrum used': /^\|move\|[^|]+\|Stomping Tantrum\|/,
};
const counts = Object.fromEntries(Object.keys(shapes).map(k => [k, 0]));
let battles = 0;
for (const file of process.argv.slice(2)) {
	const input = file.endsWith('.gz') ? fs.createReadStream(file).pipe(zlib.createGunzip()) : fs.createReadStream(file);
	for await (const line of readline.createInterface({ input })) {
		if (!line.trim()) continue;
		battles++;
		for (const step of JSON.parse(line).steps) for (const l of step.log) {
			for (const [k, re] of Object.entries(shapes)) if (re.test(l)) counts[k]++;
		}
	}
}
console.log('battles', battles);
for (const [k, v] of Object.entries(counts)) console.log(k.padEnd(32), v);
