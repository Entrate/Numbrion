'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const SRC = path.join(__dirname, '..', 'src');

function worker(status = 404) {
  const stored = {};
  let handler, calls = 0, elo = 1047.08;
  const context = { console, setTimeout, clearTimeout, AbortController, AbortSignal, importScripts: () => {},
    fetch: async (url, options) => {
      calls++;
      assert.equal(url, 'https://pokemonshowdown.com/users/alice.json');
      assert.equal(options.credentials, 'omit');
      return { ok: status === 200, status, json: async () => ({ username: 'Alice', registertime: 0, ratings: { gen9randomdoublesbattle: { elo, w: 1, l: 0 } } }) };
    },
    chrome: { storage: { local: { get: async () => stored, set: async (values) => Object.assign(stored, values) } },
      runtime: { onMessage: { addListener: (fn) => { handler = fn; } }, onInstalled: { addListener: () => {} } } },
  };
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(SRC, 'ladder.js'), 'utf8'), context);
  vm.runInContext(fs.readFileSync(path.join(SRC, 'background.js'), 'utf8'), context);
  return { stored, get calls() { return calls; }, change: (value) => { elo = value; },
    request: (force = false) => new Promise((resolve) => handler({ type: 'ladder', username: 'Alice', force }, {}, resolve)) };
}

test('valid ratings from an unregistered 404 profile are saved and cache refreshes are deduplicated', async () => {
  const w = worker();
  const first = await w.request();
  assert.equal(first.ok, true);
  assert.equal(first.data.ratings.gen9randomdoublesbattle.elo, 1047.08);
  await w.request();
  assert.equal(w.calls, 1);
  assert.equal(w.stored.ladderHistoryV1['alice:gen9randomdoublesbattle'].length, 1);
  w.change(1070.5);
  const updated = await w.request(true);
  assert.equal(updated.data.history.gen9randomdoublesbattle.length, 2);
});

test('concurrent tabs retain one shared snapshot rather than overwriting each other', async () => {
  const w = worker();
  const results = await Promise.all([w.request(), w.request()]);
  assert.ok(results.every((r) => r.ok));
  assert.equal(w.calls, 1);
  assert.equal(w.stored.ladderHistoryV1['alice:gen9randomdoublesbattle'].length, 1);
});

test('an unavailable official server does not fabricate a rating or history', async () => {
  const w = worker(503);
  const result = await w.request();
  assert.equal(result.ok, false);
  assert.match(result.error, /HTTP 503/);
  assert.equal(w.stored.ladderHistoryV1, undefined);
});
