'use strict';
// node --test tools/chrome-extension/test
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const P = require('../src/protocol.js');
const T = require('../src/page-hook.js'); // transport helpers (in a browser the file installs the hook instead)

const ROOM = 'battle-gen9randomdoublesbattle-77';

const REQUEST = {
  active: [
    {
      moves: [
        { move: 'Spore', id: 'spore', pp: 16, maxpp: 16, target: 'normal', disabled: false },
        { move: 'Pollen Puff', id: 'pollenpuff', pp: 24, maxpp: 24, target: 'any', disabled: false },
        { move: 'Rage Powder', id: 'ragepowder', pp: 32, maxpp: 32, target: 'self', disabled: false },
        { move: 'Protect', id: 'protect', pp: 16, maxpp: 16, target: 'self', disabled: false },
      ],
      canTerastallize: 'Water',
    },
    {
      moves: [
        { move: 'Fake Out', id: 'fakeout', pp: 16, maxpp: 16, target: 'normal', disabled: false },
        { move: 'Flare Blitz', id: 'flareblitz', pp: 24, maxpp: 24, target: 'normal', disabled: false },
        { move: 'Parting Shot', id: 'partingshot', pp: 32, maxpp: 32, target: 'normal', disabled: false },
        { move: 'Knock Off', id: 'knockoff', pp: 32, maxpp: 32, target: 'normal', disabled: false },
      ],
      canTerastallize: 'Grass',
    },
  ],
  side: {
    name: 'Alice',
    id: 'p1',
    pokemon: [
      { ident: 'p1: Amoonguss', details: 'Amoonguss, L88, F', condition: '320/320', active: true },
      { ident: 'p1: Incineroar', details: 'Incineroar, L80, M', condition: '290/290', active: true },
      { ident: 'p1: Gholdengo', details: 'Gholdengo, L77', condition: '250/250', active: false },
    ],
  },
  rqid: 7,
};

const INIT = [
  '>' + ROOM,
  '|init|battle',
  '|title|Alice vs. NumbrionSearch',
  '|j|☆Alice',
  '|j|☆NumbrionSearch',
  '|t:|1791787364',
  '|gametype|doubles',
  '|player|p1|Alice|101|',
  '|player|p2|NumbrionSearch|1|',
  '|gen|9',
  '|tier|[Gen 9] Random Doubles Battle',
  '|rule|Species Clause: Limit one of each Pokémon',
  '|',
  '|t:|1791787364',
  '|teamsize|p1|6',
  '|teamsize|p2|6',
  '|start',
  '|switch|p1a: Amoonguss|Amoonguss, L88, F|320/320',
  '|switch|p1b: Incineroar|Incineroar, L80, M|290/290',
  '|switch|p2a: Kingambit|Kingambit, L78, M|100/100',
  '|switch|p2b: Pelipper|Pelipper, L86, F|100/100',
  '|-weather|RainDance|[from] ability: Drizzle|[of] p2b: Pelipper',
  '|turn|1',
  '',
].join('\n');

const requestMessage = (request) => '>' + ROOM + '\n|request|' + JSON.stringify(request);

test('SockJS frames decode into Showdown messages', () => {
  assert.deepEqual(T.decodeSockJSFrame('o'), { type: 'open', messages: [] });
  assert.deepEqual(T.decodeSockJSFrame('h'), { type: 'heartbeat', messages: [] });
  const frame = 'a' + JSON.stringify(['|challstr|4|abc', '>' + ROOM + '\n|move|p1a: Amoonguss|Spore|p2a: Kingambit\n|-status|p2a: Kingambit|slp']);
  const decoded = T.decodeSockJSFrame(frame);
  assert.equal(decoded.type, 'messages');
  assert.equal(decoded.messages.length, 2);
  assert.equal(decoded.messages[1].split('\n')[2], '|-status|p2a: Kingambit|slp');
  assert.deepEqual(T.decodeSockJSFrame('m"|updateuser| Guest 1|0|1|"').messages, ['|updateuser| Guest 1|0|1|']);
  const close = T.decodeSockJSFrame('c[3000,"Go away!"]');
  assert.equal(close.type, 'close');
  assert.equal(close.code, 3000);
  assert.deepEqual(T.decodeSockJSFrame('a[broken').messages, []);
  assert.deepEqual(T.decodeSockJSFrame('').messages, []);
});

test('SocketDecoder tells SockJS from a plain WebSocket', () => {
  assert.ok(T.isSockJSUrl('wss://sim3.psim.us/showdown/123/abcd1234/websocket'));
  assert.ok(!T.isSockJSUrl('ws://localhost:8000/showdown/websocket'));

  const raw = new T.SocketDecoder('ws://localhost:8000/showdown/websocket');
  assert.deepEqual(raw.decode('|challstr|4|abc'), ['|challstr|4|abc']);
  assert.equal(raw.mode, 'raw');
  assert.equal(raw.encodeOutgoing(ROOM + '|/choose move 1|3'), ROOM + '|/choose move 1|3');
  assert.deepEqual(raw.decodeOutgoing('|/trn x,0,'), ['|/trn x,0,']);

  // A SockJS URL is recognised up front; a server that opens with "o" is too.
  for (const decoder of [new T.SocketDecoder('wss://sim3.psim.us/showdown/123/abcd1234/websocket'), new T.SocketDecoder('wss://example/ws')]) {
    assert.deepEqual(decoder.decode('o'), []);
    assert.equal(decoder.mode, 'sockjs');
    assert.deepEqual(decoder.decode('h'), []);
    assert.deepEqual(decoder.decode('a["|a|1","|b|2"]'), ['|a|1', '|b|2']);
    const out = decoder.encodeOutgoing(ROOM + '|/choose move 1 +2, move 3|7');
    assert.equal(out, JSON.stringify([ROOM + '|/choose move 1 +2, move 3|7']));
    assert.deepEqual(decoder.decodeOutgoing(out), [ROOM + '|/choose move 1 +2, move 3|7']);
  }
});

test('messages split into room and lines', () => {
  assert.deepEqual(P.splitMessage('|updateuser| Alice|1|101|{}'), { roomid: '', lines: ['|updateuser| Alice|1|101|{}'] });
  const { roomid, lines } = P.splitMessage('>' + ROOM + '\n|init|battle\n|title|A vs. B\n\n');
  assert.equal(roomid, ROOM);
  assert.deepEqual(lines, ['|init|battle', '|title|A vs. B']);
  assert.equal(P.lineType('|move|p1a: X|Tackle|p2a: Y'), 'move');
  assert.equal(P.lineType('|'), '');
  assert.equal(P.lineType('|start'), 'start');
  assert.equal(P.lineType('plain chat'), '');
});

test('the store keeps battle lines without requests and tracks the latest request', () => {
  const store = new P.BattleStore();
  store.receive('|updateuser| Alice|1|101|{"blockChallenges":false}');
  assert.deepEqual(store.user, { name: 'Alice', named: true });
  assert.equal(store.receive('>lobby\n|c|~|hello'), null, 'chat rooms are ignored');

  let r = store.receive(INIT);
  assert.ok(r.init);
  r = store.receive(requestMessage(REQUEST));
  assert.ok(r.request);
  const room = store.get(ROOM);
  assert.ok(room.complete);
  assert.equal(room.lines[0], '|init|battle');
  assert.equal(room.lines.length, INIT.split('\n').length - 2, 'all lines but the room header and the trailing empty line');
  assert.ok(room.lines.every((line) => !line.startsWith('|request|')));
  assert.equal(room.request.rqid, 7);
  assert.equal(room.request.sideId, 'p1');
  assert.equal(room.request.kind, 'move');
  assert.ok(P.isActionable(room.request));
  assert.equal(room.request.raw, JSON.stringify(REQUEST));
  assert.equal(room.turn, 1);
  assert.equal(room.players.p2, 'NumbrionSearch');
  assert.equal(room.format, '[Gen 9] Random Doubles Battle');
  assert.equal(room.actives.p2b, 'Pelipper');

  const body = P.buildAdviseBody(room, { budgetMs: 1500 });
  assert.equal(body.room, ROOM);
  assert.equal(body.request, JSON.stringify(REQUEST));
  assert.equal(body.budget_ms, 1500);
  assert.deepEqual(body.lines, room.lines);
  assert.notEqual(body.lines, room.lines, 'a copy');

  // The user chooses; the server answers with the turn and the next request (either order).
  store.noteOutgoing(ROOM + '|/choose move 1 1, move 1 2|7', 'client');
  assert.deepEqual(room.sentChoice, { choice: 'move 1 1, move 1 2', rqid: 7, source: 'client' });
  const next = Object.assign({}, REQUEST, { rqid: 8 });
  store.receive(['>' + ROOM, '|', '|t:|1791787370', '|move|p1b: Incineroar|Fake Out|p2a: Kingambit', '|-damage|p2a: Kingambit|88/100', '|upkeep', '|turn|2'].join('\n'));
  store.receive(requestMessage(next));
  assert.equal(room.turn, 2);
  assert.equal(room.request.rqid, 8);
  assert.equal(room.request.seq, 2);
  assert.equal(room.sentChoice, null, 'a new request clears the sent choice');

  // History: every request with its line index and the user's final choice; the current one has none.
  let history = store.history(room);
  assert.deepEqual(history.map((h) => [h.rqid, h.choice]), [[7, 'move 1 1, move 1 2'], [8, null]]);
  assert.equal(history[0].index, INIT.split('\n').length - 2, 'request 7 came after the init lines');
  assert.equal(history[1].index, room.lines.length);
  assert.equal(history[0].request, JSON.stringify(REQUEST));
  assert.deepEqual(P.buildAdviseBody(room, {}, store).history, history);
  assert.ok(store.historyComplete(room));
  // /undo drops the current request's choice; a /choose without rqid applies to the current request.
  store.noteOutgoing(ROOM + '|/choose move 2 -2, move 4 1', 'client');
  assert.equal(room.choices.get(8), 'move 2 -2, move 4 1');
  store.noteOutgoing(ROOM + '|/undo', 'client');
  assert.equal(room.choices.has(8), false);
  store.noteOutgoing(ROOM + '|/choose move 3, move 4 1|8', 'client');
  store.receive(['>' + ROOM, '|', '|move|p1a: Amoonguss|Rage Powder|p1a: Amoonguss', '|upkeep', '|turn|3'].join('\n'));
  store.receive(requestMessage(Object.assign({}, REQUEST, { rqid: 9 })));
  history = store.history(room);
  assert.deepEqual(history.map((h) => [h.rqid, h.choice]), [[7, 'move 1 1, move 1 2'], [8, 'move 3, move 4 1'], [9, null]]);
  // (rqid 9 is answered below by the wait request's turn.)
  store.noteOutgoing(ROOM + '|/choose move 1 1, move 3|9', 'client');

  // An invalid choice is reported for the current request.
  r = store.receive('>' + ROOM + "\n|error|[Invalid choice] Can't move: Amoonguss's Spore is disabled");
  assert.match(r.error, /Invalid choice/);
  assert.equal(room.lastError.rqid, 9);

  // Waiting and forced switches.
  store.receive(requestMessage({ wait: true, side: REQUEST.side, rqid: 10 }));
  assert.equal(room.request.kind, 'wait');
  assert.ok(!P.isActionable(room.request));
  assert.equal(P.buildAdviseBody(room, {}), null);
  store.receive(requestMessage({ forceSwitch: [true, false], side: REQUEST.side, noCancel: true, rqid: 11 }));
  assert.equal(room.request.kind, 'switch');
  assert.ok(P.isActionable(room.request));

  // A rejoin resends the whole log: lines restart, the request stays until the new one arrives.
  const before = room.lines.length;
  store.receive(INIT);
  assert.ok(before > room.lines.length);
  assert.equal(room.lines.length, INIT.split('\n').length - 2);
  assert.equal(room.request.rqid, 11);
  store.receive(requestMessage(Object.assign({}, REQUEST, { rqid: 12 })) + '\n|sentchoice|move 2 -2, move 4 1');
  assert.deepEqual(room.sentChoice, { choice: 'move 2 -2, move 4 1', rqid: 12, source: 'server' });
  assert.ok(room.lines.every((line) => !line.startsWith('|sentchoice|')));

  r = store.receive('>' + ROOM + '\n|\n|t:|1791787400\n|faint|p2a: Kingambit\n|win|Alice');
  assert.ok(r.ended);
  assert.ok(room.ended);
  assert.equal(room.winner, 'Alice');

  r = store.receive('>' + ROOM + '\n|deinit');
  assert.ok(r.deinit);
  assert.equal(store.get(ROOM), null);
});

test('a room joined without |init| is marked incomplete', () => {
  const store = new P.BattleStore();
  store.receive('>' + ROOM + '\n|move|p1a: Amoonguss|Spore|p2a: Kingambit');
  store.receive(requestMessage(REQUEST));
  assert.equal(store.get(ROOM).complete, false);
});

test('legacy "|request|N|CHOICE" puts the request on the next line', () => {
  const store = new P.BattleStore();
  store.receive(INIT);
  store.receive('>' + ROOM + '\n|request|2|move 1\n' + JSON.stringify(REQUEST));
  const room = store.get(ROOM);
  assert.equal(room.request.rqid, 7);
  assert.ok(room.lines.every((line) => !line.startsWith('{')));
});

test('choice helpers', () => {
  assert.deepEqual(P.splitChoice('move 1 +2 terastallize, move 3'), ['move 1 +2 terastallize', 'move 3']);
  assert.deepEqual(P.parseChoicePart('move 1 +2 terastallize'), { type: 'move', index: 1, target: 2, flags: ['terastallize'], text: 'move 1 +2 terastallize' });
  assert.equal(P.toClassicChoice('move 1 +2 terastallize'), 'move 1 terastallize 2');
  assert.equal(P.toClassicChoice('move 2 -1'), 'move 2 -1');
  assert.equal(P.toClassicChoice('switch 3'), 'switch 3');
  assert.deepEqual(P.parseOutgoingCommand('/choose move 1 2,move 3|7'), { choice: 'move 1 2,move 3', rqid: 7 });
  assert.deepEqual(P.parseOutgoingCommand('/choose default'), { choice: 'default', rqid: null });
  assert.deepEqual(P.parseOutgoingCommand('/undo'), { undo: true });
  assert.deepEqual(P.parseOutgoingCommand('/cancel'), { undo: true });
  assert.deepEqual(P.parseOutgoingCommand('/choose move 1 +1, move 2'), { choice: 'move 1 +1, move 2', rqid: null });
  assert.deepEqual(P.parseOutgoingCommand('/team 213456|2'), { choice: 'team 213456', rqid: 2 });
  assert.equal(P.parseOutgoingCommand('/timer on'), null);
  assert.equal(P.parseOutgoingCommand('/chooser x'), null);
  assert.deepEqual(P.splitOutgoing(ROOM + '|/choose move 1|3'), { roomid: ROOM, text: '/choose move 1|3' });

  const store = new P.BattleStore();
  store.receive(INIT);
  const room = store.get(ROOM);
  assert.deepEqual(P.describeChoice('move 1 +2 terastallize, switch 3', REQUEST, room), [
    { pokemon: 'Amoonguss', text: 'Spore → Pelipper (Tera)' },
    { pokemon: 'Incineroar', text: 'switch to Gholdengo' },
  ]);
  assert.deepEqual(P.describeChoice('move 2 -2', REQUEST, room), [{ pokemon: 'Amoonguss', text: 'Pollen Puff → Incineroar' }]);
  assert.equal(P.describeChoice('default', REQUEST, room).length, 1);
});

// ---------------------------------------------------------------- a real recorded server stream

const fixture = JSON.parse(fs.readFileSync(path.join(__dirname, 'fixtures', 'probe-battle-p2.json'), 'utf8'));
const received = fixture.messages.filter((m) => typeof m === 'string');

function expectedFromFixture() {
  const lines = [];
  const requests = [];
  let inRoom = false;
  for (const msg of received) {
    const { roomid, lines: ls } = P.splitMessage(msg);
    if (!roomid) continue;
    if (ls[0] === '|init|battle') {
      lines.length = 0;
      inRoom = true;
    }
    if (!inRoom) continue;
    for (const line of ls) {
      if (line.startsWith('|request|')) requests.push(JSON.parse(line.slice(9)));
      else lines.push(line);
    }
  }
  return { lines, requests };
}

function checkFixtureRoom(store) {
  const roomid = P.splitMessage(received.find((m) => m.startsWith('>battle-'))).roomid;
  const room = store.get(roomid);
  const { lines, requests } = expectedFromFixture();
  assert.ok(room.complete);
  assert.deepEqual(room.lines, lines);
  assert.equal(room.requestSeq, requests.length);
  assert.equal(room.request.rqid, requests[requests.length - 1].rqid);
  assert.equal(room.request.sideId, 'p2');
  assert.ok(room.ended);
  assert.equal(room.winner, lines.find((line) => line.startsWith('|win|')).slice(5));
  assert.equal(room.turn, Math.max(...lines.filter((l) => l.startsWith('|turn|')).map((l) => parseInt(l.slice(6), 10))));
  assert.equal(store.user.name, 'probea');
  return room;
}

test('a recorded local-server battle (plain WebSocket frames)', () => {
  const store = new P.BattleStore();
  const decoder = new T.SocketDecoder('ws://localhost:8000/showdown/websocket');
  let actionable = 0;
  let sent = 0;
  for (const item of fixture.messages) {
    if (typeof item !== 'string') {
      // The client's "/choose default|RQID" answers the request it just received.
      const room = store.noteOutgoing(item.sent, 'client');
      assert.deepEqual(room.sentChoice, { choice: 'default', rqid: room.request.rqid, source: 'client' });
      sent++;
      continue;
    }
    for (const msg of decoder.decode(item)) {
      const r = store.receive(msg);
      if (r && r.request && P.isActionable(r.room.request)) actionable++;
    }
  }
  const room = checkFixtureRoom(store);
  assert.ok(actionable > 5);
  assert.equal(sent, actionable);
  // Every request in this server version arrives right after the lines of its turn.
  assert.ok(room.lines.includes('|start'));
});

test('the same battle through SockJS frames (batched, with heartbeats)', () => {
  const store = new P.BattleStore();
  const decoder = new T.SocketDecoder('https://sim3.psim.us/showdown/417/x8k2m1qz/websocket');
  const frames = ['o'];
  for (let i = 0; i < received.length; i += 3) {
    frames.push('a' + JSON.stringify(received.slice(i, i + 3)));
    if (i % 9 === 0) frames.push('h');
  }
  for (const frame of frames) for (const msg of decoder.decode(frame)) store.receive(msg);
  checkFixtureRoom(store);
});

// ---------------------------------------------------------------- history for POST /api/advise

const FIXTURE_ROOM = P.splitMessage(received.find((m) => m.startsWith('>battle-'))).roomid;

test('history of the recorded battle: every request, its line index and the choice sent for it', () => {
  const store = new P.BattleStore();
  const expected = []; // {index, rqid, choice}
  let count = 0;
  for (const item of fixture.messages) {
    if (typeof item !== 'string') {
      store.noteOutgoing(item.sent, 'client');
      const command = P.parseOutgoingCommand(P.splitOutgoing(item.sent).text);
      expected.find((e) => e.rqid === command.rqid).choice = command.choice;
      continue;
    }
    const { roomid, lines } = P.splitMessage(item);
    if (roomid === FIXTURE_ROOM) {
      for (const line of lines) {
        if (line.startsWith('|request|')) expected.push({ index: count, rqid: JSON.parse(line.slice(9)).rqid, choice: null });
        else count++;
      }
    }
    store.receive(item);
  }
  const room = store.get(FIXTURE_ROOM);
  const history = store.history(room);
  assert.equal(history.length, expected.length);
  expected[expected.length - 1].choice = null; // the current request
  assert.deepEqual(history.map((h) => ({ index: h.index, rqid: h.rqid, choice: h.choice })), expected);
  assert.ok(history.some((h) => JSON.parse(h.request).wait && h.choice === null), 'wait requests have no choice');
  assert.ok(history.filter((h) => h.choice === 'default').length > 10);
  assert.ok(store.historyComplete(room));
});

test('a resent log (reconnect) keeps the history at the same battle positions', () => {
  const store = new P.BattleStore();
  let requests = 0;
  for (const item of fixture.messages) {
    if (typeof item !== 'string') {
      store.noteOutgoing(item.sent, 'client');
      continue;
    }
    const r = store.receive(item);
    if (r && r.request && ++requests === 12) break;
  }
  const room = store.get(FIXTURE_ROOM);
  const oldLines = room.lines.slice();
  const before = store.history(room);
  const battleBefore = (lines, index) => lines.slice(0, index).filter(P.isBattleLine);

  // The server resends the log (here with joins and chat in other places) and then the current request.
  const battle = oldLines.filter(P.isBattleLine);
  store.receive(['>' + FIXTURE_ROOM, '|init|battle', '|title|' + room.title, '|j|☆probeb', ...battle.slice(0, 30), '|c|☆probeb|hi', ...battle.slice(30)].join('\n'));
  store.receive('>' + FIXTURE_ROOM + '\n|request|' + room.request.raw);
  const after = store.history(room);
  assert.deepEqual(after.map((h) => [h.rqid, h.choice]), before.map((h) => [h.rqid, h.choice]));
  for (let i = 0; i < after.length; i++) {
    assert.deepEqual(battleBefore(room.lines, after[i].index), battleBefore(oldLines, before[i].index), 'entry ' + i);
  }
  assert.ok(store.historyComplete(room));

  // Another battle under the same room id: the old history does not fit and is dropped.
  store.receive('>' + FIXTURE_ROOM + '\n|init|battle\n|title|x\n|start\n|switch|p1a: Mew|Mew, L80|100/100\n|turn|1');
  store.receive('>' + FIXTURE_ROOM + '\n|request|' + room.request.raw);
  assert.equal(store.history(room).length, 1);
  assert.equal(store.historyComplete(room), false);
});

test('a battle first seen mid-way has an incomplete history', () => {
  const store = new P.BattleStore();
  store.receive(INIT.replace('|turn|1', '|turn|1\n|\n|upkeep\n|turn|2\n|\n|upkeep\n|turn|3'));
  store.receive(requestMessage(REQUEST));
  const room = store.get(ROOM);
  assert.ok(room.complete, 'all lines are known');
  assert.equal(store.historyComplete(room), false, 'but not the requests and choices of turns 1 and 2');
  assert.deepEqual(store.history(room).map((h) => h.rqid), [7]);
});
