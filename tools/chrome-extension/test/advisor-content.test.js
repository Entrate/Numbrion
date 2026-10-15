'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const fixture = require('./fixtures/probe-battle-p2.json');
const SRC = path.join(__dirname, '..', 'src');
const flush = () => new Promise((resolve) => setImmediate(resolve));

test('the battle panel refreshes advice and model identity when the selected advisor changes', async () => {
  let view, settingsListener, pageListener, panelCallbacks;
  const pending = [], frames = [];
  const stored = { settings: { showBotLog: false } };
  const status = { advisor: 'SearchBot', bots: [
    { name: 'SearchBot', checkpoint: '/models/teacher/m240.pt', search: {} },
    { name: 'PolicyBot', checkpoint: '/models/distill/m535.pt', search: null },
  ] };
  class Panel {
    constructor(callbacks) { panelCallbacks = callbacks; }
    render(next) { view = next; }
    destroy() {}
  }
  const window = { location: { origin: 'http://localhost' },
    addEventListener(type, fn) { if (type === 'message') pageListener = fn; }, removeEventListener() {}, postMessage() {} };
  const context = { console, window, document: { querySelector: () => null },
    setTimeout: () => 1, clearTimeout() {}, setInterval: () => 1, clearInterval() {},
    requestAnimationFrame: (fn) => frames.push(fn), NumbrionPanel: Panel,
    NumbrionLadderPanel: class { render() {} destroy() {} },
    chrome: { runtime: { id: 'test', sendMessage(message, callback) {
      if (message.type === 'status') callback({ ok: true, data: status });
      else if (message.type === 'advise') pending.push({ callback, bot: stored.settings.bot || status.advisor });
      else callback({ ok: true, data: { entries: [] } });
    } }, storage: { local: { get: async () => stored }, onChanged: { addListener: (fn) => { settingsListener = fn; } } } },
  };
  vm.createContext(context);
  for (const file of ['settings.js', 'advisor-status.js', 'protocol.js', 'ladder.js', 'content.js']) {
    vm.runInContext(fs.readFileSync(path.join(SRC, file), 'utf8'), context);
  }
  const render = () => { while (frames.length) frames.shift()(); };
  const post = (payload) => pageListener({ source: window, origin: window.location.origin,
    data: { __numbrion: 'numbrion-advisor-v1', dir: 'page->ext', ...payload } });
  try {
    await flush();
    for (const data of fixture.messages.slice(0, 3)) post({ type: 'msg', data });
    post({ type: 'focus', roomid: 'battle-gen9randomdoublesbattle-1' });
    render();
    assert.equal(view.model.model, 'teacher / m240.pt');
    assert.equal(view.search.text, 'Search running');
    const first = pending.shift();
    // Read the fixture's current rqid rather than relying on the room's request sequence.
    const requestText = fixture.messages[2].split('|request|')[1].trim();
    const rqid = JSON.parse(requestText).rqid;
    const respond = async (request, source, note = '') => {
      request.callback({ ok: true, data: { bot: request.bot, rqid, decision: { choice: 'move 1 +1, move 1 +1', analysis: { source, note } } } });
      await flush(); render();
    };
    await respond(first, 'search');
    assert.equal(view.search.text, 'Search used');
    assert.ok(view.advice);

    stored.settings = { ...stored.settings, bot: 'PolicyBot' };
    settingsListener({ settings: { newValue: stored.settings } }, 'local');
    await flush(); render();
    assert.equal(view.model.model, 'distill / m535.pt');
    assert.equal(view.search.text, 'Evaluating policy');
    assert.equal(view.advice, null);
    assert.equal(pending.length, 1);
    await respond(pending.shift(), 'policy', 'search off');
    assert.equal(view.search.text, 'Search off');

    stored.settings = { ...stored.settings, bot: 'SearchBot' };
    settingsListener({ settings: { newValue: stored.settings } }, 'local');
    await flush(); render();
    const superseded = pending.shift();
    panelCallbacks.onAdviseNow();
    const current = pending.shift();
    await respond(superseded, 'search');
    assert.equal(view.advice, null);
    await respond(current, 'policy', 'no world: cannot rebuild the team');
    assert.equal(view.search.kind, 'error');
    assert.match(view.search.detail, /cannot rebuild the team/);
    assert.ok(view.play.enabled, 'policy fallback stays playable');
  } finally {
    context.__numbrionAdvisorContent.shutdown();
  }
});
