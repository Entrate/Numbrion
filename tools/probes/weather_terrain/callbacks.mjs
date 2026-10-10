// Callback-level oracle vectors for the weather_terrain batch: runs the pinned Showdown handlers
// (data/{abilities,moves,conditions}.ts compiled in dist/) against stub battle contexts and writes
// every (situation -> relay, final modifier, move edits) row to callbacks.tsv for the Rust tests.
//
//   node tools/probes/weather_terrain/callbacks.mjs [PS_PATH] [--out FILE]
//
// The stubs implement exactly the queries a handler makes: field.isWeather/isTerrain,
// pokemon.effectiveWeather/isGrounded/isSemiInvulnerable/hasType/maxhp, chainModify/modify/trunc
// from the real Battle prototype. `usesTerrain` records whether isTerrain was consulted (those
// rows need the unimplemented core Field::isTerrain on the Rust side).
import fs from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { loadSim } from '../../oracle/lib/common.mjs';

const args = process.argv.slice(2);
const outIdx = args.indexOf('--out');
const outPath = outIdx >= 0 ? args.splice(outIdx, 2)[1] : fileURLToPath(
	new URL('../../../crates/engine/src/effects/abilities/weatherterrain/callbacks.tsv', import.meta.url));
const sim = loadSim(args[0] || `${process.env.HOME}/src/pokemon-showdown`);
const { Dex } = createRequire(sim.root + '/package.json')('./dist/sim');
const oracle = new sim.Battle({ formatid: 'gen9randomdoublesbattle', seed: '1,2,3,4' });

const WEATHERS = ['', 'sunnyday', 'raindance', 'sandstorm', 'snowscape'];
const TERRAINS = ['', 'electricterrain', 'grassyterrain', 'psychicterrain'];
const MOVE_TYPES = ['Normal', 'Fire', 'Water', 'Rock', 'Ground', 'Steel', 'Electric', 'Grass', 'Psychic', 'Ice', 'Flying'];

const fnOf = (ref, key) => {
	const [kind, id] = ref.split(':');
	let obj;
	if (kind === 'abilities') obj = Dex.abilities.get(id);
	else if (kind === 'moves') obj = Dex.moves.get(id);
	else if (kind === 'conditions') obj = Dex.conditions.get(id);
	else if (kind === 'terrain') obj = Dex.moves.get(id).condition;
	const fn = obj[key];
	if (typeof fn !== 'function') throw new Error(`${ref}.${key} is not a function`);
	return fn;
};

// [ref, hook, dimension overrides]. Dimensions: weather, terrain, userTypes, userSemi, foeTypes,
// foeSemi, moveType, moveId, input, imm (status-immunity name), noTarget.
const D = {
	weather: [''], terrain: [''], userTypes: ['Normal'], userSemi: [0], foeTypes: ['Normal'], foeSemi: [0],
	moveType: ['Normal'], moveId: ['thunderbolt'], input: [100], imm: [''], noTarget: [0],
};
const SPECS = [
	['abilities:chlorophyll', 'onModifySpe', { weather: WEATHERS, input: [100, 1] }],
	['abilities:swiftswim', 'onModifySpe', { weather: WEATHERS, input: [100, 1] }],
	['abilities:sandrush', 'onModifySpe', { weather: WEATHERS, input: [100, 1] }],
	['abilities:slushrush', 'onModifySpe', { weather: WEATHERS, input: [100, 1] }],
	['abilities:surgesurfer', 'onModifySpe', { terrain: TERRAINS, input: [100, 1] }],
	['abilities:solarpower', 'onModifySpA', { weather: WEATHERS, input: [100, 7] }],
	['abilities:orichalcumpulse', 'onModifyAtk', { weather: WEATHERS, input: [100, 7] }],
	['abilities:hadronengine', 'onModifySpA', { terrain: TERRAINS, input: [100, 7] }],
	['abilities:sandforce', 'onBasePower', { weather: WEATHERS, moveType: MOVE_TYPES, input: [60] }],
	['abilities:sandforce', 'onImmunity', { imm: ['sandstorm', 'hail', 'frz', 'par'] }],
	['abilities:sandrush', 'onImmunity', { imm: ['sandstorm', 'hail', 'frz', 'par'] }],
	['abilities:icebody', 'onImmunity', { imm: ['sandstorm', 'hail', 'frz', 'par'] }],
	['conditions:sunnyday', 'onImmunity', { weather: WEATHERS, imm: ['sandstorm', 'hail', 'frz', 'par'] }],
	['conditions:raindance', 'onWeatherModifyDamage', { weather: WEATHERS, moveType: MOVE_TYPES, foeTypes: ['Normal'], input: [100, 77] }],
	['conditions:sunnyday', 'onWeatherModifyDamage', { weather: WEATHERS, moveType: MOVE_TYPES, input: [100, 77] }],
	['conditions:sandstorm', 'onModifySpD', { weather: WEATHERS, userTypes: ['Normal', 'Rock', 'Ice', 'Rock,Ground'], input: [100, 191] }],
	['conditions:snowscape', 'onModifyDef', { weather: WEATHERS, userTypes: ['Normal', 'Rock', 'Ice', 'Ice,Flying'], input: [100, 191] }],
	['terrain:electricterrain', 'onBasePower', { moveType: MOVE_TYPES, userTypes: ['Normal', 'Flying', 'Electric'], userSemi: [0, 1], input: [60] }],
	['terrain:grassyterrain', 'onBasePower', { moveType: MOVE_TYPES, moveId: ['thunderbolt', 'earthquake'], userTypes: ['Normal', 'Flying'], foeTypes: ['Normal', 'Flying'], foeSemi: [0, 1], input: [60] }],
	['terrain:psychicterrain', 'onBasePower', { moveType: MOVE_TYPES, userTypes: ['Normal', 'Flying'], userSemi: [0, 1], input: [60] }],
	['moves:expandingforce', 'onBasePower', { terrain: TERRAINS, userTypes: ['Normal', 'Flying'], input: [80] }],
	['moves:expandingforce', 'onModifyMove', { terrain: TERRAINS, userTypes: ['Normal', 'Flying'] }],
	['moves:psyblade', 'onBasePower', { terrain: TERRAINS, userTypes: ['Normal', 'Flying'], input: [80] }],
	['moves:grassyglide', 'onModifyPriority', { terrain: TERRAINS, userTypes: ['Normal', 'Flying'], input: [0, 1] }],
	['moves:bleakwindstorm', 'onModifyMove', { weather: WEATHERS, noTarget: [0, 1] }],
	['moves:sandsearstorm', 'onModifyMove', { weather: WEATHERS, noTarget: [0, 1] }],
	['moves:wildboltstorm', 'onModifyMove', { weather: WEATHERS, noTarget: [0, 1] }],
	['moves:hurricane', 'onModifyMove', { weather: WEATHERS, noTarget: [0, 1] }],
	['moves:thunder', 'onModifyMove', { weather: WEATHERS, noTarget: [0, 1] }],
	['moves:blizzard', 'onModifyMove', { weather: WEATHERS }],
	['moves:weatherball', 'onModifyType', { weather: WEATHERS }],
	['moves:weatherball', 'onModifyMove', { weather: WEATHERS }],
];

function product(dims) {
	let rows = [{}];
	for (const [k, vals] of Object.entries(dims)) rows = rows.flatMap(r => vals.map(v => ({ ...r, [k]: v })));
	return rows;
}

function monStub(types, weather, semi) {
	const t = types.split(',');
	return {
		maxhp: 300, baseMaxhp: 300,
		hasType: x => t.includes(x),
		effectiveWeather: () => weather,
		isGrounded: () => !t.includes('Flying'),
		isSemiInvulnerable: () => !!semi,
	};
}

const rows = [];
for (const [ref, hook, over] of SPECS) {
	const fn = fnOf(ref, hook);
	for (const c of product({ ...D, ...over })) {
		let usesTerrain = 0;
		const ctx = {
			event: { modifier: 1 },
			field: {
				isWeather: w => [].concat(w).includes(c.weather),
				isTerrain: t => { usesTerrain = 1; return [].concat(t).includes(c.terrain); },
			},
			trunc: oracle.trunc, chainModify: sim.Battle.prototype.chainModify, modify: sim.Battle.prototype.modify,
			debug() {}, add() {}, hint() {},
		};
		const user = monStub(c.userTypes, c.weather, c.userSemi);
		const foe = monStub(c.foeTypes, c.weather, c.foeSemi);
		const move = { id: c.moveId, type: c.moveType, accuracy: 70, basePower: 50, target: 'normal', priority: 0, category: 'Special' };
		let relay;
		switch (hook) {
		case 'onModifyMove': case 'onModifyType':
			relay = fn.call(ctx, move, user, c.noTarget ? undefined : foe); break;
		case 'onModifySpe': case 'onModifySpD': case 'onModifyDef':
			relay = fn.call(ctx, c.input, user); break;
		case 'onModifySpA': case 'onModifyAtk': case 'onBasePower': case 'onWeatherModifyDamage':
			// (value, attacker, defender, move); the attacker is the holder.
			relay = fn.call(ctx, c.input, user, foe, move); break;
		case 'onModifyPriority':
			relay = fn.call(ctx, c.input, user, foe, move); break;
		case 'onImmunity':
			relay = fn.call(ctx, c.imm, user); break;
		default: throw new Error(hook);
		}
		const mod = Math.round(ctx.event.modifier * 4096);
		rows.push([
			ref, hook, c.weather, c.terrain, c.userTypes, c.userSemi, c.foeTypes, c.foeSemi, c.moveType, c.moveId,
			c.input, c.imm, c.noTarget,
			relay === undefined ? 'undefined' : String(relay), mod,
			move.accuracy === true ? 'true' : move.accuracy, move.basePower, move.type, move.target, usesTerrain,
		].join('\t'));
	}
}
const header = [
	'# Callback oracle vectors from tools/probes/weather_terrain/callbacks.mjs (pinned Showdown). Do not edit by hand.',
	'# effect\thook\tweather\tterrain\tuserTypes\tuserSemi\tfoeTypes\tfoeSemi\tmoveType\tmoveId\tinput\timmunity\tnoTarget\trelay\tmodifier\taccuracy\tbasePower\tmoveTypeOut\ttargetOut\tusesTerrain',
];
fs.writeFileSync(outPath, header.concat(rows).join('\n') + '\n');
console.log(`Wrote ${rows.length} callback vectors to ${outPath}`);
