// Legal-choice enumerator and random picker for Gen 9 random doubles.
//
// Works purely from the request JSON that Showdown sends to a player (plus the format rules:
// doubles, no team preview, Terastallization only, no Mega/Z/Dynamax). It never looks at the
// Battle object, so it only knows what a real player knows.
//
// A choice is built per active slot from "options":
//   {kind:'move',   move: 1-based index into request.active[i].moves, target: loc|null, tera: bool}
//   {kind:'switch', to:   1-based index into request.side.pokemon}
//   {kind:'pass'}
// and rendered as Showdown choice strings: `move 2 -1 terastallize`, `switch 4`, `pass`,
// joined with ", " in slot order. Target locations are relative to the chooser: 1 and 2 are the
// foe slots a and b, -1 and -2 are the own slots a and b (so an ally target of slot a is -2).
//
// Showdown semantics this file encodes (docs/showdown/02-turn-loop-and-choices.md, section 5):
//  * fainted or commanding active slots cannot act: they pass (explicitly or implicitly).
//  * moves with target type normal/any/adjacentAlly/adjacentAllyOrSelf/adjacentFoe need a target
//    (Battle.validTargetLoc, which does not care whether the target is fainted); all other target
//    types (self, spread, field, randomNormal, locked-move entries without a `target`) take none.
//  * a locked move (Outrage, a charging move, Recharge) shows a single move entry without
//    `target` and `trapped: true`; it is chosen as plain `move 1`. The same goes for Struggle,
//    including the case where every move of the (updated) request is disabled.
//  * `trapped: true` forbids switching, `maybeTrapped` allows it (Showdown may still reject it).
//  * two slots cannot switch to the same Pokemon; at most one Terastallization per side per turn,
//    and none at all once a Pokemon of the side has Terastallized.
//  * forceSwitch requests: with `out` flagged slots and `in` unfainted bench Pokemon, exactly
//    min(out, in) switches are made and the other flagged slots must pass (Side.clearChoice).
//    Non-flagged slots pass implicitly. Revival Blessing (`reviving: true`) switches in a fainted
//    party member instead.

export const CHOOSABLE_TARGETS = new Set(['normal', 'any', 'adjacentAlly', 'adjacentAllyOrSelf', 'adjacentFoe']);

export const DEFAULT_KNOBS = Object.freeze({
	/** Probability that a slot with a legal switch picks a switch instead of a move. */
	switchProb: 0.1,
	/** Probability of adding `terastallize` to the picked move when it is offered. */
	teraProb: 0.15,
	/** Probability of omitting the `pass` of a slot that auto-passes (fainted / not flagged). */
	implicitPassProb: 0.3,
	/** Probability that a forced-pass-capable slot of a forceSwitch request passes although it could switch (Revival Blessing only). */
	revivePassProb: 0.05,
});

// ---------------------------------------------------------------------------------------
// Request inspection
// ---------------------------------------------------------------------------------------

export function requestKind(req) {
	if (!req) return 'none';
	if (req.wait) return 'wait';
	if (req.teamPreview) throw new Error('team preview requests are out of scope');
	if (req.forceSwitch) return 'switch';
	if (req.active) return 'move';
	throw new Error(`unrecognized request: ${JSON.stringify(req).slice(0, 200)}`);
}

export const isFainted = mon => mon.condition.endsWith(' fnt');

/** Mirrors Battle.validTargetLoc for doubles (2 slots per side). `sourceLoc` is -(slot+1). */
function validTargetLoc(targetLoc, sourceLoc, targetType) {
	if (targetLoc === 0) return true;
	if (Math.abs(targetLoc) > 2) return false;
	const isSelf = sourceLoc === targetLoc;
	const isFoe = targetLoc > 0;
	const acrossFromTargetLoc = -(3 - targetLoc);
	const isAdjacent = targetLoc > 0 ?
		Math.abs(acrossFromTargetLoc - sourceLoc) <= 1 :
		Math.abs(targetLoc - sourceLoc) === 1;
	switch (targetType) {
	case 'randomNormal':
	case 'scripted':
	case 'normal':
		return isAdjacent;
	case 'adjacentAlly':
		return isAdjacent && !isFoe;
	case 'adjacentAllyOrSelf':
		return isAdjacent && !isFoe || isSelf;
	case 'adjacentFoe':
		return isAdjacent && isFoe;
	case 'any':
		return !isSelf;
	}
	return false;
}

/** All valid target locations of a choosable target type for the user in `slot` (0 or 1). */
export function validTargetLocs(targetType, slot) {
	if (!CHOOSABLE_TARGETS.has(targetType)) return [];
	const sourceLoc = -(slot + 1);
	return [1, 2, -1, -2].filter(loc => validTargetLoc(loc, sourceLoc, targetType));
}

/** Per-slot hidden-information flags present in a move request. */
export function hiddenFlags(req) {
	const out = { maybeDisabled: [], maybeLocked: [], maybeTrapped: [] };
	if (requestKind(req) !== 'move') return out;
	req.active.forEach((a, i) => {
		for (const k of Object.keys(out)) if (a[k]) out[k].push(i);
	});
	return out;
}

/** Short feature tags describing a request, for coverage statistics. */
export function requestFeatures(req) {
	const kind = requestKind(req);
	const tags = [`request:${kind}`];
	if (kind === 'wait') return tags;
	if (req.noCancel) tags.push('noCancel');
	if (req.update) tags.push('update');
	const mons = req.side.pokemon;
	if (kind === 'switch') {
		const flagged = req.forceSwitch.filter(Boolean).length;
		const benchAlive = mons.slice(2).filter(m => !isFainted(m)).length;
		tags.push(`forceSwitch:${flagged}flagged`);
		if (benchAlive < flagged) tags.push('forceSwitch:fewerBenchThanSlots');
		req.forceSwitch.forEach((f, i) => {
			if (f && mons[i].reviving) tags.push('forceSwitch:revivalBlessing');
			if (f && !isFainted(mons[i])) tags.push('forceSwitch:aliveSlot');
			if (f && isFainted(mons[i])) tags.push('forceSwitch:faintedSlot');
		});
		return tags;
	}
	req.active.forEach((a, i) => {
		const mon = mons[i];
		if (isFainted(mon)) { tags.push('move:faintedSlot'); return; }
		if (mon.commanding) { tags.push('move:commandingSlot'); return; }
		if (a.trapped) tags.push('move:trapped');
		if (a.maybeTrapped) tags.push('move:maybeTrapped');
		if (a.maybeDisabled) tags.push('move:maybeDisabled');
		if (a.maybeLocked) tags.push('move:maybeLocked');
		if (a.canTerastallize) tags.push('move:canTera');
		if (a.moves.length === 1 && a.moves[0].id === 'struggle') tags.push('move:struggle');
		else if (a.moves.length === 1 && a.moves[0].id === 'recharge') tags.push('move:recharge');
		else if (a.moves.length === 1 && a.moves[0].target === undefined) tags.push('move:locked');
		else if (a.moves.every(m => m.disabled)) tags.push('move:allDisabled');
		else if (a.moves.some(m => m.disabled)) tags.push('move:someDisabled');
	});
	const benchAlive = mons.slice(2).filter(m => !isFainted(m)).length;
	if (benchAlive === 0) tags.push('move:noBench');
	if (mons.some(m => m.terastallized)) tags.push('move:sideTerastallized');
	return tags;
}

// ---------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------

const PASS = Object.freeze({ kind: 'pass' });

export function optionString(opt) {
	switch (opt.kind) {
	case 'pass': return 'pass';
	case 'switch': return `switch ${opt.to}`;
	case 'move': return `move ${opt.move}` + (opt.target != null ? ` ${opt.target}` : '') + (opt.tera ? ' terastallize' : '');
	}
	throw new Error(`bad option ${JSON.stringify(opt)}`);
}

/** Canonical key of an option (also produced from Showdown's own parsed actions by check-choices). */
export function optionKey(opt) {
	switch (opt.kind) {
	case 'pass': return 'pass';
	case 'switch': return `s${opt.to}`;
	case 'move': return `m${opt.move}:${opt.target ?? '-'}:${opt.tera ? 'T' : '-'}`;
	}
	throw new Error(`bad option ${JSON.stringify(opt)}`);
}

/**
 * Whether the `pass` of slot `i` may be left out of the choice string. Showdown auto-passes fainted /
 * non-flagged slots only when a move or switch (or the end of the input) comes after them: an explicit
 * `pass` token never auto-passes (Side.choosePass uses getChoiceIndex(true)), it just consumes the next
 * slot in order. So "pass" for [non-acting, forced-pass] would be read as the pass of slot a.
 */
export function canOmitPass(parts, i) {
	return parts.slice(i + 1).every(p => !p || p.kind !== 'pass' || p.implicit);
}

/** Joins per-slot options into a choice string. Slots with `implicit` passes are omitted. */
export function buildInput(parts) {
	const strs = parts.filter(p => p && !p.implicit).map(optionString);
	if (!strs.length) return parts.map(optionString).join(', ');
	return strs.join(', ');
}

/** Slot descriptors of a move request: which slots can act. */
function moveSlots(req) {
	return [0, 1].map(i => {
		const mon = req.side.pokemon[i];
		return { i, actionable: !isFainted(mon) && !mon.commanding };
	});
}

const benchIndices = (req, pred) => req.side.pokemon
	.map((m, idx) => ({ m, idx }))
	.filter(({ m, idx }) => idx >= 2 && pred(m))
	.map(({ idx }) => idx);

// ---- move requests --------------------------------------------------------------------

function initialMoveState(req) {
	return { switchTargets: new Set(), teraUsed: req.side.pokemon.some(m => m.terastallized) };
}

/** Flat list of every legal option of an acting slot, given what the other slot already chose. */
function moveSlotOptions(req, i, st) {
	const a = req.active[i];
	const opts = [];
	const enabled = a.moves.map((m, j) => ({ m, j })).filter(({ m }) => !m.disabled);
	if (enabled.length === 0) {
		// Every move is disabled: Showdown makes the Pokemon use Struggle, selected as `move 1`.
		opts.push({ kind: 'move', move: 1, target: null, tera: false });
	}
	for (const { m, j } of enabled) {
		const targets = CHOOSABLE_TARGETS.has(m.target) ? validTargetLocs(m.target, i) : [null];
		for (const target of targets) {
			opts.push({ kind: 'move', move: j + 1, target, tera: false });
			if (a.canTerastallize && !st.teraUsed) opts.push({ kind: 'move', move: j + 1, target, tera: true });
		}
	}
	if (!a.trapped) {
		for (const idx of benchIndices(req, m => !isFainted(m))) {
			if (!st.switchTargets.has(idx)) opts.push({ kind: 'switch', to: idx + 1 });
		}
	}
	return opts;
}

function applyMoveOption(st, opt) {
	const next = { switchTargets: new Set(st.switchTargets), teraUsed: st.teraUsed };
	if (opt.kind === 'switch') next.switchTargets.add(opt.to - 1);
	if (opt.kind === 'move' && opt.tera) next.teraUsed = true;
	return next;
}

// ---- switch requests ------------------------------------------------------------------

function initialSwitchState(req) {
	const out = req.forceSwitch.filter(Boolean).length;
	const benchAlive = benchIndices(req, m => !isFainted(m)).length;
	const forced = Math.min(out, benchAlive);
	return { forcedLeft: forced, passesLeft: out - forced, switchIns: new Set() };
}

function switchSlotOptions(req, i, st) {
	if (!req.forceSwitch[i]) return [PASS];
	const opts = [];
	if (req.side.pokemon[i].reviving) {
		for (const idx of benchIndices(req, isFainted)) opts.push({ kind: 'switch', to: idx + 1 });
		// A fainted Pokemon in an active slot is also a legal Revival Blessing target.
		req.side.pokemon.forEach((m, idx) => {
			if (idx < 2 && isFainted(m)) opts.push({ kind: 'switch', to: idx + 1 });
		});
	} else if (st.forcedLeft > 0) {
		for (const idx of benchIndices(req, m => !isFainted(m))) {
			if (!st.switchIns.has(idx)) opts.push({ kind: 'switch', to: idx + 1 });
		}
	}
	if (st.passesLeft > 0) opts.push(PASS);
	return opts;
}

function applySwitchOption(req, i, st, opt) {
	const next = { forcedLeft: st.forcedLeft, passesLeft: st.passesLeft, switchIns: new Set(st.switchIns) };
	if (!req.forceSwitch[i]) return next;
	if (opt.kind === 'pass') next.passesLeft--;
	else if (req.side.pokemon[i].reviving) next.forcedLeft = Math.max(0, next.forcedLeft - 1);
	else {
		next.forcedLeft--;
		next.switchIns.add(opt.to - 1);
	}
	return next;
}

// ---- generic per-request dispatch -----------------------------------------------------

function slotModel(req) {
	const kind = requestKind(req);
	if (kind === 'move') {
		return {
			kind,
			slots: moveSlots(req),
			init: () => initialMoveState(req),
			options: (i, st) => moveSlotOptions(req, i, st),
			apply: (i, st, opt) => applyMoveOption(st, opt),
			done: () => true,
		};
	}
	if (kind === 'switch') {
		return {
			kind,
			slots: [0, 1].map(i => ({ i, actionable: !!req.forceSwitch[i] })),
			init: () => initialSwitchState(req),
			options: (i, st) => switchSlotOptions(req, i, st),
			apply: (i, st, opt) => applySwitchOption(req, i, st, opt),
			done: st => st.forcedLeft === 0,
		};
	}
	throw new Error(`no choices for a ${kind} request`);
}

/**
 * Exhaustive list of every legal joint choice of a move/switch request, in canonical form
 * (explicit passes for all non-acting slots, `move N [target] [terastallize]`).
 * Returns [{ parts: [opt, opt], input }].
 */
export function enumerateAll(req) {
	const model = slotModel(req);
	const results = [];
	const parts = [null, null];
	const rec = (k, st) => {
		if (k === 2) {
			if (model.done(st)) results.push({ parts: parts.slice(), input: buildInput(parts) });
			return;
		}
		const slot = model.slots[k];
		const opts = slot.actionable ? model.options(slot.i, st) : [PASS];
		for (const opt of opts) {
			parts[k] = opt;
			rec(k + 1, model.apply(slot.i, st, opt));
		}
		parts[k] = null;
	};
	rec(0, model.init());
	return results;
}

/**
 * Picks one legal joint choice at random. `rng` is a Showdown PRNG (random(), sample(), shuffle()).
 * Returns { input, parts } or null when the side has nothing to decide (wait request).
 */
export function pickChoice(req, rng, knobs = DEFAULT_KNOBS) {
	if (requestKind(req) === 'wait') return null;
	const model = slotModel(req);
	const parts = [null, null];
	let st = model.init();
	const order = model.slots.filter(s => s.actionable).map(s => s.i);
	if (order.length === 2) rng.shuffle(order);
	for (const i of order) {
		const opts = model.options(i, st);
		if (!opts.length) throw new Error(`no legal option for slot ${i} of ${JSON.stringify(req).slice(0, 400)}`);
		const opt = model.kind === 'move' ? pickMoveOption(opts, rng, knobs) : pickSwitchOption(req, i, opts, rng, knobs);
		parts[i] = opt;
		st = model.apply(i, st, opt);
	}
	if (!model.done(st)) throw new Error(`incomplete choice for ${JSON.stringify(req).slice(0, 400)}`);
	for (const s of model.slots) if (!s.actionable) parts[s.i] = { kind: 'pass', implicit: false };
	// Non-acting slots auto-pass in Showdown (getChoiceIndex), so their `pass` may be omitted (see canOmitPass).
	for (let i = 1; i >= 0; i--) {
		if (model.slots[i].actionable) continue;
		const omit = rng.random() < knobs.implicitPassProb;
		if (omit && canOmitPass(parts, i)) parts[i].implicit = true;
	}
	return { input: buildInput(parts), parts };
}

function pickMoveOption(opts, rng, knobs) {
	const switches = opts.filter(o => o.kind === 'switch');
	const moves = opts.filter(o => o.kind === 'move' && !o.tera);
	if (switches.length && rng.random() < knobs.switchProb) return rng.sample(switches);
	// Uniform over moves first, then over that move's targets, so spread/self moves are not underweighted.
	const moveIds = [...new Set(moves.map(o => o.move))];
	const moveId = rng.sample(moveIds);
	const base = rng.sample(moves.filter(o => o.move === moveId));
	const teraVariant = opts.find(o => o.kind === 'move' && o.tera && o.move === base.move && o.target === base.target);
	if (teraVariant && rng.random() < knobs.teraProb) return teraVariant;
	return base;
}

function pickSwitchOption(req, i, opts, rng, knobs) {
	// Where a pass is merely permitted (Revival Blessing with a bench), take it rarely.
	if (opts.length > 1 && opts.includes(PASS) && req.side.pokemon[i].reviving) {
		if (rng.random() < knobs.revivePassProb) return PASS;
		return rng.sample(opts.filter(o => o !== PASS));
	}
	return rng.sample(opts);
}
