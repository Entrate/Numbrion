// Replays one fixture battle through the pinned Showdown build with a Tracer attached.
//
// The replay itself is tools/oracle/lib/session.mjs: replayFixture() (Session = BattleStream-equivalent
// driver, sendUpdates() after every input, recorded choose order). It compares the replayed log, seeds,
// requests and choose results with the fixture; this module only adds the tracer around it and a
// `boundary` marker after every Session.snapshot(). So a trace with an empty `diffs` list proves that
// tracing did not change the battle.
import { Session, replayFixture } from './session.mjs';
import { Tracer } from './trace-core.mjs';

/**
 * @param sim  loadSim() result
 * @param fx   parsed fixture object
 * @param opts { level, stack, fromTurn, toTurn, skipEmpty, position }
 * @returns { lines, diffs, counts, records, bytes, ms }   `diffs` is replayFixture's list (empty = identical to the fixture)
 */
export function traceFixture(sim, fx, opts = {}) {
	const lines = [];
	const tracer = new Tracer(sim, { ...opts, sink: line => lines.push(line) });
	const origSnapshot = Session.prototype.snapshot;
	tracer.install();
	Session.prototype.snapshot = function () {
		const snap = origSnapshot.call(this);
		tracer.boundary(snap);
		return snap;
	};
	const t0 = performance.now();
	let diffs;
	try {
		tracer.header({
			format: fx.format, oracle: fx.oracle,
			fixture: { runSeed: fx.runSeed, index: fx.index, position: opts.position ?? null },
			battleSeed: fx.battleSeed,
		});
		tracer.arm();
		try {
			diffs = replayFixture(sim, fx);
		} catch (err) {
			// keep the partial trace: it ends right where Showdown threw
			diffs = [{ where: 'replay', what: `threw ${err && err.stack}` }];
		}
		if (tracer.frames.length && !diffs.length) diffs.push({ where: 'trace', what: `${tracer.frames.length} event frames left open` });
	} finally {
		Session.prototype.snapshot = origSnapshot;
		tracer.uninstall();
	}
	const ms = performance.now() - t0;
	return { lines, diffs, counts: tracer.counts, records: tracer.records, bytes: tracer.bytes, ms };
}

/** Same replay without a tracer (for timing and for the behaviour check). */
export function plainReplay(sim, fx) {
	const t0 = performance.now();
	const diffs = replayFixture(sim, fx);
	return { diffs, ms: performance.now() - t0 };
}
