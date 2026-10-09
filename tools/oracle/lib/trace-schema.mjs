// Trace format constants and helpers shared by trace.mjs (writer) and trace-diff.mjs (reader).
// The format itself is specified in docs/design/TRACE.md; keep the two in sync.

export const TRACE_VERSION = 1;

/** Record kind -> lowest --level that emits it. */
export const KIND_LEVEL = {
	hdr: 1, boundary: 1, choose: 1, chosen: 1, log: 1, logedit: 1, rng: 1,
	ev: 2, evx: 2,
	sort: 3, hc: 3, hcx: 3,
};

/** Kinds that only the replay driver emits (an engine does not need them inside the sim). */
export const DRIVER_KINDS = new Set(['hdr', 'boundary']);

/** Keys starting with this prefix are oracle-only annotations: never compared. */
export const NONNORMATIVE_PREFIX = '_';

/**
 * Canonical comparison form of a record: object keys sorted recursively, top-level keys starting with `_`
 * (oracle-only annotations) dropped. Two records are "the same" iff their canonical JSON texts are equal.
 * (`_` keys inside values are data and are kept.)
 */
export function canonical(rec) {
	const sorted = value => {
		if (Array.isArray(value)) return value.map(sorted);
		if (value !== null && typeof value === 'object') {
			const out = {};
			for (const key of Object.keys(value).sort()) out[key] = sorted(value[key]);
			return out;
		}
		return value;
	};
	const out = {};
	for (const key of Object.keys(rec).sort()) {
		if (!key.startsWith(NONNORMATIVE_PREFIX)) out[key] = sorted(rec[key]);
	}
	return out;
}

export const canonicalText = rec => JSON.stringify(canonical(rec));

const sameJson = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/**
 * "Trivial" event pair: `ev` immediately followed by its `evx` (no record in between) that did not change
 * the relay value. `--skip-empty` (trace.mjs) and `--squash-empty` (trace-diff.mjs) drop these. A relay
 * value that was absent (`@undefined` / null) and comes out as `true` is no change: Showdown substitutes
 * `true` for a missing relay variable.
 */
export function isTrivialPair(ev, evx) {
	if (evx.thr !== undefined) return false;
	if (sameJson(ev.v, evx.v)) return true;
	return (ev.v === '@undefined' || ev.v === null) && evx.v === true;
}
