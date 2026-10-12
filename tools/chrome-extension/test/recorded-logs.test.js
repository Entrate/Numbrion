'use strict';
// Replays the raw battle streams the play server records (scratch/showdown-logs/*.json, one file per battle and
// bot: {room, side, result, turns, stats, messages: [{time, text}], sent: [{time, text}]}) through the store.
// scratch/ is not in git: without recordings the test is skipped.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const P = require('../src/protocol.js');

const DIR = path.resolve(__dirname, '..', '..', '..', 'scratch', 'showdown-logs');
const files = fs.existsSync(DIR) ? fs.readdirSync(DIR).filter((f) => f.endsWith('.json')).sort() : [];

test('recorded play-server battles replay through the store', { skip: files.length ? false : 'no scratch/showdown-logs/*.json' }, () => {
  let checked = 0;
  for (const file of files) {
    let log;
    try {
      log = JSON.parse(fs.readFileSync(path.join(DIR, file), 'utf8'));
    } catch (e) {
      continue; // a file being written
    }
    if (!Array.isArray(log.messages) || !log.room) continue;
    const events = [
      ...log.messages.map((m) => ({ time: m.time, kind: 'in', text: m.text })),
      ...(log.sent || []).map((m) => ({ time: m.time, kind: 'out', text: m.text })),
    ].sort((a, b) => a.time - b.time || (a.kind === 'in' ? -1 : 1));
    const store = new P.BattleStore();
    let requests = 0;
    for (const event of events) {
      if (event.kind === 'in') {
        const r = store.receive(event.text);
        if (r && r.roomid === log.room && r.request) requests++;
        continue;
      }
      const room = store.noteOutgoing(event.text, 'bot');
      if (!room || !room.sentChoice) continue;
      // A bot answers the request it holds: never a newer one than the store's.
      assert.ok(room.sentChoice.rqid <= room.request.rqid, `${file}: sent rqid ${room.sentChoice.rqid} after request ${room.request.rqid}`);
    }
    const room = store.get(log.room);
    assert.ok(room, file + ': room kept');
    assert.ok(room.complete, file + ': starts at |init|');
    assert.equal(room.lines[0], '|init|battle', file);
    assert.ok(room.lines.every((line) => line && !line.startsWith('|request|') && !line.startsWith('|sentchoice|')), file);
    assert.equal(room.request.sideId, log.side, file + ': side from the request');
    if (log.stats && typeof log.stats.requests === 'number') assert.equal(requests, log.stats.requests, file + ': requests');
    if (log.result) {
      assert.ok(room.ended, file + ': ended');
      if (log.result.tie) assert.equal(room.winner, null, file);
      else assert.equal(room.winner, log.result.winner, file + ': winner');
    }
    if (typeof log.turns === 'number') assert.equal(room.turn, log.turns, file + ': turns');
    // History: one entry per request (an rqid once), each earlier one with the last choice sent for it.
    const history = store.history(room);
    const last = new Map();
    for (const m of log.sent || []) {
      const command = P.parseOutgoingCommand(P.splitOutgoing(m.text).text);
      if (command && !command.undo && command.rqid !== null) last.set(command.rqid, command.choice);
    }
    assert.equal(history[history.length - 1].request, room.request.raw, file + ': last entry is the current request');
    for (const entry of history.slice(0, -1)) {
      assert.equal(entry.choice, last.has(entry.rqid) ? last.get(entry.rqid) : null, file + ': choice for rqid ' + entry.rqid);
      assert.ok(entry.index <= room.lines.length, file);
    }
    // Complete when the recording starts by turn 1 (a bot that rejoined later misses the earlier requests).
    let turnAtFirstRequest = 0;
    for (const m of log.messages) {
      const { lines } = P.splitMessage(m.text);
      if (lines.some((line) => line.startsWith('|request|'))) break;
      for (const line of lines) if (line.startsWith('|turn|')) turnAtFirstRequest = parseInt(line.slice(6), 10);
    }
    assert.equal(store.historyComplete(room), turnAtFirstRequest <= 1, file + ': history complete?');
    checked++;
  }
  assert.ok(checked > 0, 'at least one recording checked');
});
