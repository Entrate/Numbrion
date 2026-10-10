// Replays one battle of a directed fixture file on the pinned Showdown and prints, before every
// step, the HP / item / ability state of every Pokemon whose name contains --mon.
//   node tools/probes/consumables/inspect-battle.mjs FIXTURES.jsonl.gz POSITION [--mon Swalot] [--ps ~/src/pokemon-showdown]
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { loadSim } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';
import { replaySim } from '../../oracle/lib/directed-replay.mjs';

const args = process.argv.slice(2);
const opt = (name, dflt) => {
	const i = args.indexOf(name);
	return i < 0 ? dflt : args.splice(i, 2)[1];
};
const monFilter = opt('--mon', '');
const ps = opt('--ps', path.join(os.homedir(), 'src/pokemon-showdown'));
const [file, pos] = args;
const raw = file.endsWith('.gz') ? zlib.gunzipSync(fs.readFileSync(file)).toString() : fs.readFileSync(file, 'utf8');
const fixture = JSON.parse(raw.split('\n').filter(l => l.trim())[Number(pos)]);

const sim = replaySim(loadSim(ps), fixture);
const session = new Session(sim, { seed: fixture.battleSeed, teams: fixture.teams, names: fixture.players.map(p => p.name) });
const dump = label => {
	for (const side of session.battle.sides) {
		for (const p of side.pokemon) {
			if (monFilter && !p.name.includes(monFilter)) continue;
			console.log(
				label, side.id, p.name, `hp=${p.hp}/${p.maxhp}`, `item=${p.item || '-'}`, `last=${p.lastItem || '-'}`,
				`abilityState=${JSON.stringify({ ...p.abilityState, target: undefined })}`, `status=${p.status || '-'}`,
				`active=${p.isActive}`,
			);
		}
	}
};
for (let k = 0; k < fixture.steps.length; k++) {
	const step = fixture.steps[k];
	session.snapshot();
	dump(`before step ${k} (turn ${session.battle.turn})`);
	for (const c of step.choices) session.choose(c.side, c.input);
}
