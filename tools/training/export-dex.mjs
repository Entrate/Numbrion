#!/usr/bin/env node
// Node 24, no packages. Exports the static game data the training encoder needs to data/training/dex.json.
// Usage: node tools/training/export-dex.mjs [path-to-pinned-showdown] [output]
// Vocabularies come from data/scope.json (everything random doubles can generate); properties, the type chart
// and the role-based sets come from the pinned Showdown checkout. The checkout is read-only.
import fs from 'node:fs';
import path from 'node:path';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
import {COMMIT, assertOraclePin} from '../codegen/oracle.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const PS = path.resolve(process.argv[2] || process.env.NUMBRION_SHOWDOWN || path.join(ROOT, 'scratch/pokemon-showdown'));
const OUT = path.resolve(process.argv[3] || path.join(ROOT, 'data/training/dex.json'));
assertOraclePin(PS);
const {Dex} = createRequire(path.join(PS, 'package.json'))('./dist/sim');
const dex = Dex.forFormat('gen9randomdoublesbattle');
const scope = JSON.parse(fs.readFileSync(path.join(ROOT, 'data/scope.json'), 'utf8'));
if (scope.meta.showdownCommit !== COMMIT) throw Error('Wrong scope commit');
const sets = JSON.parse(fs.readFileSync(path.join(PS, 'data/random-battles/gen9/doubles-sets.json'), 'utf8'));
const id = s => String(s || '').toLowerCase().replace(/[^a-z0-9]/g, '');

const TYPES = ['Normal', 'Fighting', 'Flying', 'Poison', 'Ground', 'Rock', 'Bug', 'Ghost', 'Steel', 'Fire', 'Water',
	'Grass', 'Electric', 'Psychic', 'Ice', 'Dragon', 'Dark', 'Fairy', 'Stellar'];
// typechart[attacking][defending]: Showdown damageTaken codes 0 neutral, 1 weak, 2 resist, 3 immune.
const MULT = [1, 2, 0.5, 0];
const typechart = TYPES.map(attack => TYPES.map(defend => {
	if (attack === 'Stellar' || defend === 'Stellar') return 1;
	return MULT[dex.types.get(defend).damageTaken[attack]];
}));

const share = counts => {
	const total = Object.values(counts || {}).reduce((a, b) => a + b, 0);
	return Object.fromEntries(Object.entries(counts || {}).sort((a, b) => b[1] - a[1]).map(([k, v]) => [k, v / total]));
};

const species = scope.species.map(s => {
	const t = s.teamgen || {};
	return {
		id: s.id, name: s.name, types: s.types, baseStats: s.baseStats, weightkg: s.weighthg / 10,
		battleOnly: s.battleOnly ? id(Array.isArray(s.battleOnly) ? s.battleOnly[0] : s.battleOnly) : null,
		level: t.declaredLevel || null, abilities: share(t.abilities), items: share(t.items),
		teraTypes: share(t.teraTypes),
		// Inclusion probability of each move: a Pokemon carries up to four.
		moves: Object.fromEntries(Object.entries(t.moves || {}).sort((a, b) => b[1] - a[1]).map(([k, v]) => [k, v / t.count])),
	};
});
// Cosmetic formes and team-generator formes appear in protocol details; map them onto their scope species.
const aliases = {};
for (const s of scope.species) {
	for (const forme of [...(s.cosmeticFormes || []), ...Object.keys(s.teamgen?.formes || {})]) {
		if (id(forme) !== s.id) aliases[id(forme)] = s.id;
	}
}

const moves = scope.moves.map(m => {
	const move = dex.moves.get(m.id);
	const secondaries = move.secondaries || [];
	return {
		id: m.id, name: m.name, type: m.type, category: m.category, basePower: m.basePower,
		accuracy: m.accuracy === true ? 101 : m.accuracy, priority: m.priority, target: m.target, pp: m.pp,
		flags: Object.keys(m.flags || {}).filter(f => m.flags[f]),
		multihit: m.multihit ?? null, critRatio: m.critRatio ?? 1, willCrit: !!move.willCrit,
		damage: move.damage ?? null, variablePower: !!(move.basePowerCallback || move.damageCallback),
		drain: move.drain ? move.drain[0] / move.drain[1] : 0, recoil: move.recoil ? move.recoil[0] / move.recoil[1] : 0,
		heal: move.heal ? move.heal[0] / move.heal[1] : 0,
		boosts: move.boosts || null, selfBoosts: move.self?.boosts || move.selfBoost?.boosts || null,
		status: move.status || null, volatileStatus: move.volatileStatus || null, sideCondition: move.sideCondition || null,
		weather: move.weather || null, terrain: move.terrain || null, pseudoWeather: move.pseudoWeather || null,
		stallingMove: !!move.stallingMove, selfSwitch: !!move.selfSwitch, forceSwitch: !!move.forceSwitch,
		breaksProtect: !!move.breaksProtect, selfdestruct: !!move.selfdestruct,
		secondaryChance: secondaries.reduce((a, s) => Math.max(a, (s.chance || 100) / 100), 0),
		secondaryStatus: secondaries.map(s => s.status).find(Boolean) || null,
		secondaryVolatile: secondaries.map(s => s.volatileStatus).find(Boolean) || null,
		secondaryBoosts: secondaries.map(s => s.boosts).find(Boolean) || null,
	};
});
const items = scope.items.map(i => ({id: i.id, name: i.name}));
const abilities = scope.abilities.map(a => ({id: a.id, name: a.name}));

const roleShare = {};
for (const s of scope.species) roleShare[s.id] = share(s.teamgen?.roles);
const doublesSets = {};
for (const [key, entry] of Object.entries(sets)) {
	doublesSets[key] = entry.sets.map(set => ({
		role: set.role, weight: roleShare[key]?.[set.role] ?? 1 / entry.sets.length,
		movepool: set.movepool.map(id), abilities: set.abilities.map(id), teraTypes: set.teraTypes || [],
	}));
}

const out = {
	meta: {showdownCommit: COMMIT, format: 'gen9randomdoublesbattle', generator: 'tools/training/export-dex.mjs'},
	types: TYPES, typechart, species, aliases, moves, items, abilities, sets: doublesSets,
};
fs.mkdirSync(path.dirname(OUT), {recursive: true});
fs.writeFileSync(OUT, JSON.stringify(out) + '\n');
console.log(`${OUT}: ${species.length} species, ${moves.length} moves, ${items.length} items, ` +
	`${abilities.length} abilities, ${Object.keys(doublesSets).length} set entries, ${Object.keys(aliases).length} aliases`);
