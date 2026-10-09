// Directed-corpus profiles: what content a battle may contain and which effects it is built around.
//
//   slice0              no items, handler-free ability, plain damaging moves
//   slice1              slice0 + secondaries, boosts, drain/recoil, statuses, flinch
//   slice1x             slice1 + heal, forced/self switches, self-destruct (declarative but switch/faint heavy)
//   slice2              slice1 + Protect, Leftovers, Sitrus Berry, Life Orb, Choice items, Intimidate, Pressure,
//                       Levitate, Regenerator, Adaptability + confusion moves ("next widen" of IMPLEMENTATION-PLAN.md)
//   batch:<name>        closed world: slice1 content + the effects of one EFFECT-BATCHES.json batch
//   effect:<kind:id>    closed world: slice1 content + one effect (and what creates it, if it is a condition)
//
// Any spec accepts `+tera` (allow Terastallization). Allow-lists are computed from data/scope.json and
// EFFECT-BATCHES.json; the literal lists here are only the capability vocabularies and the slice2 names that
// IMPLEMENTATION-PLAN.md spells out.
import { DEFAULT_KNOBS } from './choices.mjs';
import { KINDS, loadDirectedData, resolveEffect, splitKey } from './directed-data.mjs';

// ---------------------------------------------------------------------------------------
// Capability vocabularies (tokens of directed-data.mjs moveCaps)
// ---------------------------------------------------------------------------------------

/** Data keys that only parametrize the core damage pipeline (no extra state, no extra events). */
export const CORE_CAPS = [
	'd:willCrit', 'd:overrideOffensiveStat', 'd:overrideOffensivePokemon', 'd:overrideDefensiveStat', 'd:ignoreDefensive',
	'd:ignoreEvasion', 'd:ignoreImmunity', 'd:damage', 'd:thawsTarget', 'd:ignoreAbility', 'd:breaksProtect', 'd:smartTarget',
	'd:multiaccuracy',
];
const STATUSES = ['brn', 'par', 'slp', 'psn', 'tox', 'frz'];
/** slice1: "declarative moves with secondaries, primary boosts, drain/recoil, status-inflicting moves, flinch". */
export const SLICE1_CAPS = [
	...CORE_CAPS,
	'd:boosts', 'd:self', 'd:self:boosts', 'd:self:chance', 'd:selfBoost', 'd:selfBoost:boosts', 'sec:empty', 'd:drain', 'd:recoil',
	...STATUSES.map(s => `d:status:${s}`), ...STATUSES.map(s => `sec:status:${s}`),
	'sec:vol:flinch', 'sec:boosts', 'sec:self', 'sec:self:boosts',
];
/** slice1x: the remaining plain-data move features (healing, switching moves, self-destruct). */
export const SLICE1X_CAPS = [...SLICE1_CAPS, 'd:heal', 'd:forceSwitch', 'd:selfSwitch', 'd:selfdestruct'];
export const CONFUSION_CAPS = ['d:vol:confusion', 'sec:vol:confusion'];

/** Names from IMPLEMENTATION-PLAN.md "Next widen with ...". */
export const SLICE2 = {
	moves: ['protect'],
	abilities: ['intimidate', 'pressure', 'levitate', 'regenerator', 'adaptability'],
	items: ['leftovers', 'sitrusberry', 'lifeorb', 'choiceband', 'choicespecs', 'choicescarf'],
};

// ---------------------------------------------------------------------------------------
// Handler-free abilities that are inert unless certain moves exist: "inert families".
// Each lists the moves that would make the ability do something (Showdown engine references from scope.json
// `engineRefs`). A battle may only use a family when none of its trigger moves occurs in the battle.
// ---------------------------------------------------------------------------------------

const hasCap = (m, ...caps) => caps.some(c => m.caps.has(c));
const HAZARD_CONDITIONS = new Set(['spikes', 'toxicspikes', 'stealthrock', 'stickyweb']);
export const INERT_FAMILIES = {
	// sim/battle-actions.ts:323-343: a successful dance move makes every other active Dancer repeat it.
	dancer: { abilities: ['dancer'], trigger: m => !!m.flags.dance },
	// sim/pokemon.ts:1710: the holder may poison Poison/Steel types.
	corrosion: { abilities: ['corrosion'], trigger: m => hasCap(m, 'd:status:psn', 'd:status:tox', 'sec:status:psn', 'sec:status:tox') },
	// data/conditions.ts:68: Early Bird shortens the holder's sleep.
	earlybird: { abilities: ['earlybird'], trigger: m => hasCap(m, 'd:status:slp', 'sec:status:slp') || m.id === 'rest' || m.id === 'yawn' },
	// sim/pokemon.ts:2148-2251: Levitate only matters for Ground-type moves, hazards and terrain (isGrounded).
	levitate: {
		abilities: ['levitate'], always: true, // 23 of the 29 species with a handler-free ability: teams cannot be built without it
		trigger: m => m.type === 'Ground' || m.entry.data.terrain !== undefined || m.id.endsWith('terrain') || HAZARD_CONDITIONS.has(m.entry.data.sideCondition) ||
			HAZARD_CONDITIONS.has(m.id),
	},
};

// ---------------------------------------------------------------------------------------
// Profile resolution
// ---------------------------------------------------------------------------------------

export function parseSpec(spec) {
	const [base, ...flags] = spec.split('+');
	for (const f of flags) if (!['tera'].includes(f)) throw new Error(`unknown profile flag +${f}`);
	return { base, flags: new Set(flags) };
}

/** Run seed used when --seed is not given: fixed per profile so corpora are reproducible from their name. */
export function defaultRunSeed(spec, data = loadDirectedData()) {
	const { base } = parseSpec(spec);
	const fixed = { slice0: 1000, slice1: 1001, slice2: 1002, slice1x: 1003 };
	if (fixed[base] !== undefined) return fixed[base];
	if (base.startsWith('batch:')) {
		const names = [...data.batchEffects.keys()];
		const i = names.indexOf(base.slice(6));
		if (i >= 0) return 1100 + i;
	}
	let h = 2166136261;
	for (const c of base) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
	return 2000 + (h % 90000);
}

const capsOk = (m, allowed) => [...m.caps].every(c => allowed.has(c));

/** Moves a profile uses as filler: hook-free moves whose declarative content is within `caps`. */
function declarativeMoves(data, caps, { damagingOnly = false } = {}) {
	const allowed = new Set(caps);
	const out = new Set();
	for (const m of data.moves.values()) {
		if (!m.free) continue;
		if (damagingOnly && !m.damaging) continue;
		if (m.id === 'struggle') continue;
		if (!capsOk(m, allowed)) continue;
		out.add(m.id);
	}
	return out;
}

const abilityFamilies = (data, ids) => Object.entries(INERT_FAMILIES)
	.filter(([, f]) => f.abilities.every(a => data.abilities.has(a)))
	.map(([name, f]) => ({ name, always: false, ...f }));

export function makeProfile(spec, data = loadDirectedData()) {
	const { base, flags } = parseSpec(spec);
	const tera = flags.has('tera');
	const profile = {
		id: spec, base, kind: 'slice', description: '', tera,
		knobs: { ...DEFAULT_KNOBS, teraProb: tera ? DEFAULT_KNOBS.teraProb : 0 },
		fillerMoves: new Set(), abilities: new Set(), families: abilityFamilies(data), items: new Set(),
		fillerItems: new Set(), fillerItemChance: 0, focus: null, speciesAllow: new Set(), focusMoves: new Set(),
		worldMoves: new Set(), bias: { focusMoveWeight: 1, stallTurn: 40 },
	};
	if (base === 'slice0') {
		profile.description = 'no items; handler-free abilities only; plain damaging declarative moves; no Tera unless +tera';
		profile.fillerMoves = declarativeMoves(data, CORE_CAPS, { damagingOnly: true });
		profile.worldMoves = new Set(profile.fillerMoves);
	} else if (base === 'slice1') {
		profile.description = 'slice0 + declarative secondaries, boosts, drain/recoil, statuses and flinch (rules always on); no items';
		profile.fillerMoves = declarativeMoves(data, SLICE1_CAPS);
		profile.worldMoves = new Set(profile.fillerMoves);
	} else if (base === 'slice1x') {
		profile.description = 'slice1 + heal, forceSwitch/selfSwitch and self-destruct moves; no items';
		profile.fillerMoves = declarativeMoves(data, SLICE1X_CAPS);
		profile.worldMoves = new Set(profile.fillerMoves);
		profile.knobs.switchProb = 0.15;
	} else if (base === 'slice2') {
		profile.description = 'slice1 + Protect, confusion moves, Leftovers/Sitrus/Life Orb/Choice items, Intimidate/Pressure/Levitate/Regenerator/Adaptability';
		profile.fillerMoves = declarativeMoves(data, [...SLICE1_CAPS, ...CONFUSION_CAPS]);
		for (const id of SLICE2.moves) profile.fillerMoves.add(id);
		profile.worldMoves = new Set(profile.fillerMoves);
		profile.abilities = new Set(SLICE2.abilities);
		profile.items = new Set(SLICE2.items);
		profile.fillerItemChance = 0.7;
		profile.fillerItems = new Set(SLICE2.items);
		// Levitate is a regular ability here (Ground moves exist), so it is not an inert family.
		profile.families = profile.families.filter(f => f.name !== 'levitate');
		profile.bias.focusMoveWeight = 1.5;
		profile.focusMoves = new Set(SLICE2.moves);
		profile.knobs.switchProb = 0.15;
	} else if (base.startsWith('batch:') || base.startsWith('effect:')) {
		fillClosedWorld(profile, data, base);
	} else {
		throw new Error(`unknown profile ${spec}`);
	}
	return profile;
}

// ---------------------------------------------------------------------------------------
// Closed-world profiles (batch:*, effect:*)
// ---------------------------------------------------------------------------------------

function fillClosedWorld(profile, data, base) {
	// Expanded worlds can exercise grounding through focus moves or terrain abilities.
	// Levitate has no callbacks but participates in the core immunity/grounding path;
	// admit it explicitly instead of claiming it stays inert around those effects.
	profile.abilities.add('levitate');
	profile.families = profile.families.filter(f => f.name !== 'levitate');
	let keys;
	if (base.startsWith('batch:')) {
		const name = base.slice(6);
		const b = data.batchEffects.get(name);
		if (!b) throw new Error(`unknown batch ${name}; known: ${[...data.batchEffects.keys()].join(', ')}`);
		profile.kind = 'batch';
		profile.description = `closed world: slice1 content + the effects of batch ${name} (${b.description})`;
		keys = b.keys.filter(k => !k.startsWith('rules:'));
		profile.batchKeys = b.keys;
	} else {
		const key = resolveEffect(data, base.slice(7));
		profile.kind = 'effect';
		profile.id = `effect:${key}`;
		profile.base = profile.id;
		profile.description = `closed world: slice1 content + ${key}`;
		keys = [key];
		profile.batchKeys = [key];
	}
	const inWorld = new Set(profile.batchKeys); // effects with hooks that may occur
	const byKind = Object.fromEntries(KINDS.map(k => [k, []]));
	for (const k of keys) {
		const [kind, id] = splitKey(k);
		byKind[kind].push(id);
	}

	// Filler: slice1 declarative content.
	profile.fillerMoves = declarativeMoves(data, SLICE1_CAPS);
	profile.worldMoves = new Set(profile.fillerMoves);

	// Conditions: reachable through their creators that are inside the world (hook-free creator moves are in the
	// slice1 filler already when their content is simple; hooked creators must be batch effects).
	const unreachable = [];
	const templates = new Map(); // effect key -> [template]
	const addTemplate = (key, t) => {
		if (!templates.has(key)) templates.set(key, []);
		templates.get(key).push(t);
	};
	const creatorTemplates = cond => {
		const out = [];
		for (const c of data.creators.get(cond) || []) {
			const [kind, id] = splitKey(c);
			const inBatch = inWorld.has(c);
			if (kind === 'moves') {
				const m = data.moves.get(id);
				if (!m) continue;
				if (inBatch || profile.fillerMoves.has(id)) out.push({ moves: [id], via: c });
			} else if (kind === 'abilities') {
				if (inBatch) out.push({ ability: id, via: c });
			} else if (kind === 'items') {
				if (inBatch) out.push({ item: id, via: c });
			}
		}
		return out;
	};

	for (const id of byKind.moves) {
		if (!data.moves.has(id)) continue;
		profile.worldMoves.add(id);
		profile.focusMoves.add(id);
		addTemplate(`moves:${id}`, { moves: [id], via: `moves:${id}` });
	}
	for (const id of byKind.abilities) {
		profile.abilities.add(id);
		addTemplate(`abilities:${id}`, { ability: id, via: `abilities:${id}` });
	}
	for (const id of byKind.items) {
		profile.items.add(id);
		addTemplate(`items:${id}`, { item: id, via: `items:${id}` });
	}
	for (const id of byKind.species) {
		profile.speciesAllow.add(id);
		addTemplate(`species:${id}`, { species: id, via: `species:${id}` });
	}
	for (const id of byKind.conditions) {
		const ts = creatorTemplates(id);
		for (const t of ts) {
			if (t.moves) for (const m of t.moves) { profile.worldMoves.add(m); profile.focusMoves.add(m); }
			if (t.ability) profile.abilities.add(t.ability);
			if (t.item) profile.items.add(t.item);
			addTemplate(`conditions:${id}`, t);
		}
		if (!ts.length) unreachable.push(`conditions:${id}`);
	}
	// Hook-free abilities/items/moves that batch effects reference (`referencedBy`), e.g. Early Bird for conditions:slp,
	// Heavy-Duty Boots for the hazards, Light Clay for the screens: they are part of the batch's world.
	for (const kind of ['abilities', 'items', 'moves']) {
		for (const e of data.scope[kind]) {
			if (e.handlers.length || e.nested.length || Object.keys(e.constHandlers).length || e.condition) continue;
			if (!(e.referencedBy || []).some(r => inWorld.has(r))) continue;
			const key = `${kind}:${e.id}`;
			if (kind === 'abilities') { profile.abilities.add(e.id); addTemplate(key, { ability: e.id, via: key }); }
			else if (kind === 'items') { profile.items.add(e.id); addTemplate(key, { item: e.id, via: key }); }
			else { profile.worldMoves.add(e.id); profile.fillerMoves.add(e.id); }
		}
	}
	for (const k of keys) if (!templates.has(k)) unreachable.push(k);

	// Species in the world: the species of species-keyed batch effects; every other species needs a legal ability
	// (an allowed ability or a usable inert family) and is filtered in the builder.
	profile.focus = {
		keys: [...templates.keys()].sort(),
		templates,
		unreachable: [...new Set(unreachable)].sort(),
		slotsPerTeam: base.startsWith('effect:') ? 4 : 3,
	};
	profile.fillerItems = new Set(['heavydutyboots', 'lightclay'].filter(i => data.items.has(i)));
	profile.fillerItemChance = 0.3;
	profile.bias.focusMoveWeight = 4;
	// Switch-in/out heavy batches: more switching.
	const switchHooks = new Set(['onStart', 'onSwitchIn', 'onSwitchOut', 'onEnd', 'onAnySwitchIn', 'onBeforeSwitchOut', 'onFoeSwitchOut']);
	const usesSwitch = byKind.abilities.some(id => (data.abilities.get(id)?.entry.handlers || []).some(h => switchHooks.has(h)));
	profile.knobs.switchProb = usesSwitch ? 0.25 : 0.12;
	if (/formes_tera/.test(base)) {
		profile.tera = true;
		profile.knobs.teraProb = 0.5;
	} else if (profile.tera) {
		profile.knobs.teraProb = 0.15;
	}
}

export function listBatches(data = loadDirectedData()) {
	return [...data.batchEffects.keys()];
}
