// Content database of the directed corpus generator: everything is derived from data/scope.json (in-scope
// species/moves/abilities/items/conditions with their handler lists and the observed team-generator sets) and
// docs/design/EFFECT-BATCHES.json. Nothing here is a hand-maintained per-effect table; the only literal lists
// are the capability vocabularies that profiles use to say what "declarative" means (see directed-profiles.mjs).
import { KINDS, loadBatches, loadScope } from './coverage-instrument.mjs';

export { KINDS };

const toID = s => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');

// ---------------------------------------------------------------------------------------
// Moves
// ---------------------------------------------------------------------------------------

/** A move is hook-free when scope.json lists no function/constant handler, nested hook or condition for it. */
export const isHookFree = e => !e.handlers.length && !e.nested.length && !Object.keys(e.constHandlers).length && !e.condition;

/**
 * Capability tokens of a move's declarative content (its `data` keys and secondaries). Profiles allow a hook-free
 * move iff its tokens are a subset of the profile's allowed tokens, e.g. `d:boosts` (primary boosts),
 * `d:status:par`, `d:self:boosts`, `sec:status:brn`, `sec:vol:flinch`, `d:drain`, `d:recoil`.
 */
export function moveCaps(m) {
	const caps = new Set();
	for (const [k, v] of Object.entries(m.data)) {
		if (k === 'status') caps.add(`d:status:${v}`);
		else if (k === 'volatileStatus') caps.add(`d:vol:${v}`);
		else if (k === 'weather') caps.add(`d:weather:${v}`);
		else if (k === 'self' || k === 'selfBoost') {
			caps.add(`d:${k}`);
			for (const [kk, vv] of Object.entries(v)) caps.add(kk === 'volatileStatus' ? `d:${k}:vol:${vv}` : kk === 'chance' ? 'd:self:chance' : `d:${k}:${kk}`);
		} else caps.add(`d:${k}`);
	}
	for (const s of m.secondaries || []) {
		for (const [k, v] of Object.entries(s)) {
			if (k === 'chance') continue;
			if (k === 'status') caps.add(`sec:status:${v}`);
			else if (k === 'volatileStatus') caps.add(`sec:vol:${v}`);
			else if (k === 'self') {
				caps.add('sec:self');
				for (const [kk, vv] of Object.entries(v)) caps.add(kk === 'volatileStatus' ? `sec:self:vol:${vv}` : `sec:self:${kk}`);
			} else caps.add(`sec:${k}`);
		}
		if (!Object.keys(s).filter(k => k !== 'chance').length) caps.add('sec:empty');
	}
	return caps;
}

const SPREAD_TARGETS = new Set(['allAdjacent', 'allAdjacentFoes']);

function buildMove(e) {
	return {
		id: e.id, name: e.name, type: e.type, category: e.category, basePower: e.basePower, priority: e.priority,
		target: e.target, flags: e.flags || {}, multihit: e.multihit, critRatio: e.critRatio, accuracy: e.accuracy,
		damaging: e.category !== 'Status', spread: SPREAD_TARGETS.has(e.target), free: isHookFree(e), caps: moveCaps(e), entry: e,
	};
}

// ---------------------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------------------

let cached = null;

export function loadDirectedData() {
	if (cached) return cached;
	const scope = loadScope();
	const batches = loadBatches();
	const moves = new Map(scope.moves.map(m => [m.id, buildMove(m)]));
	const abilities = new Map(scope.abilities.map(a => [a.id, {
		id: a.id, name: a.name, entry: a, hookFree: isHookFree(a), engineRefs: a.engineRefs,
	}]));
	const items = new Map(scope.items.map(i => [i.id, {
		id: i.id, name: i.name, entry: i, hookFree: isHookFree(i), itemUser: i.data.itemUser ? i.data.itemUser.map(toID) : null,
		forcedForme: i.data.forcedForme ?? null, isBerry: i.isBerry, isChoice: i.isChoice,
	}]));

	const wmap = o => Object.entries(o || {}).map(([k, n]) => [k, n]);
	const species = new Map();
	for (const s of scope.species) {
		if (!s.generatorKey || !s.inTeams || !s.teamgen) continue; // only what the random-doubles generator can produce
		const tg = s.teamgen;
		species.set(s.id, {
			id: s.id, name: s.name, baseSpecies: s.baseSpecies, baseId: toID(s.baseSpecies), types: s.types, baseStats: s.baseStats,
			gender: s.gender, level: tg.declaredLevel,
			formes: wmap(tg.formes), abilities: wmap(tg.abilities), items: wmap(tg.items), moves: wmap(tg.moves), teraTypes: wmap(tg.teraTypes),
			abilitySet: new Set(Object.keys(tg.abilities)), itemSet: new Set(Object.keys(tg.items)), moveSet: new Set(Object.keys(tg.moves)),
			speciesCondition: s.speciesCondition ? s.speciesCondition.id : null,
			requiredItem: s.requiredItem, requiredItems: s.requiredItems, requiredAbility: s.requiredAbility, requiredMove: s.requiredMove,
			requiredTeraType: s.requiredTeraType, battleOnly: s.battleOnly, entry: s,
		});
	}

	// Effects (kind:id) of every batch, and the batch of every effect.
	const batchEffects = new Map(); // batch name -> { keys, byKind }
	for (const g of batches.groups) {
		const byKind = Object.fromEntries(KINDS.map(k => [k, []]));
		for (const key of g.keys) {
			const [k, id] = [key.slice(0, key.indexOf(':')), key.slice(key.indexOf(':') + 1)];
			byKind[k].push(id);
		}
		batchEffects.set(g.batch, { name: g.batch, description: g.description, keys: g.keys, byKind });
	}

	// Creators of every condition, from scope.json `via` ("moves:protect (own condition)", "abilities:flamebody.onDamagingHit",
	// "moves:blueflare.secondary.status", ...). Conditions created by other conditions or by the engine have no creator here.
	const creators = new Map();
	for (const c of scope.conditions) {
		const set = new Set();
		for (const v of c.via || []) {
			const m = /^(moves|abilities|items):([a-z0-9]+)/.exec(v);
			if (m) set.add(`${m[1]}:${m[2]}`);
		}
		creators.set(c.id, set);
	}

	cached = { scope, batches, moves, abilities, items, species, batchEffects, creators };
	return cached;
}

export function effectKey(kind, id) {
	return `${kind}:${id}`;
}

export function splitKey(key) {
	const i = key.indexOf(':');
	return [key.slice(0, i), key.slice(i + 1)];
}

/** Resolves `effect:<spec>`: `kind:id`, or a bare id when it names exactly one in-scope effect with hooks or content. */
export function resolveEffect(data, spec) {
	if (/^[a-z]+:/.test(spec) && KINDS.includes(spec.slice(0, spec.indexOf(':')))) {
		const [k, id] = splitKey(spec);
		if (!data.scope[k].some(e => e.id === id)) throw new Error(`no in-scope ${k} effect ${id}`);
		return spec;
	}
	const hits = KINDS.filter(k => k !== 'rules' && data.scope[k].some(e => e.id === spec));
	// A bare id naming both a move and its condition (protect) means the move.
	const pref = ['moves', 'abilities', 'items', 'species', 'conditions'];
	const ordered = pref.filter(k => hits.includes(k));
	if (!ordered.length) throw new Error(`unknown effect ${spec}`);
	if (ordered.length > 1 && !(ordered.length === 2 && ordered[1] === 'conditions')) {
		throw new Error(`ambiguous effect ${spec}: ${ordered.map(k => `${k}:${spec}`).join(', ')}`);
	}
	return `${ordered[0]}:${spec}`;
}
