// Reference values for the direct (outside-a-move) tests of the protect_redirection batch:
// Helping Hand's restart multiplier stacking (data/moves.ts:8588-8600) and Stall's counter tripling,
// 729 cap and duration reset (data/conditions.ts:437-460), read from the real pinned handlers.
//
//   node tools/probes/protect_redirection/restart-vectors.mjs [PS_PATH]
//
// Output is a JSON document; the Rust tests (conditions/protectredirection/tests.rs) hard-code the
// numbers and the exact log lines printed here.
import { loadSim } from '../../oracle/lib/common.mjs';
import { Session } from '../../oracle/lib/session.mjs';

const sim = loadSim(process.argv[2] || `${process.env.HOME}/src/pokemon-showdown`);
const set = (species, ability, moves) =>
	['', species, '', ability, moves.join(','), 'Serious', '252,0,128,0,128,0', 'M', '', '', '100', ',,,,,'].join('|');
const p1 = [set('Clefairy', 'Friend Guard', ['followme', 'calmmind']), set('Garchomp', 'Rough Skin', ['earthquake', 'calmmind'])].join(']');
const p2 = [set('Kingambit', 'Defiant', ['brickbreak', 'calmmind']), set('Dondozo', 'Unaware', ['calmmind', 'brickbreak'])].join(']');
const s = new Session(sim, { seed: [1, 2, 3, 4], teams: [p1, p2] });
s.snapshot();
const b = s.battle;
const [a, c] = b.sides[0].active;

const out = { helpinghand: [], stall: [] };
const seedBefore = b.prng.getSeed();
const from = b.log.length;
for (let i = 0; i < 3; i++) {
	const r = a.addVolatile('helpinghand', c);
	out.helpinghand.push({ result: r, multiplier: a.volatiles.helpinghand.effectState?.multiplier ?? a.volatiles.helpinghand.multiplier, duration: a.volatiles.helpinghand.duration });
}
out.helpingHandLog = b.log.slice(from);
const from2 = b.log.length;
for (let i = 0; i < 8; i++) {
	const r = c.addVolatile('stall');
	const v = c.volatiles.stall;
	out.stall.push({ result: r, counter: v.counter, duration: v.duration });
}
out.stallLog = b.log.slice(from2);
out.seedUnchanged = b.prng.getSeed() === seedBefore;

// StallMove events (one randomChance(1, counter) each): the volatile is deleted on failure and
// re-added (counter 3) so that every run starts from an existing stall; successful runs keep it.
out.stallMoveStartSeed = b.prng.getSeed();
out.stallMove = [];
c.removeVolatile('stall');
c.addVolatile('stall');
for (let i = 0; i < 12; i++) {
	const had = !!c.volatiles.stall;
	const before = c.volatiles.stall?.counter;
	const ok = b.runEvent('StallMove', c);
	out.stallMove.push({ counterBefore: before, ok, seed: b.prng.getSeed(), hasStall: !!c.volatiles.stall });
	if (!c.volatiles.stall) c.addVolatile('stall');
	else if (i % 3 === 2) c.addVolatile('stall'); // restart: counter *= 3
	void had;
}
console.log(JSON.stringify(out, null, 1));
