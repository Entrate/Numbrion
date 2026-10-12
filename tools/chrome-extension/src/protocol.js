/*
 * Showdown protocol bookkeeping for the content script (ISOLATED world) and the node tests: battle rooms, requests
 * and choices. No DOM, no chrome.* APIs. (Transport framing, SockJS or a plain WebSocket, is decoded by
 * page-hook.js in the MAIN world; Chrome injects a file into a frame only once, so the two worlds cannot share
 * one file.)
 *
 * Message: ">roomid\n" followed by protocol lines, or just lines for the global room. A battle room keeps every
 * line except "|request|" (the side's request JSON) and "|sentchoice|" (the choice already sent, after a
 * reconnect); those are request state, not battle lines.
 */
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.NumbrionProtocol = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  // ---------------------------------------------------------------- messages

  /** {roomid, lines}: roomid is '' for global messages; empty lines are dropped. */
  function splitMessage(msg) {
    msg = String(msg);
    let roomid = '';
    if (msg[0] === '>') {
      const nl = msg.indexOf('\n');
      if (nl < 0) return { roomid: msg.slice(1).trim(), lines: [] };
      roomid = msg.slice(1, nl).trim();
      msg = msg.slice(nl + 1);
    }
    const lines = msg.split('\n').map((line) => (line.endsWith('\r') ? line.slice(0, -1) : line)).filter((line) => line !== '');
    return { roomid, lines };
  }

  /** "move" for "|move|...", "" for "|" and for lines that do not start with "|". */
  function lineType(line) {
    if (typeof line !== 'string' || line[0] !== '|') return '';
    const end = line.indexOf('|', 1);
    return end < 0 ? line.slice(1) : line.slice(1, end);
  }

  function isBattleRoomId(roomid) {
    return typeof roomid === 'string' && roomid.startsWith('battle-');
  }

  function requestKind(data) {
    if (!data || typeof data !== 'object') return 'none';
    if (data.wait) return 'wait';
    if (data.teamPreview) return 'team';
    if (data.forceSwitch) return 'switch';
    if (Array.isArray(data.active)) return 'move';
    return 'unknown';
  }

  function parseRequest(text) {
    text = text == null ? '' : String(text);
    let data = null;
    if (text && text !== 'null') {
      try {
        data = JSON.parse(text);
      } catch (e) {
        data = null;
      }
    }
    const kind = requestKind(data);
    return {
      raw: text,
      data,
      kind,
      rqid: data && data.rqid !== undefined ? data.rqid : null,
      sideId: data && data.side && data.side.id ? data.side.id : null,
      sideName: data && data.side && data.side.name ? data.side.name : null,
    };
  }

  /** A request the user has to answer with a choice (moves or a forced switch). */
  function isActionable(request) {
    return !!request && (request.kind === 'move' || request.kind === 'switch');
  }

  /** "move 1 +2 terastallize, move 3" -> ["move 1 +2 terastallize", "move 3"]. */
  function splitChoice(choice) {
    return String(choice || '')
      .split(',')
      .map((part) => part.trim())
      .filter((part) => part !== '');
  }

  const MOVE_FLAGS = ['mega', 'megax', 'megay', 'zmove', 'ultra', 'dynamax', 'max', 'terastallize'];

  /** Parses one slot's choice: {type, index, target, flags}. */
  function parseChoicePart(part) {
    const tokens = String(part).trim().split(/\s+/);
    const type = tokens[0] || '';
    const result = { type, index: null, target: null, flags: [], text: part };
    if (type === 'move' || type === 'switch') {
      const index = parseInt(tokens[1], 10);
      if (!Number.isNaN(index)) result.index = index;
      for (const token of tokens.slice(2)) {
        if (/^[+-]?\d+$/.test(token)) result.target = parseInt(token, 10);
        else if (MOVE_FLAGS.includes(token)) result.flags.push(token);
      }
    }
    return result;
  }

  /** The classic client's own form of a move choice ("move 1 terastallize 2"), for its "you chose" summary. */
  function toClassicChoice(part) {
    const parsed = parseChoicePart(part);
    if (parsed.type !== 'move' || parsed.index === null) return String(part).trim();
    let text = 'move ' + parsed.index;
    for (const flag of parsed.flags) text += ' ' + flag;
    if (parsed.target !== null) text += ' ' + parsed.target;
    return text;
  }

  /**
   * A battle command the page sent: "/choose move 1 2,move 3|7" -> {choice: "move 1 2,move 3", rqid: 7};
   * "/team 213|2" -> {choice: "team 213", rqid: 2}; the rqid is null when the client left it out (the server
   * then applies the choice to the current request). "/undo" and "/cancel" -> {undo: true}.
   */
  function parseOutgoingCommand(text) {
    text = String(text || '').trim();
    if (/^\/(undo|cancel)\b/.test(text)) return { undo: true };
    let body;
    if (text.startsWith('/choose ')) body = text.slice(8);
    else if (text.startsWith('/team ')) body = 'team ' + text.slice(6);
    else return null;
    let rqid = null;
    const bar = body.lastIndexOf('|');
    if (bar >= 0) {
      const tail = body.slice(bar + 1);
      if (/^\d+$/.test(tail)) rqid = parseInt(tail, 10);
      body = body.slice(0, bar);
    }
    return { choice: body.trim(), rqid };
  }

  /** "battle-x|/choose move 1|3" -> {roomid, text}. */
  function splitOutgoing(message) {
    message = String(message || '');
    const bar = message.indexOf('|');
    if (bar < 0) return { roomid: '', text: message };
    return { roomid: message.slice(0, bar), text: message.slice(bar + 1) };
  }

  // ---------------------------------------------------------------- per-room state

  /**
   * Lines that are not the battle itself (chat, joins, timestamps, client notices, live-only errors). When the
   * server resends a battle's log (a reconnect), only the battle lines are sure to come back in the same order,
   * so request positions are carried over by counting those.
   */
  const NON_BATTLE_TYPES = new Set([
    'j', 'J', 'join', 'l', 'L', 'leave', 'n', 'N', 'name', 'c', 'c:', 'chat', 'chatmsg', 'chatmsg-raw', 'raw', 'html',
    'uhtml', 'uhtmlchange', 'error', 'bigerror', 'inactive', 'inactiveoff', 'init', 'title', 't:', 'timestamp',
    'player', 'deinit', 'noinit', 'expire', 'notify', 'tempnotify', 'tempnotifyoff', 'hidelines', 'unlink',
    'debug', 'controlshtml', 'fieldhtml', 'selectorhtml', 'cantleave', 'allowleave', 'askreg', 'userlist', 'badge',
    'spectator', 'spectatorleave', 'popup', 'pm', 'usercount', 'tournament', 'sentchoice', 'request',
  ]);

  function isBattleLine(line) {
    return typeof line === 'string' && line[0] === '|' && !NON_BATTLE_TYPES.has(lineType(line));
  }

  class RoomState {
    constructor(roomid) {
      this.roomid = roomid;
      this.request = null; // parseRequest() + {seq, receivedAt, linesAtArrival}
      this.requestSeq = 0;
      // Every request received, in order: {index, anchor, turn, rqid, raw, kind}. index: lines received before
      // it; anchor: battle lines among those; turn: the battle's turn when it arrived.
      this.requests = [];
      this.choices = new Map(); // rqid -> the last choice text the user sent for it
      this.carried = null; // {battle, requests}: history from before a resent log, mapped onto the new lines
      this.historyBroken = false; // a resent log did not match: the earlier history is gone
      this.reset();
    }

    /** Called for a new room and for "|init|" (the server resends the whole log on (re)join). */
    reset() {
      this.lines = [];
      this.hasInit = false;
      this.turn = 0;
      this.ended = false;
      this.winner = null;
      this.players = {};
      this.title = '';
      this.format = '';
      this.gametype = '';
      this.lastError = null; // {text, rqid, at}
      this.actives = {}; // "p1a" -> name
      this.sentChoice = null; // {choice, rqid, source}
      this.battleLines = 0;
      this.version = (this.version || 0) + 1;
    }

    /** Every line was seen from the start of the battle (needed by the advisor to rebuild the battle). */
    get complete() {
      return this.hasInit;
    }

    /** The side ("p1"/"p2") the user plays, from the request. */
    get mySide() {
      return this.request && this.request.sideId ? this.request.sideId : null;
    }
  }

  class BattleStore {
    constructor(options) {
      this.rooms = new Map();
      this.user = null;
      this.now = (options && options.now) || (() => Date.now());
    }

    get(roomid) {
      return this.rooms.get(roomid) || null;
    }

    room(roomid) {
      let room = this.rooms.get(roomid);
      if (!room) {
        room = new RoomState(roomid);
        this.rooms.set(roomid, room);
      }
      return room;
    }

    /**
     * Feeds one server message. Returns null for messages of non-battle rooms, otherwise
     * {roomid, room, newLines, request, deinit, init, error, ended}.
     */
    receive(msg) {
      const { roomid, lines } = splitMessage(msg);
      if (!roomid) {
        for (const line of lines) this._global(line);
        return null;
      }
      const isInit = lines.length > 0 && lines[0].startsWith('|init|');
      if (!isBattleRoomId(roomid) && !(isInit && lines[0] === '|init|battle') && !this.rooms.has(roomid)) return null;
      const result = { roomid, room: null, newLines: 0, request: false, deinit: false, init: false, error: null, ended: false };
      if (lines.some((line) => line === '|deinit' || line.startsWith('|deinit|'))) {
        this.rooms.delete(roomid);
        result.deinit = true;
        return result;
      }
      if (lines.some((line) => line.startsWith('|noinit|') || line === '|noinit')) {
        // The room does not exist (anymore) on the server.
        result.room = this.get(roomid);
        return result;
      }
      const room = this.room(roomid);
      result.room = room;
      let legacyRequestNext = false;
      for (const line of lines) {
        if (legacyRequestNext) {
          // Old servers: "|request|N|CHOICE" followed by the request JSON on the next line.
          legacyRequestNext = false;
          this._setRequest(room, line);
          result.request = true;
          continue;
        }
        const type = lineType(line);
        switch (type) {
          case 'request': {
            const text = line.slice(9);
            if (/^\d\|/.test(text)) {
              legacyRequestNext = true;
              continue;
            }
            this._setRequest(room, text);
            result.request = true;
            continue;
          }
          case 'sentchoice': {
            const rqid = room.request ? room.request.rqid : null;
            room.sentChoice = { choice: line.slice(12), rqid, source: 'server' };
            if (rqid !== null) room.choices.set(rqid, line.slice(12));
            continue;
          }
          case 'init':
            if (room.requests.length) {
              // The server resends the whole log (a reconnect): keep the history and place it on the new lines
              // once they are in (see _carryHistory).
              room.carried = { battle: room.lines.filter(isBattleLine), requests: room.requests };
              room.requests = [];
            }
            room.reset();
            room.hasInit = true;
            result.init = true;
            break;
          case 'title':
            room.title = line.slice(7);
            break;
          case 'tier':
            room.format = line.slice(6);
            break;
          case 'gametype':
            room.gametype = line.slice(10);
            break;
          case 'player': {
            const parts = line.split('|');
            if (parts[2] && parts[3]) room.players[parts[2]] = parts[3];
            break;
          }
          case 'turn':
            room.turn = parseInt(line.slice(6), 10) || room.turn;
            break;
          case 'switch':
          case 'drag':
          case 'replace': {
            const ident = line.split('|')[2] || '';
            const m = /^(p\d[a-z]): (.*)$/.exec(ident);
            if (m) room.actives[m[1]] = m[2];
            break;
          }
          case 'win':
            room.ended = true;
            room.winner = line.slice(5);
            result.ended = true;
            break;
          case 'tie':
            room.ended = true;
            room.winner = null;
            result.ended = true;
            break;
          case 'error':
            room.lastError = { text: line.slice(7), rqid: room.request ? room.request.rqid : null, at: this.now() };
            result.error = room.lastError.text;
            break;
          default:
            break;
        }
        room.lines.push(line);
        if (isBattleLine(line)) room.battleLines++;
        result.newLines++;
      }
      room.lastMessageAt = this.now();
      room.version++;
      return result;
    }

    /**
     * The page sent "roomid|/choose ...|rqid" (the client, or "Play this" through it). The last choice per rqid
     * is kept for the history; "/undo" and "/cancel" drop the current request's choice.
     */
    noteOutgoing(message, source) {
      const { roomid, text } = splitOutgoing(message);
      const room = this.get(roomid);
      if (!room) return null;
      const command = parseOutgoingCommand(text);
      if (!command) return null;
      const current = room.request ? room.request.rqid : null;
      if (command.undo) {
        room.sentChoice = null;
        if (current !== null) room.choices.delete(current);
      } else {
        const rqid = command.rqid !== null ? command.rqid : current;
        room.sentChoice = { choice: command.choice, rqid, source: source || 'client' };
        if (rqid !== null) room.choices.set(rqid, command.choice);
      }
      room.version++;
      return room;
    }

    _setRequest(room, text) {
      if (room.carried) this._carryHistory(room);
      const request = parseRequest(text);
      request.seq = ++room.requestSeq;
      request.receivedAt = this.now();
      request.linesAtArrival = room.lines.length;
      room.request = request;
      room.sentChoice = null;
      room.requests.push({
        index: room.lines.length,
        anchor: room.battleLines,
        turn: room.turn,
        rqid: request.rqid,
        raw: request.raw,
        kind: request.kind,
      });
    }

    /**
     * After a resent log: put the requests from before it at the same battle positions in the new lines, if the
     * new lines repeat the old battle lines; otherwise drop them (the history is then incomplete).
     */
    _carryHistory(room) {
      const { battle, requests } = room.carried;
      room.carried = null;
      const positions = []; // positions[k]: index just after the (k+1)-th battle line of the new lines
      room.lines.forEach((line, i) => {
        if (isBattleLine(line)) positions.push(i + 1);
      });
      const needed = Math.max(0, ...requests.map((r) => r.anchor));
      let matches = positions.length >= needed;
      if (matches) {
        let k = 0;
        for (const line of room.lines) {
          if (k >= needed) break;
          if (!isBattleLine(line)) continue;
          if (line !== battle[k]) {
            matches = false;
            break;
          }
          k++;
        }
      }
      if (!matches) {
        room.historyBroken = true;
        return;
      }
      room.requests = requests.map((r) => Object.assign({}, r, { index: r.anchor ? positions[r.anchor - 1] : 0 })).concat(room.requests);
    }

    /**
     * The "history" of POST /api/advise: every request of the room in order with the user's final choice (null for
     * the last entry, the current request). An rqid seen twice (a request resent on reconnect) keeps its last
     * entry.
     */
    history(room) {
      if (room.carried) this._carryHistory(room);
      const entries = [];
      const seen = new Set();
      for (let i = room.requests.length - 1; i >= 0; i--) {
        const r = room.requests[i];
        if (r.rqid !== null && seen.has(r.rqid)) continue;
        seen.add(r.rqid);
        entries.push(r);
      }
      entries.reverse();
      return entries.map((r, i) => ({
        index: Math.min(r.index, room.lines.length),
        rqid: r.rqid,
        request: r.raw,
        choice: i === entries.length - 1 || r.rqid === null ? null : room.choices.has(r.rqid) ? room.choices.get(r.rqid) : null,
      }));
    }

    /**
     * Whether the history reaches back to the battle's first request. Not when the extension came late, the page
     * was reloaded mid-battle (the server resends the lines, not the past requests) or a resent log did not match.
     */
    historyComplete(room) {
      if (room.historyBroken || !room.requests.length) return !room.historyBroken && room.turn <= 1;
      const first = room.carried && room.carried.requests.length ? room.carried.requests[0] : room.requests[0];
      return first.turn <= 1;
    }

    _global(line) {
      if (lineType(line) === 'updateuser') {
        const parts = line.split('|');
        let name = parts[2] || '';
        // The first character is the user's rank symbol (a space for none).
        if (name && !/[A-Za-z0-9]/.test(name[0])) name = name.slice(1);
        this.user = { name: name.trim(), named: parts[3] === '1' };
      }
    }
  }

  /** The body of POST /api/advise for a room, or null when there is nothing to ask. */
  function buildAdviseBody(room, options, store) {
    if (!room || !room.request || !isActionable(room.request)) return null;
    const body = { room: room.roomid, lines: room.lines.slice(), request: room.request.raw };
    if (store) body.history = store.history(room);
    if (options && options.budgetMs) body.budget_ms = options.budgetMs;
    if (options && options.bot) body.bot = options.bot;
    return body;
  }

  // ---------------------------------------------------------------- display helpers

  /** Names of the user's pokemon and moves for showing a choice without the advisor's analysis. */
  function describeChoice(choice, requestData, room) {
    const parts = splitChoice(choice);
    const side = requestData && requestData.side ? requestData.side : null;
    const mine = side && Array.isArray(side.pokemon) ? side.pokemon : [];
    const active = requestData && Array.isArray(requestData.active) ? requestData.active : [];
    const sideId = side && side.id ? side.id : null;
    const foeId = sideId === 'p1' ? 'p2' : sideId === 'p2' ? 'p1' : null;
    const nameOf = (mon) => (mon && mon.ident ? mon.ident.replace(/^p\d[a-z]?: /, '') : '?');
    if (parts.length === 1 && parts[0] === 'default') return [{ pokemon: '', text: 'default (the server picks the first legal option)' }];
    return parts.map((part, slot) => {
      const parsed = parseChoicePart(part);
      const who = nameOf(mine[slot]);
      if (parsed.type === 'move') {
        const moves = active[slot] && Array.isArray(active[slot].moves) ? active[slot].moves : [];
        const move = moves[parsed.index - 1];
        let text = move ? move.move : 'move ' + parsed.index;
        if (parsed.target !== null && room) {
          const letter = 'abc'[Math.abs(parsed.target) - 1] || '?';
          const key = (parsed.target > 0 ? foeId : sideId) + letter;
          const target = room.actives && room.actives[key];
          text += ' → ' + (target || (parsed.target > 0 ? 'foe ' : 'ally ') + Math.abs(parsed.target));
        }
        if (parsed.flags.includes('terastallize')) text += ' (Tera)';
        return { pokemon: who, text };
      }
      if (parsed.type === 'switch') return { pokemon: who, text: 'switch to ' + nameOf(mine[parsed.index - 1]) };
      return { pokemon: who, text: part };
    });
  }

  return {
    splitMessage,
    lineType,
    isBattleRoomId,
    requestKind,
    parseRequest,
    isActionable,
    splitChoice,
    parseChoicePart,
    toClassicChoice,
    parseOutgoingCommand,
    isBattleLine,
    splitOutgoing,
    RoomState,
    BattleStore,
    buildAdviseBody,
    describeChoice,
  };
});
