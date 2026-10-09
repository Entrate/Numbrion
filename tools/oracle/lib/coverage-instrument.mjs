// Handler-coverage instrumentation of the pinned Showdown build (never modifies the checkout).
//
// How it works. Showdown's effect callbacks (`onStart`, `onModifyDamage`, `secondary.onHit`, ...) live in
// plain data tables (`Dex.data.Abilities/Items/Moves/Conditions/Rulesets`). Before any Dex object is built
// (the Dex deep-freezes every object it creates) this module walks the in-scope entries of those tables and
// replaces each callback by a counting wrapper that forwards `this` and all arguments untouched:
//
//     obj.onStart = function (...args) { counters[id]++; return orig.apply(this, args); }
//
// Every invocation path of the simulator (singleEvent, runEvent, priorityEvent, fieldEvent, eachEvent,
// direct calls such as move.basePowerCallback or condition.durationCallback) therefore ends up in the
// counter of the exact callback that ran, which is more precise than wrapping Battle.runEvent/singleEvent
// (those also visit handlers that are suppressed, filtered or skipped afterwards).
//
// The hook universe is built after wrapping by walking the Dex objects of all in-scope effects with the
// SAME walk as tools/codegen/gen-dex.mjs (`walkHooks`), i.e. the universe is exactly the Rust engine's
// generated HOOKS manifest (function sites, constant sites, absent ordering-only sites). Aliases (the same
// function reachable as `secondary.onHit` and `secondaries.0.onHit`, or a rule that is also a condition) map
// to one counter. Callbacks inherited by several species from one Conditions entry (Arceus-*, Zacian,
// Zamazenta) are attributed to `pokemon.baseSpecies` at call time.
//
// Constant hooks (`onCriticalHit: false`, `onLockMove: 'recharge'`) cannot be wrapped; they are counted when
// Battle.getCallback / singleEvent resolves them ("collected", an upper bound for "handled").
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { loadSim } from './common.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
export const REPO_ROOT = path.resolve(here, '../../..');
export const KINDS = ['species', 'moves', 'abilities', 'items', 'conditions', 'rules'];

export function loadScope() {
	return JSON.parse(fs.readFileSync(path.join(REPO_ROOT, 'data/scope.json'), 'utf8'));
}

export function loadBatches() {
	return JSON.parse(fs.readFileSync(path.join(REPO_ROOT, 'docs/design/EFFECT-BATCHES.json'), 'utf8'));
}

// ---------------------------------------------------------------------------------------
// The hook walk (copy of tools/codegen/gen-dex.mjs decode/walkHooks; keep in sync)
// ---------------------------------------------------------------------------------------

const DIRECT_KEYS = new Set(['basePowerCallback', 'damageCallback', 'durationCallback', 'beforeMoveCallback', 'beforeTurnCallback', 'priorityChargeCallback', 'effect']);
const NESTED_KEYS = ['self', 'selfBoost', 'secondary', 'secondaries', 'fling'];

/** Calls `visit({key, site, value, synthetic})` for every hook site of an effect object. */
export function walkHooks(obj, visit, site = '', emitted = new Set()) {
	if (!obj || typeof obj !== 'object') return;
	for (const [k, v] of Object.entries(obj)) {
		if (k === 'condition') continue;
		const on = /^on[A-Z]/.test(k) && k !== 'onPlate';
		const suffix = /(?:SubOrder|Order|Priority)$/.test(k);
		const metadata = on && suffix && Object.keys(obj).some(x => x !== k && k === x + 'Priority' || k === x + 'Order' || k === x + 'SubOrder');
		const isMeta = on && (metadata || /(?:SubOrder|Order)$/.test(k) || (k.endsWith('Priority') && k !== 'onModifyPriority' && k !== 'onFractionalPriority'));
		if ((on && !isMeta && v !== undefined) || DIRECT_KEYS.has(k) && typeof v === 'function') {
			emitted.add(`${site}\u0000${k}`);
			visit({ key: k, site, value: v, synthetic: false });
		} else if (isMeta) {
			const base = k.replace(/(?:SubOrder|Order|Priority)$/, '');
			if (obj[base] === undefined && !emitted.has(`${site}\u0000${base}`)) {
				emitted.add(`${site}\u0000${base}`);
				visit({ key: base, site, value: undefined, synthetic: true });
			}
		}
		if (v && typeof v === 'object' && NESTED_KEYS.includes(k)) {
			if (Array.isArray(v)) v.forEach((x, i) => walkHooks(x, visit, site ? `${site}.${k}.${i}` : `${k}.${i}`, emitted));
			else walkHooks(v, visit, site ? `${site}.${k}` : k, emitted);
		}
	}
}

export const hookLabel = (effect, site, key) => `${effect} ${site ? `${site}.` : ''}${key}`;

// ---------------------------------------------------------------------------------------
// Instrumentation
// ---------------------------------------------------------------------------------------

const WRAP_NESTED = new Set(NESTED_KEYS);
const isHookName = k => /^on[A-Z]/.test(k) || DIRECT_KEYS.has(k);

/**
 * Loads the simulator, wraps the in-scope callbacks and returns the counting API.
 * `counts()` returns a plain snapshot; `reset()` zeroes everything.
 */
export function instrument(psPath, scope = loadScope()) {
	const sim = loadSim(psPath);
	const req = createRequire(path.join(sim.root, 'package.json'));
	const { Dex } = req('./dist/sim');
	const dex = Dex.forFormat('gen9randomdoublesbattle');
	const data = dex.data; // loads the tables; no Dex effect objects exist yet
	if (Object.isFrozen(data.Moves.protect)) throw new Error('Dex data was already frozen; instrument() must run before any Dex.get()');

	let counters = new Float64Array(4096);
	let nextId = 0;
	const speciesCounts = new Map(); // `species:<id>\u0000<key>` -> n
	const constCounts = new Map(); // `kind:id\u0000key` -> n
	const wrappedIds = []; // cid -> label of the raw site (debugging)

	const wrap = (fn, label, speciesDynamic) => {
		if (fn.__cov !== undefined) return fn;
		const id = nextId++;
		if (id >= counters.length) {
			const bigger = new Float64Array(counters.length * 2);
			bigger.set(counters);
			counters = bigger;
		}
		wrappedIds[id] = label;
		let wrapper;
		if (speciesDynamic) {
			const key = label.slice(label.lastIndexOf(' ') + 1);
			wrapper = function (...args) {
				let mon = null;
				for (const a of args) if (a && typeof a === 'object' && a.baseSpecies && a.baseSpecies.id) { mon = a; break; }
				const k = `species:${mon ? mon.baseSpecies.id : '?'}\u0000${key}`;
				speciesCounts.set(k, (speciesCounts.get(k) || 0) + 1);
				counters[id]++;
				return fn.apply(this, args);
			};
		} else {
			wrapper = function (...args) {
				counters[id]++;
				return fn.apply(this, args);
			};
		}
		Object.defineProperty(wrapper, '__cov', { value: id });
		return wrapper;
	};

	const seen = new WeakSet();
	const wrapObject = (obj, label, speciesDynamic = false) => {
		if (!obj || typeof obj !== 'object' || seen.has(obj)) return;
		seen.add(obj);
		for (const k of Object.keys(obj)) {
			const v = obj[k];
			if (k === 'condition') continue; // separate effect
			if (typeof v === 'function' && isHookName(k)) {
				obj[k] = wrap(v, `${label} ${k}`, speciesDynamic);
			} else if (v && typeof v === 'object' && WRAP_NESTED.has(k)) {
				if (Array.isArray(v)) v.forEach((x, i) => wrapObject(x, `${label} ${k}.${i}`, speciesDynamic));
				else wrapObject(v, `${label} ${k}`, speciesDynamic);
			}
		}
	};

	const speciesConditionIds = new Set(scope.species.filter(s => s.speciesCondition).map(s => s.speciesCondition.id));
	const tables = { moves: data.Moves, abilities: data.Abilities, items: data.Items };
	for (const kind of ['moves', 'abilities', 'items']) {
		for (const e of scope[kind]) {
			const raw = tables[kind][e.id];
			if (raw) wrapObject(raw, `${kind}:${e.id}`);
		}
	}
	// Conditions resolve in Dex.conditions.getByID order: Rulesets, Conditions table, Moves/Abilities/Items `.condition`.
	for (const e of scope.conditions) {
		const id = e.id;
		let raw;
		if (data.Rulesets[id]) raw = data.Rulesets[id];
		else if (data.Conditions[id]) raw = data.Conditions[id];
		else raw = [data.Moves[id], data.Abilities[id], data.Items[id]].find(x => x && x.condition)?.condition;
		if (raw) wrapObject(raw, `conditions:${id}`);
	}
	for (const e of scope.rules) {
		if (data.Rulesets[e.id]) wrapObject(data.Rulesets[e.id], `rules:${e.id}`);
	}
	for (const id of speciesConditionIds) {
		if (data.Conditions[id]) wrapObject(data.Conditions[id], `speciesCondition:${id}`, true);
	}

	// Constant hooks: count when the dispatcher resolves them.
	const effectKind = effect => {
		switch (effect && effect.effectType) {
		case 'Ability': return 'abilities';
		case 'Item': return 'items';
		case 'Move': return 'moves';
		case 'Pokemon': return 'species';
		case 'Format': case 'Rule': case 'ValidatorRule': return 'rules';
		default: return 'conditions';
		}
	};
	const noteConst = (effect, name) => {
		if (!effect || !effect.id) return;
		const k = `${effectKind(effect)}:${effect.id}\u0000${name}`;
		constCounts.set(k, (constCounts.get(k) || 0) + 1);
	};
	const proto = sim.Battle.prototype;
	if (!proto.__covPatched) {
		const origGet = proto.getCallback;
		proto.getCallback = function (target, effect, callbackName) {
			const cb = origGet.call(this, target, effect, callbackName);
			if (cb !== undefined && typeof cb !== 'function') noteConst(effect, callbackName);
			return cb;
		};
		const origSingle = proto.singleEvent;
		proto.singleEvent = function (eventid, effect, ...rest) {
			if (effect && typeof effect === 'object') {
				const cb = effect[`on${eventid}`];
				if (cb !== undefined && typeof cb !== 'function') noteConst(effect, `on${eventid}`);
			}
			return origSingle.call(this, eventid, effect, ...rest);
		};
		Object.defineProperty(proto, '__covPatched', { value: true });
	}

	return {
		sim, dex, wrappedIds,
		reset() {
			counters.fill(0);
			speciesCounts.clear();
			constCounts.clear();
		},
		/** Non-zero counters as plain data (structured-clone friendly). */
		snapshot() {
			const fn = [];
			for (let i = 0; i < nextId; i++) if (counters[i]) fn.push([i, counters[i]]);
			return { fn, species: [...speciesCounts], consts: [...constCounts] };
		},
		wrapperCount: () => nextId,
	};
}

// ---------------------------------------------------------------------------------------
// The universe (built from the wrapped Dex; main thread only, needs the same module instance as instrument())
// ---------------------------------------------------------------------------------------

/**
 * Returns the hook universe of the in-scope effects:
 *   hooks: [{ effect, key, site, label, kind: 'fn'|'const'|'absent', cid, speciesDynamic }]
 * `cid` is the wrapper counter id for function hooks (null when a function was not instrumented).
 */
export function buildUniverse(inst, scope = loadScope()) {
	const { dex } = inst;
	const hooks = [];
	const getters = {
		species: id => dex.species.get(id),
		moves: id => dex.moves.get(id),
		abilities: id => dex.abilities.get(id),
		items: id => dex.items.get(id),
		conditions: id => dex.conditions.get(id),
		rules: id => dex.formats.get(id),
	};
	for (const kind of KINDS) {
		const ids = scope[kind].map(e => e.id).sort();
		for (const id of ids) {
			const effect = `${kind}:${id}`;
			const obj = getters[kind](id);
			walkHooks(obj, ({ key, site, value, synthetic }) => {
				const h = { effect, key, site, label: hookLabel(effect, site, key), kind: 'absent', cid: null, speciesDynamic: false };
				if (!synthetic) {
					if (typeof value === 'function') {
						h.kind = 'fn';
						h.cid = value.__cov === undefined ? null : value.__cov;
						h.speciesDynamic = kind === 'species' && value.__cov !== undefined && inst.wrappedIds[value.__cov].startsWith('speciesCondition:');
					} else h.kind = 'const';
				}
				hooks.push(h);
			});
		}
	}
	return { hooks };
}

/** Converts a worker snapshot into counts per hook label. */
export function countsByLabel(universe, snap) {
	const byCid = new Map(snap.fn);
	const species = new Map(snap.species);
	const consts = new Map(snap.consts);
	const out = new Map();
	for (const h of universe.hooks) {
		let n = 0;
		if (h.kind === 'fn') {
			if (h.speciesDynamic) n = species.get(`${h.effect}\u0000${h.key}`) || 0;
			else if (h.cid !== null) n = byCid.get(h.cid) || 0;
		} else if (h.kind === 'const') {
			n = consts.get(`${h.effect}\u0000${h.site ? `${h.site}.` : ''}${h.key}`) || 0;
		}
		if (n) out.set(h.label, n);
	}
	return out;
}
