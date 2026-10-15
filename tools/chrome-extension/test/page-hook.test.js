'use strict';
// The MAIN-world hook in a fake window: WebSocket wrapping, SockJS decoding, de-duplication against the client's
// receive hook, replay on hello, backfill from client state, and "play" through the classic client.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const SRC = path.join(__dirname, '..', 'src');
const TAG = 'numbrion-advisor-v1';
const ROOM = 'battle-gen9randomdoublesbattle-5';

class FakeWebSocket {
  constructor(url) {
    this.url = url;
    this.readyState = 1;
    this.listeners = {};
    this.sent = [];
    this.onmessage = null;
  }
  addEventListener(type, fn) {
    (this.listeners[type] = this.listeners[type] || []).push(fn);
  }
  send(data) {
    this.sent.push(data);
  }
  /** Server -> page: our listener first, then the page's onmessage, like a browser. */
  deliver(data) {
    for (const fn of this.listeners.message || []) fn({ data });
    if (this.onmessage) this.onmessage({ data });
  }
}

function makePage() {
  const posted = [];
  const listeners = [];
  const intervals = [];
  const origin = 'http://localhost.psim.us';
  const context = {
    console,
    WebSocket: FakeWebSocket,
    location: { origin, pathname: '/' },
    setInterval: (fn) => intervals.push(fn),
    clearInterval: () => {},
    setTimeout,
    postMessage: (payload) => posted.push(JSON.parse(JSON.stringify(payload))),
    addEventListener: (type, fn) => listeners.push(fn),
  };
  context.window = context;
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(SRC, 'page-hook.js'), 'utf8'), context, { filename: 'page-hook.js' });
  const page = {
    context,
    posted,
    tick: () => intervals.forEach((fn) => fn()),
    fromExtension: (data) => {
      const source = vm.runInContext('window', context); // the context's global, not the sandbox object
      for (const fn of listeners) fn({ source, origin, data: Object.assign({ __numbrion: TAG, dir: 'ext->page' }, data) });
    },
    messages: () => posted.filter((p) => p.type === 'msg').map((p) => p.data),
    run: (code) => vm.runInContext(code, context),
  };
  return page;
}

const INIT = '>' + ROOM + '\n|init|battle\n|title|Alice vs. Bot\n|player|p1|Alice|1|\n|player|p2|Bot|1|\n|start\n|switch|p1a: Amoonguss|Amoonguss, L88|100/100\n|turn|1';
const REQ = { active: [{ moves: [{ move: 'Spore', id: 'spore', target: 'normal' }] }], side: { id: 'p1', name: 'Alice', pokemon: [] }, rqid: 7 };

test('the hook wraps WebSocket without changing it and relays decoded messages after hello', () => {
  const page = makePage();
  const ws = page.run('new WebSocket("ws://localhost:8000/showdown/websocket")');
  assert.ok(ws instanceof FakeWebSocket);
  assert.equal(page.run('WebSocket.name'), 'FakeWebSocket');

  ws.deliver('|updateuser| Alice|1|101|{}');
  ws.deliver(INIT);
  ws.deliver('>' + ROOM + '\n|request|' + JSON.stringify(REQ));
  ws.deliver('>lobby\n|c|~|hi');
  assert.deepEqual(page.messages(), [], 'nothing is posted before a content script says hello');

  page.fromExtension({ type: 'hello', session: 's1' });
  const replay = page.posted.filter((p) => p.type === 'msg');
  assert.deepEqual(replay.map((p) => p.data), ['|updateuser| Alice|1|101|{}', INIT, '>' + ROOM + '\n|request|' + JSON.stringify(REQ)]);
  assert.ok(replay.every((p) => p.replayFor === 's1'));
  assert.ok(page.posted.some((p) => p.type === 'hello-ack' && p.session === 's1'));

  // A repeated hello from the same session does not replay again.
  page.posted.length = 0;
  page.fromExtension({ type: 'hello', session: 's1' });
  assert.deepEqual(page.messages(), []);

  ws.deliver('>' + ROOM + '\n|\n|move|p1a: Amoonguss|Spore|p2a: X');
  assert.deepEqual(page.messages(), ['>' + ROOM + '\n|\n|move|p1a: Amoonguss|Spore|p2a: X']);

  // Outgoing choices are observed.
  ws.send(ROOM + '|/choose move 1 1|7');
  assert.deepEqual(ws.sent, [ROOM + '|/choose move 1 1|7']);
  assert.ok(page.posted.some((p) => p.type === 'sent' && p.data === ROOM + '|/choose move 1 1|7'));
});

test('SockJS frames are decoded and messages seen on the socket are not repeated by the client hook', () => {
  const page = makePage();
  const received = [];
  page.context.app = { rooms: {}, receive: (data) => received.push(data) };
  page.tick(); // installs the receive hook
  page.fromExtension({ type: 'hello', session: 's1' });
  const ws = page.run('new WebSocket("https://sim3.psim.us/showdown/417/x8k2m1qz/websocket")');
  ws.onmessage = (event) => {
    // SockJS: one frame, several messages, each handed to the client.
    if (event.data[0] === 'a') for (const msg of JSON.parse(event.data.slice(1))) page.context.app.receive(msg);
  };
  ws.deliver('o');
  ws.deliver('a' + JSON.stringify(['|challstr|4|abc', INIT]));
  ws.deliver('h');
  assert.deepEqual(page.messages(), ['|challstr|4|abc', INIT]);
  assert.deepEqual(received, ['|challstr|4|abc', INIT], 'the client still gets everything');

  // Messages that reach the client another way (e.g. Preact's connection worker) are captured by the hook.
  page.context.app.receive('>' + ROOM + '\n|request|' + JSON.stringify(REQ));
  assert.equal(page.messages().length, 3);

  // Sending through the SockJS socket uses SockJS framing.
  page.context.app = undefined;
  const result = sendPlay(page, ROOM, 'move 1 1', 7);
  assert.equal(result.ok, true);
  assert.equal(result.method, 'socket');
  assert.deepEqual(ws.sent, [JSON.stringify([ROOM + '|/choose move 1 1|7'])]);
});

function sendPlay(page, roomid, choice, rqid, extra) {
  page.posted.length = 0;
  page.fromExtension(Object.assign({ type: 'play', id: 1, roomid, choice, rqid }, extra));
  const reply = page.posted.find((p) => p.type === 'play-result');
  assert.ok(reply, 'play-result posted');
  return reply.result;
}

function preferencePage(named = false) {
  const page = makePage();
  const renamed = [];
  const button = { outerHTML: '' };
  const team = { outerHTML: '' };
  const form = { querySelector: (selector) => selector.includes('name="format"') ? button : team };
  const main = { curFormat: 'gen9randombattle', curTeamIndex: 0, el: { querySelector: () => form },
    renderFormats: (format) => '<button>' + format + '</button>', renderTeams: (format) => '<button>' + format + ' team</button>' };
  const user = { loaded: true, challstr: '4|test', get: (key) => key === 'named' ? named : key === 'name' ? 'Alice' : null, rename: (name) => renamed.push(name) };
  page.context.app = { rooms: { '': main }, user, curRoom: { id: '' }, send: () => {}, receive: () => {} };
  page.context.BattleFormats = { gen9randomdoublesbattle: { name: '[Gen 9] Random Doubles Battle' }, gen9randombattle: { name: '[Gen 9] Random Battle' } };
  return { page, renamed, button, team, main };
}

test('refresh restores doubles and the remembered username through the normal login flow', () => {
  const { page, renamed, button, main } = preferencePage();
  page.fromExtension({ type: 'preferences', username: 'Alice', format: 'gen9randomdoublesbattle' });
  page.tick();
  page.tick();
  assert.equal(main.curFormat, 'gen9randomdoublesbattle');
  assert.match(button.outerHTML, /gen9randomdoublesbattle/);
  assert.deepEqual(renamed, ['Alice'], 'attempted once, never repeatedly asks for authentication');
  page.fromExtension({ type: 'hello', session: 'prefs' });
  main.curFormat = 'gen9randombattle';
  page.tick();
  assert.equal(page.posted.filter((p) => p.type === 'profile').at(-1).format, 'gen9randombattle', 'a later manual choice is remembered, not overridden');
});

test('existing login and explicit logout are respected', () => {
  const signedIn = preferencePage(true);
  signedIn.page.fromExtension({ type: 'preferences', username: 'Bob', format: 'gen9randomdoublesbattle' });
  assert.deepEqual(signedIn.renamed, [], 'does not replace a signed-in account');
  const loggedOut = preferencePage();
  loggedOut.page.tick();
  loggedOut.page.context.app.send('/logout');
  loggedOut.page.fromExtension({ type: 'preferences', username: 'Alice', format: 'gen9randomdoublesbattle' });
  assert.deepEqual(loggedOut.renamed, [], 'does not log back in after an explicit logout');
  assert.ok(loggedOut.page.posted.some((p) => p.type === 'profile' && p.loggedOut));
});

test('play goes through the classic client and switches its controls to waiting', () => {
  const page = makePage();
  const sent = [];
  let updated = 0;
  const room = {
    battle: { stepQueue: ['|player|p1|Alice|1|', '|start', '|turn|1'] },
    request: Object.assign({ requestType: 'move' }, REQ),
    title: 'Alice vs. Bot',
    send: (text) => page.context.app.send(text, ROOM), // like the classic Room.send
    closeNotification: () => {},
    updateControls() {
      updated++;
    },
  };
  page.context.app = { rooms: { [ROOM]: room }, receive: () => {}, send: (data, roomid) => sent.push(roomid + '|' + data), curRoom: { id: ROOM } };
  page.tick(); // installs the receive and send hooks
  page.fromExtension({ type: 'hello', session: 's1' });

  const result = sendPlay(page, ROOM, 'move 1 +2 terastallize, move 3', 7, { classicChoices: ['move 1 terastallize 2', 'move 3'] });
  assert.deepEqual(result, { ok: true, method: 'classic client' });
  assert.deepEqual(sent, [ROOM + '|/choose move 1 +2 terastallize, move 3|7']);
  assert.deepEqual(JSON.parse(JSON.stringify(room.choice)), { choices: ['move 1 terastallize 2', 'move 3'], switchFlags: {}, switchOutFlags: {}, waiting: true });
  assert.equal(updated, 1);
  // The choice reaches the content script through the outgoing capture (it is a choice like any other).
  assert.deepEqual(page.posted.filter((p) => p.type === 'sent').map((p) => [p.data, p.source]), [[ROOM + '|/choose move 1 +2 terastallize, move 3|7', 'client']]);

  // Advice for an older request is refused.
  const stale = sendPlay(page, ROOM, 'move 1 1', 6);
  assert.equal(stale.ok, false);
  assert.match(stale.error, /request 7/);
  assert.equal(sent.length, 1);
});

test('battle rooms the hook never saw are backfilled from the client state', () => {
  const page = makePage();
  const room = {
    battle: { stepQueue: ['|player|p1|Alice|1|', '|player|p2|Bot|1|', '|start', '|turn|1'] },
    request: Object.assign({ requestType: 'move' }, REQ),
    title: 'Alice vs. Bot',
    send: () => {},
  };
  page.context.app = { rooms: { [ROOM]: room, lobby: {} }, receive: () => {}, curRoom: { id: ROOM } };
  page.fromExtension({ type: 'hello', session: 's1' });
  const msgs = page.posted.filter((p) => p.type === 'msg');
  assert.equal(msgs.length, 2);
  assert.ok(msgs.every((p) => p.backfill));
  assert.equal(msgs[0].data, '>' + ROOM + '\n|init|battle\n|title|Alice vs. Bot\n|player|p1|Alice|1|\n|player|p2|Bot|1|\n|start\n|turn|1');
  assert.equal(msgs[1].data, '>' + ROOM + '\n|request|' + JSON.stringify(REQ), 'requestType (added by the client) is dropped');
  assert.ok(page.posted.some((p) => p.type === 'focus' && p.roomid === ROOM));

  // Not twice.
  page.posted.length = 0;
  page.tick();
  assert.deepEqual(page.messages(), []);
});

test('outgoing battle commands are captured once, from the client or the socket, and replayed in order', () => {
  const page = makePage();
  const ws = page.run('new WebSocket("ws://localhost:8000/showdown/websocket")');
  // A classic-like client: app.send(data, room) puts "room|data" on the socket.
  page.context.app = {
    rooms: {},
    receive: () => {},
    send(data, room) {
      ws.send((room && room !== true ? room + '|' : room === true ? '' : '|') + data);
    },
  };
  page.tick();
  page.fromExtension({ type: 'hello', session: 's1' });
  ws.deliver(INIT);
  ws.deliver('>' + ROOM + '\n|request|' + JSON.stringify(REQ));
  page.posted.length = 0;

  page.context.app.send('/choose move 1 +1, move 2|7', ROOM); // the client's own send
  ws.send(ROOM + '|/undo'); // straight to the socket (no client send around it)
  page.context.app.send('/choose move 2 -1, move 1 1|7', ROOM);
  page.context.app.send('/team 213456|2', ROOM);
  page.context.app.send('/cancel', ROOM);
  page.context.app.send('/trn someone,0,'); // not a battle command
  page.context.app.send('hello there', ROOM); // chat
  const sent = page.posted.filter((p) => p.type === 'sent').map((p) => p.data);
  assert.deepEqual(sent, [
    ROOM + '|/choose move 1 +1, move 2|7',
    ROOM + '|/undo',
    ROOM + '|/choose move 2 -1, move 1 1|7',
    ROOM + '|/team 213456|2',
    ROOM + '|/cancel',
  ]);
  assert.equal(ws.sent.length, 7, 'everything still went out');

  // A content script that starts now gets the room's messages and commands in their original order.
  page.posted.length = 0;
  page.fromExtension({ type: 'hello', session: 's2' });
  const replay = page.posted.filter((p) => p.replayFor === 's2').map((p) => (p.type === 'sent' ? 'out ' + p.data : 'in ' + p.data.split('\n')[1]));
  assert.deepEqual(replay, [
    'in |init|battle',
    'in |request|' + JSON.stringify(REQ),
    'out ' + ROOM + '|/choose move 1 +1, move 2|7',
    'out ' + ROOM + '|/undo',
    'out ' + ROOM + '|/choose move 2 -1, move 1 1|7',
    'out ' + ROOM + '|/team 213456|2',
    'out ' + ROOM + '|/cancel',
  ]);
});

test('a resent log (reconnect) is appended to the room\'s replay, not restarted', () => {
  const page = makePage();
  const ws = page.run('new WebSocket("ws://localhost:8000/showdown/websocket")');
  ws.deliver(INIT);
  ws.deliver('>' + ROOM + '\n|request|' + JSON.stringify(REQ));
  ws.send(ROOM + '|/choose move 1 1|7');
  ws.deliver(INIT);
  page.fromExtension({ type: 'hello', session: 's1' });
  const replay = page.posted.filter((p) => p.replayFor === 's1' && (p.type === 'sent' || p.data.startsWith('>')));
  assert.equal(replay.length, 4);
});
