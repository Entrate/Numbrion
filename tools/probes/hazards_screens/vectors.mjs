// hazards_screens batch: callback-level vectors produced by running the REAL pinned handler functions
// (data/moves.ts conditions) against stub `this`/Pokemon objects.
//
//   node tools/probes/hazards_screens/vectors.mjs [path/to/pokemon-showdown]
//
// Output: crates/engine/src/effects/conditions/hazardsscreens/vectors.tsv
//   S <screen> <initial/4096> <category> <crit> <infiltrates> <sameSide> <targetIsSource> <reflect> <lightscreen> <modifier*4096> <relay>
//   T <initial/4096> <modifier*4096> <relay>                         (Tailwind onModifySpe)
//   D <condition> <hasSource> <hasLightClay> <duration>               (durationCallback)
import fs from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';

const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const { Dex } = createRequire(sim.root + '/package.json')('./dist/sim');
const oracle = new sim.Battle({ formatid: 'gen9randomdoublesbattle', seed: '1,2,3,4' });
const chainModify = sim.Battle.prototype.chainModify;
const rows = ['# kind\t...fields (see tools/probes/hazards_screens/vectors.mjs)'];
const rel = v => (v === undefined ? 'undefined' : String(v));

for (const screen of ['reflect', 'lightscreen', 'auroraveil']) {
	const cond = Dex.moves.get(screen).condition;
	for (const initial of [4096, 6144]) {
		for (const category of ['Physical', 'Special', 'Status']) {
			for (const crit of [0, 1]) for (const infiltrates of [0, 1]) for (const sameSide of [1, 0]) for (const targetIsSource of [0, 1]) {
				for (const reflect of [0, 1]) for (const lightscreen of [0, 1]) {
					if (screen !== 'auroraveil' && (reflect || lightscreen)) continue;
					const ctx = {
						event: { modifier: initial / 4096 }, trunc: oracle.trunc, chainModify, debug() {},
						activePerHalf: 2, getCategory: m => m.category || 'Physical',
						effectState: { target: { hasAlly: () => !!sameSide } },
					};
					const attacker = {};
					const move = { category, infiltrates: !!infiltrates };
					const defender = targetIsSource ? attacker : {
						side: { getSideCondition: id => ((id === 'reflect' && reflect) || (id === 'lightscreen' && lightscreen) ? {} : null) },
						getMoveHitData: () => ({ crit: !!crit }),
					};
					if (targetIsSource) {
						defender.side = { getSideCondition: id => ((id === 'reflect' && reflect) || (id === 'lightscreen' && lightscreen) ? {} : null) };
						defender.getMoveHitData = () => ({ crit: !!crit });
					}
					const relay = cond.onAnyModifyDamage.call(ctx, 100, attacker, defender, move);
					rows.push(['S', screen, initial, category, crit, infiltrates, sameSide, targetIsSource, reflect, lightscreen,
						Math.round(ctx.event.modifier * 4096), rel(relay)].join('\t'));
				}
			}
		}
	}
}
const tailwind = Dex.moves.get('tailwind').condition;
for (const initial of [4096, 6144, 8192]) {
	const ctx = { event: { modifier: initial / 4096 }, trunc: oracle.trunc, chainModify, debug() {} };
	const relay = tailwind.onModifySpe.call(ctx, 100, {});
	rows.push(['T', initial, Math.round(ctx.event.modifier * 4096), rel(relay)].join('\t'));
}
for (const [name, cond, params] of [
	['reflect', Dex.moves.get('reflect').condition, 3],
	['lightscreen', Dex.moves.get('lightscreen').condition, 3],
	['auroraveil', Dex.moves.get('auroraveil').condition, 3],
	['tailwind', tailwind, 3],
	['trickroom', Dex.moves.get('trickroom').condition, 2],
]) {
	for (const hasSource of [0, 1]) for (const clay of [0, 1]) {
		const ctx = { add() {}, debug() {} };
		const source = hasSource ? { hasItem: i => !!clay && i === 'lightclay', hasAbility: () => false } : undefined;
		const duration = params === 3 ? cond.durationCallback.call(ctx, {}, source, null) : cond.durationCallback.call(ctx, source, null);
		rows.push(['D', name, hasSource, clay, duration].join('\t'));
	}
}
const dest = fileURLToPath(new URL('../../../crates/engine/src/effects/conditions/hazardsscreens/vectors.tsv', import.meta.url));
fs.writeFileSync(dest, rows.join('\n') + '\n');
console.log(`wrote ${rows.length - 1} vectors to ${dest}`);
