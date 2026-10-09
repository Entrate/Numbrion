// Replay helpers shared by gen-directed.mjs and coverage.mjs.
//
// Directed fixtures carry hand-built teams, so the first check of session.mjs `replayFixture`
// ("regenerated teams" == fx.teams, which re-runs Showdown's random-team generator from fx.teamSeeds) cannot
// apply to them. `replaySim` gives replayFixture a sim whose team "generator" returns the fixture's own packed
// team, so every other check (derived seeds, log deltas, PRNG seeds, requests as JSON text, accept/reject
// results, error texts, request updates, end record) runs unchanged on the real Session code.
// The team determinism of a directed fixture is checked separately by `gen-directed.mjs --verify-file`, which
// rebuilds the teams from the profile and the recorded seeds.
import { FORMAT_ID, seedString } from './common.mjs';
import { replayFixture, replayViaBattleStream } from './session.mjs';

/** True for fixtures written by gen-directed.mjs (they have a `profile` key; random fixtures do not). */
export const isDirected = fx => typeof fx.profile === 'string';

export function replaySim(sim, fx, { always = false } = {}) {
	if (!always && !isDirected(fx)) return sim;
	const packedBySeed = new Map(fx.teamSeeds.map((s, i) => [seedString(s), fx.teams[i]]));
	const Teams = {
		getGenerator(format, seed) {
			if (format !== FORMAT_ID) throw new Error(`unexpected format ${format}`);
			const packed = packedBySeed.get(seed);
			if (packed === undefined) throw new Error(`no recorded team for seed ${seed}`);
			return { getTeam: () => [{ __packed: packed }] };
		},
		pack: team => (team.length === 1 && team[0].__packed !== undefined ? team[0].__packed : sim.Teams.pack(team)),
		unpack: s => sim.Teams.unpack(s),
	};
	return { ...sim, Teams };
}

/** Same two checks as `gen-fixtures.mjs --verify-replay`: direct replay, then through Showdown's BattleStream. */
export function verifyFixture(sim, fx, opts) {
	const s = replaySim(sim, fx, opts);
	let diffs = replayFixture(s, fx);
	if (!diffs.length) diffs = replayViaBattleStream(s, fx);
	return diffs;
}

export { replayFixture, replayViaBattleStream };
