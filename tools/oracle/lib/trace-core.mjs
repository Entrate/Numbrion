// Runtime tracer for the pinned Showdown build: records PRNG draws, log lines, events and handler
// lists of ONE battle as JSON-line records (format: docs/design/TRACE.md).
//
// Nothing in the Showdown checkout is modified. `Tracer.install()` replaces methods on the loaded
// module instance's `Battle.prototype`, `PRNG.prototype` and `Gen5RNG.prototype` (and installs a `log`
// accessor on `Battle.prototype`); `uninstall()` puts everything back. Every wrapper is
// behaviour-neutral: it calls the original with the original arguments and `this`, and only reads
// state (the replay in trace-replay.mjs asserts that log, seeds and requests still equal the fixture).
//
// The tracer follows exactly one battle: the first Battle whose `log` is assigned after `arm()`.
import fs from 'node:fs';
import path from 'node:path';
import { normalizeLogLine } from './common.mjs';
import { TRACE_VERSION, isTrivialPair } from './trace-schema.mjs';

const LOG_SLOT = Symbol('numbrion.trace.log');
const INDEX_KEY = /^(?:0|[1-9]\d*)$/;

// Relay values / args nested deeper or wider than this are cut off ("@..."): see TRACE.md section 3.
const MAX_VALUE_DEPTH = 4;
const MAX_ARRAY = 64;
const MAX_KEYS = 48;

// Frames that only forward to the PRNG or the event system; never useful as a call site.
const PLUMBING = new Set(['random', 'randomChance', 'sample', 'shuffle', 'singleEvent', 'runEvent', 'priorityEvent',
	'eachEvent', 'fieldEvent', 'speedSort', 'apply', 'call']);

export class Tracer {
	/**
	 * @param sim            loadSim() result (needs Battle, PRNG, Gen5RNG)
	 * @param opts.level     1: markers + log + rng; 2: + event calls; 3: + sorted handler lists, handler calls
	 * @param opts.stack     add oracle-only `_cs`/`_fx`/`_ev` annotations to rng records (default: level >= 3)
	 * @param opts.fromTurn  only records emitted while battle.turn >= fromTurn
	 * @param opts.toTurn    only records emitted while battle.turn <= toTurn
	 * @param opts.skipEmpty drop `ev`/`evx` pairs that contain no other record and leave the relay value unchanged
	 * @param opts.sink      (jsonLine: string) => void
	 */
	constructor(sim, opts) {
		this.sim = sim;
		this.level = opts.level ?? 3;
		if (![1, 2, 3].includes(this.level)) throw new Error(`bad trace level ${this.level}`);
		this.stack = opts.stack ?? this.level >= 3;
		this.from = opts.fromTurn ?? -Infinity;
		this.to = opts.toTurn ?? Infinity;
		this.skipEmpty = !!opts.skipEmpty;
		this.sink = opts.sink;

		this.battle = null;
		this.armed = false;
		this.installed = false;
		this.restores = [];

		this.frames = []; // open ev frames
		this.prioPending = false; // priorityEvent() is about to call runEvent()
		this.findDepth = 0; // nesting of findEventHandlers (it recurses for array targets)
		this.rngCtx = null; // set while a top-level randomChance/sample/shuffle runs
		this.draws = 0; // Gen5RNG.next() calls on the traced battle's PRNG
		this.shuffles = 0; // top-level PRNG.shuffle calls on the traced battle's PRNG
		this.step = 0; // boundary counter
		this.cn = new WeakMap(); // handler object -> callback name it was resolved for
		this.fileLines = new Map(); // dist file -> lines (for data call-site entry names)

		this.counts = {}; // records written, per kind
		this.records = 0;
		this.bytes = 0;
	}

	// -----------------------------------------------------------------------------------
	// Output
	// -----------------------------------------------------------------------------------

	inWindow() {
		const t = this.battle ? this.battle.turn : undefined;
		if (t === undefined) return this.from <= 0;
		return t >= this.from && t <= this.to;
	}

	write(rec) {
		const line = JSON.stringify(rec);
		this.counts[rec.k] = (this.counts[rec.k] || 0) + 1;
		this.records++;
		this.bytes += line.length + 1;
		this.sink(line);
	}

	/** Emit a record if the turn window allows it. Returns whether it was written. */
	emit(rec) {
		if (!this.inWindow()) return false;
		if (this.skipEmpty) this.flushFrames();
		this.write(rec);
		return true;
	}

	flushFrames() {
		for (const f of this.frames) {
			if (f.win && !f.emitted) {
				this.write(f.rec);
				f.emitted = true;
			}
		}
	}

	/** Open an `ev` frame. `rec.d` is set to the number of enclosing frames. */
	enter(rec, kind, eventid) {
		rec.d = this.frames.length;
		const frame = { rec, win: this.inWindow(), emitted: false, kind, eventid, handlers: null, sorted: false };
		this.frames.push(frame);
		if (frame.win && !this.skipEmpty) {
			this.write(rec);
			frame.emitted = true;
		}
		return frame;
	}

	/** Close the frame; its `evx` goes out iff the matching `ev` did. */
	leave(frame, xrec) {
		// --skip-empty: a frame without child records is dropped unless it changed the relay value
		if (this.skipEmpty && frame.win && !frame.emitted && !isTrivialPair(frame.rec, xrec)) this.flushFrames();
		const top = this.frames.pop();
		if (top !== frame) throw new Error('trace frame stack out of sync');
		xrec.d = this.frames.length;
		if (frame.emitted) this.write(xrec);
	}

	header(info) {
		this.write({
			k: 'hdr', v: TRACE_VERSION, level: this.level, ...info,
			from: Number.isFinite(this.from) ? this.from : null, to: Number.isFinite(this.to) ? this.to : null,
			stack: this.stack, skipEmpty: this.skipEmpty,
		});
	}

	/** Decision boundary marker, called by the replay driver right after Session.snapshot(). */
	boundary(snap) {
		const b = this.battle;
		this.emit({ k: 'boundary', step: this.step++, turn: snap.turn, state: b && b.ended ? 'end' : snap.state, seed: snap.seed });
	}

	// -----------------------------------------------------------------------------------
	// Value representations (TRACE.md section 3)
	// -----------------------------------------------------------------------------------

	/** `p1a` for an active Pokemon, `p1:3` (index in the side's original team) otherwise. */
	monRef(p) {
		if (p.isActive) return p.getSlot();
		const side = p.side;
		let idx = side.team ? side.team.indexOf(p.set) : -1;
		if (idx < 0) idx = side.pokemon.indexOf(p);
		return `${side.id}:${idx}`;
	}

	/** `ability:intimidate`, `move:tackle`, `condition:confusion`, `str:drain` ... */
	fx(e) {
		if (e === null || e === undefined) return null;
		if (typeof e === 'string') return `str:${e}`;
		if (e.effectType === undefined) {
			if (e.id === '') return null; // Battle#effect's "no effect" sentinel `{ id: '' }`
			if (typeof e.id === 'string') return `str:${e.id}`; // bare `{ id }` pseudo-effects such as strugglerecoil
			return 'anon'; // inline handler containers (a move's `secondary` object) have no id at all
		}
		return `${String(e.effectType).toLowerCase()}:${e.id}`;
	}

	/** Reference string for a Pokemon / Side / Field / Battle / Effect object, else null. */
	refOrNull(x) {
		if (typeof x.getSlot === 'function' && x.side) return this.monRef(x);
		if (x.sideConditions !== undefined && typeof x.id === 'string' && Array.isArray(x.pokemon)) return `side:${x.id}`;
		if (x.pseudoWeather !== undefined) return 'field';
		if (typeof x.speedSort === 'function') return 'battle';
		if (typeof x.effectType === 'string' && typeof x.id === 'string') return this.fx(x);
		return null;
	}

	/** Typed reference field (`t`, `s`, `h`, `x`, ...): null/false stay as they are. */
	ref(x) {
		if (x === null || x === undefined) return null;
		if (x === false) return false;
		if (typeof x === 'string') return `str:${x}`;
		if (Array.isArray(x)) return x.map(m => this.ref(m));
		if (typeof x !== 'object') return `?:${String(x)}`;
		return this.refOrNull(x) ?? '?';
	}

	/** JSON-safe value (relay values, handler returns): `@...` strings mark non-JSON things. */
	jv(x, depth = 0) {
		if (x === undefined) return '@undefined';
		if (x === null || typeof x === 'boolean') return x;
		if (typeof x === 'number') return Number.isFinite(x) ? x : `@${x}`;
		if (typeof x === 'string') return x.charCodeAt(0) === 64 ? `@${x}` : x;
		if (typeof x === 'bigint') return `@bigint:${x}`;
		if (typeof x === 'function') return '@fn';
		if (typeof x === 'symbol') return '@symbol';
		const r = this.refOrNull(x);
		if (r !== null) return `@${r}`;
		if (depth >= MAX_VALUE_DEPTH) return '@...';
		if (Array.isArray(x)) {
			const out = x.slice(0, MAX_ARRAY).map(v => this.jv(v, depth + 1));
			if (x.length > MAX_ARRAY) out.push('@...');
			return out;
		}
		if (x instanceof Set) return '@set';
		if (x instanceof Map) return '@map';
		const out = {};
		const keys = Object.keys(x);
		for (const key of keys.slice(0, MAX_KEYS)) out[key] = this.jv(x[key], depth + 1);
		if (keys.length > MAX_KEYS) out['@...'] = true;
		return out;
	}

	speciesOf(x) {
		return x && typeof x === 'object' && typeof x.getSlot === 'function' && x.species ? x.species.id : undefined;
	}

	// -----------------------------------------------------------------------------------
	// Call sites (oracle-only annotations)
	// -----------------------------------------------------------------------------------

	/** Nearest sim/ and data/ frames of the JS stack, as `Type.fn@line` / `data/file:fn@line[entry]`. */
	callSite(max = 3) {
		const prevPrep = Error.prepareStackTrace;
		const prevLimit = Error.stackTraceLimit;
		let sites;
		try {
			Error.stackTraceLimit = 48;
			Error.prepareStackTrace = (_err, s) => s;
			const holder = {};
			Error.captureStackTrace(holder);
			sites = holder.stack;
		} finally {
			Error.prepareStackTrace = prevPrep;
			Error.stackTraceLimit = prevLimit;
		}
		const out = [];
		if (!Array.isArray(sites)) return out;
		for (const s of sites) {
			const file = s.getFileName();
			if (!file) continue;
			const m = /\/dist\/(sim|data)\/(.+)\.js$/.exec(file);
			if (!m || m[2] === 'prng') continue;
			const fn = s.getFunctionName() || '<anon>';
			if (PLUMBING.has(fn)) continue;
			const line = s.getLineNumber();
			if (m[1] === 'data') out.push(`data/${m[2]}:${fn}@${line}[${this.dataEntry(file, line)}]`);
			else out.push(`${s.getTypeName() || '?'}.${fn}@${line}`);
			if (out.length >= max) break;
		}
		return out;
	}

	/** Key of the top-level entry of a dist/data file that contains `line` (e.g. the move or ability id). */
	dataEntry(file, line) {
		let lines = this.fileLines.get(file);
		if (!lines) {
			try { lines = fs.readFileSync(file, 'utf8').split('\n'); } catch { lines = []; }
			this.fileLines.set(file, lines);
		}
		for (let i = Math.min(line, lines.length) - 1; i >= 0; i--) {
			const m = /^ {2}(?:"([^"]+)"|([A-Za-z0-9_$]+)): \{$/.exec(lines[i]);
			if (m) return m[1] ?? m[2];
		}
		return '?';
	}

	// -----------------------------------------------------------------------------------
	// Install / uninstall
	// -----------------------------------------------------------------------------------

	/** The next Battle whose `log` is assigned becomes the traced battle. */
	arm() {
		this.armed = true;
		this.battle = null;
	}

	patch(obj, name, make) {
		const orig = obj[name];
		if (typeof orig !== 'function') throw new Error(`cannot trace: ${name} is not a function`);
		obj[name] = make(orig);
		this.restores.push(() => { obj[name] = orig; });
	}

	install() {
		if (this.installed) throw new Error('tracer already installed');
		this.installed = true;
		const T = this;
		const BC = this.sim.Battle;
		const B = BC.prototype;
		const P = this.sim.PRNG.prototype;
		const G = this.sim.Gen5RNG.prototype;

		// ---- log: a Proxy around battle.log reports appends and in-place edits --------------
		Object.defineProperty(B, 'log', {
			configurable: true,
			enumerable: false,
			get() { return this[LOG_SLOT]; },
			set(v) { this[LOG_SLOT] = T.wrapLog(this, v); },
		});
		this.restores.push(() => {
			delete B.log;
			if (this.battle && Object.prototype.hasOwnProperty.call(this.battle, LOG_SLOT)) {
				Object.defineProperty(this.battle, 'log', {
					value: this.battle[LOG_SLOT], writable: true, configurable: true, enumerable: true,
				});
			}
		});

		// ---- PRNG ----------------------------------------------------------------------------
		this.patch(G, 'next', orig => function () {
			if (T.battle && T.battle.prng && this === T.battle.prng.rng) T.draws++;
			return orig.call(this);
		});
		const rngWrapper = method => orig => function (...args) {
			const b = T.battle;
			if (!b || this !== b.prng) return orig.apply(this, args);
			if (T.rngCtx) { // nested call made by randomChance / sample / shuffle
				const r = orig.apply(this, args);
				if (method === 'random') T.rngCtx.picks.push(r);
				return r;
			}
			const ctx = { picks: [], n0: T.draws, s0: this.getSeed() };
			T.rngCtx = ctx;
			if (method === 'shuffle') T.shuffles++;
			let r;
			try {
				r = orig.apply(this, args);
			} finally {
				T.rngCtx = null;
			}
			T.onDraw(this, method, args, r, ctx);
			return r;
		};
		for (const method of ['random', 'randomChance', 'sample', 'shuffle']) this.patch(P, method, rngWrapper(method));

		// ---- markers -------------------------------------------------------------------------
		this.patch(B, 'choose', orig => function (sideid, input) {
			if (this !== T.battle) return orig.call(this, sideid, input);
			const wrote = T.emit({ k: 'choose', side: sideid, input });
			let ok = false;
			try {
				ok = orig.call(this, sideid, input);
				return ok;
			} finally {
				if (wrote) T.write({ k: 'chosen', side: sideid, ok });
			}
		});

		if (this.level < 2) return;

		// ---- events (level 2) ----------------------------------------------------------------
		this.patch(B, 'singleEvent', orig => function (eventid, effect, state, target, source, sourceEffect, relayVar, customCallback) {
			if (this !== T.battle) return orig.apply(this, arguments);
			const callback = customCallback || (effect ? effect[`on${eventid}`] : undefined);
			if (callback === undefined) return orig.apply(this, arguments); // no handler: not an event call
			const rec = {
				k: 'ev', d: 0, f: 'single', e: eventid, ef: T.fx(effect), t: T.ref(target), s: T.ref(source),
				x: T.fx(sourceEffect), v: T.jv(relayVar),
			};
			T.annotate(rec, target, source);
			return T.call(rec, 'single', eventid, () => orig.apply(this, arguments));
		});

		this.patch(B, 'runEvent', orig => function (eventid, target, source, sourceEffect, relayVar, onEffect, fastExit) {
			if (this !== T.battle) return orig.apply(this, arguments);
			const prio = T.prioPending;
			T.prioPending = false;
			const rec = {
				k: 'ev', d: 0, f: prio ? 'priority' : 'run', e: eventid, t: T.ref(target || this), s: T.ref(source),
				x: T.fx(sourceEffect), v: T.jv(relayVar),
			};
			if (onEffect) rec.oe = true;
			if (fastExit && !prio) rec.fe = true;
			T.annotate(rec, target, source);
			return T.call(rec, prio ? 'priority' : 'run', eventid, () => orig.apply(this, arguments));
		});

		this.patch(B, 'priorityEvent', orig => function () {
			if (this !== T.battle) return orig.apply(this, arguments);
			T.prioPending = true;
			try {
				return orig.apply(this, arguments);
			} finally {
				T.prioPending = false;
			}
		});

		this.patch(B, 'eachEvent', orig => function (eventid, effect, relayVar) {
			if (this !== T.battle) return orig.apply(this, arguments);
			const rec = { k: 'ev', d: 0, f: 'each', e: eventid, x: T.fx(effect || this.effect), v: T.jv(relayVar) };
			return T.call(rec, 'each', eventid, () => orig.apply(this, arguments));
		});

		this.patch(B, 'fieldEvent', orig => function (eventid, targets) {
			if (this !== T.battle) return orig.apply(this, arguments);
			const rec = { k: 'ev', d: 0, f: 'field', e: eventid, t: targets ? targets.map(p => T.ref(p)) : null };
			return T.call(rec, 'field', eventid, () => orig.apply(this, arguments));
		});

		if (this.level < 3) return;

		// ---- handler lists and handler calls (level 3) ---------------------------------------
		this.patch(B, 'resolvePriority', orig => function (h, callbackName) {
			const handler = orig.apply(this, arguments);
			if (this === T.battle) {
				T.cn.set(handler, callbackName);
				if (typeof handler.callback === 'function') handler.callback = T.wrapCallback(handler, callbackName);
			}
			return handler;
		});

		this.patch(B, 'findEventHandlers', orig => function (target, eventName) {
			if (this !== T.battle) return orig.apply(this, arguments);
			T.findDepth++;
			let handlers;
			try {
				handlers = orig.apply(this, arguments);
			} finally {
				T.findDepth--;
			}
			const frame = T.frames[T.frames.length - 1];
			if (T.findDepth === 0 && frame && (frame.kind === 'run' || frame.kind === 'priority') && frame.handlers === null) {
				frame.handlers = handlers;
				// runEvent sorts Invulnerability/TryHit/DamagingHit/EntryHazard lists with Array#sort(compareLeftToRightOrder)
				// and fastExit lists with compareRedirectOrder; a per-array `sort` hook reports those.
				Object.defineProperty(handlers, 'sort', {
					configurable: true,
					writable: true,
					enumerable: false,
					value(cmp) {
						Array.prototype.sort.call(this, cmp);
						const mode = cmp === BC.compareLeftToRightOrder ? 'ltr' : cmp === BC.compareRedirectOrder ? 'redirect' : 'other';
						T.emitSort('event', frame, this, mode, 0);
						return this;
					},
				});
			}
			return handlers;
		});

		this.patch(B, 'speedSort', orig => function (list, comparator) {
			if (this !== T.battle) return orig.apply(this, arguments);
			const frame = T.frames[T.frames.length - 1];
			const sh0 = T.shuffles;
			const n = list.length;
			const first = list[0];
			const eventList = frame && frame.handlers === list;
			// fieldEvent builds its own handler list; like runEvent's it is reported from one handler on
			const fieldList = !!frame && frame.kind === 'field' && !frame.sorted && n >= 1 &&
				first.effect !== undefined && first.effectHolder !== undefined;
			if (fieldList) frame.sorted = true;
			const r = orig.apply(this, arguments);
			if (eventList) T.emitSort('event', frame, list, 'speed', T.shuffles - sh0);
			else if (fieldList || n >= 2) T.emitSort(null, frame, list, 'speed', T.shuffles - sh0);
			return r;
		});
	}

	uninstall() {
		for (const undo of this.restores.reverse()) undo();
		this.restores = [];
		this.installed = false;
		this.battle = null;
		this.armed = false;
	}

	// -----------------------------------------------------------------------------------
	// Hook bodies
	// -----------------------------------------------------------------------------------

	annotate(rec, target, source) {
		const ts = this.speciesOf(target);
		if (ts !== undefined) rec._ts = ts;
		const ss = this.speciesOf(source);
		if (ss !== undefined) rec._ss = ss;
	}

	/** Runs `fn` inside an `ev`/`evx` pair. */
	call(rec, kind, eventid, fn) {
		const frame = this.enter(rec, kind, eventid);
		let out;
		try {
			out = fn();
		} catch (err) {
			this.leave(frame, { k: 'evx', d: 0, e: eventid, thr: String(err && err.message) });
			throw err;
		}
		this.leave(frame, { k: 'evx', d: 0, e: eventid, v: this.jv(out) });
		return out;
	}

	wrapLog(battle, arr) {
		if (this.armed && this.battle === null) {
			this.battle = battle;
			this.armed = false;
		}
		if (battle !== this.battle) return arr;
		const T = this;
		return new Proxy(arr, {
			set(target, prop, value) {
				const old = target.length;
				const ok = Reflect.set(target, prop, value);
				if (typeof prop === 'string' && INDEX_KEY.test(prop) && typeof value === 'string') {
					const i = Number(prop);
					T.emit({ k: i >= old ? 'log' : 'logedit', i, line: normalizeLogLine(value) });
				}
				return ok;
			},
			get(target, prop, receiver) {
				if (prop === 'splice') {
					// Battle#attrLastMove removes a `|-anim|` line this way; report it as one deletion.
					return (...a) => {
						const removed = Array.prototype.splice.apply(target, a);
						for (let k = 0; k < removed.length; k++) T.emit({ k: 'logedit', i: Number(a[0]) + k, line: null });
						return removed;
					};
				}
				return Reflect.get(target, prop, receiver);
			},
		});
	}

	onDraw(prng, method, args, result, ctx) {
		const n = this.draws - ctx.n0;
		if (n === 0) return;
		while (args.length && args[args.length - 1] === undefined) args.pop();
		const rec = { k: 'rng', m: method, a: null, r: null, n, s0: seedWords(ctx.s0), s1: seedWords(prng.getSeed()) };
		if (method === 'sample') {
			rec.a = [args[0].length];
			rec.r = ctx.picks[0];
			rec._it = this.jv(result);
		} else if (method === 'shuffle') {
			const items = args[0];
			rec.a = [args[1] ?? 0, args[2] ?? items.length];
			rec.r = ctx.picks;
		} else {
			rec.a = args.map(a => this.jv(a));
			rec.r = result;
		}
		if (this.stack) {
			rec._cs = this.callSite();
			const b = this.battle;
			if (b.effect && b.effect.id) rec._fx = this.fx(b.effect);
			if (b.event && b.event.id) rec._ev = b.event.id;
		}
		this.emit(rec);
	}

	wrapCallback(handler, cn) {
		const T = this;
		const cb = handler.callback;
		return function (...args) {
			if (this !== T.battle) return cb.apply(this, args);
			const rec = { k: 'hc', d: T.frames.length, cn, x: T.fx(handler.effect), h: T.ref(handler.effectHolder) };
			if (args.length === 4) rec.v = T.jv(args[0]);
			const wrote = T.emit(rec);
			let ret;
			try {
				ret = cb.apply(this, args);
			} catch (err) {
				if (wrote) T.write({ k: 'hcx', d: rec.d, thr: String(err && err.message) });
				throw err;
			}
			if (wrote) T.write({ k: 'hcx', d: rec.d, r: T.jv(ret) });
			return ret;
		};
	}

	/**
	 * One `sort` record. `w` is the list label: 'event' (runEvent/priorityEvent handler list), or, when
	 * null, derived from the list contents and the open frame: 'field' (fieldEvent handlers), 'each'
	 * (eachEvent actives), 'queue' (action queue), 'switchin' (BattleActions#runSwitch), 'mons'.
	 */
	emitSort(w, frame, list, sm, sh) {
		const first = list[0];
		const isHandler = first && first.effect !== undefined && first.effectHolder !== undefined;
		const isAction = first && typeof first.choice === 'string';
		const isMon = first && typeof first.getSlot === 'function';
		let items;
		let e;
		let cs;
		if (w === 'event') {
			if (list.length === 0) return; // nothing to report for an empty list
			e = frame.eventid;
			items = list.map(h => this.handlerItem(h, sm, e));
		} else if (isHandler) {
			w = frame && frame.kind === 'field' ? 'field' : 'handlers';
			e = frame ? frame.eventid : undefined;
			items = list.map(h => this.handlerItem(h, sm, e));
		} else if (isAction) {
			w = 'queue';
			items = list.map(a => this.actionItem(a));
		} else if (isMon) {
			items = list.map(m => ({ h: this.monRef(m), sp: m.speed || 0 }));
			if (frame && frame.kind === 'each' && !frame.sorted) {
				w = 'each';
				e = frame.eventid;
				frame.sorted = true;
			} else {
				cs = this.callSite(2);
				w = cs.some(c => c.includes('.runSwitch@')) ? 'switchin' : 'mons';
			}
		} else {
			w = 'other';
			items = list.map(x => this.jv(x));
		}
		const rec = { k: 'sort', d: this.frames.length, w };
		if (e !== undefined) rec.e = e;
		rec.sm = sm;
		rec.sh = sh;
		rec.it = items;
		if (cs && this.stack) rec._cs = cs;
		this.emit(rec);
	}

	handlerItem(h, sm, eventid) {
		const it = {
			x: this.fx(h.effect), h: this.ref(h.effectHolder), cn: this.cn.get(h) ?? `on${eventid}`,
			o: h.order || 0, p: h.priority || 0, so: h.subOrder || 0, sp: h.speed || 0,
			eo: sm === 'redirect' ? (h.effectHolder && h.effectHolder.abilityState ? h.effectHolder.abilityState.effectOrder || 0 : 0) : h.effectOrder || 0,
		};
		if (h.index !== undefined) it.ix = h.index;
		if (typeof h.callback !== 'function') it.cv = this.jv(h.callback);
		return it;
	}

	actionItem(a) {
		const it = { c: a.choice, h: this.ref(a.pokemon) };
		if (a.choice === 'move' || a.choice === 'beforeTurnMove' || a.choice === 'priorityChargeMove') {
			if (a.moveid || a.move) it.m = a.moveid || a.move.id;
		} else if (a.target && typeof a.target === 'object') {
			it.to = this.ref(a.target);
		}
		it.o = a.order || 0;
		it.p = a.priority || 0;
		it.sp = a.speed || 0;
		it.so = a.subOrder || 0;
		it.eo = a.effectOrder || 0;
		return it;
	}
}

function seedWords(str) {
	return str.split(',').map(Number);
}
