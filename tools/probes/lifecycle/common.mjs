// Shared helpers for the lifecycle (owner L) oracle probes.
import fs from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {loadSim, FORMAT_ID} from '../../oracle/lib/common.mjs';

export const here = path.dirname(fileURLToPath(import.meta.url));
export const root = path.resolve(here, '../../..');
export const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
export {FORMAT_ID};

/** mulberry32: a tiny deterministic generator for choosing cases (not the battle PRNG). */
export function rng(seed) {
	let a = seed >>> 0;
	return () => {
		a = (a + 0x6D2B79F5) >>> 0;
		let t = a;
		t = Math.imul(t ^ (t >>> 15), t | 1);
		t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
		return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
	};
}

const scope = JSON.parse(fs.readFileSync(path.join(root, 'data/scope.json'), 'utf8'));
/** Abilities with no callbacks at all: they cannot fire in any lifecycle flow. */
export const INERT_ABILITIES = scope.abilities.filter(a => a.fnCount === 0 && !(a.handlers || []).length).map(a => a.id);
export const INERT_MOVES = scope.moves.filter(m => m.fnCount === 0).map(m => m.id);
const SKIP_SPECIES = /^(arceus|zacian|zamazenta)/;
export let SPECIES = scope.species.filter(s => s.inTeams && !SKIP_SPECIES.test(s.id)).map(s => ({id: s.id, name: s.name}));
/** Drop species whose Terastallization or faint needs forme changes (owner D's formeChange). */
export function withoutFormeTera() {
	SPECIES = SPECIES.filter(s => !/^(ogerpon|terapagos|morpeko)/.test(s.id));
}
export const TYPES = ['Normal', 'Fighting', 'Flying', 'Poison', 'Ground', 'Rock', 'Bug', 'Ghost', 'Steel', 'Fire', 'Water', 'Grass', 'Electric', 'Psychic', 'Ice', 'Dragon', 'Dark', 'Fairy', 'Stellar'];

export function pick(r, list) {
	return list[Math.floor(r() * list.length)];
}

/** A random set; `base` fixes fields so that exact speed ties are likely. */
export function randomSet(r, base = {}) {
	const species = base.species || pick(r, SPECIES);
	const moves = [];
	const n = 1 + Math.floor(r() * 4);
	while (moves.length < n) {
		const m = pick(r, INERT_MOVES);
		if (!moves.includes(m)) moves.push(m);
	}
	return {
		species,
		ability: base.ability || pick(r, INERT_ABILITIES),
		moves: base.moves || moves,
		level: base.level || pick(r, [100, 100, 100, 90, 88, 50]),
		tera: base.tera || pick(r, TYPES),
		gender: base.gender ?? pick(r, ['', '', 'M', 'F']),
	};
}

export function pack(set) {
	return `|${set.species.id}||${set.ability}|${set.moves.join(',')}|Serious||${set.gender}|||${set.level}|,,,,,${set.tera}`;
}

/** Two teams of 4-6 sets, with duplicated sets so that speeds tie across and within sides. */
export function randomTeams(r) {
	const teams = [[], []];
	for (const team of teams) {
		const size = 4 + Math.floor(r() * 3);
		while (team.length < size) {
			const roll = r();
			const all = teams[0].concat(teams[1]);
			if (roll < 0.35 && all.length) team.push({...pick(r, all)});
			else team.push(randomSet(r));
		}
	}
	return teams;
}

export function monIndex(p) {
	return p ? p.side.n * 6 + p.side.team.indexOf(p.set) : '-';
}

export function seedWords(b) {
	return String(b.prng.getSeed()).replace(/^gen5,/, '');
}

export const STOP = Symbol('stop');
/** The pinned, un-intercepted endTurn. */
export const realEndTurn = sim.Battle.prototype.endTurn;

/** A Battle whose second setPlayer runs the real start() until the first endTurn, then stops. */
export function startBattle(seed, packed, hooks = {}) {
	const b = new sim.Battle({formatid: FORMAT_ID, seed, send: () => {}});
	b.endTurn = function () {
		if (hooks.beforeEndTurn) hooks.beforeEndTurn(this);
		throw STOP;
	};
	if (hooks.afterAction) {
		const original = b.runAction;
		b.runAction = function (action) {
			const choice = action.choice;
			const result = original.call(this, action);
			hooks.afterAction(this, choice);
			return result;
		};
	}
	b.setPlayer('p1', {name: 'A', team: packed[0]});
	try {
		b.setPlayer('p2', {name: 'B', team: packed[1]});
	} catch (e) {
		if (e !== STOP) throw e;
	}
	return b;
}

export function queueSummary(b) {
	return b.queue.list.map(a => `${a.choice}:${monIndex(a.pokemon)}:${a.order}:${a.priority || 0}:${a.speed}`).join('/') || '-';
}

export function stateSummary(b) {
	const active = b.sides.flatMap(s => s.active.map(p => monIndex(p))).join(',');
	const party = b.sides.map(s => s.pokemon.map(p => monIndex(p)).join(',')).join(';');
	const speeds = b.sides.flatMap(s => s.pokemon.map(p => p.speed)).join(',');
	const orders = b.sides.flatMap(s => s.pokemon.map(p => `${p.abilityState.effectOrder}.${p.itemState.effectOrder}`)).join(',');
	return {active, party, speeds, orders, seed: seedWords(b), effectOrder: b.effectOrder, speedOrder: b.speedOrder.join(','), queue: queueSummary(b)};
}
