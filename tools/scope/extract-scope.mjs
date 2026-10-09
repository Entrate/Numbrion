#!/usr/bin/env node
// Computes a conservative content catalog for "[Gen 9] Random Doubles Battle"
// (gen9randomdoublesbattle) by running Showdown's real team generator and then
// closing over every effect that in-scope effects (and the engine) can create.
//
// Usage:
//   node tools/scope/extract-scope.mjs <pokemon-showdown-path> [--teams 50000]
//        [--checkpoints 1000,5000,10000,20000,50000] [--json <path>] [--md <path>]
//
// Requires a built Showdown checkout (<path>/dist). Single process, no workers.
// Outputs (default, relative to this repo):
//   data/scope.json             machine-readable scope
//   docs/showdown/06-scope.md   human summary
//
// Method (everything below is derived from Showdown at runtime; the only hand-written
// tables are the explicit dynamic-reference resolutions, the engine call-site
// classification, the "Ignored" list and the hard-effect notes, all flagged as such):
//  1. Generate N teams with Teams.getGenerator(FORMAT, seed).getTeam(), one fresh
//     generator per deterministic sodium seed, recording every produced value.
//  2. Closure: worklist over effects. For each in-scope effect, walk its raw data
//     (status/volatileStatus/sideCondition/... fields at any depth) and the source text
//     of every function it defines (addVolatile/addSideCondition/addSlotCondition/
//     setWeather/setTerrain/addPseudoWeather/setStatus/trySetStatus/setAbility/setItem/
//     formeChange/species.get/moves.get/useMove/... with literal arguments). Calls with
//     non-literal arguments are collected and must be resolved by DYNAMIC_RESOLUTIONS
//     (the script warns about any unresolved one).
//  3. Engine (sim/*.ts) call sites that create effects are scanned the same way and
//     classified in ENGINE_SITES.
//  4. Per effect: handler keys (function-valued on*/…Callback), constant handlers,
//     priority/order keys, nested handlers, key data fields, source file:line.

import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';
import crypto from 'node:crypto';

const FORMAT_ID = 'gen9randomdoublesbattle';
const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------
const argv = process.argv.slice(2);
if (!argv[0] || argv[0].startsWith('--')) {
	console.error('usage: node extract-scope.mjs <pokemon-showdown-path> [--teams N] [--checkpoints a,b,c] [--json p] [--md p]');
	process.exit(2);
}
const PS = path.resolve(argv[0]);
const opts = {
	teams: 50000,
	checkpoints: null,
	json: path.join(REPO, 'data', 'scope.json'),
	md: path.join(REPO, 'docs', 'showdown', '06-scope.md'),
	seedPrefix: 'numbrion-scope',
};
for (let i = 1; i < argv.length; i += 2) {
	const key = argv[i].replace(/^--/, '');
	if (!(key in opts)) throw new Error(`unknown option ${argv[i]}`);
	opts[key] = argv[i + 1];
}
opts.teams = Number(opts.teams);
opts.checkpoints = (opts.checkpoints ? String(opts.checkpoints).split(',').map(Number) :
	[1000, 2000, 5000, 10000, 20000, 50000, 100000, 200000, 500000, 1000000]).filter(n => n <= opts.teams);
if (!opts.checkpoints.includes(opts.teams)) opts.checkpoints.push(opts.teams);
opts.checkpoints.sort((a, b) => a - b);

const require = createRequire(path.join(PS, 'package.json'));
const { Dex, Teams } = require('./dist/sim');
const toID = s => String(s ?? '').toLowerCase().replace(/[^a-z0-9]+/g, '');
const log = (...a) => console.error('[scope]', ...a);

const format = Dex.formats.get(FORMAT_ID);
if (!format.exists) throw new Error(`format ${FORMAT_ID} not found`);
const dex = Dex.forFormat(format);
const ruleTable = Dex.formats.getRuleTable(format);
const D = dex.data;

// ---------------------------------------------------------------------------
// Source line index (TS sources, for file:line references)
// ---------------------------------------------------------------------------
const srcCache = new Map();
function loadSrc(rel) {
	if (srcCache.has(rel)) return srcCache.get(rel);
	const full = path.join(PS, rel);
	const lines = fs.existsSync(full) ? fs.readFileSync(full, 'utf8').split('\n') : [];
	const keys = new Map();
	const order = [];
	const re = /^\t(?:'([^']+)'|"([^"]+)"|([A-Za-z0-9_$]+)):\s*\{/;
	lines.forEach((l, i) => {
		const m = re.exec(l);
		if (m) {
			const k = m[1] ?? m[2] ?? m[3];
			if (!keys.has(k)) keys.set(k, i + 1);
			order.push(i + 1);
		}
	});
	const entry = { rel, lines, keys, order };
	srcCache.set(rel, entry);
	return entry;
}
function srcRef(rel, key) {
	const s = loadSrc(rel);
	const line = s.keys.get(key);
	return line ? `${rel}:${line}` : null;
}
/** line of a nested `\t\t<inner>: {` (e.g. condition) inside a top-level block */
function innerRef(rel, key, inner) {
	const s = loadSrc(rel);
	const start = s.keys.get(key);
	if (!start) return null;
	const next = s.order.find(n => n > start) ?? s.lines.length;
	for (let i = start; i < next; i++) {
		if (new RegExp(`^\\t\\t${inner}:`).test(s.lines[i - 1])) return `${rel}:${i}`;
	}
	return null;
}
function grepSrc(rel, regex) {
	const s = loadSrc(rel);
	const out = [];
	s.lines.forEach((l, i) => { if (regex.test(l)) out.push({ ref: `${rel}:${i + 1}`, text: l.trim() }); });
	return out;
}
function formatSrcRef() {
	const hits = grepSrc('config/formats.ts', /name: "\[Gen 9\] Random Doubles Battle"/);
	return hits[0]?.ref ?? null;
}

// ---------------------------------------------------------------------------
// 1. Team generation
// ---------------------------------------------------------------------------
const seedFor = i => 'sodium,' + crypto.createHash('sha256').update(`${opts.seedPrefix}:${i}`).digest('hex').slice(0, 32);

const CATS = ['species', 'speciesId', 'item', 'ability', 'move', 'teraType', 'level', 'gender', 'nature',
	'evs', 'ivs', 'role', 'setKeys', 'shiny', 'genderMismatch', 'formeOf', 'speciesAbility', 'speciesItem', 'speciesMove', 'speciesTeraType', 'speciesRole'];
const seen = Object.fromEntries(CATS.map(c => [c, new Map()]));
function see(cat, value, idx) {
	const m = seen[cat];
	const e = m.get(value);
	if (e) e.count++;
	else m.set(value, { first: idx, count: 1 });
}
const perSpecies = new Map();
const satur = [];
let failures = 0;
const badTeamSizes = new Map();
const statKeys = ['hp', 'atk', 'def', 'spa', 'spd', 'spe'];

const tGen0 = performance.now();
for (let i = 0; i < opts.teams; i++) {
	let team;
	try {
		team = Teams.getGenerator(FORMAT_ID, seedFor(i)).getTeam();
	} catch (e) {
		failures++;
		if (failures < 5) log(`team ${i} failed: ${e.message}`);
		continue;
	}
	if (team.length !== 6) badTeamSizes.set(team.length, (badTeamSizes.get(team.length) || 0) + 1);
	for (const set of team) {
		const sp = dex.species.get(set.species);
		const spId = sp.id;
		const genId = set.speciesId ?? spId;
		const item = toID(set.item);
		const ability = toID(set.ability);
		see('species', spId, i);
		see('speciesId', genId, i);
		see('item', item, i);
		see('ability', ability, i);
		for (const m of set.moves) {
			see('move', toID(m), i);
			see('speciesMove', `${genId}|${toID(m)}`, i);
		}
		see('teraType', set.teraType, i);
		see('level', set.level, i);
		see('gender', `${sp.gender || 'random'}:${set.gender}`, i);
		see('nature', set.nature ?? '(unset)', i);
		see('evs', statKeys.map(s => set.evs?.[s] ?? '-').join('/'), i);
		see('ivs', statKeys.map(s => set.ivs?.[s] ?? '-').join('/'), i);
		see('role', set.role, i);
		see('setKeys', Object.keys(set).sort().join(','), i);
		see('shiny', String(!!set.shiny), i);
		if (sp.gender && set.gender !== sp.gender) see('genderMismatch', `${set.species} (species gender ${sp.gender}) generated as ${set.gender}`, i);
		see('formeOf', `${spId}<${genId}`, i);
		see('speciesAbility', `${genId}|${ability}`, i);
		see('speciesItem', `${genId}|${item}`, i);
		see('speciesTeraType', `${genId}|${set.teraType}`, i);
		see('speciesRole', `${genId}|${set.role}`, i);
		let ps = perSpecies.get(genId);
		if (!ps) {
			ps = { count: 0, formes: {}, levels: {}, abilities: {}, items: {}, moves: {}, teraTypes: {}, roles: {}, genders: {}, evs: {}, ivs: {} };
			perSpecies.set(genId, ps);
		}
		ps.count++;
		const bump = (o, k) => { o[k] = (o[k] || 0) + 1; };
		bump(ps.formes, set.species);
		bump(ps.levels, set.level);
		bump(ps.abilities, ability);
		bump(ps.items, item);
		for (const m of set.moves) bump(ps.moves, toID(m));
		bump(ps.teraTypes, set.teraType);
		bump(ps.roles, set.role);
		bump(ps.genders, set.gender);
		bump(ps.evs, statKeys.map(s => set.evs[s]).join('/'));
		bump(ps.ivs, statKeys.map(s => set.ivs[s]).join('/'));
	}
	const n = i + 1;
	if (opts.checkpoints.includes(n)) {
		satur.push({ teams: n, ...Object.fromEntries(CATS.map(c => [c, seen[c].size])) });
		log(`${n} teams: ${CATS.slice(0, 7).map(c => `${c}=${seen[c].size}`).join(' ')} (${((performance.now() - tGen0) / 1000).toFixed(1)}s)`);
	}
}
const genSeconds = (performance.now() - tGen0) / 1000;

// Universe declared in doubles-sets.json (what the generator could at most produce)
const setsJson = JSON.parse(fs.readFileSync(path.join(PS, 'data/random-battles/gen9/doubles-sets.json'), 'utf8'));
const declared = { species: new Set(), moves: new Set(), abilities: new Set(), teraTypes: new Set(), roles: new Set(), levels: new Set() };
for (const [sid, data] of Object.entries(setsJson)) {
	declared.species.add(sid);
	if (data.level) declared.levels.add(data.level);
	for (const s of data.sets) {
		declared.roles.add(s.role);
		for (const m of s.movepool) declared.moves.add(toID(m));
		for (const a of s.abilities || []) declared.abilities.add(toID(a));
		for (const t of s.teraTypes || []) declared.teraTypes.add(t);
	}
}

// Static item universe of the doubles item path: string literals naming items inside
// getPriorityItem/getDoublesItem (data/random-battles/gen9/teams.ts) + requiredItem(s) of generator species.
const staticItems = (() => {
	const rel = 'data/random-battles/gen9/teams.ts';
	const lines = loadSrc(rel).lines;
	const out = new Map(); // itemId -> [where]
	for (const method of ['getPriorityItem', 'getDoublesItem']) {
		const start = lines.findIndex(l => l.startsWith(`\t${method}(`));
		if (start < 0) continue;
		let end = lines.findIndex((l, i) => i > start && /^\t[}]$/.test(l));
		if (end < 0) end = lines.length;
		for (let i = start; i <= end; i++) {
			const names = [...lines[i].matchAll(/'([^']+)'/g)].map(m => m[1]);
			// template literals like `${this.sample(['Aguav', 'Figy'])} Berry`
			for (const m of lines[i].matchAll(/\$\{this\.sample\(\[([^\]]+)\]\)\}([^`]*)`/g)) {
				for (const part of m[1].matchAll(/'([^']+)'/g)) names.push(part[1] + m[2]);
			}
			for (const name of names) {
				const it = dex.items.get(name);
				if (it.exists && it.name === name) {
					if (!out.has(it.id)) out.set(it.id, []);
					out.get(it.id).push(`${rel}:${i + 1}`);
				}
			}
		}
	}
	for (const sid of Object.keys(setsJson)) {
		const sp = dex.species.get(sid);
		for (const it of [].concat(sp.requiredItems || [], sp.requiredItem || [])) {
			const item = dex.items.get(it);
			if (!item.exists) continue;
			if (!out.has(item.id)) out.set(item.id, []);
			out.get(item.id).push(`requiredItem(s) of ${sp.name}`);
		}
	}
	return out;
})();

// ---------------------------------------------------------------------------
// 2. Closure
// ---------------------------------------------------------------------------
const KINDS = ['species', 'moves', 'abilities', 'items', 'conditions', 'rules'];
const scope = Object.fromEntries(KINDS.map(k => [k, new Map()])); // id -> {via: Set}
const queue = [];
function add(kind, id, via) {
	if (!id) return false;
	let e = scope[kind].get(id);
	if (e) { e.via.add(via); return false; }
	e = { via: new Set([via]) };
	scope[kind].set(id, e);
	queue.push([kind, id]);
	return true;
}

// Condition kinds. Determined by what creates them and by Dex data.
const condKind = new Map(); // id -> Set(kind)
function addCond(id, kind, via) {
	id = toID(id);
	if (!id) return;
	if (!condKind.has(id)) condKind.set(id, new Set());
	condKind.get(id).add(kind);
	add('conditions', id, via);
}

/**
 * Add a forme only if it can actually exist in this format: some in-scope species shares its
 * baseSpecies and, for battle-only formes, one of its `battleOnly` source formes is in scope.
 */
function addSpecies(id, via) {
	const f = dex.species.get(id);
	if (!f.exists) return add('species', toID(id), via);
	if (scope.species.has(f.id)) return add('species', f.id, via);
	const sameBase = [...scope.species.keys()].some(s => dex.species.get(s).baseSpecies === f.baseSpecies);
	if (!sameBase) { formeRejected.set(f.id, `${via}: no in-scope ${f.baseSpecies}`); return false; }
	if (!f.battleOnly) {
		formeRejected.set(f.id, `${via}: not a battle-only forme and never generated`);
		return false;
	}
	if (![].concat(f.battleOnly).some(n => scope.species.has(toID(n)))) {
		formeRejected.set(f.id, `${via}: battleOnly source ${[].concat(f.battleOnly).join('/')} not in scope`);
		return false;
	}
	return add('species', f.id, via);
}
const formeRejected = new Map();

/** Where is condition `id` defined (mirrors DexConditions.getByID, sim/dex-conditions.ts) */
function conditionDef(id) {
	if (Object.prototype.hasOwnProperty.call(D.Rulesets, id)) return { from: 'rule', raw: D.Rulesets[id], src: srcRef('data/rulesets.ts', id) };
	if (Object.prototype.hasOwnProperty.call(D.Conditions, id)) return { from: 'conditions', raw: D.Conditions[id], src: srcRef('data/conditions.ts', id) };
	if (D.Moves[id]?.condition) return { from: 'move', raw: D.Moves[id].condition, src: innerRef('data/moves.ts', id, 'condition') };
	if (D.Abilities[id]?.condition) return { from: 'ability', raw: D.Abilities[id].condition, src: innerRef('data/abilities.ts', id, 'condition') };
	if (D.Items[id]?.condition) return { from: 'item', raw: D.Items[id].condition, src: innerRef('data/items.ts', id, 'condition') };
	if (id === 'recoil' || id === 'drain') return { from: 'pseudo', raw: {}, src: 'sim/dex-conditions.ts' };
	return { from: 'none', raw: null, src: null };
}

const species = id => dex.species.get(id);

// Seed scope with generated content
for (const id of seen.species.keys()) add('species', id, 'teamgen:species');
for (const id of seen.speciesId.keys()) add('species', id, 'teamgen:speciesId');
for (const id of seen.move.keys()) add('moves', id, 'teamgen');
for (const id of seen.ability.keys()) add('abilities', id, 'teamgen');
for (const id of seen.item.keys()) add('items', id, 'teamgen');

// Format + rules (ruleTable keys, skipping +/-/!/* entries like battle.ts:296-307 does)
const ruleIds = [...ruleTable.keys()].filter(r => !'+*-!'.includes(r.charAt(0)));
for (const r of ruleIds) add('rules', r, 'format ruleTable');

// ----- Engine call sites (sim/*.ts), classified by hand -----
// Every engine site that creates/uses an effect by name. The loop below matches each grep hit
// against this table; a hit not listed here is either non-literal plumbing (method definitions,
// variable arguments -> "no-new") or, if it names an effect literally, reported as unclassified.
const ENGINE_FILES = ['sim/battle.ts', 'sim/battle-actions.ts', 'sim/pokemon.ts', 'sim/side.ts', 'sim/field.ts', 'sim/battle-queue.ts'];
const ENGINE_SITE_RE = /(addVolatile|addSideCondition|addSlotCondition|setWeather|setTerrain|addPseudoWeather|setStatus|trySetStatus|formeChange|setAbility|setItem|useMove|runMove|moves\.get|getActiveMove|species\.get|abilities\.get|items\.get|conditions\.get|conditions\.getByID)\(|id: '(struggle|recharge)'|moveid: 'struggle'/;
const ENGINE_LITERAL_RE = /(addVolatile|addSideCondition|addSlotCondition|setWeather|setTerrain|addPseudoWeather|setStatus|trySetStatus|formeChange|setAbility|setItem|useMove|runMove|moves\.get|getActiveMove|species\.get|abilities\.get|items\.get|conditions\.get)\(\s*['"`]|id: '(struggle|recharge)'|moveid: 'struggle'/;
const ENGINE_SITES = [
	// [file, substring of line, action, note]
	['sim/battle.ts', "this.field.addPseudoWeather(rule)", 'rules-as-pseudoweather', 'every format rule with a battle event handler is added as a pseudo-weather at construction'],
	['sim/battle.ts', "action.pokemon.addVolatile('dynamax')", 'ignore', 'Dynamax (not in Gen 9)'],
	['sim/battle.ts', "this.dex.conditions.get('skillswap')", 'condition-ref:skillswap', 'Battle.skillSwap uses the Skill Swap move-condition as sourceEffect for SetAbility'],
	['sim/battle.ts', "this.singleEvent('BattleStart', this.dex.conditions.getByID(pokemon.species.id)", 'species-condition', 'species BattleStart (zacian/zamazenta)'],
	['sim/battle-actions.ts', 'pokemon.addVolatile(move.id)', 'cantusetwice', 'flags.cantusetwice: volatile named after the move id (hint only)'],
	['sim/battle-actions.ts', "this.dex.conditions.get('lockedmove')", 'condition-ref:lockedmove', 'sourceEffect for locked moves'],
	['sim/battle-actions.ts', "this.dex.abilities.get('Illusion')", 'ability-ref:illusion', 'Illusion End on Terastallize / damage'],
	['sim/battle-actions.ts', "this.runMove(move.id, dancer", 'dancer', 'Dancer re-uses in-scope dance moves'],
	['sim/battle-actions.ts', "this.dex.conditions.get('zpower')", 'ignore', 'Z-Power (Z-moves disabled)'],
	['sim/battle-actions.ts', "addSlotCondition(pokemon, 'healreplacement'", 'ignore', 'Z-Memento/Z-Parting Shot effect (Z-moves disabled)'],
	['sim/battle-actions.ts', "pokemon.addVolatile('followme', pokemon, zPower)", 'ignore', 'Z-status effect (Z-moves disabled)'],
	['sim/battle-actions.ts', "pokemon.addVolatile('focusenergy', pokemon, zPower)", 'ignore', 'Z-status effect (Z-moves disabled)'],
	['sim/battle-actions.ts', 'target.trySetStatus(moveData.status', 'move-data', 'move.status / secondary.status (collected from data)'],
	['sim/battle-actions.ts', 'target.setStatus(moveData.forceStatus', 'move-data', 'move.forceStatus (collected from data)'],
	['sim/battle-actions.ts', 'target.addVolatile(moveData.volatileStatus', 'move-data', 'move.volatileStatus (collected from data)'],
	['sim/battle-actions.ts', 'target.side.addSideCondition(moveData.sideCondition', 'move-data', 'move.sideCondition (collected from data)'],
	['sim/battle-actions.ts', 'target.side.addSlotCondition(target, moveData.slotCondition', 'move-data', 'move.slotCondition (collected from data)'],
	['sim/battle-actions.ts', 'this.battle.field.setWeather(moveData.weather', 'move-data', 'move.weather (collected from data)'],
	['sim/battle-actions.ts', 'this.battle.field.setTerrain(moveData.terrain', 'move-data', 'move.terrain (collected from data)'],
	['sim/battle-actions.ts', 'this.battle.field.addPseudoWeather(moveData.pseudoWeather', 'move-data', 'move.pseudoWeather (collected from data)'],
	['sim/battle-actions.ts', 'pokemon.formeChange(speciesid, pokemon.getItem(), true)', 'ignore', 'Mega Evolution / Ultra Burst (no Mega Stones / Z-Crystals generated)'],
	['sim/battle-actions.ts', 'pokemon.formeChange(ogerponSpecies, null, true)', 'ogerpon-tera', 'Terastallize: Ogerpon -> Ogerpon-*-Tera'],
	['sim/battle-actions.ts', "pokemon.formeChange('Terapagos-Stellar', null, true)", 'forme:terapagosstellar', 'Terastallize: Terapagos -> Terapagos-Stellar'],
	['sim/pokemon.ts', 'this.addVolatile(volatile)', 'transform-crit-volatiles', 'Transform copies dragoncheer/focusenergy/laserfocus (gmaxchistrike ignored); copies existing volatiles only'],
	['sim/pokemon.ts', "this.formeChange('Giratina", 'ignore', 'Gen 4 Transform forme logic'],
	['sim/pokemon.ts', 'this.formeChange(targetForme)', 'ignore', 'Gen 4 Transform forme logic (Arceus)'],
	['sim/pokemon.ts', "this.battle.dex.moves.get('hiddenpower')", 'ignore', 'Hidden Power not generated in Gen 9'],
	['sim/pokemon.ts', "id: 'recharge'", 'pseudo-move:recharge', 'getMoves(): locked move "recharge" (mustrecharge onLockMove)'],
	['sim/pokemon.ts', "id: 'struggle'", 'move:struggle', 'getMoves(): Struggle when no usable moves'],
	['sim/pokemon.ts', 'source.addVolatile(linkedStatus', 'linked-volatile', 'linked volatiles (4th addVolatile arg, collected from handler source)'],
	['sim/pokemon.ts', "this.setStatus('')", 'no-new', 'cure'],
	['sim/pokemon.ts', "this.setItem('')", 'no-new', 'item removal'],
	['sim/pokemon.ts', "this.setAbility('')", 'no-new', 'ability removal'],
	['sim/battle-actions.ts', 'this.dex.conditions.get(move.name)', 'mindblown-recoil', 'recoil effect for moves with mindBlownRecoil is the bare condition named after the move'],
	['sim/battle-actions.ts', 'this.dex.species.get(species.otherFormes[0])', 'ignore', 'Mega Evolution eligibility (canMegaEvo)'],
	['sim/battle-actions.ts', 'this.dex.species.get(megaEvolution)', 'ignore', 'Mega Evolution eligibility (canMegaEvo)'],
	['sim/side.ts', "moveid: 'struggle'", 'move:struggle', 'chooseMove(): Struggle'],
];
const engineSites = [];
const engineUnclassified = [];
for (const file of ENGINE_FILES) {
	for (const hit of grepSrc(file, ENGINE_SITE_RE)) {
		if (/^\s*(\/\/|\*)/.test(hit.text)) continue;
		const cls = ENGINE_SITES.find(([f, sub]) => f === file && hit.text.includes(sub));
		if (cls) {
			engineSites.push({ ref: hit.ref, text: hit.text, action: cls[2], note: cls[3] });
		} else if (ENGINE_LITERAL_RE.test(hit.text)) {
			// a literal effect name in an unlisted engine site: must be classified by hand
			engineUnclassified.push(hit);
		} else {
			engineSites.push({ ref: hit.ref, text: hit.text, action: 'no-new', note: 'non-literal plumbing (method definition or variable argument)' });
		}
	}
}
add('moves', 'struggle', 'engine: sim/pokemon.ts getMoves / sim/side.ts chooseMove');
// recoil / drain are pseudo-conditions used as damage/heal effects (sim/dex-conditions.ts getByID)
addCond('recoil', 'pseudo-effect', 'engine: recoil damage effect');
addCond('drain', 'pseudo-effect', 'engine: drain heal effect');
addCond('lockedmove', 'volatile', 'engine: sim/battle-actions.ts lockedmove sourceEffect');
addCond('skillswap', 'pseudo-effect', 'engine: Battle.skillSwap sourceEffect');

// ----- Dynamic (non-literal) references found in handler source, resolved by hand -----
// key: `${kind}:${id}` (kind of the effect owning the function) -> resolver.
// Each resolver gets the full scope and returns {add: [[kind,id,via]...], note}.
const inScope = (kind, id) => scope[kind].has(id);
const scopeMoves = () => [...scope.moves.keys()].map(id => dex.moves.get(id));
const DYNAMIC_RESOLUTIONS = {
	// --- move callers ---
	'moves:sleeptalk': () => ({ note: 'calls a random move from the user\'s own moveset (in scope)' }),
	'moves:copycat': () => ({ note: 'calls this.lastMove (any in-scope move)' }),
	'moves:assist': () => ({ note: 'calls a move from party movesets (in scope)' }),
	'moves:mefirst': () => ({ note: 'calls the target\'s queued move (in scope)' }),
	'moves:instruct': () => ({ note: 'repeats target.lastMove (in scope)' }),
	'moves:magiccoat': () => ({ note: 'reflects the incoming move (in scope)' }),
	'abilities:magicbounce': () => ({ note: 'reflects the incoming move (in scope)' }),
	'moves:snatch': () => ({ note: 'steals an in-scope move' }),
	'moves:metronome': () => ({
		add: dex.moves.all().filter(m => (!m.isNonstandard || m.isNonstandard === 'Unobtainable') && m.flags['metronome'])
			.map(m => ['moves', m.id, 'closure: Metronome pool (moves.ts metronome onHit filter)']),
		note: 'random move with flags.metronome and (!isNonstandard || Unobtainable)',
	}),
	'moves:naturepower': (fnSrcs) => ({
		add: literalMoves(fnSrcs).map(id => ['moves', id, 'closure: Nature Power terrain move']),
		note: 'terrain-dependent literal move ids',
	}),
	// --- volatiles named after a move id ---
	'conditions:twoturnmove': () => ({
		add: scopeMoves().filter(m => m.flags['charge']).map(m => ['conditions', m.id, 'closure: twoturnmove addVolatile(effect.id) for charge move']),
		note: 'adds a volatile named after the charging move id (flags.charge)',
	}),
	// --- ability/item copying (no new ids) ---
	'moves:skillswap': () => ({ note: 'swaps in-scope abilities' }),
	'moves:roleplay': () => ({ note: 'copies an in-scope ability' }),
	'moves:entrainment': () => ({ note: 'gives the user\'s in-scope ability' }),
	'moves:doodle': () => ({ note: 'copies an in-scope ability to user and ally' }),
	'abilities:trace': () => ({ note: 'copies an in-scope ability' }),
	'abilities:receiver': () => ({ note: 'copies a fainted ally\'s in-scope ability' }),
	'abilities:powerofalchemy': () => ({ note: 'copies a fainted ally\'s in-scope ability' }),
	'abilities:wanderingspirit': () => ({ note: 'swaps in-scope abilities' }),
	'abilities:mummy': () => ({ note: 'spreads Mummy (in scope via setAbility literal or own id)' }),
	'abilities:lingeringaroma': () => ({ note: 'spreads Lingering Aroma' }),
	'abilities:imposter': () => ({ note: 'Transform: copies species/ability/moves of an in-scope foe' }),
	'moves:transform': () => ({ note: 'Transform: copies species/ability/moves of an in-scope target' }),
	'moves:trick': () => ({ note: 'swaps in-scope items' }),
	'moves:switcheroo': () => ({ note: 'swaps in-scope items' }),
	'moves:thief': () => ({ note: 'steals an in-scope item' }),
	'moves:covet': () => ({ note: 'steals an in-scope item' }),
	'moves:bestow': () => ({ note: 'gives an in-scope item' }),
	'moves:recycle': () => ({ note: 'restores the user\'s own consumed in-scope item' }),
	'abilities:harvest': () => ({ note: 'restores the user\'s own consumed berry' }),
	'abilities:pickup': () => ({ note: 'picks up an in-scope consumed item' }),
	'abilities:magician': () => ({ note: 'steals an in-scope item' }),
	'abilities:pickpocket': () => ({ note: 'steals an in-scope item' }),
	'abilities:symbiosis': () => ({ note: 'passes an in-scope item' }),
	'items:stickybarb': () => ({ note: 'moves itself to the attacker' }),
	// --- forme changes ---
	'abilities:disguise': (fnSrcs) => ({ add: literalSpecies(fnSrcs).map(id => ['species', id, 'closure: Disguise']) }),
	'abilities:gulpmissile': (fnSrcs) => ({ add: literalSpecies(fnSrcs).map(id => ['species', id, 'closure: Gulp Missile']) }),
	'abilities:hungerswitch': (fnSrcs) => ({ add: literalSpecies(fnSrcs).map(id => ['species', id, 'closure: Hunger Switch']) }),
	'abilities:shieldsdown': () => ({ note: 'reverts to pokemon.set.species (the team forme, in scope)' }),
	'moves:relicsong': () => ({ add: ['meloettapirouette', 'meloetta'].map(id => ['species', id, 'closure: Relic Song (\'Meloetta\' + forme)']) }),
	// --- statuses picked from a literal list ---
	'moves:triattack': (fnSrcs) => ({ add: literalStatuses(fnSrcs).map(id => ['status', id, 'closure: Tri Attack sample']) }),
	'moves:direclaw': (fnSrcs) => ({ add: literalStatuses(fnSrcs).map(id => ['status', id, 'closure: Dire Claw sample']) }),
	'abilities:synchronize': () => ({ note: 'passes the holder\'s own (in-scope) status back to the source' }),
};
function literalStatuses(srcs) {
	const out = new Set();
	for (const s of srcs) for (const m of s.matchAll(/["'](brn|par|slp|frz|psn|tox)["']/g)) out.add(m[1]);
	return [...out];
}
function literalMoves(srcs) {
	const out = new Set();
	for (const s of srcs) for (const m of s.matchAll(/["']([a-z0-9]+)["']/g)) if (D.Moves[m[1]]) out.add(m[1]);
	return [...out];
}
function literalSpecies(srcs) {
	const out = new Set();
	for (const s of srcs) for (const m of s.matchAll(/["']([A-Za-z0-9 .:%'-]+)["']/g)) {
		const sp = dex.species.get(m[1]);
		// species names ("Mimikyu-Busted") or exact species ids ("cramorantgulping")
		if (sp.exists && (/[A-Z]/.test(m[1].charAt(0)) || m[1] === sp.id)) out.add(sp.id);
	}
	return [...out];
}

// ----- Source scanning of an effect -----
const LIT = `\\s*["'\`]([^"'\`]+)["'\`]`;
const CALL_RES = [
	// [regex, handler] ; regex group 1 = literal (or undefined for dynamic detection below)
	[new RegExp(`\\.addVolatile\\(${LIT}`, 'g'), (id, via) => addCond(id, 'volatile', via)],
	[new RegExp(`\\.addSideCondition\\(${LIT}`, 'g'), (id, via) => addCond(id, 'sideCondition', via)],
	[new RegExp(`\\.addSlotCondition\\([^,()]+,${LIT}`, 'g'), (id, via) => addCond(id, 'slotCondition', via)],
	[new RegExp(`\\.setWeather\\(${LIT}`, 'g'), (id, via) => addCond(id, 'weather', via)],
	[new RegExp(`\\.setTerrain\\(${LIT}`, 'g'), (id, via) => addCond(id, 'terrain', via)],
	[new RegExp(`\\.addPseudoWeather\\(${LIT}`, 'g'), (id, via) => addCond(id, 'pseudoWeather', via)],
	[new RegExp(`\\.(?:setStatus|trySetStatus)\\(${LIT}`, 'g'), (id, via) => addCond(id, 'status', via)],
	[new RegExp(`\\.setAbility\\(${LIT}`, 'g'), (id, via) => add('abilities', toID(id), via)],
	[new RegExp(`\\.setItem\\(${LIT}`, 'g'), (id, via) => add('items', toID(id), via)],
	[new RegExp(`\\.formeChange\\(${LIT}`, 'g'), (id, via) => addSpecies(id, via)],
	[new RegExp(`species\\.get\\(${LIT}`, 'g'), (id, via) => addSpecies(id, via)],
	[new RegExp(`(?:moves\\.get|getActiveMove|useMove|runMove)\\(${LIT}`, 'g'), (id, via) => add('moves', toID(id), via)],
	[new RegExp(`abilities\\.get\\(${LIT}`, 'g'), (id, via) => add('abilities', toID(id), via + ' (referenced as effect object)')],
	[new RegExp(`items\\.get\\(${LIT}`, 'g'), (id, via) => add('items', toID(id), via + ' (referenced as effect object)')],
	[new RegExp(`conditions\\.get\\(${LIT}`, 'g'), (id, via) => addCond(id, 'referenced', via)],
	// data assignments inside handlers: move.volatileStatus = 'x', {status: 'x'}, ...
	[/\b(volatileStatus|status|forceStatus|sideCondition|slotCondition|weather|terrain|pseudoWeather)\s*[:=]\s*["'`]([a-z0-9]+)["'`]/g,
		(id, via, m) => addCond(m[2], dataKeyKind(m[1]), via), true],
];
const DYN_RE = /\.(addVolatile|addSideCondition|addSlotCondition|setWeather|setTerrain|addPseudoWeather|setStatus|trySetStatus|setAbility|setItem|formeChange|useMove|runMove)\(\s*(?!["'`)])([^,)]*)/g;
function dataKeyKind(k) {
	return { volatileStatus: 'volatile', status: 'status', forceStatus: 'status', sideCondition: 'sideCondition', slotCondition: 'slotCondition', weather: 'weather', terrain: 'terrain', pseudoWeather: 'pseudoWeather' }[k];
}
/** Linked volatiles: addVolatile(x, src, effect, 'linked') */
const LINKED_RE = /\.addVolatile\(\s*["'`]([^"'`]+)["'`]\s*,[^,()]*(?:\([^()]*\))?[^,()]*,[^,()]*(?:\([^()]*\))?[^,()]*,\s*["'`]([^"'`]+)["'`]\s*\)/g;

const SKIP_DATA_KEYS = new Set(['desc', 'shortDesc', 'zMove', 'maxMove', 'contestType', 'gmaxUnreleased']);

/** Walk raw data; returns [{path, fn}] for functions and calls onData(key, value, path) for leaves */
function walkRaw(obj, onFn, onData, pth = '', depth = 0) {
	if (!obj || typeof obj !== 'object' || depth > 4) return;
	for (const [k, v] of Object.entries(obj)) {
		if (SKIP_DATA_KEYS.has(k)) continue;
		const p = pth ? (Array.isArray(obj) ? `${pth}[${k}]` : `${pth}.${k}`) : k;
		if (typeof v === 'function') onFn(p, v);
		else if (v && typeof v === 'object') walkRaw(v, onFn, onData, p, depth + 1);
		else onData(k, v, p);
	}
}

const dynamicRefs = []; // {effect, path, call, arg, resolution}
const scanned = new Set();
function rawFor(kind, id) {
	switch (kind) {
	case 'moves': return D.Moves[id];
	case 'abilities': return D.Abilities[id];
	case 'items': return D.Items[id];
	case 'conditions': return conditionDef(id).raw;
	case 'rules': return D.Rulesets[id];
	case 'species': {
		const sp = species(id);
		const cond = D.Conditions[toID(sp.baseSpecies)];
		return cond ?? null;
	}
	}
	return null;
}

function scanEffect(kind, id) {
	const key = `${kind}:${id}`;
	if (scanned.has(key)) return;
	scanned.add(key);
	const raw = rawFor(kind, id);
	const fnSrcs = [];
	if (kind === 'species') {
		const sp = species(id);
		if (!sp.exists) return;
		// battle-only formes reachable from this forme are added by the forme pass below
	}
	if (!raw) return;
	const via = `${kind}:${id}`;
	walkRaw(raw, (p, fn) => {
		// a move's own condition is scanned when the condition itself is in scope
		if (kind !== 'conditions' && p.startsWith('condition.')) return;
		const src = fn.toString();
		fnSrcs.push(src);
		const v = `${via}.${p.split('.').pop()}`;
		for (const [re, h, isData] of CALL_RES) {
			re.lastIndex = 0;
			for (const m of src.matchAll(re)) h(isData ? null : m[1], v, m);
		}
		for (const m of src.matchAll(LINKED_RE)) addCond(m[2], 'volatile', `${v} (linked volatile)`);
		for (const m of src.matchAll(DYN_RE)) {
			dynamicRefs.push({ effect: key, path: p, call: m[1], arg: m[2].trim().slice(0, 60) });
		}
	}, (k, v, p) => {
		if (kind !== 'conditions' && p.startsWith('condition.')) return;
		if (typeof v !== 'string') return;
		const ck = dataKeyKind(k);
		if (ck && v) addCond(v, ck, `${via}.${p}`);
	});
	// Fling data on items
	if (kind === 'items' && raw.fling) {
		if (raw.fling.status) addCond(raw.fling.status, 'status', `${via}.fling.status`);
		if (raw.fling.volatileStatus) addCond(raw.fling.volatileStatus, 'volatile', `${via}.fling.volatileStatus`);
	}
	// Moves/abilities/items with their own condition: the condition is reachable whenever
	// something creates the id; we include it conservatively and record whether a creator was found.
	if (['moves', 'abilities', 'items'].includes(kind) && raw.condition) {
		addCond(id, 'own-condition', `${via} (own condition)`);
	}
	// Dynamic resolutions keyed by effect
	const res = DYNAMIC_RESOLUTIONS[key];
	if (res) {
		const r = res(fnSrcs) || {};
		for (const [k2, id2, v2] of r.add || []) {
			if (k2 === 'conditions') addCond(id2, 'volatile', v2);
			else if (k2 === 'status') addCond(id2, 'status', v2);
			else if (k2 === 'species') addSpecies(id2, v2);
			else add(k2, id2, v2);
		}
		resolvedNotes.set(key, r.note || (r.add ? `${r.add.some(a => a[0] === 'species') ? 'forme candidates (kept only if reachable, see formeRejected)' : 'adds'}: ${r.add.map(a => a[1]).join(', ')}` : ''));
	}
}
const resolvedNotes = new Map();

// ----- Species formes -----
function formePass() {
	let changed = false;
	const all = dex.species.all();
	for (const id of [...scope.species.keys()]) {
		const sp = species(id);
		if (!sp.exists) continue;
		// Ogerpon tera formes (sim/battle-actions.ts terastallize)
		if (sp.baseSpecies === 'Ogerpon' && !sp.name.endsWith('-Tera')) {
			const tera = sp.id === 'ogerpon' ? 'ogerpontealtera' : sp.id + 'tera';
			changed = addSpecies(tera, `engine: terastallize (${sp.name}) sim/battle-actions.ts`) || changed;
		}
		if (sp.id === 'terapagos' || sp.id === 'terapagosterastal') {
			changed = addSpecies('terapagosstellar', 'engine: terastallize (Terapagos) sim/battle-actions.ts') || changed;
		}
		// formes whose battleOnly is this forme and whose trigger is in scope
		for (const f of all) {
			if (f.baseSpecies !== sp.baseSpecies || f.id === sp.id || scope.species.has(f.id)) continue;
			const bo = f.battleOnly;
			const fromThis = Array.isArray(bo) ? bo.includes(sp.name) : bo === sp.name;
			if (!fromThis) continue;
			const trigger = formeTrigger(f);
			if (trigger.ok) changed = add('species', f.id, `battleOnly of ${sp.name}; trigger ${trigger.why}`) || changed;
			else formeRejected.set(f.id, `battleOnly of ${sp.name}; ${trigger.why}`);
		}
	}
	// abilities['0'] of formes reached by a permanent forme change become the ability
	// (sim/pokemon.ts formeChange isPermanent -> setAbility(species.abilities[slot]))
	for (const id of scope.species.keys()) {
		const sp = species(id);
		if (!sp.exists) continue;
		const teamForme = seen.species.has(id) || seen.speciesId.has(id);
		if (!teamForme) {
			changed = add('abilities', toID(sp.abilities['0']), `forme ${sp.name} abilities[0]`) || changed;
		}
	}
	return changed;
}
function formeTrigger(f) {
	if (f.isNonstandard === 'Gigantamax' || f.forme === 'Gmax' || f.name.endsWith('-Gmax')) return { ok: false, why: 'ignored: Gigantamax' };
	if (f.isMega || f.forme.startsWith('Mega')) return { ok: inScope('items', toID(f.requiredItem)), why: `Mega requires ${f.requiredItem}` };
	if (f.isPrimal) return { ok: inScope('items', toID(f.requiredItem)), why: `Primal requires ${f.requiredItem}` };
	if (f.forme === 'Ultra') return { ok: false, why: 'ignored: Ultra Burst needs a Z-Crystal' };
	// requiredAbility formes are only reachable if that ability's handlers (or a dynamic
	// resolution) actually forme-change into them, e.g. Gen 9 Battle Bond no longer makes Ash-Greninja
	if (f.requiredAbility) return { ok: false, why: `requiredAbility ${f.requiredAbility}${inScope('abilities', toID(f.requiredAbility)) ? ' (in scope)' : ''}, but no formeChange to it in any in-scope handler` };
	if (f.requiredMove) return { ok: inScope('moves', toID(f.requiredMove)), why: `requiredMove ${f.requiredMove}` };
	if (f.requiredItem) return { ok: inScope('items', toID(f.requiredItem)), why: `requiredItem ${f.requiredItem}` };
	if (f.requiredTeraType) return { ok: true, why: `Terastallization (${f.requiredTeraType})` };
	// no declared trigger: must be produced by a formeChange literal in an in-scope handler
	return { ok: false, why: 'no declared trigger (reachable only via a formeChange/species.get literal)' };
}

// ----- Engine-classified actions that add content -----
function engineActions() {
	let changed = false;
	for (const s of engineSites) {
		if (s.action === 'rules-as-pseudoweather') {
			for (const r of ruleIds) {
				const f = dex.formats.get(r);
				const has = Object.keys(f).some(k => k.startsWith('on') && !RULE_NON_PSEUDO_KEYS.includes(k));
				if (has && !scope.conditions.has(r)) { addCond(r, 'pseudoWeather', `engine: ${s.ref} (rule with event handlers)`); changed = true; }
			}
		} else if (s.action === 'cantusetwice') {
			for (const m of scopeMoves()) {
				if (m.flags['cantusetwice'] && !scope.conditions.has(m.id)) { addCond(m.id, 'volatile', `engine: ${s.ref} (flags.cantusetwice)`); changed = true; }
			}
		} else if (s.action === 'mindblown-recoil') {
			for (const m of scopeMoves()) {
				if (m.mindBlownRecoil && !scope.conditions.has(m.id)) { addCond(m.id, 'referenced', `engine: ${s.ref} (mindBlownRecoil effect)`); changed = true; }
			}
		}
		// transform-crit-volatiles: copies only volatiles the target already has; nothing new
	}
	return changed;
}
const RULE_NON_PSEUDO_KEYS = ['onBegin', 'onTeamPreview', 'onBattleStart', 'onValidateRule', 'onValidateTeam', 'onChangeSet', 'onValidateSet'];

// ----- Fixpoint -----
for (;;) {
	while (queue.length) {
		const [kind, id] = queue.shift();
		scanEffect(kind, id);
	}
	const changed = formePass() | engineActions();
	if (!changed && !queue.length) break;
}
for (const id of scope.species.keys()) formeRejected.delete(id);

// Dynamic refs: attach resolution or flag
const unresolved = [];
for (const d of dynamicRefs) {
	const note = resolvedNotes.get(d.effect) ?? resolveGenericDynamic(d);
	d.resolution = note ?? null;
	if (!note) unresolved.push(d);
}
/** Generic dynamic patterns that cannot introduce new ids */
function resolveGenericDynamic(d) {
	const a = d.arg;
	// self-referencing ids: this.effect.id / effect.id / this.effectState.* etc. referring to the owning effect
	if (/^(this\.effect(\.id)?|effect\.id|this\.effect\.name|this\.effect\.fullname)$/.test(a)) {
		if (d.call === 'useMove' || d.call === 'runMove') return null;
		return `self/handled-effect reference (${a}); no new id`;
	}
	if (/^(target|source|pokemon|attacker|ally|foe|dancer)\.(getAbility\(\)|ability|item|getItem\(\)|lastItem|baseAbility|set\.species|species|status|lastMove\.id)$/.test(a) ||
		/^(target|source|pokemon|ally)\.(getAbility|getItem)\(\)\.(id|name)$/.test(a) ||
		/^(myItem|yourItem|item|ability|oldAbility|targetAbility|sourceAbility|newAbility|abilityName|abilityID|targetItem|sourceItem|itemid)$/.test(a)) {
		return `copies/moves an existing in-scope value (${a}); no new id`;
	}
	if (/^species\.abilities\[/.test(a)) return 'forme ability (abilities[0] of a reachable forme, added by the forme pass)';
	if (/^(""|''|``)$/.test(a)) return 'empty (removal)';
	return null;
}

// Missing effects (referenced id that does not exist in the Dex)
const missing = [];
for (const kind of ['moves', 'abilities', 'items', 'species']) {
	for (const id of scope[kind].keys()) {
		const ok = kind === 'moves' ? dex.moves.get(id).exists : kind === 'abilities' ? dex.abilities.get(id).exists :
			kind === 'items' ? (id === '' || dex.items.get(id).exists) : species(id).exists;
		if (!ok) missing.push({ kind, id, via: [...scope[kind].get(id).via] });
	}
}
for (const id of scope.conditions.keys()) {
	const def = conditionDef(id);
	if (def.from === 'none') scope.conditions.get(id).undefinedCondition = true;
}

// ---------------------------------------------------------------------------
// 3. Per-effect records
// ---------------------------------------------------------------------------
const DATA_ON_KEYS = new Set(['onPlate', 'onMemory', 'onDrive']);
function handlerInfo(raw, { skipCondition = true } = {}) {
	const handlers = [];
	const constHandlers = {};
	const priorities = {};
	const nested = [];
	let srcChars = 0;
	let fnCount = 0;
	if (!raw) return { handlers, constHandlers, priorities, nested, fnCount, srcChars };
	for (const [k, v] of Object.entries(raw)) {
		if (typeof v === 'function') {
			handlers.push(k);
			fnCount++;
			srcChars += v.toString().length;
		} else if (/^on[A-Z]/.test(k) && !DATA_ON_KEYS.has(k)) {
			if (typeof v === 'number' && /(Priority|Order|SubOrder)$/.test(k)) priorities[k] = v;
			else constHandlers[k] = v;
		}
	}
	walkRaw(raw, (p, fn) => {
		if (!p.includes('.') && !p.includes('[')) return;
		if (skipCondition && p.startsWith('condition.')) return;
		nested.push(p);
		fnCount++;
		srcChars += fn.toString().length;
	}, () => {});
	return { handlers: handlers.sort(), constHandlers, priorities, nested, fnCount, srcChars };
}
function clean(v, depth = 0) {
	if (typeof v === 'function') return '<fn>';
	if (v === undefined) return undefined;
	if (Array.isArray(v)) return v.map(x => clean(x, depth + 1));
	if (v && typeof v === 'object') {
		if (depth > 5) return '<deep>';
		const o = {};
		for (const [k, x] of Object.entries(v)) {
			if (SKIP_DATA_KEYS.has(k)) continue;
			const c = clean(x, depth + 1);
			if (c !== undefined) o[k] = c;
		}
		return o;
	}
	return v;
}
function dataFields(raw, drop = []) {
	const o = {};
	if (!raw) return o;
	for (const [k, v] of Object.entries(raw)) {
		if (typeof v === 'function' || SKIP_DATA_KEYS.has(k) || drop.includes(k)) continue;
		if (/^on[A-Z]/.test(k) && !DATA_ON_KEYS.has(k)) continue; // in handler info
		if (k === 'condition') continue;
		const c = clean(v);
		if (c !== undefined) o[k] = c;
	}
	return o;
}
const viaList = e => [...e.via].sort();

const records = { species: [], moves: [], abilities: [], items: [], conditions: [], rules: [] };

for (const [id, e] of scope.species) {
	const sp = species(id);
	const raw = rawFor('species', id);
	const ps = perSpecies.get(id);
	records.species.push({
		id, name: sp.name, baseSpecies: sp.baseSpecies, forme: sp.forme, num: sp.num,
		types: sp.types, baseStats: sp.baseStats, abilities: sp.abilities, weighthg: sp.weighthg,
		gender: sp.gender || null, genderRatio: sp.genderRatio,
		battleOnly: sp.battleOnly ?? null, changesFrom: sp.changesFrom ?? null,
		requiredItem: sp.requiredItem ?? null, requiredItems: sp.requiredItems ?? null,
		requiredAbility: sp.requiredAbility ?? null, requiredMove: sp.requiredMove ?? null,
		requiredTeraType: sp.requiredTeraType ?? null, maxHP: sp.maxHP ?? null,
		cosmeticFormes: sp.cosmeticFormes ?? null,
		inTeams: seen.species.has(id), generatorKey: seen.speciesId.has(id),
		src: srcRef('data/pokedex.ts', id),
		speciesCondition: raw ? { id: toID(sp.baseSpecies), src: srcRef('data/conditions.ts', toID(sp.baseSpecies)), ...handlerInfo(raw) } : null,
		via: viaList(e),
		teamgen: ps ? { ...ps, declaredLevel: setsJson[id]?.level ?? null } : null,
	});
}
for (const [id, e] of scope.moves) {
	const m = dex.moves.get(id);
	const raw = D.Moves[id];
	const hi = handlerInfo(raw);
	const cond = raw?.condition ? handlerInfo(raw.condition) : null;
	records.moves.push({
		id, name: m.name, num: m.num, exists: m.exists, isNonstandard: m.isNonstandard ?? null,
		type: m.type, category: m.category, basePower: m.basePower, accuracy: m.accuracy, pp: m.pp,
		priority: m.priority, target: m.target, flags: m.flags,
		multihit: m.multihit ?? null, critRatio: m.critRatio,
		secondaries: clean(m.secondaries) ?? null,
		data: dataFields(raw, ['name', 'num', 'type', 'category', 'basePower', 'accuracy', 'pp', 'priority', 'target', 'flags', 'multihit', 'critRatio', 'secondary', 'secondaries', 'isNonstandard']),
		...hi,
		condition: cond ? { src: innerRef('data/moves.ts', id, 'condition'), ...cond } : null,
		inTeams: seen.move.has(id), declared: declared.moves.has(id),
		src: srcRef('data/moves.ts', id),
		via: viaList(e),
	});
}
for (const [id, e] of scope.abilities) {
	const a = dex.abilities.get(id);
	const raw = D.Abilities[id];
	const hi = handlerInfo(raw);
	records.abilities.push({
		id, name: a.name, num: a.num, exists: a.exists, isNonstandard: a.isNonstandard ?? null,
		flags: a.flags, data: dataFields(raw, ['name', 'num', 'flags', 'rating', 'isNonstandard']),
		...hi,
		condition: raw?.condition ? { src: innerRef('data/abilities.ts', id, 'condition'), ...handlerInfo(raw.condition) } : null,
		inTeams: seen.ability.has(id), declared: declared.abilities.has(id),
		refOnly: [...e.via].every(v => v.includes('(referenced as effect object)')),
		src: srcRef('data/abilities.ts', id),
		via: viaList(e),
	});
}
for (const [id, e] of scope.items) {
	if (!id) continue;
	const it = dex.items.get(id);
	const raw = D.Items[id];
	const hi = handlerInfo(raw);
	records.items.push({
		id, name: it.name, num: it.num, exists: it.exists, isNonstandard: it.isNonstandard ?? null,
		isBerry: it.isBerry, isGem: it.isGem, isChoice: !!raw?.isChoice, fling: it.fling ?? null,
		naturalGift: raw?.naturalGift ?? null,
		data: dataFields(raw, ['name', 'num', 'spritenum', 'isNonstandard', 'fling', 'isBerry', 'isGem', 'isChoice', 'naturalGift', 'gen']),
		...hi,
		condition: raw?.condition ? { src: innerRef('data/items.ts', id, 'condition'), ...handlerInfo(raw.condition) } : null,
		inTeams: seen.item.has(id),
		refOnly: [...e.via].every(v => v.includes('(referenced as effect object)')),
		src: srcRef('data/items.ts', id),
		via: viaList(e),
	});
}
for (const [id, e] of scope.conditions) {
	const def = conditionDef(id);
	const kinds = [...(condKind.get(id) || [])].filter(k => k !== 'own-condition' && k !== 'referenced');
	const explicitCreator = [...e.via].some(v => !v.endsWith('(own condition)'));
	records.conditions.push({
		id, name: def.raw?.name ?? (D.Moves[id]?.name || D.Abilities[id]?.name || D.Items[id]?.name || id),
		kinds: kinds.length ? kinds : [...(condKind.get(id) || [])],
		definedIn: def.from, src: def.src,
		exists: def.from !== 'none',
		data: dataFields(def.raw, ['name']),
		...handlerInfo(def.raw, { skipCondition: false }),
		explicitCreator,
		via: viaList(e),
	});
}
for (const [id, e] of scope.rules) {
	const f = dex.formats.get(id);
	const raw = D.Rulesets[id];
	const hi = handlerInfo(raw);
	const nonBattle = ['onValidateRule', 'onValidateTeam', 'onChangeSet', 'onValidateSet', 'checkCanLearn', 'validateSet', 'onTeamPreview'];
	records.rules.push({
		id, name: f.name, effectType: f.effectType, src: srcRef('data/rulesets.ts', id),
		...hi,
		battleHandlers: hi.handlers.filter(h => !nonBattle.includes(h)),
		addedAsPseudoWeather: Object.keys(f).some(k => k.startsWith('on') && !RULE_NON_PSEUDO_KEYS.includes(k)),
		via: viaList(e),
	});
}
for (const k of KINDS) if (k !== 'rules') records[k].sort((a, b) => a.id.localeCompare(b.id)); // rules keep ruleTable order

// ---------------------------------------------------------------------------
// Cross references, engine references and PRNG usage per effect
// ---------------------------------------------------------------------------
const LIT_RE = /(["'`])([^"'`\n]{1,48})\1/g;
const isFlagIndex = (text, idx) => /flags\[$/.test(text.slice(Math.max(0, idx - 6), idx));
const engineLits = new Map(); // literal -> [file:line]
for (const file of ENGINE_FILES) {
	loadSrc(file).lines.forEach((l, i) => {
		if (/^\s*(\/\/|\*)/.test(l)) return;
		for (const m of l.matchAll(LIT_RE)) {
			if (isFlagIndex(l, m.index)) continue;
			if (!engineLits.has(m[2])) engineLits.set(m[2], []);
			engineLits.get(m[2]).push(`${file}:${i + 1}`);
		}
	});
}
/** function sources owned by an effect (a move/ability/item's own condition belongs to the condition) */
function ownFns(kind, id) {
	const raw = rawFor(kind, id);
	const out = [];
	if (!raw) return out;
	walkRaw(raw, (p, fn) => {
		if (kind !== 'conditions' && p.startsWith('condition.')) return;
		out.push({ path: p, src: fn.toString() });
	}, () => {});
	return out;
}
const effectLits = new Map(); // literal -> Set(effectKey)
const fnsByKey = new Map();
for (const k of KINDS) {
	for (const r of records[k]) {
		const key = `${k}:${r.id}`;
		const fns = ownFns(k, r.id);
		fnsByKey.set(key, fns);
		for (const { src } of fns) {
			for (const m of src.matchAll(LIT_RE)) {
				if (isFlagIndex(src, m.index)) continue;
				if (!effectLits.has(m[2])) effectLits.set(m[2], new Set());
				effectLits.get(m[2]).add(key);
			}
		}
	}
}
const TYPE_NAMES = new Set(dex.types.names());
const PRNG_RE = /this\.(random|randomChance|sample|shuffle|speedSort|getRandomTarget|getRandomSwitchable)\(([^()]*(?:\([^()]*\))?[^()]*)\)|this\.prng\.(\w+)\(/g;
for (const k of ['moves', 'abilities', 'items', 'conditions', 'species']) {
	for (const r of records[k]) {
		const key = `${k}:${r.id}`;
		const twin = k === 'conditions' ? ['moves', 'abilities', 'items'].map(t => `${t}:${r.id}`) : [`conditions:${r.id}`];
		const lits = [r.id];
		if (r.name && r.name.length > 2 && !TYPE_NAMES.has(r.name) && r.name !== r.id) lits.push(r.name);
		const eng = new Set();
		const by = new Set();
		for (const l of lits) {
			for (const ref of engineLits.get(l) || []) eng.add(ref);
			for (const ek of effectLits.get(l) || []) if (ek !== key && !twin.includes(ek)) by.add(ek);
		}
		r.engineRefs = [...eng].sort();
		r.referencedBy = [...by].sort();
		const prng = [];
		for (const { path: p, src } of fnsByKey.get(key) || []) {
			for (const m of src.matchAll(PRNG_RE)) prng.push(`${p}: ${m[1] ? `${m[1]}(${m[2].trim()})` : `prng.${m[3]}()`}`);
		}
		r.prng = prng;
	}
}

// Pseudo-effects: ad-hoc effect objects/ids the engine uses as damage/heal/lock sources
const PSEUDO_EFFECTS = [
	['confused', 'data/conditions.ts', /toID\('confused'\)/, 'confusion self-hit: damage effect is the ad-hoc ActiveMove {id: "confused", effectType: "Move", type: "???"}'],
	['strugglerecoil', 'sim/battle-actions.ts', /id: 'strugglerecoil'/, 'Struggle recoil: directDamage with {id: "strugglerecoil"} (1/4 max HP, not affected by Magic Guard/Rock Head)'],
	['recoil', 'sim/dex-conditions.ts', /id === 'recoil'/, 'recoil damage effect (Condition "Recoil")'],
	['drain', 'sim/dex-conditions.ts', /id === 'drain'/, 'drain heal effect (Condition "Drain")'],
	['recharge', 'sim/pokemon.ts', /id: 'recharge'/, 'locked pseudo-move while mustrecharge (not a Dex move; exists=false)'],
	['struggle', 'sim/pokemon.ts', /id: 'struggle'/, 'Struggle is forced when no move is usable'],
	['fnt', 'sim/battle.ts', /status = 'fnt'/, 'fainted Pokemon get status "fnt"'],
	['mindBlownRecoil', 'sim/battle-actions.ts', /mindBlownRecoil \?/, 'self-damage effect is conditions.get(move.name), a bare non-existent condition named after the move'],
];
const pseudoEffects = PSEUDO_EFFECTS.map(([id, file, re, note]) => ({ id, refs: grepSrc(file, re).map(h => h.ref), note }));

// Rule references outside data/rulesets.ts
const RULE_REF_FILES = [...ENGINE_FILES, 'data/abilities.ts', 'data/moves.ts', 'data/items.ts', 'data/conditions.ts'];
for (const r of records.rules) {
	const re = new RegExp(`['"\`]${r.id}['"\`]`);
	r.referencedAt = RULE_REF_FILES.flatMap(f => grepSrc(f, re).map(h => h.ref));
}

// ---------------------------------------------------------------------------
// Complexity + hard effects
// ---------------------------------------------------------------------------
function complexity(r, kind) {
	let fn = r.fnCount, chars = r.srcChars;
	if (r.condition) { fn += r.condition.fnCount; chars += r.condition.srcChars; }
	if (r.speciesCondition) { fn += r.speciesCondition.fnCount; chars += r.speciesCondition.srcChars; }
	return { fn, chars, score: chars + 150 * fn };
}
for (const k of KINDS) for (const r of records[k]) r.complexity = complexity(r, k);

// Hand-written notes for the mechanically hardest effects, roughly most-to-least involved.
// Only entries that are in scope are emitted.
const HARD_NOTES = {
	'abilities:illusion': 'onBeforeSwitchIn picks the last non-fainted party member after its own slot (not Ogerpon/Terapagos once terastallized); disguise shows in switch/details lines; ends on damaging hit with `replace` + `-end`; Illusion Level Mod changes details (sim/pokemon.ts getDetails).',
	'abilities:imposter': 'On switch-in transforms into the foe in the mirrored slot (`foe.active[len-1-position]`), i.e. full Pokemon.transformInto semantics at switch-in time.',
	'moves:transform': 'Pokemon.transformInto: copies species, types, stored stats (not HP), boosts, moves at 5 PP, ability, crit volatiles; many fail cases; Ogerpon/Terapagos targets block later Tera.',
	'abilities:trace': 'Seeks on start; onUpdate samples (PRNG sample) one adjacent foe whose ability lacks notrace; keeps seeking until a valid foe appears.',
	'abilities:dancer': 'Engine special case (sim/battle-actions.ts useMoveInner): after a successful dance move every other active Dancer re-runs it, sorted by storedStats.spe ascending (ties: most recent ability first), with special target rules.',
	'moves:instruct': 'Re-queues the target\'s lastMove at lastMoveTargetLoc via queue.prioritizeAction; long fail list (charge/recharge/failinstruct/Beak Blast/Focus Punch/Shell Trap/0 PP).',
	'abilities:magicbounce': 'Bounces reflectable moves (onTryHit) and side-targeted ones for the ally side (onAllyTryHitSide) by calling useMove with a fresh ActiveMove marked hasBounced.',
	'moves:protect': 'onPrepareHit requires queue.willAct() and a successful StallMove event; adds stall volatile; protect condition blocks protect-flagged moves (breaksProtect / Feint / unprotectable exceptions).',
	'conditions:stall': 'Consecutive Protect: StallMove does randomChance(1, counter); counter starts 3 and triples on restart up to counterMax 729; duration 2.',
	'moves:wideguard': 'Side condition for one turn blocking allAdjacent/allAdjacentFoes moves; consumes the stall counter (onHitSide adds stall); cancels a fresh lockedmove.',
	'moves:burningbulwark': 'Protect variant (like Spiky Shield / Baneful Bunker): blocks like Protect, burns contact attackers (checkMoveMakesContact) from the condition onTryHit; a smartTarget move is silently retargeted (no -activate); cancels a fresh lockedmove; the Z/Max onHit branch is dead in Gen 9.',
	'moves:followme': 'Doubles redirection: volatile with onFoeRedirectTarget (priority 1) returning the user if validTarget; clears smartTarget; fails when activePerHalf is 1.',
	'abilities:stormdrain': 'onAnyRedirectTarget for single-target Water moves (randomNormal/adjacentFoe normalised to normal), clears smartTarget; absorbs with +1 SpA or immunity message.',
	'moves:dragondarts': 'smartTarget two-hit move: hits each foe once in doubles, or retargets the second hit when the first target is invalid/immune/protected (engine smartTarget logic).',
	'moves:populationbomb': 'multihit 10 with multiaccuracy: accuracy rolled per hit (stops on miss); Loaded Dice changes hit count/accuracy handling; Skill Link; Technician.',
	'moves:substitute': 'User pays 1/4 HP; substitute condition intercepts damage in onTryPrimaryHit (own damage calc, recoil and drain applied there, -activate [damage]); bypasssub/infiltrates skip it; removes partial trapping on start.',
	'moves:shedtail': 'Pays ceil(maxhp/2) via directDamage, creates a Substitute and selfSwitch "shedtail" so only the substitute is passed; fails if cannot switch or Commander.',
	'moves:batonpass': 'selfSwitch "copyvolatile": replacement inherits boosts and copyable volatiles (noCopy excluded); fails if no switch target.',
	'moves:partingshot': 'Drops Atk/SpA then selfSwitch only if a boost actually happened (or target has Mirror Armor); switch request in the middle of the turn.',
	'moves:dragontail': 'forceSwitch: target is dragged out and replaced by getRandomSwitchable (PRNG sample) at the end of the move; fails silently on various conditions.',
	'moves:revivalblessing': 'Adds revivalblessing slot condition + selfSwitch so the player must pick a fainted party member to revive mid-turn (special request); interacts with doubles slot occupancy.',
	'moves:trick': 'takeItem on both sides plus TakeItem singleEvents for permission; Sticky Hold immunity; restores items on failure; swapped items run their Start handlers.',
	'moves:knockoff': '1.5x power only if the TakeItem singleEvent allows removal (plates/masks/Rusted items/Booster Energy on Paradox are fixed); removal in onAfterHit, not through substitute.',
	'abilities:magician': 'After its move, speedSorts move.hitTargets (PRNG on speed ties) and steals the first takeable item.',
	'moves:terablast': 'Type = teraType when terastallized; Physical if boosted Atk > SpA; Stellar: 100 BP and self -1 Atk/-1 SpA; Stellar damage rules in the engine.',
	'moves:terastarstorm': 'For Terapagos-Stellar: type Stellar, target becomes allAdjacentFoes, Physical if terastallized and Atk > SpA.',
	'abilities:terashift': 'onSwitchIn (priority 2): Terapagos permanently becomes Terapagos-Terastal (ability -> Tera Shell via formeChange isPermanent).',
	'abilities:terashell': 'At full HP every damaging hit is not very effective (checked per target / per hit, including spread moves).',
	'abilities:teraformzero': 'When Terapagos becomes Stellar (engine terastallize -> Terapagos-Stellar), clears weather and terrain.',
	'abilities:embodyaspectteal': 'Ogerpon Tera formes (engine terastallize -> Ogerpon-*-Tera, permanent formeChange with source=null) get Embody Aspect: one stat +1 once per battle on start.',
	'moves:shellsidearm': 'Chooses Physical vs Special by comparing two simplified damage formulas; calls randomChance(1, 2) only when they are equal; physical version makes contact.',
	'abilities:protosynthesis': 'On start/weather change adds/removes protosynthesis volatile; condition picks bestStat (getBestStat(false, true)), 5325/4096 boost (1.5x Spe); Booster Energy variant persists without sun.',
	'abilities:quarkdrive': 'Electric Terrain twin of Protosynthesis (quarkdrive volatile, bestStat, Booster Energy).',
	'items:boosterenergy': 'onStart/onUpdate: if Protosynthesis/Quark Drive without sun/terrain, useItem then addVolatile; cannot be removed from Paradox Pokemon.',
	'abilities:disguise': 'Mimikyu: onDamage -> 0 and busted; onCriticalHit false and onEffectiveness 0 tricks; onUpdate permanent formeChange to Mimikyu-Busted then 1/8 damage whose effect is a Species object.',
	'abilities:iceface': 'Eiscue blocks one physical hit (busted -> Eiscue-Noice in onUpdate), restored on snow at start/weather change; crit/effectiveness tricks like Disguise.',
	'abilities:shieldsdown': 'Minior: Meteor forme above 50% HP, Core (set.species) at or below, checked on start and residual; Meteor is status- and Yawn-immune.',
	'abilities:hungerswitch': 'Morpeko alternates Full Belly/Hangry at residual (order 29) unless terastallized; Aura Wheel type follows.',
	'abilities:gulpmissile': 'Surf loads Gulping (>50% HP) or Gorging forme; when hit: 1/4 attacker max HP damage plus Def -1 or paralysis, then reverts.',
	'abilities:zerotohero': 'Palafin becomes Palafin-Hero permanently on switch-out; activation message on the next switch-in.',
	'moves:relicsong': 'onAfterMoveSecondarySelf toggles Meloetta Aria/Pirouette (non-permanent formeChange); spread move with 10% sleep.',
	'conditions:confusion': 'Duration random(2, 6) (random(3, 6) from Axe Kick); each BeforeMove decrements, prints -activate, randomChance(33, 100) self-hit for getConfusionDamage(40) with pseudo-move {id: "confused"}.',
	'conditions:slp': 'Duration random(2, 5); Early Bird double decrement; sleepUsable moves still run; Sleep Clause Mod (onSetStatus) and Rest (fixed 3) interplay.',
	'conditions:frz': 'BeforeMove randomChance(1, 5) thaw; defrost moves and thawsTarget/Fire hits thaw; Shaymin-Sky reverts to Shaymin permanently (ability becomes Natural Cure).',
	'conditions:partiallytrapped': 'durationCallback random(5, 7) (Grip Claw 8); residual 1/8 (Binding Band 1/6) while the source stays active; traps.',
	'conditions:lockedmove': 'Rampage lock: trueDuration random(2, 4); ends with confusion (fatigue) unless interrupted; sleep releases without confusion.',
	'conditions:twoturnmove': 'Charge turn: stores move id, adds a volatile named after the move, remembers target location (random foe if called move), runs PrepareHit, locks the move.',
	'moves:electroshot': 'Charge move: +1 SpA on the charge turn, skips charging in rain (effectiveWeather), ChargeMove event (Power Herb), else twoturnmove.',
	'conditions:choicelock': 'Locks to the first move used while holding a Choice item (not bounced/Snatch); disables other moves; removed when item is not a choice item.',
	'conditions:sunnyday': 'Weather: 5 turns via durationCallback (Heat Rock not in scope); onWeatherModifyDamage Fire 1.5x / Water 0.5x keyed on the defender\'s effectiveWeather(); frz immunity; field residual (order 1) prints the upkeep line then runs eachEvent("Weather"); drives Protosynthesis, Orichalcum Pulse, Chlorophyll, Solar Beam, Harvest.',
	'conditions:psychicterrain': 'Terrain: blocks priority moves against grounded targets (including the ally side in doubles), 1.3x Psychic for grounded users; Expanding Force spread conversion.',
	'moves:trickroom': 'Pseudo-weather (5 turns) that reverses speed ordering in the action queue; using it again ends it.',
	'moves:auroraveil': 'Only in snow; screen modifier 2732/4096 in doubles (0.5 singles), not vs crits/infiltrator; Light Clay 8 turns.',
	'moves:helpinghand': 'Priority +5 ally move; 1.5x base power multiplier that multiplies again if used twice; fails if the ally already moved.',
	'moves:pollenpuff': 'Against an ally: base power 0 and heals 50% instead of damaging; blocked by Heal Block on the user.',
	'moves:encore': 'Locks the target into its last move for 3 turns (+1 if it has not moved yet); onOverrideAction rewrites an already-queued action.',
	'moves:suckerpunch': 'Fails unless the target\'s queued action is a non-status move (queue.willMove lookahead).',
	'items:whiteherb': 'Restores negative boosts; re-checked on AnySwitchIn, AnyAfterMove and residual (order 29); boosts captured in effectState for onUse; Fling effect.',
	'abilities:intimidate': 'On start lowers each adjacent foe\'s Atk in order (substitute -> -immune); boost runs TryBoost/AfterBoost (Mirror Armor, Clear Body, Defiant, Competitive, Rattled...).',
	'abilities:naturalcure': 'Cures on switch-out; doubles onCheckShow logic decides whether the cure is revealed in the protocol.',
	'abilities:beadsofruin': 'Ruin abilities: onAny modifier tracked per move (move.ruinedSpD) so only one holder applies; holders of the same ability are exempt.',
	'abilities:moldbreaker': 'Sets move.ignoreAbility so breakable abilities are suppressed for the duration of the move (also Turboblaze/Teravolt/Mycelium Might).',
	'abilities:protean': 'Before the move (onPrepareHit), once per switch-in, changes type to the move type; excluded for bounced/future/called moves.',
	'moves:curse': 'Ghost vs non-Ghost behaviour decided at onModifyMove/onTryHit (target rewritten, volatileStatus/onHit deleted for non-Ghosts); Ghost curse costs 1/2 HP and deals 1/4 per turn.',
	'conditions:healblock': 'Psychic Noise heal block (2 turns via durationCallback): disables heal-flagged moves, blocks healing (onTryHeal), special Pollen Puff handling.',
	'items:rustedsword': 'With the zacian species condition: at BattleStart Zacian becomes Zacian-Crowned (setSpecies, ability reset) and Iron Head becomes Behemoth Blade; item cannot be removed.',
};
const hardCandidates = [];
for (const k of ['moves', 'abilities', 'items', 'conditions', 'species']) {
	for (const r of records[k]) {
		// conditions owned by a move/ability/item are scored together with their owner
		if (k === 'conditions' && ['move', 'ability', 'item'].includes(r.definedIn) && scope[{ move: 'moves', ability: 'abilities', item: 'items' }[r.definedIn]].has(r.id)) continue;
		const note = HARD_NOTES[`${k}:${r.id}`];
		hardCandidates.push({ key: `${k}:${r.id}`, kind: k, id: r.id, name: r.name, score: r.complexity.score, fn: r.complexity.fn, note: note || null, src: r.src });
	}
}
hardCandidates.sort((a, b) => b.score - a.score);

// ---------------------------------------------------------------------------
// 4. Output
// ---------------------------------------------------------------------------
// Probes: closure sources named in the task, reported as in/out of scope
const PROBES = {
	moves: ['sleeptalk', 'copycat', 'instruct', 'metronome', 'mirrormove', 'mefirst', 'assist', 'snatch', 'magiccoat', 'transform', 'mimic', 'sketch',
		'naturepower', 'trick', 'switcheroo', 'thief', 'covet', 'bestow', 'knockoff', 'recycle', 'fling', 'skillswap', 'roleplay', 'entrainment',
		'doodle', 'worryseed', 'simplebeam', 'gastroacid', 'bugbite', 'pluck', 'teatime', 'allyswitch', 'futuresight', 'doomdesire',
		'revivalblessing', 'batonpass', 'shedtail', 'struggle', 'focuspunch', 'beakblast', 'shelltrap', 'pursuit', 'skydrop', 'perishsong'],
	abilities: ['trace', 'imposter', 'receiver', 'powerofalchemy', 'mummy', 'lingeringaroma', 'wanderingspirit', 'pickup', 'harvest', 'magician',
		'pickpocket', 'symbiosis', 'dancer', 'magicbounce', 'illusion', 'neutralizinggas', 'commander', 'costar', 'opportunist', 'emergencyexit',
		'wimpout', 'parentalbond', 'stancechange', 'zenmode', 'schooling', 'powerconstruct', 'forecast', 'flowergift', 'mimicry', 'moody', 'cudchew'],
	items: ['ejectbutton', 'ejectpack', 'redcard', 'mirrorherb', 'roomservice', 'adrenalineorb', 'abilityshield', 'utilityumbrella', 'airballoon',
		'quickclaw', 'custapberry', 'focusband', 'kingsrock', 'metronome', 'stickybarb', 'lumberry', 'leppaberry', 'terrainextender', 'heatrock',
		'damprock', 'smoothrock', 'icyrock', 'gripclaw', 'bindingband', 'safetygoggles', 'protectivepads', 'punchingglove', 'shedshell'],
};
const probes = Object.fromEntries(Object.entries(PROBES).map(([k, ids]) => [k, {
	inScope: ids.filter(id => scope[k].has(id)), notInScope: ids.filter(id => !scope[k].has(id)),
}]));

const satDeltas = satur.map((s, i) => {
	const prev = satur[i - 1];
	const out = { teams: s.teams };
	for (const c of CATS) out[c] = { total: s[c], new: prev ? s[c] - prev[c] : s[c] };
	return out;
});
const lastSeen = cat => [...seen[cat].entries()].sort((a, b) => b[1].first - a[1].first).slice(0, 10).map(([v, e]) => ({ value: v, firstTeam: e.first, count: e.count }));
const rarest = cat => [...seen[cat].entries()].sort((a, b) => a[1].count - b[1].count).slice(0, 15).map(([v, e]) => ({ value: v, count: e.count, firstTeam: e.first }));

const showdownCommit = (() => {
	try {
		const head = fs.readFileSync(path.join(PS, '.git', 'HEAD'), 'utf8').trim();
		if (head.startsWith('ref: ')) return fs.readFileSync(path.join(PS, '.git', head.slice(5)), 'utf8').trim();
		return head;
	} catch { return null; }
})();

const teamgenSummary = {
	teams: opts.teams, seconds: +genSeconds.toFixed(1), failures, badTeamSizes: Object.fromEntries(badTeamSizes),
	seedScheme: `sodium,<first 32 hex of sha256("${opts.seedPrefix}:<i>")>, fresh generator per team`,
	counts: Object.fromEntries(CATS.map(c => [c, seen[c].size])),
	saturation: satDeltas,
	lastFirstSeen: Object.fromEntries(['species', 'item', 'ability', 'move', 'teraType', 'speciesAbility', 'speciesItem', 'speciesMove'].map(c => [c, lastSeen(c)])),
	rarest: Object.fromEntries(['item', 'ability', 'move', 'species', 'teraType'].map(c => [c, rarest(c)])),
	values: {
		levels: [...seen.level.keys()].sort((a, b) => a - b),
		teraTypes: [...seen.teraType.keys()].sort(),
		genders: Object.fromEntries([...seen.gender.entries()].map(([k, v]) => [k, v.count])),
		natures: Object.fromEntries([...seen.nature.entries()].map(([k, v]) => [k, v.count])),
		evs: Object.fromEntries([...seen.evs.entries()].sort((a, b) => b[1].count - a[1].count).map(([k, v]) => [k, v.count])),
		ivs: Object.fromEntries([...seen.ivs.entries()].sort((a, b) => b[1].count - a[1].count).map(([k, v]) => [k, v.count])),
		roles: Object.fromEntries([...seen.role.entries()].map(([k, v]) => [k, v.count])),
		setKeys: Object.fromEntries([...seen.setKeys.entries()].map(([k, v]) => [k, v.count])),
		shiny: Object.fromEntries([...seen.shiny.entries()].map(([k, v]) => [k, v.count])),
		genderMismatch: Object.fromEntries([...seen.genderMismatch.entries()].map(([k, v]) => [k, v.count])),
	},
	declaredNotGenerated: {
		species: [...declared.species].filter(s => !seen.speciesId.has(s)).sort(),
		moves: [...declared.moves].filter(s => !seen.move.has(s)).sort(),
		abilities: [...declared.abilities].filter(s => !seen.ability.has(s)).sort(),
		teraTypes: [...declared.teraTypes].filter(s => !seen.teraType.has(s)).sort(),
		roles: [...declared.roles].filter(s => !seen.role.has(s)).sort(),
	},
	itemsStaticNotGenerated: [...staticItems.entries()].filter(([id]) => !seen.item.has(id)).map(([id, where]) => ({ id, where: where.slice(0, 4) })),
	itemsGeneratedNotStatic: [...seen.item.keys()].filter(id => id && !staticItems.has(id)),
	generatedNotDeclared: {
		moves: [...seen.move.keys()].filter(s => !declared.moves.has(s)).sort(),
		abilities: [...seen.ability.keys()].filter(s => !declared.abilities.has(s)).sort(),
	},
};

const out = {
	meta: {
		format: format.name, formatId: FORMAT_ID, formatSrc: formatSrcRef(), mod: format.mod, gameType: format.gameType,
		ruleset: format.ruleset, showdownPath: PS, showdownCommit, generatedAt: new Date().toISOString(),
		generator: 'tools/scope/extract-scope.mjs',
	},
	counts: {
		species: records.species.length,
		speciesInTeams: records.species.filter(r => r.inTeams || r.generatorKey).length,
		moves: records.moves.length, movesInTeams: records.moves.filter(r => r.inTeams).length,
		abilities: records.abilities.length, abilitiesInTeams: records.abilities.filter(r => r.inTeams).length,
		abilitiesRefOnly: records.abilities.filter(r => r.refOnly).length,
		items: records.items.length, itemsInTeams: records.items.filter(r => r.inTeams).length,
		itemsRefOnly: records.items.filter(r => r.refOnly).length,
		conditions: records.conditions.length,
		conditionsByKind: (() => { const o = {}; for (const r of records.conditions) for (const k of r.kinds) o[k] = (o[k] || 0) + 1; return o; })(),
		rules: records.rules.length,
		teraTypes: seen.teraType.size,
		levels: seen.level.size,
	},
	teamgen: teamgenSummary,
	closure: {
		dynamicRefs, unresolvedDynamicRefs: unresolved, engineSites, engineUnclassified,
		formeRejected: Object.fromEntries(formeRejected), missing,
		conditionsWithoutExplicitCreator: records.conditions.filter(r => !r.explicitCreator).map(r => r.id),
		undefinedConditions: records.conditions.filter(r => !r.exists).map(r => ({ id: r.id, via: r.via })),
	},
	species: records.species, moves: records.moves, abilities: records.abilities, items: records.items,
	conditions: records.conditions, rules: records.rules,
	pseudoEffects,
	probes,
	hardEffects: null,
	pureData: null,
};

// Pure data: no function handlers, no constant handlers, no nested handlers, no condition handlers
const isPure = r => r.fnCount === 0 && Object.keys(r.constHandlers).length === 0 &&
	(!r.condition || (r.condition.fnCount === 0 && !Object.keys(r.condition.constHandlers).length)) &&
	(!r.speciesCondition);
const isTrivial = r => isPure(r) && !r.engineRefs.length && !r.referencedBy.length;
// Data fields/flags that the generic engine interprets specially (so "pure data" is not "plain damage")
const SPECIAL_MOVE_FIELDS = ['smartTarget', 'forceSwitch', 'selfSwitch', 'multiaccuracy', 'multihit', 'selfdestruct', 'ohko',
	'sleepUsable', 'stallingMove', 'breaksProtect', 'tracksTarget', 'hasCrashDamage', 'mindBlownRecoil', 'struggleRecoil',
	'stealsBoosts', 'willCrit', 'overrideOffensiveStat', 'overrideOffensivePokemon', 'overrideDefensiveStat', 'ignoreDefensive',
	'ignoreEvasion', 'ignoreAbility', 'ignoreImmunity', 'thawsTarget', 'isFutureMove', 'pseudoWeather', 'weather', 'terrain',
	'sideCondition', 'slotCondition', 'volatileStatus', 'callsMove', 'damage', 'drain', 'recoil', 'heal', 'selfBoost', 'self',
	'hasSheerForceBoost', 'critRatio', 'noPPBoosts'];
const SPECIAL_MOVE_FLAGS = ['cantusetwice', 'charge', 'recharge', 'futuremove', 'pledgecombo', 'heal', 'sound', 'powder', 'dance', 'wind', 'slicing', 'bullet', 'bite', 'punch', 'pulse', 'contact'];
for (const r of records.moves) {
	const raw = D.Moves[r.id] || {};
	r.dataFeatures = [
		...SPECIAL_MOVE_FIELDS.filter(k => raw[k] !== undefined && raw[k] !== null && raw[k] !== false && !(k === 'critRatio' && raw[k] === 1)),
		...SPECIAL_MOVE_FLAGS.filter(f => r.flags?.[f]).map(f => `flag:${f}`),
		...(r.target && !['normal', 'self', 'any'].includes(r.target) ? [`target:${r.target}`] : []),
	];
}
out.pureData = {
	// no handlers AND never named by the engine (sim/*.ts) or by another in-scope effect's handlers
	trivial: Object.fromEntries(['moves', 'abilities', 'items', 'conditions'].map(k => [k, records[k].filter(isTrivial).map(r => r.id)])),
	trivialMoveFeatures: Object.fromEntries(records.moves.filter(isTrivial).filter(r => r.dataFeatures.length).map(r => [r.id, r.dataFeatures])),
	// no handlers of their own, but behaviour lives in the engine or in other effects' handlers
	noHandlersButReferenced: Object.fromEntries(['moves', 'abilities', 'items', 'conditions'].map(k => [k,
		records[k].filter(r => isPure(r) && !isTrivial(r)).map(r => ({ id: r.id, engineRefs: r.engineRefs.slice(0, 8), referencedBy: r.referencedBy.slice(0, 8) }))])),
	speciesWithoutHandlers: records.species.filter(r => !r.speciesCondition).length,
};

// Hard effects: every curated note whose effect is in scope, in the (hand-chosen) table order.
// Handler size alone under-ranks effects implemented in the engine (Dancer, Transform, Tera Shell).
const hardOrder = Object.keys(HARD_NOTES);
const hard = hardCandidates.filter(h => h.note).sort((a, b) => hardOrder.indexOf(a.key) - hardOrder.indexOf(b.key));
out.hardEffects = hard;
out.hardNotesNotInScope = Object.keys(HARD_NOTES).filter(k => !hard.some(h => h.key === k));
out.complexityTop = hardCandidates.slice(0, 80).map(h => ({ key: h.key, score: h.score, fn: h.fn, curated: !!h.note }));

fs.mkdirSync(path.dirname(opts.json), { recursive: true });
fs.writeFileSync(opts.json, JSON.stringify(out, null, '\t') + '\n');
log(`wrote ${opts.json}`);

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------
const md = [];
const rel = p => path.relative(REPO, p);
const esc = s => String(s ?? '').replace(/\|/g, '\\|');
const tbl = (head, rows) => {
	md.push(`| ${head.join(' | ')} |`);
	md.push(`| ${head.map(() => '---').join(' | ')} |`);
	for (const r of rows) md.push(`| ${r.map(esc).join(' | ')} |`);
	md.push('');
};
const formeParents = id => [...seen.formeOf.keys()].filter(k => k.startsWith(id + '<') && !k.endsWith('<' + id)).map(k => k.split('<')[1]).join('/');
const formeCount = id => [...seen.formeOf.entries()].filter(([k]) => k.startsWith(id + '<')).reduce((a, [, v]) => a + v.count, 0);
const shortVia = r => r.via.filter(v => !v.startsWith('teamgen')).slice(0, 3).join('; ');

md.push(`# 06 — Scope of ${format.name}`);
md.push('');
md.push(`Generated by \`node ${rel(path.join(REPO, 'tools/scope/extract-scope.mjs'))} ${PS}\` — do not edit by hand.`);
md.push(`Machine-readable data: \`${rel(opts.json)}\`. Showdown commit \`${showdownCommit}\`, format \`${FORMAT_ID}\` (${out.meta.formatSrc}), mod \`${format.mod}\`, gameType \`${format.gameType}\`.`);
md.push('');
md.push('## How this was computed');
md.push('');
md.push(`1. Ran the real generator \`Teams.getGenerator('${FORMAT_ID}', seed).getTeam()\` for ${opts.teams.toLocaleString('en-US')} deterministic sodium seeds (one fresh generator per seed; ${genSeconds.toFixed(0)} s, ${failures} failures). Seed i = \`${teamgenSummary.seedScheme}\`.`);
md.push('2. Closure worklist over every in-scope effect: data fields (`status`, `volatileStatus`, `sideCondition`, `slotCondition`, `weather`, `terrain`, `pseudoWeather`, `forceStatus`, nested in `self`/`secondaries`/`fling`) plus the source text of every function the effect defines, matched for `addVolatile`/`addSideCondition`/`addSlotCondition`/`setWeather`/`setTerrain`/`addPseudoWeather`/`setStatus`/`trySetStatus`/`setAbility`/`setItem`/`formeChange`/`species.get`/`moves.get`/`getActiveMove`/`useMove`/`runMove`/`abilities.get`/`items.get`/`conditions.get` with literal arguments, plus linked volatiles. Calls with non-literal arguments are listed in `closure.dynamicRefs` and resolved by an explicit table (`DYNAMIC_RESOLUTIONS` in the script).');
md.push('3. Battle formes: any forme whose `battleOnly` is an in-scope forme and whose trigger (`requiredAbility`/`requiredMove`/`requiredItem`/Tera) is in scope, plus `formeChange`/`species.get` literals; Ogerpon/Terapagos Tera formes from `sim/battle-actions.ts` terastallize. A permanent forme change sets `abilities[0]` (`sim/pokemon.ts` formeChange), so those abilities are added.');
md.push('4. Engine sites in `sim/*.ts` that create effects are grepped and classified (`ENGINE_SITES`); every in-scope move/ability/item `condition` is included conservatively (flagged when no explicit creator was found).');
md.push('');
md.push('## Scope guarantees and limits');
md.push('');
md.push('This is a conservative porting catalog, not a formal proof that every listed record is instantiated in a legal generated battle. Generator observations are cross-checked against every declared species, move, ability, role and Tera type in doubles-sets.json. Item selection is executable code: unobserved static item literals still require branch analysis; saturation alone cannot prove absence. The item saturation count includes the empty item string; the item record count excludes it.');
md.push('');
md.push('Handler-source matching is a source scan with explicit dynamic-reference resolutions, not a TypeScript control-flow analysis. Zero unresolved references means every detected call was classified, not that the scanner proves exhaustive reachability. Engine references and inactive branches can conservatively add records: `lockedmove` is used as a source-effect object for locked charging/recharge actions, even though no generated Outrage-family move creates its volatile; `skillswap` is cataloged from the generic engine helper even though the move is not generated. Consult `via`, `kinds`, and the owning source before allocating persistent state for a catalog entry.');
md.push('');
md.push('`closure.undefinedConditions` records names without a standalone condition definition. Some are intentional empty per-move volatile/source-effect records (charging move ids, Blood Moon/Gigaton Hammer hints, and High Jump Kick failure). They are not automatically missing mechanics; inspect the engine caller. Format-local ids and compact-state bounds must be based on this distinction rather than on record counts alone. Recompute this catalog when the pinned source, generator configuration or team source changes.');
md.push('');
md.push('## Counts');
md.push('');
tbl(['what', 'in scope', 'of which produced by team generator', 'notes'], [
	['species/formes', out.counts.species, out.counts.speciesInTeams, `${seen.speciesId.size} generator keys, ${seen.species.size} distinct set.species formes`],
	['moves', out.counts.moves, out.counts.movesInTeams, 'incl. Struggle and closure additions'],
	['abilities', out.counts.abilities, out.counts.abilitiesInTeams, `${out.counts.abilitiesRefOnly} referenced only as effect objects`],
	['items', out.counts.items, out.counts.itemsInTeams, `${out.counts.itemsRefOnly} referenced only as effect objects`],
	['conditions', out.counts.conditions, '-', Object.entries(out.counts.conditionsByKind).map(([k, v]) => `${k} ${v}`).join(', ')],
	['format rules', out.counts.rules, '-', `${records.rules.filter(r => r.addedAsPseudoWeather).length} become field pseudo-weathers`],
	['tera types', seen.teraType.size, seen.teraType.size, [...seen.teraType.keys()].sort().join(', ')],
	['levels', seen.level.size, seen.level.size, `${Math.min(...seen.level.keys())}-${Math.max(...seen.level.keys())}`],
]);
md.push('## Saturation (cumulative distinct values after N teams; +new since previous checkpoint)');
md.push('');
const satCats = ['speciesId', 'species', 'move', 'ability', 'item', 'teraType', 'level', 'evs', 'ivs', 'speciesAbility', 'speciesItem', 'speciesMove', 'speciesTeraType'];
tbl(['teams', ...satCats], satDeltas.map(s => [s.teams.toLocaleString('en-US'), ...satCats.map(c => `${s[c].total} (+${s[c].new})`)]));
md.push('Latest first appearances (team index, 0-based) — a late first appearance means the value is rare, not that the set is unsaturated:');
md.push('');
for (const c of ['item', 'ability', 'move', 'speciesItem', 'speciesAbility']) {
	md.push(`- **${c}**: ${teamgenSummary.lastFirstSeen[c].slice(0, 6).map(x => `${x.value} @${x.firstTeam} (n=${x.count})`).join(', ')}`);
}
md.push('');
md.push('Rarest values (count over all generated Pokemon):');
md.push('');
for (const c of ['item', 'ability', 'move']) md.push(`- **${c}**: ${teamgenSummary.rarest[c].slice(0, 10).map(x => `${x.value} (${x.count})`).join(', ')}`);
md.push('');
md.push('Declared in `doubles-sets.json` but never generated:');
md.push('');
for (const [k, v] of Object.entries(teamgenSummary.declaredNotGenerated)) md.push(`- ${k}: ${v.length ? v.join(', ') : '(none)'}`);
md.push('');
md.push('Items are computed by code, not declared in the JSON. Static cross-check: item-name literals in `getPriorityItem`/`getDoublesItem` plus `requiredItem(s)` of generator species:');
md.push('');
md.push(`- static universe: ${staticItems.size} items; generated: ${[...seen.item.keys()].filter(Boolean).length} (+ no item)`);
md.push(`- in the static universe but never generated (singles-only branches, unmet conditions, or Z-Crystals in requiredItems): ${teamgenSummary.itemsStaticNotGenerated.map(x => `${x.id} [${x.where[0]}]`).join(', ') || 'none'}`);
md.push(`- generated but not a literal in those methods: ${teamgenSummary.itemsGeneratedNotStatic.join(', ') || 'none'}`);
md.push('');
md.push('## Team-generation facts (what sets look like)');
md.push('');
md.push(`- Set keys: ${Object.entries(teamgenSummary.values.setKeys).map(([k, v]) => `\`${k}\` (${v})`).join('; ')}`);
md.push(`- Natures: ${Object.entries(teamgenSummary.values.natures).map(([k, v]) => `${k} (${v})`).join(', ')} — no nature is set, the battle uses a neutral nature.`);
md.push(`- Gender (speciesGender:setGender): ${Object.entries(teamgenSummary.values.genders).map(([k, v]) => `${k} (${v})`).join(', ')}. The generator always sets gender, so the \`this.battle.sample(['M', 'F'])\` fallback in sim/pokemon.ts (Pokemon constructor) is never reached; set.gender wins over the forme's fixed gender.`);
md.push(`- Gender differing from the forme's fixed gender (kept as-is by the Pokemon constructor): ${Object.entries(teamgenSummary.values.genderMismatch).map(([k, v]) => `${k} x${v}`).join('; ') || 'none'}`);
md.push(`- Shiny: ${Object.entries(teamgenSummary.values.shiny).map(([k, v]) => `${k} (${v})`).join(', ')} — shiny is part of the protocol details string (sim/pokemon.ts getUpdatedDetails), so it matters for log equality.`);
md.push(`- EV patterns hp/atk/def/spa/spd/spe: ${Object.entries(teamgenSummary.values.evs).map(([k, v]) => `${k} (${v})`).join(', ')}`);
md.push(`- IV patterns: ${Object.entries(teamgenSummary.values.ivs).map(([k, v]) => `${k} (${v})`).join(', ')}`);
md.push(`- Levels: ${teamgenSummary.values.levels.join(', ')}`);
md.push(`- Roles: ${Object.entries(teamgenSummary.values.roles).map(([k, v]) => `${k} (${v})`).join(', ')}`);
md.push(`- Team sizes other than 6: ${badTeamSizes.size ? JSON.stringify(Object.fromEntries(badTeamSizes)) : 'none'}`);
{
	const noItem = [...perSpecies.entries()].filter(([, ps]) => ps.items['']).map(([id, ps]) => `${id} (${ps.items['']}/${ps.count})`);
	md.push(`- Pokemon generated with no item (item ''): ${noItem.length ? noItem.join(', ') : 'none'}`);
}
md.push('');
md.push('## Closure additions (not produced directly by the generator)');
md.push('');
const closureRows = [];
for (const k of ['species', 'moves', 'abilities', 'items']) {
	for (const r of records[k]) {
		if (k === 'species' ? (r.inTeams || r.generatorKey) : r.inTeams) continue;
		closureRows.push([k, r.id, r.refOnly ? 'ref-only' : '', r.via.slice(0, 4).join('; ')]);
	}
}
tbl(['kind', 'id', 'flag', 'reached via'], closureRows);
md.push('Battle formes rejected (trigger not in scope):');
md.push('');
{
	const groups = new Map();
	for (const [id, why] of formeRejected) {
		const g = /Mega requires/.test(why) ? 'Mega (no Mega Stone generated)' : /Primal requires/.test(why) ? 'Primal (no orb generated)' :
			/Gigantamax/.test(why) ? 'Gigantamax' : /Ultra Burst/.test(why) ? 'Ultra Burst' : null;
		if (g) { if (!groups.has(g)) groups.set(g, []); groups.get(g).push(id); } else md.push(`- ${id}: ${why}`);
	}
	for (const [g, ids] of groups) md.push(`- ${g}: ${ids.length} formes (${ids.join(', ')})`);
}
md.push('');
md.push('### Probes: closure sources checked explicitly');
md.push('');
for (const [k, v] of Object.entries(probes)) {
	md.push(`- ${k} in scope: ${v.inScope.join(', ') || '(none)'}`);
	md.push(`- ${k} NOT in scope (never generated nor reachable): ${v.notInScope.join(', ') || '(none)'}`);
}
md.push('');
md.push('### Dynamic (non-literal) references and their resolution');
md.push('');
const dynGroups = new Map();
for (const d of dynamicRefs) {
	const k = `${d.effect}`;
	if (!dynGroups.has(k)) dynGroups.set(k, []);
	dynGroups.get(k).push(d);
}
tbl(['effect', 'calls', 'resolution'], [...dynGroups.entries()].sort().map(([k, ds]) => [k,
	[...new Set(ds.map(d => `${d.call}(${d.arg})`))].join(', ').slice(0, 140),
	[...new Set(ds.map(d => d.resolution ?? 'UNRESOLVED'))].join('; ').slice(0, 160)]));
md.push(`Unresolved dynamic references: ${unresolved.length}${unresolved.length ? ' — ' + unresolved.map(d => `${d.effect}.${d.path} ${d.call}(${d.arg})`).join('; ') : ''}`);
md.push('');
md.push('### Engine call sites (sim/*.ts)');
md.push('');
const engRows = engineSites.filter(s => s.action !== 'no-new');
tbl(['site', 'action', 'note'], engRows.map(s => [s.ref, s.action, s.note]));
md.push(`Plumbing sites classified "no-new": ${engineSites.length - engRows.length}. Unclassified engine sites: ${engineUnclassified.length}${engineUnclassified.length ? ' — ' + engineUnclassified.map(h => `${h.ref} \`${h.text.slice(0, 80)}\``).join('; ') : ''}.`);
md.push('');
md.push('## Format rules');
md.push('');
md.push('In ruleTable order (this is the order `onBegin` handlers run, which emits the `|rule|` protocol lines). Rules with any `on*` key other than onBegin/onBattleStart/validation are added to the field as pseudo-weathers at Battle construction (sim/battle.ts), so they participate in event dispatch.');
md.push('');
tbl(['rule', 'src', 'battle handlers', 'pseudo-weather?', 'referenced at'], records.rules.map(r => [r.id, r.src, r.battleHandlers.join(', ') || '-', r.addedAsPseudoWeather ? 'yes' : 'no', r.referencedAt.join(', ') || '-']));
md.push('## Pseudo-effects (engine-made effect ids)');
md.push('');
tbl(['id', 'where', 'what'], pseudoEffects.map(p => [p.id, p.refs.join(', '), p.note]));

md.push('## Species / formes');
md.push('');
tbl(['id', 'name', 'types', 'level', 'abilities seen', 'n', 'battleOnly / via', 'handlers'], records.species.map(r => [
	r.id, r.name, r.types.join('/'), r.teamgen ? Object.keys(r.teamgen.levels).join(',') : (formeParents(r.id) ? 'as parent' : '-'),
	r.teamgen ? Object.keys(r.teamgen.abilities).join(', ') : (formeParents(r.id) ? `set.species forme of ${formeParents(r.id)}` : Object.values(r.abilities).join(', ')),
	r.teamgen?.count ?? formeCount(r.id),
	r.inTeams || r.generatorKey ? (r.battleOnly ? `battleOnly ${[].concat(r.battleOnly).join('/')}` : '') : shortVia(r),
	r.speciesCondition ? `${r.speciesCondition.id}: ${r.speciesCondition.handlers.join(', ')}` : '',
]));
md.push('## Moves');
md.push('');
md.push('`h` = function handlers on the move (incl. nested secondaries/self), `c` = handlers on its `condition`.');
md.push('');
tbl(['id', 'type', 'cat', 'BP', 'acc', 'pri', 'target', 'h', 'c', 'handlers', 'data features', 'src', 'via'], records.moves.map(r => [
	r.id, r.type, r.category, r.basePower, r.accuracy === true ? '-' : r.accuracy, r.priority, r.target,
	r.fnCount, r.condition ? r.condition.fnCount : '', [...r.handlers, ...r.nested, ...Object.keys(r.constHandlers)].join(' '),
	r.dataFeatures.filter(f => !f.startsWith('target:')).join(' '), r.src, r.inTeams ? '' : shortVia(r),
]));
md.push('## Abilities');
md.push('');
tbl(['id', 'h', 'c', 'handlers', 'flags', 'src', 'via'], records.abilities.map(r => [
	r.id + (r.refOnly ? ' (ref-only)' : ''), r.fnCount, r.condition ? r.condition.fnCount : '',
	[...r.handlers, ...Object.keys(r.constHandlers)].join(' '), Object.keys(r.flags || {}).join(' '), r.src, r.inTeams ? '' : shortVia(r),
]));
md.push('## Items');
md.push('');
tbl(['id', 'h', 'c', 'handlers', 'fling', 'notes', 'src'], records.items.map(r => [
	r.id + (r.refOnly ? ' (ref-only)' : ''), r.fnCount, r.condition ? r.condition.fnCount : '',
	[...r.handlers, ...r.nested, ...Object.keys(r.constHandlers)].join(' '),
	r.fling ? `${r.fling.basePower}${r.fling.status ? ' ' + r.fling.status : ''}${r.fling.volatileStatus ? ' ' + r.fling.volatileStatus : ''}` : '-',
	[r.isBerry && 'berry', r.isGem && 'gem', r.isChoice && 'choice', r.data.onPlate && `plate:${r.data.onPlate}`, r.data.itemUser && `user:${r.data.itemUser.join('/')}`].filter(Boolean).join(' '),
	r.src,
]));
md.push('## Conditions');
md.push('');
md.push('`kinds` = how in-scope effects create it. `explicit` = a creator was found (otherwise it is included only because its owning in-scope move/ability/item defines it).');
md.push('');
tbl(['id', 'kinds', 'defined in', 'h', 'handlers', 'explicit', 'created by (first 3)'], records.conditions.map(r => [
	r.id, r.kinds.join(','), r.src ?? (r.exists ? r.definedIn : 'NOT DEFINED (bare id)'), r.fnCount,
	[...r.handlers, ...Object.keys(r.constHandlers)].join(' '), r.explicitCreator ? 'yes' : 'no', r.via.slice(0, 3).join('; '),
]));
md.push('## Hard effects');
md.push('');
md.push('The most mechanically complex in-scope effects (hand-written notes in `HARD_NOTES`, roughly most involved first). `score` = handler source chars + 150 x handler count (own condition included) as computed by the script; effects implemented mostly in the engine score low despite being hard. `complexityTop` in the JSON has the pure score ranking.');
md.push('');
tbl(['#', 'effect', 'score', 'fns', 'why', 'src'], hard.map((h, i) => [i + 1, h.key, h.score, h.fn, h.note, h.src ?? '']));
md.push('## PRNG-consuming handlers');
md.push('');
md.push('Effects whose own handlers call the battle PRNG (random/randomChance/sample/shuffle/speedSort/getRandomTarget/getRandomSwitchable). Engine-level PRNG use (speed ties, accuracy, crits, damage roll, secondaries, multihit counts) is not listed here.');
md.push('');
tbl(['effect', 'calls'], ['moves', 'abilities', 'items', 'conditions'].flatMap(k => records[k].filter(r => r.prng.length).map(r => [`${k}:${r.id}`, r.prng.join('; ')])));
md.push('## Behaviour implemented outside the effect');
md.push('');
md.push('In-scope effects whose id/name appears as a string literal in the engine (`sim/*.ts`, excluding `flags[...]` lookups). These need engine-side special cases in addition to (or instead of) their handlers.');
md.push('');
tbl(['effect', 'own fns', 'engine refs'], ['moves', 'abilities', 'items', 'conditions'].flatMap(k => records[k].filter(r => r.engineRefs.length).map(r => [`${k}:${r.id}`, r.fnCount, r.engineRefs.slice(0, 12).join(', ') + (r.engineRefs.length > 12 ? ` (+${r.engineRefs.length - 12})` : '')])));
md.push('## Trivial pure-data effects');
md.push('');
md.push('No handlers (function or constant, incl. nested and own condition) **and** never named by the engine or by another in-scope effect: implementable from data alone (through the generic move/condition machinery).');
md.push('');
for (const [k, v] of Object.entries(out.pureData.trivial)) {
	md.push(`- **${k}** (${v.length}): ${v.map(id => k === 'moves' && out.pureData.trivialMoveFeatures[id] ? `${id} [${out.pureData.trivialMoveFeatures[id].join(' ')}]` : id).join(', ')}`);
}
md.push('');
md.push('Bracketed = data fields/flags/targets the generic engine treats specially (drain, recoil, selfSwitch, smartTarget, multihit, cantusetwice, spread targets, ...). Moves without brackets are plain damage/boost/secondary data.');
md.push('');
md.push('No handlers of their own, but named elsewhere (behaviour lives in the engine or other effects):');
md.push('');
for (const [k, v] of Object.entries(out.pureData.noHandlersButReferenced)) md.push(`- **${k}** (${v.length}): ${v.map(x => `${x.id} [${[...x.engineRefs.slice(0, 2), ...x.referencedBy.slice(0, 3)].join(', ')}]`).join('; ')}`);
md.push('');
md.push(`Species without species-condition handlers: ${out.pureData.speciesWithoutHandlers} of ${records.species.length}.`);
md.push('');
md.push('## Ignored (explicitly out of scope for this format)');
md.push('');
for (const [what, why] of IGNORED()) md.push(`- **${what}** — ${why}`);
md.push('');

function IGNORED() {
	return [
		['Dynamax / Gigantamax', 'gen 9: no Dynamax; `sim/battle.ts` dynamax action, `data/conditions.ts` dynamax, Max/G-Max moves and Gmax formes are never reachable.'],
		['Z-Moves', 'no Z-Crystals generated; zPower/healreplacement/Z-status effects in `sim/battle-actions.ts` ignored.'],
		['Mega Evolution / Primal Reversion / Ultra Burst', `no Mega Stones, Red/Blue Orb or Ultranecrozium Z generated (items in scope: ${records.items.filter(r => r.data.megaStone || r.data.zMove || r.name.endsWith(' Orb') && ['blueorb', 'redorb'].includes(r.id)).map(r => r.id).join(', ') || 'none'}).`],
		['gen < 9 branches', 'every `this.gen <= 8` (and older) branch in sim/ and data/ (e.g. Gen 4 Transform forme logic in `sim/pokemon.ts`).'],
		['Team Preview', 'not in the ruleset; random teams lead with the first two slots.'],
		['Pokemon of the Day (`potd` rule)', 'in the ruleTable, but only acts when `global.Config.potd` is set (`data/random-battles/gen9/teams.ts` randomTeam); unset in the plain simulator.'],
		['Timers, chat, inactivity, forfeits-by-timer', 'server concerns, not simulator.'],
		['Multi battles, free-for-all', 'gameType is doubles.'],
		['Validation-only rules', '`obtainable*`, `evlimit`, `speciesclause`, `-unreleased`, `-tag:unobtainable`, `-nonexistent` only act in the team validator (random teams are not validated).'],
		['Natures / Hidden Power / happiness / Pokeball / hpType / dynamaxLevel / gigantamax', 'not set by the generator (neutral nature; happiness default 255). Note: `shiny` IS set and appears in protocol details.'],
		['Debug commands', '`sim/battle-stream.ts` >eval / >debug setters.'],
		['Past-only content', 'moves/items/abilities with isNonstandard Past are only in scope if generated or reached by the closure (see per-effect `isNonstandard`).'],
	];
}
fs.mkdirSync(path.dirname(opts.md), { recursive: true });
fs.writeFileSync(opts.md, md.join('\n'));
log(`wrote ${opts.md}`);

// Console summary for the caller
console.log(JSON.stringify({ counts: out.counts, saturation: satDeltas.map(s => ({ teams: s.teams, species: s.speciesId.total, move: s.move.total, ability: s.ability.total, item: s.item.total, teraType: s.teraType.total })), unresolved: unresolved.length, engineUnclassified: engineUnclassified.length, missing: missing.length }, null, 1));
