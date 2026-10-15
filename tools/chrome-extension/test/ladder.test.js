'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const L = require('../src/ladder.js');

const rating = { elo: 1047.0811, wins: 1, losses: 0, ties: 0 };

test('official Elo parsing preserves decimals and ignores missing ratings', () => {
  assert.deepEqual(L.ratingsFrom({ ratings: { gen9randomdoublesbattle: { elo: '1047.0811', w: '1', l: 0 }, gen9ou: { elo: null }, bad: { elo: 'not a number' } } }), { gen9randomdoublesbattle: rating });
  assert.deepEqual(L.ratingsFrom({ ratings: {} }), {});
  assert.throws(() => L.ratingsFrom({ error: 'unavailable' }), /no ladder data/);
});

test('refreshes do not create duplicate snapshots, but a loss at the Elo floor is recorded', () => {
  const first = L.record({}, 'Alice', { gen9randombattle: rating }, 100);
  const same = L.record(first, 'ALICE', { gen9randombattle: rating }, 200);
  assert.equal(same['alice:gen9randombattle'].length, 1);
  const loss = L.record(same, 'Alice', { gen9randombattle: { ...rating, losses: 1 } }, 300);
  assert.deepEqual(loss['alice:gen9randombattle'].map((p) => p.at), [100, 300]);
  assert.equal(first['alice:gen9randombattle'].length, 1, 'input history is not mutated');
});

test('different users and formats keep separate histories', () => {
  let history = L.record({}, 'Alice', { gen9randombattle: rating, gen9randomdoublesbattle: { ...rating, elo: 1200 } }, 100);
  history = L.record(history, 'Bob', { gen9randombattle: { ...rating, elo: 1500 } }, 200);
  assert.equal(Object.keys(L.historyFor(history, 'Alice')).length, 2);
  assert.equal(L.historyFor(history, 'Bob').gen9randombattle[0].elo, 1500);
  assert.deepEqual(L.historyFor(history, 'NewPlayer'), {});
});

test('single and flat Elo histories produce finite chart coordinates in time order', () => {
  const single = L.chart([{ at: 100, elo: 1000 }]);
  assert.ok(Number.isFinite(single.points[0].x) && Number.isFinite(single.points[0].y));
  const flat = L.chart([{ at: 300, elo: 1000 }, { at: 100, elo: 1000 }, { at: 200, elo: 1000 }]);
  assert.deepEqual(flat.points.map((p) => p.at), [100, 200, 300]);
  assert.ok(flat.points.every((p) => p.y === flat.points[0].y));
  assert.deepEqual(flat.points.map((p) => p.x), [36, 231, 426]);
  assert.equal(L.chart([]), null);
});
