#!/usr/bin/env node
// Catalog of every distinct battle.log line *shape* in the oracle fixtures, plus the invariant
// checks that docs/showdown/05-protocol.md quotes.
//
//   node tools/oracle/log-shapes.mjs [--out docs/showdown/05-log-shapes.txt]
//        [--examples 3] [--top 12] [--json shapes.json] [--ps ~/src/pokemon-showdown] [files.jsonl ...]
//
// --ps <showdown checkout> adds section 4: a cross-check of the literal message kinds emitted by the source of every
// in-scope effect (data/scope.json, compiled functions in <ps>/dist) and by sim/*.ts against the kinds seen in the fixtures.
// Default inputs: data/fixtures/sample-50.jsonl and fuzz-out/fuzz-2000.jsonl (streamed line by line;
// each line is one battle, see docs/design/FIXTURES.md). The per-battle log is the concatenation of
// steps[].log and end.log. `|t:|` is already normalized in the fixtures.
//
// A SHAPE is the message kind (`|move|`, `|-damage|`, ...) plus, for every argument, a class:
//   POKE            `p1a: Name`  (active-slot ident)          POKE_NOSLOT  `p1: Name` (bench ident / side name)
//   SIDE            `p1`                                        SLOT         `p1a`      SLOTS  `p2a,p2b`
//   DETAILS         `Species, L84, F, shiny, tera:Fire`        INT          `-?\d+`
//   HP/exact        `123/321`, HP/pct `38/100`; both also cover `0 fnt` and a trailing status word (per-argument "HP variants")
//   EFFECT(move|ability|item|pokemon)   `move: Protect` ...    word `[a-z0-9]+`   Name  any other short string
//   TEXT            free text kinds (rule, tier, -hint, -message, player name, win ...)
//   EMPTY           empty string
//   [tag]           bracket tag with no value, e.g. `[silent]`
//   [tag] CLASS     bracket tag with a value of class CLASS, e.g. `[from] EFFECT(ability)`, `[of] POKE`
// A `|split|pN` marker plus the next two entries form one SPLIT shape: S: = secret line, P: = public line.
// The second catalog ("keyed shapes") additionally keeps the first word/Name/EFFECT argument literal, which is
// the discriminator of multiplexed kinds (-activate, -start, -sidestart, cant, ...).
import fs from 'node:fs';
import readline from 'node:readline';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '..', '..');

// ---------------------------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------------------------
const argv = process.argv.slice(2);
const opts = { out: null, examples: 3, top: 12, json: null, ps: null, files: [] };
for (let i = 0; i < argv.length; i++) {
	const a = argv[i];
	if (a === '--out') opts.out = argv[++i];
	else if (a === '--examples') opts.examples = Number(argv[++i]);
	else if (a === '--top') opts.top = Number(argv[++i]);
	else if (a === '--json') opts.json = argv[++i];
	else if (a === '--ps') opts.ps = argv[++i];
	else if (a === '-h' || a === '--help') {
		console.log(fs.readFileSync(fileURLToPath(import.meta.url), 'utf8').split('\n').slice(1, 22).map(l => l.slice(3)).join('\n'));
		process.exit(0);
	} else opts.files.push(a);
}
if (!opts.files.length) {
	opts.files = [path.join(repo, 'data/fixtures/sample-50.jsonl'), path.join(repo, 'fuzz-out/fuzz-2000.jsonl')];
}

// ---------------------------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------------------------
const FREE_TEXT_KINDS = new Set(['rule', 'tier', '-hint', '-message', 'message', 'raw', 'html', 'uhtml', 'error', 'bigerror',
	'debug', 'player', 'win', 'c', 'chat', 'n', 'inactive', 'inactiveoff', 'j', 'l']);
// kind -> argument index that holds a details string
const DETAILS_POS = { switch: 1, drag: 1, replace: 1, detailschange: 1, poke: 1 };
const RX = {
	poke: /^p[1-4][a-f]: ./s,
	pokeNoSlot: /^p[1-4]: /,
	side: /^p[1-4]$/,
	slot: /^p[1-4][a-f]$/,
	slots: /^p[1-4][a-f](?:,p[1-4][a-f])+$/,
	int: /^-?\d+$/,
	hp: /^(\d+)\/(\d+)(?: ([a-z]+))?$/,
	fnt: /^0 fnt$/,
	details: /^([^,]+)((?:, L\d+)?)((?:, [MFN])?)((?:, shiny)?)((?:, tera:[A-Za-z]+)?)$/,
	effect: /^(move|ability|item|pokemon|nature): (.+)$/s,
	word: /^[a-z0-9_-]+$/,
	tag: /^\[([A-Za-z0-9]+)\](?: (.*))?$/s,
};

/** Class of a plain (non-tag) argument. `role` is 'S' (secret line), 'P' (public line) or null. */
function classOf(kind, idx, v, role) {
	if (v === '') return 'EMPTY';
	if (idx === DETAILS_POS[kind] && RX.details.test(v)) return 'DETAILS';
	let m;
	if ((m = RX.hp.exec(v))) {
		const pct = m[2] === '100' && role !== 'S';
		return role === 'P' || (role === null && pct) ? 'HP/pct' : 'HP/exact';
	}
	if (RX.fnt.test(v)) return role === 'P' ? 'HP/pct' : role === 'S' ? 'HP/exact' : 'HP/fnt';
	if (RX.poke.test(v)) return 'POKE';
	if (RX.pokeNoSlot.test(v)) return 'POKE_NOSLOT';
	if (RX.slots.test(v)) return 'SLOTS';
	if (RX.slot.test(v)) return 'SLOT';
	if (RX.side.test(v)) return 'SIDE';
	if (RX.int.test(v)) return 'INT';
	if (FREE_TEXT_KINDS.has(kind)) return 'TEXT';
	if ((m = RX.effect.exec(v))) return `EFFECT(${m[1]})`;
	if (RX.word.test(v)) return 'word';
	return 'Name';
}

const VOCAB_CLASSES = /^(word|Name|EFFECT\(.*\))$/;
const isHpClass = c => c.startsWith('HP/');
/** Variant label recorded in place of the literal for an HP argument: plain, with status word, or fainted. */
function hpVariant(v) {
	if (v === '0 fnt') return 'fnt (`0 fnt`)';
	return RX.hp.test(v) && / [a-z]+$/.test(v) ? 'hp+status' : 'hp';
}

/**
 * Returns {cls, vocab}: cls is the class string of the argument (tags: `[from] EFFECT(ability)`), vocab is the literal
 * to record for variable-vocabulary classes (or null).
 */
function classArg(kind, idx, v, role) {
	const tm = RX.tag.exec(v);
	if (tm && !(idx === DETAILS_POS[kind])) {
		const [, name, val] = tm;
		if (val === undefined) return { cls: `[${name}]`, vocab: null, tag: name };
		const vc = classOf(kind, -1, val, null);
		return { cls: `[${name}] ${vc}`, vocab: VOCAB_CLASSES.test(vc) ? `[${name}] ${val}` : null, tag: name };
	}
	const cls = classOf(kind, idx, v, role);
	if (isHpClass(cls)) return { cls, vocab: hpVariant(v), hp: true };
	return { cls, vocab: VOCAB_CLASSES.test(cls) ? v : null };
}

// ---------------------------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------------------------
const VOCAB_CAP = 4000;
const shapes = new Map(); // key -> {kind, display, count, examples:[], vocab:[Map]}
const keyed = new Map(); // key -> {kind, display, count, example}
const kindCount = new Map();
const checks = {
	battles: 0, entries: 0, nonPipe: [], newline: 0, splitTriples: 0, splitBroken: 0,
	hpChecked: 0, hpOk: 0, hpBad: [], hpKinds: new Map(),
	hpNaive: { roundDiffers: 0, floorDiffers: 0, clamp99: 0, one: 0, hundred: 0, statusSuffix: 0, fnt: 0 },
	splitDiff: [], splitSide: { ok: 0, bad: [] }, splitSecretEqPublic: 0,
	unsplitHp: new Map(), splitKinds: new Map(), kindsBothWays: new Map(),
	details: new Map(), identVsSpecies: { same: 0, differ: new Map() },
	tAdj: { prev: new Map(), next: new Map() }, bigram: new Map(),
	moveTarget: { blankStill: 0, blankNoStill: [], stillNotBlank: [], withTarget: 0 },
	splitOrdinal: new Map(), emptyPublic: new Map(),
	tagOrder: new Map(),
	obsSigs: new Set(), tagKinds: new Map(), hintRepeat: new Map(), turnSeq: { ok: 0, bad: [] }, startOrder: { ok: 0, bad: [] },
};
const BIGRAM_KINDS = new Set(['upkeep', 'turn', 'start', 'win', 'tie', 'teamsize', 'gametype', 'player', 'gen', 'tier', 'rule']);

function inc(map, key, by = 1) { map.set(key, (map.get(key) || 0) + by); }
function pushLimited(arr, v, n = 8) { if (arr.length < n) arr.push(v); }

function shapeOfLine(line, role, isSecretOrPublic) {
	// returns {kind, cls: [], vocab: [], argVocab: [] (per arg literal or null)}
	const parts = line.split('|');
	if (parts[0] !== '' || parts.length < 2) return { kind: '(raw)', cls: [line === '' ? 'EMPTY-ENTRY' : 'RAW'], vocab: [], parts, raw: true };
	const kind = parts[1];
	const args = parts.slice(2);
	const cls = [];
	const vocab = [];
	const hp = [];
	for (let i = 0; i < args.length; i++) {
		const c = classArg(kind, i, args[i], role);
		cls.push(c.cls);
		vocab.push(c.vocab);
		hp.push(!!c.hp);
	}
	return { kind, cls, vocab, hp, parts, args };
}

function showShape(sh) {
	if (sh.raw) return `(${sh.cls[0]})`;
	return `|${sh.kind}${sh.cls.length ? '|' + sh.cls.join('|') : ''}`;
}
const kindLabel = k => (k === '' ? '| (blank line, battle.add(\'\'))' : `|${k}|`);

function recordShape(key, kind, display, exampleText, vocabs, hpFlags = []) {
	let s = shapes.get(key);
	if (!s) {
		s = { kind, display, count: 0, examples: [], exampleKeys: new Set(), vocab: [], hpPos: new Set() };
		shapes.set(key, s);
	}
	s.count++;
	// vocab bookkeeping
	for (let i = 0; i < vocabs.length; i++) {
		const v = vocabs[i];
		if (v === null || v === undefined) continue;
		if (hpFlags[i]) s.hpPos.add(i);
		let m = s.vocab[i];
		if (!m) m = s.vocab[i] = new Map();
		if (m.size < VOCAB_CAP || m.has(v)) inc(m, v);
	}
	if (s.examples.length < opts.examples) {
		const sig = vocabs.filter(Boolean).join('\u0001');
		if (!s.exampleKeys.has(sig) || s.examples.length === 0) {
			s.exampleKeys.add(sig);
			s.examples.push(exampleText);
		}
	}
	// keyed catalog: first vocab literal kept
	const firstVocab = vocabs.findIndex((v, i) => v && !hpFlags[i]);
	const kkey = firstVocab >= 0 ? `${key}\u0001${firstVocab}=${vocabs[firstVocab]}` : key;
	let k = keyed.get(kkey);
	if (!k) {
		k = { kind, display, lit: firstVocab >= 0 ? { pos: firstVocab, val: vocabs[firstVocab] } : null, count: 0, example: exampleText };
		keyed.set(kkey, k);
	}
	k.count++;
}

function handleSplit(side, secretLine, publicLine) {
	checks.splitTriples++;
	const S = shapeOfLine(secretLine, 'S');
	const P = publicLine === '' ? { kind: '(empty)', cls: [], vocab: [], parts: [''], empty: true } : shapeOfLine(publicLine, 'P');
	inc(kindCount, 'split');
	tagStats(S.kind, S.args);
	if (P.empty) {
		inc(checks.emptyPublic, S.kind);
		const key = `SPLIT ${S.kind}\u0001${showShape(S)}\u0001(empty public entry)`;
		recordShape(key, S.kind, `SPLIT |split|SIDE  S: ${showShape(S)}  P: ""`, `|split|${side} / ${secretLine} / (empty)`, S.vocab, S.hp);
		inc(checks.splitKinds, S.kind);
		return;
	}
	inc(kindCount, S.kind);
	inc(checks.splitKinds, S.kind);
	if (S.kind !== P.kind || S.cls.length !== P.cls.length) {
		checks.splitBroken++;
		pushLimited(checks.splitDiff, `${secretLine} // ${publicLine}`);
	} else {
		// all args equal except HP-class ones
		let same = true;
		for (let i = 0; i < S.args.length; i++) {
			const isHp = isHpClass(S.cls[i]);
			if (!isHp && S.args[i] !== P.args[i]) same = false;
			if (isHp) checkHp(S.kind, S.args[i], P.args[i], S.cls[i], P.cls[i]);
		}
		if (!same) pushLimited(checks.splitDiff, `${secretLine} // ${publicLine}`);
		else if (!S.cls.some(isHpClass)) checks.splitSecretEqPublic++;
	}
	// split side must equal side of the first ident in the secret line
	const ident = S.args && S.args.find(a => RX.poke.test(a) || RX.pokeNoSlot.test(a));
	if (ident) {
		if (ident.slice(0, 2) === side) checks.splitSide.ok++;
		else pushLimited(checks.splitSide.bad, `${side} ${secretLine}`);
	}
	const key = `SPLIT ${S.kind}\u0001${showShape(S)}\u0001${showShape(P)}`;
	recordShape(key, S.kind, `SPLIT |split|SIDE  S: ${showShape(S)}  P: ${showShape(P)}`,
		`|split|${side} / ${secretLine} / ${publicLine}`, S.vocab, S.hp);
}

function checkHp(kind, sec, pub, scls, pcls) {
	checks.hpChecked++;
	inc(checks.hpKinds, kind);
	let expect;
	let m;
	if (sec === '0 fnt') expect = '0 fnt';
	else {
		m = RX.hp.exec(sec);
		const hp = Number(m[1]);
		const max = Number(m[2]);
		const exact = 100 * hp / max;
		let pct = Math.ceil(exact);
		if (pct === 100 && hp < max) { pct = 99; checks.hpNaive.clamp99++; }
		if (Math.round(exact) !== pct) checks.hpNaive.roundDiffers++;
		if (Math.floor(exact) !== pct) checks.hpNaive.floorDiffers++;
		if (pct === 1) checks.hpNaive.one++;
		if (pct === 100) checks.hpNaive.hundred++;
		if (m[3]) checks.hpNaive.statusSuffix++;
		expect = `${pct}/100` + (m[3] ? ` ${m[3]}` : '');
	}
	if (sec === '0 fnt') checks.hpNaive.fnt++;
	if (expect === pub) checks.hpOk++;
	else pushLimited(checks.hpBad, `${sec} -> ${pub} (expected ${expect})`);
}

function handlePlain(line, prevKind, nextKind) {
	checks.entries++;
	if (line.includes('\n')) checks.newline++;
	if (!line.startsWith('|')) pushLimited(checks.nonPipe, JSON.stringify(line), 8);
	const sh = shapeOfLine(line, null);
	inc(kindCount, sh.kind);
	for (let i = 0; i < (sh.args || []).length; i++) {
		if (isHpClass(sh.cls[i])) inc(checks.unsplitHp, sh.kind);
	}
	recordShape(`${showShape(sh)}`, sh.kind, showShape(sh), line, sh.vocab, sh.hp);
	if (sh.kind === 't:') { inc(checks.tAdj.prev, prevKind); inc(checks.tAdj.next, nextKind); }
	if (BIGRAM_KINDS.has(sh.kind)) { inc(checks.bigram, `${prevKind} > ${sh.kind} > ${nextKind}`); }
	if (sh.kind === 'move' && sh.args) {
		const hasStill = sh.args.includes('[still]');
		const blank = sh.args[2] === '';
		if (blank && hasStill) checks.moveTarget.blankStill++;
		else if (blank) pushLimited(checks.moveTarget.blankNoStill, line);
		else if (hasStill) pushLimited(checks.moveTarget.stillNotBlank, line);
		else checks.moveTarget.withTarget++;
	}
	tagStats(sh.kind, sh.args);
}

/** Bracket-tag statistics: relative order of tags in lines with >= 2 tags, and which kinds carry which tag. */
function tagStats(kind, args) {
	if (!args) return;
	const tags = args.map(a => RX.tag.exec(a)).filter(Boolean).map(m => m[1]);
	checks.obsSigs.add(`${kind}/${args.length}/${tags.join(',')}`);
	if (tags.length >= 2) inc(checks.tagOrder, `${kind}: ${tags.join(' ')}`);
	for (const t of tags) {
		if (!checks.tagKinds.has(t)) checks.tagKinds.set(t, new Map());
		inc(checks.tagKinds.get(t), kind);
	}
}

/** Details/ident statistics from switch-like lines (secret or plain). */
function detailsStats(line) {
	const parts = line.split('|');
	const kind = parts[1];
	if (!(kind in DETAILS_POS)) return;
	const d = parts[DETAILS_POS[kind] + 2];
	if (d === undefined) return;
	const m = RX.details.exec(d);
	if (!m) { inc(checks.details, `${kind}: UNPARSED`); return; }
	const flags = [m[2] && 'L', m[3] && 'G', m[4] && 'shiny', m[5] && 'tera'].filter(Boolean).join('+') || '-';
	inc(checks.details, `${kind}: ${flags}`);
	const ident = parts[2];
	const name = ident.slice(ident.indexOf(': ') + 2);
	if (name === m[1]) checks.identVsSpecies.same++;
	else inc(checks.identVsSpecies.differ, `${name} | ${m[1]}`);
}

/** Whole-battle structural checks: turn numbering, opening order, repeated hints. */
function battleChecks(lines) {
	let want = 1;
	let turnsOk = true;
	const hints = new Map();
	let startAt = -1;
	for (let i = 0; i < lines.length; i++) {
		const l = lines[i];
		if (l.startsWith('|turn|')) {
			if (Number(l.slice(6)) !== want) turnsOk = false;
			want++;
		} else if (l === '|start') startAt = i;
		else if (l.startsWith('|-hint|')) hints.set(l, (hints.get(l) || 0) + 1);
	}
	if (turnsOk) checks.turnSeq.ok++;
	else pushLimited(checks.turnSeq.bad, 'non-consecutive |turn| numbers');
	for (const [h, n] of hints) checks.hintRepeat.set(h, Math.max(checks.hintRepeat.get(h) || 0, n));
	// opening: |teamsize|p1|N |teamsize|p2|N |start then four secret switch lines p1a p1b p2a p2b
	const idents = [];
	for (let i = startAt + 1; i < lines.length && idents.length < 4; i++) {
		if (lines[i].startsWith('|split|') && lines[i + 1].startsWith('|switch|')) idents.push(lines[i + 1].split('|')[2].slice(0, 3));
	}
	const opening = lines[startAt - 2]?.startsWith('|teamsize|p1|') && lines[startAt - 1]?.startsWith('|teamsize|p2|');
	if (opening && idents.join() === 'p1a,p1b,p2a,p2b') checks.startOrder.ok++;
	else pushLimited(checks.startOrder.bad, `opening order ${idents.join()}`);
}

async function processFile(file) {
	const rl = readline.createInterface({ input: fs.createReadStream(file, { highWaterMark: 1 << 20 }), crlfDelay: Infinity });
	for await (const text of rl) {
		if (!text.trim()) continue;
		const b = JSON.parse(text);
		checks.battles++;
		const lines = [];
		for (const s of b.steps) for (const l of s.log) lines.push(l);
		for (const l of b.end.log) lines.push(l);
		battleChecks(lines);
		for (let i = 0; i < lines.length; i++) {
			const line = lines[i];
			if (line.startsWith('|split|')) {
				const side = line.slice(7);
				if (i + 2 >= lines.length) { checks.splitBroken++; continue; }
				checks.entries += 3;
				handleSplit(side, lines[i + 1], lines[i + 2]);
				detailsStats(lines[i + 1]);
				i += 2;
				continue;
			}
			const prevKind = i > 0 ? (lines[i - 1].split('|')[1] ?? '(empty entry)') : '(start)';
			const nextKind = i + 1 < lines.length ? (lines[i + 1].split('|')[1] ?? '(empty entry)') : '(end)';
			handlePlain(line, prevKind || '(blank line)', nextKind || '(blank line)');
			detailsStats(line);
		}
	}
}

// ---------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------
function sortKinds(a, b) {
	const ka = a.replace(/^-/, '');
	const kb = b.replace(/^-/, '');
	return ka < kb ? -1 : ka > kb ? 1 : a < b ? -1 : 1;
}

function fmtTop(map, n) {
	const arr = [...map].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
	return { distinct: arr.length, top: arr.slice(0, n) };
}

function renderCatalog() {
	const out = [];
	const byKind = new Map();
	for (const s of shapes.values()) {
		if (!byKind.has(s.kind)) byKind.set(s.kind, []);
		byKind.get(s.kind).push(s);
	}
	const kinds = [...byKind.keys()].sort(sortKinds);
	const nShapes = shapes.size;
	out.push('# battle.log line shapes -- generated by tools/oracle/log-shapes.mjs (do not edit by hand)');
	out.push(`# inputs: ${opts.files.map(f => path.relative(repo, f) || f).join(', ')}`);
	const named = kinds.filter(k => k !== '').length;
	out.push(`# battles: ${checks.battles}   log entries: ${checks.entries}   message kinds: ${named} named + the |split| marker + the blank line "|"   distinct shapes: ${nShapes} (a SPLIT triple is one shape)`);
	out.push('# A "|split|pN" marker plus the following two entries are folded into one SPLIT shape (S = secret line, P = public line).');
	out.push('# Class legend: POKE `p1a: Name` | POKE_NOSLOT `p1: Name` | SIDE `p1` | SLOT `p1a` | SLOTS `p2a,p2b` | DETAILS `Sp, L84, F, shiny, tera:T`');
	out.push('#   HP/exact `123/321` | HP/pct `38/100` (both also cover `0 fnt` and a trailing status word: see the "HP variants" list) | EFFECT(move|ability|item|pokemon) `move: Protect`');
	out.push('#   word `[a-z0-9]+` | Name other short string | TEXT free text | INT | EMPTY | `[tag]` / `[tag] CLASS` bracket tags (positional, order matters)');
	out.push('# Counts: number of occurrences over all inputs. Per-argument value lists: "argN" is the 0-based index after the kind.');
	out.push('');
	out.push('## 0. KIND SUMMARY (by frequency; "split" counts |split|pN markers, the kind of a folded line is counted once for its secret line)');
	out.push('');
	const kc = [...kindCount].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
	for (const [k, n] of kc) {
		const sh = byKind.get(k) ? byKind.get(k).length : 0;
		out.push(`  ${String(n).padStart(9)}  ${kindLabel(k)}${sh ? `   (${sh} shape${sh > 1 ? 's' : ''})` : ''}`);
	}
	out.push('');
	out.push('## 1. SHAPES (sorted by kind, then frequency)');
	out.push('');
	for (const kind of kinds) {
		const list = byKind.get(kind).sort((a, b) => b.count - a.count || (a.display < b.display ? -1 : 1));
		const total = list.reduce((s, x) => s + x.count, 0);
		out.push(`=== ${kindLabel(kind)}   total ${total}, ${list.length} shape${list.length > 1 ? 's' : ''}`);
		for (const s of list) {
			out.push(`  [${s.count}] ${s.display}`);
			for (let i = 0; i < s.vocab.length; i++) {
				const m = s.vocab[i];
				if (!m) continue;
				const { distinct, top } = fmtTop(m, opts.top);
				const label = s.display.startsWith('SPLIT') ? `S.arg${i}` : `arg${i}`;
				const hp = s.hpPos && s.hpPos.has(i);
				out.push(`      ${label}: ${hp ? 'HP variants' : `${distinct} distinct${distinct > top.length ? `, top ${top.length}` : ''}`}: ${top.map(([v, n]) => `${v} (${n})`).join('; ')}`);
			}
			for (const e of s.examples) out.push(`      e.g. ${e}`);
		}
		out.push('');
	}
	return out;
}

const KEYED_SKIP = new Set(['move', '-prepare', '-anim', '-terastallize']);
function renderKeyed() {
	const out = [];
	out.push('## 2. KEYED SHAPES (shape + the first word/Name/EFFECT argument kept literal; complete list per kind, capped at 400 entries/kind)');
	out.push('');
	const byKind = new Map();
	for (const k of keyed.values()) {
		if (KEYED_SKIP.has(k.kind)) continue;
		if (!byKind.has(k.kind)) byKind.set(k.kind, []);
		byKind.get(k.kind).push(k);
	}
	out.push(`(not keyed: ${[...KEYED_SKIP].map(k => `|${k}|`).join(' ')}; their first literal is a move or type name, see the value lists in section 1)`);
	out.push('');
	for (const kind of [...byKind.keys()].sort(sortKinds)) {
		const list = byKind.get(kind).sort((a, b) => b.count - a.count || (a.display + a.lit?.val < b.display + b.lit?.val ? -1 : 1));
		out.push(`=== ${kindLabel(kind)}   ${list.length} keyed shape${list.length > 1 ? 's' : ''}`);
		for (const k of list.slice(0, 400)) {
			const lit = k.lit ? `   [arg${k.lit.pos} = ${k.lit.val}]` : '';
			out.push(`  [${k.count}] ${k.display}${lit}`);
		}
		if (list.length > 400) out.push(`  ... ${list.length - 400} more`);
		out.push('');
	}
	return out;
}

function renderChecks() {
	const c = checks;
	const out = [];
	const fm = (map, n = 20) => fmtTop(map, n).top.map(([k, v]) => `${k} (${v})`).join('; ');
	out.push('## 3. CHECKS (invariants verified over all inputs; quoted by docs/showdown/05-protocol.md)');
	out.push('');
	out.push(`entries not starting with "|": ${c.nonPipe.length ? c.nonPipe.join(', ') : 'none'};  entries containing a newline: ${c.newline}`);
	out.push(`split triples: ${c.splitTriples};  malformed (kind/arity mismatch between secret and public): ${c.splitBroken}`);
	out.push(`split kinds (secret line kind): ${fm(c.splitKinds)}`);
	out.push(`split triples whose public entry is the empty string "" (secret-only, e.g. side-specific -hint): ${fm(c.emptyPublic)}`);
	out.push(`HP-bearing lines found OUTSIDE a split triple: ${c.unsplitHp.size ? fm(c.unsplitHp) : 'none'}`);
	out.push(`split side == side of the first ident in the secret line: ${c.splitSide.ok} ok, ${c.splitSide.bad.length} bad ${c.splitSide.bad.join(' ; ')}`);
	out.push(`secret/public lines differing in a non-HP argument (should be none): ${c.splitDiff.length ? c.splitDiff.join(' ; ') : 'none'}`);
	out.push(`split triples with no HP argument where secret == public: ${c.splitSecretEqPublic}`);
	out.push('');
	out.push('HP percentage rule: public = `ceil(100*hp/maxhp)/100`, but 99 when that is 100 and hp < maxhp; status word copied; `0 fnt` for hp 0');
	out.push(`  checked ${c.hpChecked} secret/public HP pairs: ${c.hpOk} match, ${c.hpChecked - c.hpOk} mismatch ${c.hpBad.join(' ; ')}`);
	out.push(`  by kind: ${fm(c.hpKinds)}`);
	const n = c.hpNaive;
	out.push(`  discriminating power: ceil!=round in ${n.roundDiffers} pairs, ceil!=floor in ${n.floorDiffers}, the 99-clamp fired ${n.clamp99} times, pct==1 ${n.one} times, pct==100 ${n.hundred} times, with status suffix ${n.statusSuffix}, fnt ${n.fnt}`);
	out.push('');
	out.push(`details-string flag combinations (kind: flags; L=level, G=gender, shiny, tera): ${fm(c.details, 60)}`);
	out.push(`ident name == details species name: ${c.identVsSpecies.same} lines; differing (ident name | details species): ${fm(c.identVsSpecies.differ, 40)}`);
	out.push('');
	out.push(`|t:| predecessor kind: ${fm(c.tAdj.prev)};  successor kind: ${fm(c.tAdj.next)}`);
	out.push(`kind bigrams around structural lines (prev > kind > next): ${fm(c.bigram, 60)}`);
	out.push('');
	const mt = c.moveTarget;
	out.push(`|move| target field (arg2): blank with [still]: ${mt.blankStill}; blank without [still]: ${mt.blankNoStill.length ? mt.blankNoStill.join(' ; ') : 0}; non-blank with [still]: ${mt.stillNotBlank.length ? mt.stillNotBlank.join(' ; ') : 0}; non-blank otherwise: ${mt.withTarget}`);
	out.push(`bracket-tag order in lines with >= 2 tags: ${fm(c.tagOrder, 60)}`);
	out.push('');
	out.push('bracket-tag inventory (tag: kinds it is attached to, with counts; split lines are counted once, from their secret line):');
	for (const [t, m] of [...c.tagKinds].sort((a, b) => a[0] < b[0] ? -1 : 1)) out.push(`  [${t}]  ${fm(m, 20)}`);
	out.push('');
	out.push(`|turn|N numbers consecutive from 1 in ${c.turnSeq.ok} of ${c.battles} battles ${c.turnSeq.bad.join(' ; ')}`);
	out.push(`opening is |teamsize|p1 |teamsize|p2 |start, then switch-ins in the order p1a p1b p2a p2b: ${c.startOrder.ok} of ${c.battles} battles ${c.startOrder.bad.join(' ; ')}`);
	out.push(`max repeats of the same |-hint| text within one battle: ${[...c.hintRepeat].sort((a, b) => b[1] - a[1]).map(([k, v]) => `${v}x ${k.slice(7, 60)}`).join('; ')}`);
	out.push('');
	return out;
}


/** Section 4 (needs --ps): literal message kinds in the source versus the kinds seen in the fixtures. */
function renderCrossCheck() {
	const ps = path.resolve(opts.ps.replace(/^~/, os.homedir()));
	const require = createRequire(path.join(ps, 'package.json'));
	const { Dex } = require('./dist/sim/dex');
	const scope = JSON.parse(fs.readFileSync(path.join(repo, 'data/scope.json'), 'utf8'));
	const seen = new Set([...shapes.values()].map(s => s.kind));
	seen.add('split');
	const srcOf = (o, acc = [], depth = 0) => {
		if (!o || depth > 6) return acc;
		for (const v of Object.values(o)) {
			if (typeof v === 'function') acc.push(v.toString());
			else if (v && typeof v === 'object' && !Array.isArray(v)) srcOf(v, acc, depth + 1);
		}
		return acc;
	};
	// Static call signatures kind/arity/tags of add()/addMove() calls with a literal kind, via a small balanced-paren scanner.
	const sigEmitters = new Map();
	function* callsOf(src) {
		const re = /\.(?:add|addMove)\(/g;
		let m;
		while ((m = re.exec(src))) {
			let i = re.lastIndex;
			let depth = 1;
			const args = [];
			let cur = '';
			while (i < src.length && depth > 0) {
				const ch = src[i];
				if (ch === "'" || ch === '"' || ch === '`') {
					let j = i + 1;
					while (j < src.length && src[j] !== ch) {
						if (src[j] === '\\') j += 2;
						else if (ch === '`' && src[j] === '$' && src[j + 1] === '{') {
							let d = 1;
							j += 2;
							while (j < src.length && d > 0) { if (src[j] === '{') d++; else if (src[j] === '}') d--; j++; }
						} else j++;
					}
					cur += src.slice(i, j + 1);
					i = j + 1;
					continue;
				}
				if ('([{'.includes(ch)) depth++;
				else if (')]}'.includes(ch)) { depth--; if (depth === 0) break; }
				if (ch === ',' && depth === 1) { args.push(cur.trim()); cur = ''; } else cur += ch;
				i++;
			}
			if (cur.trim()) args.push(cur.trim());
			yield args;
		}
	}
	const emitters = new Map(); // kind -> Set(effect)
	const tagEmitters = new Map(); // bracket tag -> Set(effect or sim file:line)
	const tagRx = /['"`]\[([A-Za-z0-9]+)\]/g;
	const note = (kind, who) => { if (!emitters.has(kind)) emitters.set(kind, new Set()); emitters.get(kind).add(who); };
	const rx = /\.(?:add|addMove)\(\s*(['"`])([^'"`$]*)\1/g;
	const scanFns = (label, id, name, table) => {
		const src = srcOf(table[id]).join('\n');
		for (const m of src.matchAll(rx)) note(m[2], `${label}:${name}`);
		for (const args of callsOf(src)) {
			const k = /^['"`]([^'"`$]*)['"`]$/.exec(args[0] || '');
			if (!k || k[1] === '' || k[1] === 'move') continue;
			const tags = args.slice(1).map(a => /^['"`]\[([A-Za-z0-9]+)\]/.exec(a)?.[1]).filter(Boolean);
			const sig = `${k[1]}/${args.length - 1}/${tags.join(',')}`;
			if (!sigEmitters.has(sig)) sigEmitters.set(sig, new Set());
			sigEmitters.get(sig).add(`${label}:${name}`);
		}
		for (const m of src.matchAll(tagRx)) {
			if (!tagEmitters.has(m[1])) tagEmitters.set(m[1], new Set());
			tagEmitters.get(m[1]).add(`${label}:${name}`);
		}
	};
	for (const m of scope.moves) scanFns('move', m.id, m.name, Dex.data.Moves);
	for (const a of scope.abilities) scanFns('ability', a.id, a.name, Dex.data.Abilities);
	for (const i of scope.items) scanFns('item', i.id, i.name, Dex.data.Items);
	for (const c of scope.conditions) if (Dex.data.Conditions[c.id]) scanFns('condition', c.id, c.name, Dex.data.Conditions);
	for (const r of scope.rules) if (Dex.data.Rulesets[r.id]) scanFns('rule', r.id, r.name, Dex.data.Rulesets);
	const out = [];
	out.push('## 4. SOURCE CROSS-CHECK (literal first argument of add()/addMove() calls)');
	out.push('');
	out.push(`In-scope effect handlers (data/scope.json: ${scope.moves.length} moves, ${scope.abilities.length} abilities, ${scope.items.length} items, ${scope.conditions.length} conditions, ${scope.rules.length} rules; compiled functions from ${path.relative(os.homedir(), ps) ? '~/' + path.relative(os.homedir(), ps) : ps}/dist):`);
	const kindsHere = [...emitters.keys()].filter(k => k).sort(sortKinds);
	out.push(`  ${kindsHere.length} distinct literal kinds; ${kindsHere.filter(k => !seen.has(k)).length} not seen in the fixtures`);
	for (const k of kindsHere.filter(k => !seen.has(k))) out.push(`  NOT SEEN |${k}|  emitted by ${[...emitters.get(k)].slice(0, 12).join(', ')}`);
	out.push('  (kinds produced through a variable, e.g. add(msg, ...) in Battle.boost, are not visible to this scan)');
	const unseenTags = [...tagEmitters.keys()].filter(t => !checks.tagKinds.has(t)).sort();
	out.push(`  bracket-tag literals in those sources: ${tagEmitters.size} distinct; not seen in the fixtures: ${unseenTags.map(t => `[${t}] (${[...tagEmitters.get(t)].slice(0, 6).join(', ')})`).join('; ') || 'none'}`);
	const unseenSigs = [...sigEmitters.keys()].filter(sg => !checks.obsSigs.has(sg)).sort();
	out.push(`  static call signatures kind/arity/tags (arity = argument count after the kind): ${sigEmitters.size} in the sources, ${unseenSigs.length} never observed:`);
	for (const sg of unseenSigs) out.push(`    UNOBSERVED ${sg}  emitted by ${[...sigEmitters.get(sg)].slice(0, 6).join(', ')}`);
	out.push('');
	out.push('Engine (sim/*.ts) literal kinds not seen in the fixtures (file:line):');
	const engine = new Map();
	const engineTags = new Map();
	for (const f of ['battle.ts', 'battle-actions.ts', 'pokemon.ts', 'field.ts', 'side.ts']) {
		const lines = fs.readFileSync(path.join(ps, 'sim', f), 'utf8').split('\n');
		lines.forEach((ln, i) => {
			if (/^\s*(\/\/|\*|\/\*)/.test(ln)) return;
			const re = /(?:this|battle)\.(?:add|addMove)\(\s*(['"`])([^'"`$|]*)\1|addSplit\([^\[]*\[\s*(['"`])([^'"`]*)\3/g;
			for (const m of ln.matchAll(tagRx)) {
				if (checks.tagKinds.has(m[1])) continue;
				if (!engineTags.has(m[1])) engineTags.set(m[1], []);
				engineTags.get(m[1]).push(`sim/${f}:${i + 1}`);
			}
			for (const m of ln.matchAll(re)) {
				const k = m[2] ?? m[4];
				if (k === undefined || k === '' || seen.has(k)) continue;
				if (!engine.has(k)) engine.set(k, []);
				engine.get(k).push(`sim/${f}:${i + 1}`);
			}
		});
	}
	for (const k of [...engine.keys()].sort(sortKinds)) out.push(`  |${k}|  ${engine.get(k).join(', ')}`);
	out.push('Engine (sim/*.ts) bracket-tag literals not seen in the fixtures (file:line):');
	for (const t of [...engineTags.keys()].sort()) out.push(`  [${t}]  ${engineTags.get(t).join(', ')}`);
	out.push('');
	return out;
}

const t0 = Date.now();
for (const f of opts.files) {
	await processFile(f);
	console.error(`processed ${f} (${((Date.now() - t0) / 1000).toFixed(1)}s, ${checks.battles} battles so far)`);
}
const text = [...renderCatalog(), ...renderKeyed(), ...renderChecks(), ...(opts.ps ? renderCrossCheck() : [])].join('\n') + '\n';
if (opts.out) fs.writeFileSync(opts.out, text);
else process.stdout.write(text);
if (opts.json) {
	fs.writeFileSync(opts.json, JSON.stringify([...shapes.values()].map(s => ({
		kind: s.kind, shape: s.display, count: s.count, examples: s.examples,
		vocab: s.vocab.map(m => (m ? [...m].sort((a, b) => b[1] - a[1]) : null)),
	})), null, 1));
}
console.error(`kinds: ${new Set([...shapes.values()].map(s => s.kind)).size}, shapes: ${shapes.size}, keyed shapes: ${keyed.size}, ${((Date.now() - t0) / 1000).toFixed(1)}s`);
