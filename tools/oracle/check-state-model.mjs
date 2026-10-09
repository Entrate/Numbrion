// Reproducible probes for docs/showdown/04-state-model.md against a built oracle.
// Usage: node tools/oracle/check-state-model.mjs /path/to/pokemon-showdown
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import path from 'node:path';
const require = createRequire(path.resolve(process.argv[2], 'package.json'));
const { Battle, Teams, PRNG } = require('./dist/sim');
let checks = 0;
function check(name, fn) { fn(); checks++; console.log(`ok ${checks} - ${name}`); }
const b = new Battle({ formatid: 'gen9randomdoublesbattle', seed: [1, 2, 3, 4], send() {} });
check('permanent Sleep Clause state exists before players', () => {
  assert.deepEqual(Object.keys(b.field.pseudoWeather), ['sleepclausemod']);
  assert.equal(b.field.pseudoWeather.sleepclausemod.duration, undefined);
});
check('rules omit preview and Endless Battle Clause', () => {
  assert.equal(b.ruleTable.has('teampreview'), false);
  assert.equal(b.ruleTable.has('endlessbattleclause'), false);
});
const teams = [1, 2].map(n => Teams.getGenerator(b.format, [n, 7, 11, 13]).getTeam());
check('generator supplies six neutral-nature sets with gender', () => {
  for (const team of teams) {
    assert.equal(team.length, 6);
    for (const set of team) { assert.equal(set.nature, undefined); assert.ok(set.gender); }
  }
});
b.setPlayer('p1', { name: 'A', team: Teams.pack(teams[0]) });
b.setPlayer('p2', { name: 'B', team: Teams.pack(teams[1]) });
const p = b.p1.active[0], ally = b.p1.active[1], foe = b.p2.active[0];
check('HP Percentage and Cancel Mod flags become active', () => {
  assert.equal(b.reportPercentages, true); assert.equal(b.supportCancel, true);
});
check('active slots agree with the party permutation', () => {
  for (const side of b.sides) {
    side.pokemon.forEach((mon, i) => assert.equal(mon.position, i));
    side.active.forEach((mon, i) => assert.equal(mon, side.pokemon[i]));
  }
});
check('field position numbering differs from active iteration order', () => {
  assert.deepEqual(b.getAllActive().map(mon => mon.getFieldPositionValue()), [0, 2, 1, 3]);
});
check('Gen 9 baseMaxhp equals maxhp', () => {
  for (const side of b.sides) for (const mon of side.pokemon) assert.equal(mon.baseMaxhp, mon.maxhp);
});
check('initial move slots alias their base slot objects', () => {
  assert.notEqual(p.moveSlots, p.baseMoveSlots);
  p.moveSlots.forEach((slot, i) => assert.equal(slot, p.baseMoveSlots[i]));
});
check('PP deduction propagates into persistent base slots', () => {
  const before = p.baseMoveSlots[0].pp;
  assert.equal(p.deductPP(p.moveSlots[0].id), 1);
  assert.equal(p.baseMoveSlots[0].pp, before - 1);
});
check('disabled and PP used flags survive in the aliased slot record', () => {
  p.moveSlots[0].disabled = 'hidden';
  assert.equal(p.baseMoveSlots[0].disabled, 'hidden');
  assert.equal(p.baseMoveSlots[0].used, true);
});
check('active effect states allocate post-incremented effectOrder', () => {
  const n = b.effectOrder;
  assert.equal(b.initEffectState({ id: 'protect', target: p }).effectOrder, n);
  assert.equal(b.effectOrder, n + 1);
});
check('inactive Pokemon effect states do not allocate effectOrder', () => {
  const n = b.effectOrder;
  assert.equal(b.initEffectState({ id: 'protect', target: b.p1.pokemon[2] }).effectOrder, 0);
  assert.equal(b.effectOrder, n);
});
check('Side states allocate effectOrder', () => {
  const n = b.effectOrder;
  assert.equal(b.initEffectState({ id: 'reflect', target: b.p1 }).effectOrder, n);
  assert.equal(b.effectOrder, n + 1);
});
check('targetless field states do not allocate effectOrder', () => {
  const n = b.effectOrder;
  assert.equal(b.initEffectState({ id: 'trickroom' }).effectOrder, 0);
  assert.equal(b.effectOrder, n);
});
check('explicit effectOrder bypasses counter allocation', () => {
  const n = b.effectOrder;
  assert.equal(b.initEffectState({ id: 'protect', target: p }, 123).effectOrder, 123);
  assert.equal(b.effectOrder, n);
});
check('clearEffectState preserves target while deleting extra fields', () => {
  const state = { id: 'leftovers', effectOrder: 9, target: p, source: foe, duration: 5, started: true };
  b.clearEffectState(state);
  assert.deepEqual(state, { id: '', effectOrder: 0, target: p });
});
check('Condition subOrder depends on mutable state.target', () => {
  const effect = b.dex.conditions.get('trickroom');
  const order = target => b.resolvePriority({ effect, state: { target }, effectHolder: b.field }, 'onFieldResidual').subOrder;
  // Explicit residual SubOrder wins; use a callback without one to test defaults.
  const defaultOrder = target => b.resolvePriority({ effect, state: { target }, effectHolder: b.field }, 'onProbe').subOrder;
  assert.equal(order(undefined), 1);
  assert.equal(defaultOrder(undefined), 2); assert.equal(defaultOrder(b.field), 5);
});
check('slot Condition subOrder changes when target becomes Pokemon', () => {
  const effect = b.dex.conditions.get('wish');
  const order = target => b.resolvePriority({ effect, state: { target, isSlotCondition: true }, effectHolder: p }, 'onResidual').subOrder;
  assert.equal(order(b.p1), 3); assert.equal(order(p), 2);
});
check('attack records distinguish false from numeric zero', () => {
  p.attackedBy = [];
  p.gotAttacked('tackle', 0, foe); p.gotAttacked('tackle', false, foe);
  assert.equal(p.getLastAttackedBy().damageValue, false);
  assert.equal(p.getLastDamagedBy(true).damageValue, 0);
});
check('getLastDamagedBy(true) skips ally damage', () => {
  p.gotAttacked('tackle', 10, ally);
  assert.equal(p.getLastDamagedBy(true).source, foe);
});
check('repeated attacks retain an unbounded raw history', () => {
  for (let i = 0; i < 30; i++) p.gotAttacked('tackle', i, foe);
  assert.equal(p.attackedBy.length, 33); assert.equal(p.getLastAttackedBy().damage, 29);
});
check('clearVolatile retains persistent HP/status and attack count', () => {
  const hp = p.hp, status = p.status;
  p.timesAttacked = 300; p.activeTurns = 300; p.activeMoveActions = 300;
  p.boosts.atk = 3;
  p.clearVolatile();
  assert.equal(p.hp, hp); assert.equal(p.status, status); assert.equal(p.timesAttacked, 300);
  assert.equal(p.activeTurns, 300); assert.equal(p.activeMoveActions, 300);
});
check('clearVolatile resets action history, boosts and result sentinels', () => {
  assert.equal(p.boosts.atk, 0); assert.deepEqual(p.attackedBy, []);
  assert.equal(p.moveThisTurnResult, undefined); assert.equal(p.moveLastTurnResult, undefined);
  assert.equal(p.lastMove, null); assert.equal(p.moveSlots[0], p.baseMoveSlots[0]);
});
check('clearVolatile restores temporary Tera prohibition', () => {
  p.canTerastallize = false; p.clearVolatile(); assert.equal(p.canTerastallize, p.teraType);
  p.canTerastallize = null; p.clearVolatile(); assert.equal(p.canTerastallize, null);
});
check('both RNG seed formats round-trip the current state', () => {
  for (const seed of ['gen5,0001000200030004', 'sodium,0123456789abcdef0123456789abcdef']) {
    const rng = new PRNG(seed); rng.random();
    const clone = new PRNG(rng.getSeed()); assert.equal(rng.random(), clone.random());
  }
});
b.destroy();
console.log(`${checks} state-model probes passed`);
