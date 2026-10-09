// Constructs the two packed teams of one directed battle from a profile.
//
// Everything is a pure function of (profile, runSeed, index): the team PRNGs are Showdown Gen5 PRNGs seeded with
// the fixture's own `teamSeeds` (deriveSeeds), and the focus-effect schedule is a deterministic shuffled deck.
// Sets are shaped exactly like RandomTeams output (name/species/gender/shiny/level/moves/ability/evs/ivs/item/
// teraType) and packed with Teams.pack; level, EVs and IVs follow the generator's rules (data/random-battles/
// gen9/teams.ts randomSet): level from the species' set data, 85 EVs / 31 IVs, 0 Atk when no move uses Atk, 0 Spe with
// Gyro Ball / Trick Room, random gender unless fixed, shiny with chance 1/1024.
import { createHash } from 'node:crypto';
import { seedString } from './common.mjs';

const toID = s => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');

function weightedPick(rng, items, weight) {
	let total = 0;
	const ws = items.map(x => { const w = Math.max(0, weight(x)); total += w; return w; });
	if (!(total > 0)) return null;
	let r = rng.random() * total;
	for (let i = 0; i < items.length; i++) {
		r -= ws[i];
		if (r < 0) return items[i];
	}
	return items[items.length - 1];
}

function wordsOfHash(text) {
	const h = createHash('sha256').update(text).digest();
	return [0, 1, 2, 3].map(k => h.readUInt16BE(2 * k));
}

/** The n-th focus effect of the corpus: decks of all focus keys, each deck a fresh deterministic shuffle. */
export function deckAt(sim, profile, runSeed, n) {
	const keys = profile.focus.keys;
	const cycle = Math.floor(n / keys.length);
	const rng = new sim.PRNG(seedString(wordsOfHash(`numbrion-directed-v1/deck/${profile.id}/${runSeed}/${cycle}`)));
	const deck = keys.slice();
	rng.shuffle(deck);
	return deck[n % keys.length];
}

// ---------------------------------------------------------------------------------------

class Battle {
	constructor(sim, profile, data, runSeed, index, seeds) {
		this.sim = sim;
		this.profile = profile;
		this.data = data;
		this.runSeed = runSeed;
		this.index = index;
		this.rng = seeds.teamSeeds.map(s => new sim.PRNG(seedString(s)));
		this.mons = [[], []]; // built sets with metadata
		this.active = new Set(); // inert families in force for this battle: their trigger moves are banned from fillers
		this.exempt = new Set(); // focus-template moves (exempt from the ban of an `always` family)
		this.moveIds = new Set(); // every move chosen so far in the battle
		this.notes = { focus: [], skipped: [], loose: 0, legal: 0 };
	}

	familyByName(name) {
		return this.profile.families.find(f => f.name === name);
	}

	bannedByActive(move) {
		for (const name of this.active) if (this.familyByName(name).trigger(move)) return true;
		return false;
	}

	/**
	 * Decides which inert families are in force. `always` families (Levitate: not enough other handler-free
	 * species exist) stay on; the others are rolled, and dropped when a focus-template move triggers them.
	 */
	chooseFamilies(templateMoves) {
		for (const m of templateMoves) this.exempt.add(m);
		const mvs = templateMoves.map(id => this.data.moves.get(id)).filter(Boolean);
		for (const f of this.profile.families) {
			if (f.always) { this.active.add(f.name); continue; }
			if (this.rng[0].random() < 0.5 && !mvs.some(m => f.trigger(m))) this.active.add(f.name);
		}
	}
}

function speciesAllowed(b, sp, template) {
	const p = b.profile;
	if (sp.speciesCondition && !p.speciesAllow.has(sp.id)) return false;
	if ((sp.battleOnly || sp.requiredAbility || sp.requiredMove || sp.requiredTeraType) && !p.speciesAllow.has(sp.id)) return false;
	const req = sp.requiredItem ? [sp.requiredItem] : sp.requiredItems || [];
	if (req.length && !req.some(n => p.items.has(toID(n)))) return false;
	if (template.species && sp.id !== template.species) return false;
	if (template.ability && !sp.abilitySet.has(template.ability)) return false;
	if (template.item) {
		const it = b.data.items.get(template.item);
		if (it.itemUser && !it.itemUser.includes(sp.id) && !it.itemUser.includes(sp.baseId)) return false;
		if (it.forcedForme && toID(it.forcedForme) !== sp.id) return false;
	}
	return true;
}

/** Abilities `sp` may carry in this battle; each is { id, family|null, weight }. */
function abilityOptions(b, sp, template) {
	if (template.ability) return sp.abilitySet.has(template.ability) ? [{ id: template.ability, family: null, weight: 1 }] : [];
	const out = [];
	for (const [id, n] of sp.abilities) {
		if (b.profile.abilities.has(id)) { out.push({ id, family: null, weight: 2 * n }); continue; }
		const fam = b.profile.families.find(f => b.active.has(f.name) && f.abilities.includes(id));
		if (fam) out.push({ id, family: fam.name, weight: n });
	}
	return out;
}

function usableItems(b, sp, template) {
	if (template.item) return [template.item];
	const p = b.profile;
	// A species whose required item is in the world must hold it.
	const req = sp.requiredItem ? [sp.requiredItem] : sp.requiredItems || [];
	if (req.length) return req.map(toID).filter(i => p.items.has(i));
	return [];
}

function pickItem(b, rng, sp, template) {
	const forced = usableItems(b, sp, template);
	if (forced.length) return b.data.items.get(rng.sample(forced)).name;
	const p = b.profile;
	if (p.fillerItems.size && rng.random() < p.fillerItemChance) {
		const cands = [...p.fillerItems].filter(i => {
			const it = b.data.items.get(i);
			return !it.itemUser && !it.forcedForme;
		});
		if (cands.length) return b.data.items.get(rng.sample(cands)).name;
	}
	return '';
}

const isNoAttackStatMove = (m, set) => {
	if (m.entry.data.damage !== undefined || m.entry.handlers.includes('damageCallback')) return true;
	if (m.id === 'shellsidearm') return false;
	if (m.id === 'terablast') return false;
	return m.category !== 'Physical' || m.id === 'bodypress' || m.id === 'foulplay';
};

function pickMoves(b, rng, sp, template, ability, banFn) {
	const p = b.profile;
	const chosen = [...(template.moves || [])];
	const have = new Set(chosen);
	const teamTypes = new Set();
	for (const id of chosen) teamTypes.add(b.data.moves.get(id).type);
	const pool = [...p.worldMoves].filter(id => !have.has(id) && !banFn(b.data.moves.get(id)));
	let damaging = chosen.filter(id => b.data.moves.get(id).damaging).length;
	while (chosen.length < 4) {
		const needDamage = damaging < 2 && chosen.length >= 2;
		const cand = pool.filter(id => !have.has(id)).map(id => b.data.moves.get(id)).filter(m => !needDamage || m.damaging);
		const m = weightedPick(rng, cand, mv => {
			let w = 1;
			if (sp.moveSet.has(mv.id)) w *= 8;
			if (sp.types.includes(mv.type)) w *= 2;
			if (!teamTypes.has(mv.type)) w *= 1.5;
			if (!mv.damaging) w *= 0.6;
			if (p.focusMoves.has(mv.id)) w *= 2.5;
			return w;
		});
		if (!m) break;
		chosen.push(m.id);
		have.add(m.id);
		teamTypes.add(m.type);
		if (m.damaging) damaging++;
		if (sp.moveSet.has(m.id)) b.notes.legal++; else b.notes.loose++;
	}
	return chosen;
}

function makeMon(b, side, template) {
	const rng = b.rng[side];
	const team = b.mons[side];
	const used = new Set(team.map(m => m.sp.baseId));
	const tmplMoves = template.moves || [];
	let cands = [];
	for (const sp of b.data.species.values()) {
		if (used.has(sp.baseId)) continue;
		if (!speciesAllowed(b, sp, template)) continue;
		if (!abilityOptions(b, sp, template).length) continue;
		cands.push(sp);
	}
	if (!cands.length) return null;
	// Species that legally carry the template's moves/item first (loosened only when none does).
	const score = sp => (tmplMoves.every(m => sp.moveSet.has(m)) ? 2 : 0) + (template.item && sp.itemSet.has(template.item) ? 2 : 0);
	const best = Math.max(...cands.map(score));
	cands = cands.filter(sp => score(sp) === best);
	const sp = rng.sample(cands);

	const opts = abilityOptions(b, sp, template);
	const ab = weightedPick(rng, opts, o => o.weight);
	const ability = b.data.abilities.get(ab.id);
	const banFn = m => b.bannedByActive(m);
	const moves = pickMoves(b, rng, sp, template, ability, banFn);
	for (const id of moves) b.moveIds.add(id);
	const item = pickItem(b, rng, sp, template);
	const forme = weightedPick(rng, sp.formes, ([, n]) => n)[0];
	const teraType = weightedPick(rng, sp.teraTypes, ([, n]) => n)[0];
	const gender = sp.baseSpecies === 'Greninja' ? 'M' : (sp.gender || (rng.random(2) ? 'F' : 'M'));
	const shiny = rng.randomChance(1, 1024);
	const evs = { hp: 85, atk: 85, def: 85, spa: 85, spd: 85, spe: 85 };
	const ivs = { hp: 31, atk: 31, def: 31, spa: 31, spd: 31, spe: 31 };
	if (!moves.length) throw new Error(`no legal move for ${sp.id} in battle ${b.index} under ${b.profile.id}`);
	const mm = moves.map(id => b.data.moves.get(id));
	// Same stat rules as RandomTeams.randomSet (doubles): HP EVs only move for the Sitrus/Minior special cases.
	while (evs.hp > 1) {
		const hp = Math.floor(Math.floor(2 * sp.baseStats.hp + ivs.hp + Math.floor(evs.hp / 4) + 100) * sp.level / 100 + 10);
		if ((moves.includes('substitute') && item === 'Sitrus Berry') || sp.id === 'minior') {
			if (hp % 4 === 0) break;
		} else if ((moves.includes('bellydrum') || moves.includes('filletaway') || moves.includes('shedtail')) &&
			(item === 'Sitrus Berry' || ability.name === 'Gluttony')) {
			if (hp % 2 === 0) break;
		} else if (moves.includes('substitute') && moves.includes('endeavor')) {
			if (hp % 4 > 0) break;
		} else break;
		evs.hp -= 4;
	}
	if (mm.every(m => isNoAttackStatMove(m)) && !moves.includes('transform')) { evs.atk = 0; ivs.atk = 0; }
	if (moves.includes('gyroball') || moves.includes('trickroom')) { evs.spe = 0; ivs.spe = 0; }
	const set = {
		name: sp.baseSpecies, species: forme, speciesId: sp.id, gender, shiny, level: sp.level, moves, ability: ability.name,
		evs, ivs, item, teraType,
	};
	return { sp, set, template, family: ab.family };
}

/**
 * Builds the two teams of battle `index` of run `runSeed` under `profile`.
 * Returns { sets: [[set x6], [set x6]], packed: [string, string], notes }.
 */
export function buildTeams(sim, profile, data, { runSeed, index, seeds }) {
	const b = new Battle(sim, profile, data, runSeed, index, seeds);
	const plan = [[], []];
	if (profile.focus) {
		const k = profile.focus.slotsPerTeam;
		for (let side = 0; side < 2; side++) {
			for (let j = 0; j < k; j++) {
				const key = deckAt(sim, profile, runSeed, index * 2 * k + side * k + j);
				const ts = profile.focus.templates.get(key);
				plan[side].push({ key, template: b.rng[side].sample(ts) });
			}
		}
	}
	b.chooseFamilies(plan.flatMap(p => p.flatMap(x => x.template.moves || [])));
	// Focus mons first (they fix the battle's moves), then fillers.
	for (let side = 0; side < 2; side++) {
		for (const { key, template } of plan[side]) {
			const mon = makeMon(b, side, template);
			if (mon) { b.mons[side].push(mon); b.notes.focus.push(key); }
			else b.notes.skipped.push(key);
		}
	}
	for (let side = 0; side < 2; side++) {
		let guard = 0;
		while (b.mons[side].length < 6) {
			const mon = makeMon(b, side, {});
			if (!mon) {
				if (++guard > 3) throw new Error(`cannot fill team ${side} of battle ${index} under ${profile.id}`);
				continue;
			}
			b.mons[side].push(mon);
		}
		b.rng[side].shuffle(b.mons[side]);
	}
	// Invariant: an inert family in force has no trigger move among the 12 movesets (always-on families: except
	// focus-template moves, e.g. a Ground-type move of a batch under test next to a Levitate holder).
	for (const name of b.active) {
		const fam = b.familyByName(name);
		for (const id of b.moveIds) {
			if (fam.always && b.exempt.has(id)) continue;
			if (fam.trigger(data.moves.get(id))) throw new Error(`inert family ${name} violated by ${id} (battle ${index}, ${profile.id})`);
		}
	}
	const sets = b.mons.map(team => team.map(m => m.set));
	return { sets, packed: sets.map(t => sim.Teams.pack(t)), notes: b.notes, active: [...b.active] };
}
