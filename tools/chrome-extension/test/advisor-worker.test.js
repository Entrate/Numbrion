'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const SRC = path.join(__dirname, '..', 'src');
const flush = () => new Promise((resolve) => setImmediate(resolve));

function worker() {
  let handler, timerId = 0;
  const calls = [], timers = new Map();
  const context = { console, AbortController, importScripts() {},
    setTimeout(fn, delay) { timers.set(++timerId, { fn, delay }); return timerId; },
    clearTimeout: (id) => timers.delete(id),
    fetch(url, options) {
      return new Promise((resolve, reject) => {
        calls.push({ url, options, resolve, reject });
        options.signal.addEventListener('abort', () => reject(Object.assign(new Error('aborted'), { name: 'AbortError' })));
      });
    },
    chrome: { storage: { local: { get: async () => ({ settings: { budgetMs: 800, bot: 'TwoPly' } }) } },
      runtime: { onMessage: { addListener: (fn) => { handler = fn; } }, onInstalled: { addListener() {} } } },
  };
  vm.createContext(context);
  for (const file of ['settings.js', 'background.js']) vm.runInContext(fs.readFileSync(path.join(SRC, file), 'utf8'), context);
  return { calls, timers,
    request: (message) => new Promise((resolve) => assert.equal(handler(message, {}, resolve), true)),
    answer(call, status, data) { call.resolve({ ok: status < 400, status, json: async () => data }); },
  };
}

test('the worker preserves the two_phase opt-in while applying the selected bot and budget', async () => {
  const w = worker();
  const result = w.request({ type: 'advise', body: { room: 'battle-test', two_phase: true, request: '{}' } });
  await flush();
  const call = w.calls.shift();
  assert.equal(call.url, 'http://127.0.0.1:8765/api/advise');
  assert.equal(call.options.method, 'POST');
  assert.deepEqual(JSON.parse(call.options.body), { room: 'battle-test', two_phase: true, request: '{}', budget_ms: 800, bot: 'TwoPly' });
  w.answer(call, 200, { ok: true, phase: 1, pending: true, advise_id: 'id' });
  assert.equal((await result).data.pending, true);
  assert.equal(w.timers.size, 0);
});

test('the follow-up uses the documented 20 s GET and percent-encodes the id', async () => {
  const w = worker();
  const result = w.request({ type: 'adviseResult', adviseId: 'id/?' });
  await flush();
  const call = w.calls.shift();
  assert.equal(call.url, 'http://127.0.0.1:8765/api/advise/id%2F%3F?timeout=20');
  assert.equal(call.options.body, undefined);
  assert.equal(call.options.cache, 'no-store');
  assert.equal([...w.timers.values()][0].delay, 25000);
  w.answer(call, 200, { ok: true, phase: 2, pending: false, advise_id: 'id/?' });
  assert.equal((await result).data.phase, 2);
  assert.equal(w.timers.size, 0);
});

test('the worker aborts a stalled phase-2 GET after 25 s and returns a handled timeout', async () => {
  const w = worker();
  const result = w.request({ type: 'adviseResult', adviseId: 'id' });
  await flush();
  [...w.timers.values()][0].fn();
  const response = await result;
  assert.equal(response.ok, false);
  assert.equal(response.offline, false);
  assert.match(response.error, /timed out after 25 s/);
  assert.equal(w.calls[0].options.signal.aborted, true);
  assert.equal(w.timers.size, 0);
});

test('phase-2 HTTP, non-JSON, and network errors return handled failures', async (t) => {
  for (const status of [404, 409, 500]) await t.test('HTTP ' + status, async () => {
    const w = worker();
    const result = w.request({ type: 'adviseResult', adviseId: 'id' });
    await flush();
    w.answer(w.calls[0], status, { ok: false, error: 'server error', ...(status === 409 ? { superseded: true } : {}) });
    const response = await result;
    assert.equal(response.ok, false);
    assert.equal(response.error, 'server error');
    if (status === 409) assert.equal(response.data.superseded, true);
  });
  const w = worker();
  const result = w.request({ type: 'adviseResult', adviseId: 'id' });
  await flush();
  w.calls[0].resolve({ ok: false, status: 502, json: async () => { throw new Error('not JSON'); } });
  assert.match((await result).error, /HTTP 502 \(not JSON\)/);
  const offline = w.request({ type: 'adviseResult', adviseId: 'id' });
  await flush();
  w.calls[1].reject(new Error('connection refused'));
  assert.equal((await offline).offline, true);
});
