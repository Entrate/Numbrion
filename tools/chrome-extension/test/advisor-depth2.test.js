'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const fixture = require('./fixtures/probe-battle-p2.json');
const SRC = path.join(__dirname, '..', 'src');
const ROOM = 'battle-gen9randomdoublesbattle-1';
const request = JSON.parse(fixture.messages[2].split('|request|')[1].trim());
const flush = () => new Promise((resolve) => setImmediate(resolve));

// Exercise the real panel rendering with only the DOM operations it uses to build its body.
class Node {
  constructor(tag, text = '') { this.tag = tag; this.children = []; this.attrs = {}; this.listeners = {}; this.text = text; }
  set textContent(text) { this.text = text; this.children = []; }
  get textContent() { return this.text + this.children.map((child) => child.textContent).join(' '); }
  append(...children) { this.children.push(...children); }
  setAttribute(key, value) { this.attrs[key] = value; }
  addEventListener(type, fn) { this.listeners[type] = fn; }
}

async function harness(supportsTwoPhase = true, deferStatus = false) {
  let view, body, foot, pageListener, settingsListener, callbacks;
  const pending = [], frames = [], timers = new Map(), statusCallbacks = [];
  let timerId = 0;
  const stored = { settings: { showBotLog: false } };
  const window = { location: { origin: 'http://localhost' },
    addEventListener(type, fn) { if (type === 'message') pageListener = fn; }, removeEventListener() {}, postMessage() {} };
  const context = { console, window, Node, Date: class extends Date { static now() { return 1760000000000; } },
    document: { querySelector: () => null, createElement: (tag) => new Node(tag), createTextNode: (text) => new Node('#text', text) },
    setTimeout(fn, delay) { timers.set(++timerId, { fn, delay }); return timerId; },
    clearTimeout: (id) => timers.delete(id), setInterval: () => 1, clearInterval() {},
    requestAnimationFrame: (fn) => frames.push(fn),
    NumbrionLadderPanel: class { render() {} destroy() {} },
    chrome: { runtime: { id: 'test', sendMessage(message, callback) {
      if (message.type === 'status') {
        const response = { ok: true, data: { advisor: 'SearchBot', ...(supportsTwoPhase ? { two_phase_advise: true } : {}), bots: [{ name: 'SearchBot', search: {} }] } };
        if (deferStatus) statusCallbacks.push(() => callback(response));
        else callback(response);
      }
      else if (message.type === 'advise' || message.type === 'adviseResult') pending.push({ message, callback });
      else callback({ ok: true, data: { entries: [] } });
    } }, storage: { local: {
      get: async () => stored,
      set: async (values) => {
        Object.assign(stored, values);
        settingsListener(Object.fromEntries(Object.entries(values).map(([key, newValue]) => [key, { newValue }])), 'local');
      },
    }, onChanged: { addListener: (fn) => { settingsListener = fn; } } } },
  };
  vm.createContext(context);
  for (const file of ['settings.js', 'advisor-status.js', 'protocol.js', 'ladder.js', 'panel.js']) {
    vm.runInContext(fs.readFileSync(path.join(SRC, file), 'utf8'), context);
  }
  const RealPanel = context.NumbrionPanel;
  context.NumbrionPanel = class extends RealPanel {
    constructor(cb) { super(cb); callbacks = cb; }
    render(next) {
      view = next;
      if (next.visible) { body = this.renderBody(next); foot = this.renderFoot(next); }
    }
    destroy() {}
  };
  vm.runInContext(fs.readFileSync(path.join(SRC, 'content.js'), 'utf8'), context);
  const render = () => { while (frames.length) frames.shift()(); };
  const post = (payload) => pageListener({ source: window, origin: window.location.origin,
    data: { __numbrion: 'numbrion-advisor-v1', dir: 'page->ext', ...payload } });
  await flush();
  for (const data of fixture.messages.slice(0, 3)) post({ type: 'msg', data });
  post({ type: 'focus', roomid: ROOM });
  render();
  return {
    pending, callbacks, post, render, context,
    get view() { return view; }, get foot() { return foot; },
    get text() { return body.map((node) => node.textContent).join(' '); },
    async respond(call, data) { call.callback({ ok: true, data }); await flush(); render(); },
    async fail(call, response) { call.callback(response); await flush(); render(); },
    async finishStatus() { while (statusCallbacks.length) statusCallbacks.shift()(); await flush(); render(); },
    settle() { for (const [id, timer] of [...timers]) if (timer.delay === 150) { timers.delete(id); timer.fn(); } render(); },
    poll() {
      const next = [...timers].find(([, timer]) => timer.delay > 19000 && timer.delay <= 20000);
      assert.ok(next, 'pending responses schedule no more than one poll per 20 s');
      timers.delete(next[0]); next[1].fn();
    },
    get pollsScheduled() { return [...timers.values()].filter((timer) => timer.delay > 19000 && timer.delay <= 20000).length; },
    close() { context.__numbrionAdvisorContent.shutdown(); },
  };
}

const decision = (choice = 'move 1 +1, move 1 +1', depth2) => ({ choice, analysis: {
  source: 'search', time_ms: 480, slots: [{ pokemon: 'Ally', text: choice }],
  ...(depth2 ? { depth2 } : {}),
} });
const depth2 = { rounds: 2, changed: true, chosen: 'Ally: Protect', skipped: null, total_ms: 1823,
  candidates: [
    { choice: 'Ally: Attack', x: 0.7, q: 0.12, q2: [0.08, null] },
    { choice: 'Ally: Protect', x: 0.3, q: 0.1, q2: [0.2, 0.3] },
  ] };

test('a final depth2 result renders each round beside layer-1 x and q, including the selected candidate', async () => {
  const h = await harness();
  try {
    await h.respond(h.pending.shift(), { room: ROOM, rqid: request.rqid, decision: decision('move 2, move 2', depth2) });
    assert.match(h.text, /Changed by layer 2/);
    assert.match(h.text, /layer 2 1.82 s/);
    assert.match(h.text, /L1 x 70%.*L1 q \+0.12.*Q2 r1 \+0.08.*Q2 r2 —/);
    assert.match(h.text, /✓ Ally: Protect.*L1 x 30%.*Q2 r2 \+0.30/);
    assert.equal(h.view.advice.choice, 'move 2, move 2');
    assert.ok(h.view.play.enabled);
  } finally { h.close(); }
});

test('the layer-2 toggle defaults on, saves through the panel, and hides values while preserving the final pick', async () => {
  const h = await harness();
  try {
    assert.equal(h.context.NumbrionSettings.normalize(null).showDepth2, true);
    await h.respond(h.pending.shift(), { rqid: request.rqid, decision: decision('move 2, move 2', depth2) });
    const toggle = h.foot.find((node) => node.textContent.includes('Layer-2 values')).children[0];
    assert.equal(toggle.checked, true);
    toggle.checked = false;
    toggle.listeners.change();
    await flush(); h.render();
    assert.equal(h.view.settings.showDepth2, false);
    assert.doesNotMatch(h.text, /L1 x|Q2 r/);
    assert.match(h.text, /Changed by layer 2/);
    assert.equal(h.view.advice.choice, 'move 2, move 2');
    assert.equal(h.pending.length, 0, 'display-only changes do not start a search');
    h.callbacks.onToggle('showDepth2', true);
    await flush(); h.render();
    assert.match(h.text, /Q2 r2 \+0.30/);
  } finally { h.close(); }
});

test('an old single-phase server remains playable and never starts a follow-up', async () => {
  const h = await harness(false);
  try {
    const initial = h.pending.shift();
    assert.equal(initial.message.body.two_phase, undefined);
    await h.respond(initial, { rqid: request.rqid, decision: decision() });
    assert.match(h.text, /1-turn search/);
    assert.doesNotMatch(h.text, /deepening|layer 2 unavailable/);
    assert.equal(h.pending.length, 0);
    assert.ok(h.view.play.enabled);
  } finally { h.close(); }
});

const phase1 = (id = 'first', rqid = request.rqid) => ({ room: ROOM, rqid, phase: 1, pending: true, advise_id: id, decision: decision() });
const phase2 = (id = 'first', rqid = request.rqid) => ({ room: ROOM, rqid, phase: 2, pending: false, advise_id: id, decision: decision('move 2, move 2', depth2) });

test('phase 1 is playable immediately while phase 2 deepens, then the final pick updates in place', async () => {
  const h = await harness();
  try {
    const initial = h.pending.shift();
    assert.equal(initial.message.body.two_phase, true);
    await h.respond(initial, phase1());
    assert.equal(h.view.advice.choice, decision().choice);
    assert.ok(h.view.play.enabled);
    assert.match(h.text, /1-turn search · deepening…/);
    const followup = h.pending.shift();
    assert.equal(followup.message.type, 'adviseResult');
    assert.equal(followup.message.adviseId, 'first');
    await h.respond(followup, phase2());
    assert.equal(h.view.advice.choice, 'move 2, move 2');
    assert.match(h.text, /Changed by layer 2/);
    assert.doesNotMatch(h.text, /deepening/);
    assert.match(h.text, /Q2 r2 \+0.30/);
    assert.equal(h.pending.length, 0);
  } finally { h.close(); }
});

test('advice waits for capability discovery when the room is focused before status arrives', async () => {
  const h = await harness(true, true);
  try {
    assert.equal(h.pending.length, 0);
    await h.finishStatus();
    assert.equal(h.pending.length, 1);
    assert.equal(h.pending[0].message.body.two_phase, true);
  } finally { h.close(); }
});

test('turning details off while phase 2 runs still updates the final pick', async () => {
  const h = await harness();
  try {
    await h.respond(h.pending.shift(), phase1());
    const followup = h.pending.shift();
    h.callbacks.onToggle('showDepth2', false);
    await flush(); h.render();
    assert.match(h.text, /deepening/);
    await h.respond(followup, phase2());
    assert.equal(h.view.advice.choice, 'move 2, move 2');
    assert.match(h.text, /Changed by layer 2/);
    assert.doesNotMatch(h.text, /Q2 r|L1 x/);
    assert.ok(h.view.play.enabled);
  } finally { h.close(); }
});

test('an old phase 2 cannot overwrite a newer request, including while the new phase 2 is pending', async () => {
  const h = await harness();
  try {
    await h.respond(h.pending.shift(), phase1());
    const old = h.pending.shift();
    h.post({ type: 'msg', data: '>' + ROOM + '\n|request|' + JSON.stringify({ ...request, rqid: request.rqid + 1 }) });
    h.settle();
    await h.respond(h.pending.shift(), phase1('new', request.rqid + 1));
    const current = h.pending.shift();
    await h.respond(old, phase2());
    assert.equal(h.view.advice.choice, decision().choice);
    assert.match(h.text, /deepening/);
    assert.doesNotMatch(h.text, /Changed by layer 2/);
    await h.respond(current, phase2('new', request.rqid + 1));
    assert.equal(h.view.advice.choice, 'move 2, move 2');
  } finally { h.close(); }
});

test('a forced refresh for the same rqid also ignores the previous phase 2', async () => {
  const h = await harness();
  try {
    await h.respond(h.pending.shift(), phase1());
    const old = h.pending.shift();
    h.callbacks.onAdviseNow();
    await h.respond(h.pending.shift(), phase1('refresh'));
    await h.respond(old, phase2());
    assert.equal(h.view.advice.choice, decision().choice);
    assert.match(h.text, /deepening/);
  } finally { h.close(); }
});

test('closing and reopening the same room/rqid cannot revive the old phase 2', async () => {
  const h = await harness();
  try {
    await h.respond(h.pending.shift(), phase1());
    const old = h.pending.shift();
    h.post({ type: 'msg', data: '>' + ROOM + '\n|deinit' });
    for (const data of fixture.messages.slice(0, 3)) h.post({ type: 'msg', data });
    h.settle();
    await h.respond(h.pending.shift(), phase1('reopened'));
    await h.respond(old, phase2());
    assert.equal(h.view.advice.choice, decision().choice);
    assert.match(h.text, /deepening/);
  } finally { h.close(); }
});

test('phase-2 timeout and API errors preserve playable phase-1 advice without an error state', async (t) => {
  for (const response of [
    { ok: false, error: 'timed out after 25 s' },
    { ok: false, error: 'HTTP 500' },
    { ok: false, error: 'unknown or expired advise id' },
    { ok: false, error: 'superseded by a newer request', data: { superseded: true } },
    { ok: false, offline: true, error: 'API offline' },
  ]) await t.test(response.error, async () => {
    const h = await harness();
    try {
      await h.respond(h.pending.shift(), phase1());
      await h.fail(h.pending.shift(), response);
      assert.equal(h.view.advice.choice, decision().choice);
      assert.ok(h.view.play.enabled);
      assert.equal(h.view.status.kind, 'ok');
      assert.equal(h.view.search.kind, 'ok');
      assert.doesNotMatch(h.text, /deepening/);
      assert.match(h.text, /keeping 1-turn advice/);
      assert.equal(h.pending.length, 0);
    } finally { h.close(); }
  });
});

test('user actions and room closure invalidate phase 2 before it can replace the visible pick', async (t) => {
  for (const action of ['choose', 'sentchoice', 'play', 'close', 'disable']) await t.test(action, async () => {
    const h = await harness();
    try {
      await h.respond(h.pending.shift(), phase1());
      const old = h.pending.shift();
      if (action === 'choose') h.post({ type: 'sent', data: ROOM + '|/choose move 3, move 3|' + request.rqid });
      if (action === 'sentchoice') h.post({ type: 'msg', data: '>' + ROOM + '\n|sentchoice|move 3, move 3' });
      if (action === 'play') h.callbacks.onPlay();
      if (action === 'close') h.post({ type: 'msg', data: '>' + ROOM + '\n|deinit' });
      if (action === 'disable') { h.callbacks.onToggle('enabled', false); await flush(); }
      await h.respond(old, phase2());
      if (action === 'close' || action === 'disable') assert.equal(h.view.visible, false);
      else {
        assert.equal(h.view.advice.choice, decision().choice);
        assert.doesNotMatch(h.text, /Changed by layer 2|deepening/);
      }
    } finally { h.close(); }
  });
});

test('pending long polls keep the first pick, respect the 20 s interval, and stop after six waits', async () => {
  const h = await harness();
  try {
    await h.respond(h.pending.shift(), phase1());
    const pending = { ...phase1() }; delete pending.decision;
    for (let i = 0; i < 6; i++) {
      await h.respond(h.pending.shift(), pending);
      assert.equal(h.view.advice.choice, decision().choice);
      assert.equal(h.pending.length, 0, 'an early pending response must not trigger an immediate fetch');
      if (i < 5) h.poll();
    }
    assert.match(h.text, /Layer 2 timed out/);
    assert.equal(h.pollsScheduled, 0);
  } finally { h.close(); }
});

test('a scheduled follow-up is cancelled when the room closes', async () => {
  const h = await harness();
  try {
    await h.respond(h.pending.shift(), phase1());
    const pending = { ...phase1() }; delete pending.decision;
    await h.respond(h.pending.shift(), pending);
    assert.equal(h.pollsScheduled, 1);
    h.post({ type: 'msg', data: '>' + ROOM + '\n|deinit' });
    assert.equal(h.pollsScheduled, 0);
    assert.equal(h.pending.length, 0);
  } finally { h.close(); }
});

test('malformed or mismatched follow-up identities never replace the first pick', async (t) => {
  for (const patch of [{ room: 'battle-other' }, { rqid: request.rqid + 1 }, { advise_id: 'other' }, { phase: 1, pending: false }, { decision: null }]) {
    await t.test(JSON.stringify(patch), async () => {
      const h = await harness();
      try {
        await h.respond(h.pending.shift(), phase1());
        await h.respond(h.pending.shift(), { ...phase2(), ...patch });
        assert.equal(h.view.advice.choice, decision().choice);
        assert.match(h.text, /keeping 1-turn advice/);
      } finally { h.close(); }
    });
  }
});

test('completed rounds confirm the pick; skipped deepening never claims confirmation', async () => {
  const h = await harness();
  try {
    await h.respond(h.pending.shift(), { rqid: request.rqid, decision: decision(undefined, { ...depth2, changed: false }) });
    assert.match(h.text, /Confirmed by layer 2/);
    h.callbacks.onAdviseNow();
    await h.respond(h.pending.shift(), { rqid: request.rqid, decision: decision(undefined,
      { rounds: 0, changed: false, skipped: 'the budget planned no second layer' }) });
    assert.match(h.text, /Layer 2 skipped/);
    assert.match(h.text, /the budget planned no second layer/);
    assert.doesNotMatch(h.text, /Confirmed/);
  } finally { h.close(); }
});

test('an initial answer received after a choice was sent is ignored, even after undo', async () => {
  const h = await harness();
  try {
    const first = h.pending.shift();
    h.post({ type: 'sent', data: ROOM + '|/choose move 2, move 2|' + request.rqid });
    h.post({ type: 'sent', data: ROOM + '|/undo' });
    await h.respond(first, { rqid: request.rqid, decision: decision() });
    assert.equal(h.view.advice, null);
    assert.equal(h.view.search.text, 'Search stopped');
    assert.equal(h.pending.length, 0);
  } finally { h.close(); }
});
