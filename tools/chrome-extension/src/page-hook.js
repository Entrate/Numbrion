/*
 * MAIN-world hook (runs at document_start, before the Showdown client's scripts). Self-contained: Chrome injects
 * an extension file into a frame only once, so this world cannot share protocol.js with the content script.
 *
 * Capture: wraps window.WebSocket so every socket the page opens gets our own "message" listener (added in the
 * constructor, so it runs before the client's onmessage) and an observed send(). The client talks to the server
 * over a plain WebSocket (one Showdown message per frame) or SockJS, whose WebSocket transport wraps messages in
 * frames: "o" (open), "h" (heartbeat), 'a["msg", ...]' (messages), 'm"msg"' (one message) and
 * 'c[code,"reason"]' (close); SockJS frames are decoded here. The client's own receive function (classic
 * `app.receive`, Preact `PS.receive`) is wrapped as well, which covers transports that are not a page WebSocket
 * (Preact's connection worker, SockJS XHR fallbacks) and late injection; messages already seen on a wrapped
 * socket are skipped there.
 *
 * Outgoing: the battle commands the page sends ("ROOMID|/choose CHOICE|RQID", "/team", "/undo", "/cancel") are
 * observed on the wrapped sockets' send() and on the client's own send functions (classic `app.send`, Preact
 * `PS.send`, which also cover late injection and the connection worker). The advisor needs the user's past
 * choices (docs/play/ADVISOR-API.md, "history"); "Play this" goes through the same paths.
 *
 * Relay: messages are posted to the ISOLATED content script with window.postMessage (same origin, tagged). Battle
 * room messages, incoming and outgoing in order, are also kept here (per room, from its first |init|) so a
 * content script that starts later, or again after an extension reload, gets the whole history on its "hello".
 * Rooms the client already has but whose |init| was never seen (injected late) are rebuilt from the client's
 * state (battle.stepQueue and request); their past choices are unknown.
 *
 * Commands from the content script: "play" submits a choice through the client (see play()).
 *
 * In node (tests) the file exports its transport helpers instead of installing the hook.
 */
(function () {
  'use strict';

  // ------------------------------------------------------------- transport

  function decodeSockJSFrame(frame) {
    if (typeof frame !== 'string' || !frame.length) return { type: 'unknown', messages: [] };
    const body = frame.slice(1);
    switch (frame[0]) {
      case 'o':
        return { type: 'open', messages: [] };
      case 'h':
        return { type: 'heartbeat', messages: [] };
      case 'a':
        try {
          const list = JSON.parse(body);
          if (Array.isArray(list)) return { type: 'messages', messages: list.map(String) };
        } catch (e) { /* fall through */ }
        return { type: 'unknown', messages: [] };
      case 'm':
        try {
          return { type: 'messages', messages: [String(JSON.parse(body))] };
        } catch (e) {
          return { type: 'unknown', messages: [] };
        }
      case 'c':
        try {
          const [code, reason] = JSON.parse(body);
          return { type: 'close', code, reason, messages: [] };
        } catch (e) {
          return { type: 'close', messages: [] };
        }
      default:
        return { type: 'unknown', messages: [] };
    }
  }

  /** SockJS WebSocket transport URLs look like .../<prefix>/<3 digit server>/<session>/websocket. */
  function isSockJSUrl(url) {
    return /\/\d{3}\/[^/?#]+\/websocket\/?([?#].*)?$/.test(String(url || ''));
  }

  /**
   * Turns the frames of one socket into Showdown messages. The mode is "sockjs" or "raw"; it is guessed from the
   * URL and confirmed by the first frame (a SockJS server always opens with "o").
   */
  class SocketDecoder {
    constructor(url) {
      this.url = String(url || '');
      this.mode = isSockJSUrl(this.url) ? 'sockjs' : 'unknown';
    }

    decode(frame) {
      if (typeof frame !== 'string') return [];
      if (this.mode === 'unknown') this.mode = frame === 'o' ? 'sockjs' : 'raw';
      if (this.mode === 'raw') return [frame];
      return decodeSockJSFrame(frame).messages;
    }

    /** Messages the page sent (to observe its own /choose commands). */
    decodeOutgoing(data) {
      if (typeof data !== 'string') return [];
      if (this.mode === 'sockjs' || (this.mode === 'unknown' && data[0] === '[')) {
        try {
          const list = JSON.parse(data);
          if (Array.isArray(list)) return list.map(String);
        } catch (e) { /* not SockJS */ }
        return this.mode === 'sockjs' ? [] : [data];
      }
      return [data];
    }

    encodeOutgoing(message) {
      return this.mode === 'sockjs' ? JSON.stringify([message]) : message;
    }
  }

  /** Room id and first line of a server message (">roomid\n|init|battle..."). */
  function messageHead(msg) {
    if (msg[0] !== '>') return { roomid: '', first: msg.split('\n', 1)[0] };
    const nl = msg.indexOf('\n');
    if (nl < 0) return { roomid: msg.slice(1).trim(), first: '' };
    const end = msg.indexOf('\n', nl + 1);
    return { roomid: msg.slice(1, nl).trim(), first: msg.slice(nl + 1, end < 0 ? undefined : end) };
  }

  const isBattleRoomId = (roomid) => typeof roomid === 'string' && roomid.startsWith('battle-');

  const transport = { decodeSockJSFrame, isSockJSUrl, SocketDecoder, messageHead };
  if (typeof module === 'object' && module.exports) {
    module.exports = transport;
    return;
  }

  // ------------------------------------------------------------- hook

  const MARK = '__numbrionAdvisorHook';
  if (window[MARK]) return;
  Object.defineProperty(window, MARK, { value: { version: 1 }, enumerable: false });

  const TAG = 'numbrion-advisor-v1';
  const ORIGIN = window.location.origin;
  const sockets = []; // {ws, decoder, showdown}
  const roomBuffers = new Map(); // roomid -> [message | {out: message}] from its first |init|
  let lastUserMessage = null;
  const recentFromSocket = []; // [{msg, at}] for de-duplicating client-hook captures
  const recentFromClient = []; // [{msg, at}] outgoing: the client's send runs before the socket's
  const sessions = new Set();
  let lastFocus = null;

  function post(payload) {
    payload.__numbrion = TAG;
    payload.dir = 'page->ext';
    try {
      window.postMessage(payload, ORIGIN);
    } catch (e) { /* ignore */ }
  }

  function remember(msg) {
    const { roomid, first } = messageHead(msg);
    if (!roomid) {
      if (msg.includes('|updateuser|')) lastUserMessage = msg;
      return;
    }
    if (!isBattleRoomId(roomid)) return;
    if (/\n\|deinit(\||\n|$)/.test(msg)) {
      roomBuffers.delete(roomid);
    } else if (first.startsWith('|init|') && !(roomBuffers.get(roomid) || []).length) {
      roomBuffers.set(roomid, [msg]);
    } else {
      // (A later |init| is the server resending the log after a reconnect: kept in order, so the requests and
      // choices from before it survive; the content script places them on the resent lines.)
      const buffer = roomBuffers.get(roomid);
      if (buffer) buffer.push(msg);
      else roomBuffers.set(roomid, [msg]);
    }
  }

  function capture(msg, source) {
    if (typeof msg !== 'string' || !msg) return;
    const now = Date.now();
    while (recentFromSocket.length && (now - recentFromSocket[0].at > 5000 || recentFromSocket.length > 500)) {
      recentFromSocket.shift();
    }
    if (source === 'client') {
      const index = recentFromSocket.findIndex((entry) => entry.msg === msg);
      if (index >= 0) {
        recentFromSocket.splice(index, 1);
        return;
      }
    } else {
      recentFromSocket.push({ msg, at: now });
    }
    remember(msg);
    if (sessions.size) post({ type: 'msg', data: msg });
  }

  const BATTLE_COMMAND = /^\/(choose|team|undo|cancel)\b/;

  /** A message the page sends to the server ("ROOMID|/choose ..."): battle commands are kept and relayed. */
  function captureOutgoing(message, source) {
    if (typeof message !== 'string') return;
    const bar = message.indexOf('|');
    if (bar < 0) return;
    const roomid = message.slice(0, bar);
    if (!isBattleRoomId(roomid) || !BATTLE_COMMAND.test(message.slice(bar + 1).trim())) return;
    const now = Date.now();
    while (recentFromClient.length && (now - recentFromClient[0].at > 5000 || recentFromClient.length > 100)) {
      recentFromClient.shift();
    }
    if (source === 'socket') {
      const index = recentFromClient.findIndex((entry) => entry.msg === message);
      if (index >= 0) {
        recentFromClient.splice(index, 1);
        return; // already seen in the client's send
      }
    } else {
      recentFromClient.push({ msg: message, at: now });
    }
    const buffer = roomBuffers.get(roomid);
    if (buffer) buffer.push({ out: message });
    else roomBuffers.set(roomid, [{ out: message }]);
    if (sessions.size) post({ type: 'sent', data: message, source });
  }

  function observeOutgoing(entry, data) {
    for (const message of entry.decoder.decodeOutgoing(data)) captureOutgoing(message, 'socket');
  }

  const NativeWebSocket = window.WebSocket;
  if (typeof NativeWebSocket === 'function' && typeof Proxy === 'function') {
    window.WebSocket = new Proxy(NativeWebSocket, {
      construct(target, args, newTarget) {
        const ws = Reflect.construct(target, args, newTarget === window.WebSocket ? target : newTarget);
        try {
          const entry = { ws, decoder: new SocketDecoder(args[0]), showdown: false };
          sockets.push(entry);
          ws.addEventListener('message', (event) => {
            try {
              if (typeof event.data !== 'string') return;
              for (const msg of entry.decoder.decode(event.data)) {
                if (!entry.showdown && /(^|\n)\|(challstr|updateuser|init|formats)\|/.test(msg)) entry.showdown = true;
                capture(msg, 'socket');
              }
            } catch (e) { /* never break the page */ }
          });
          ws.addEventListener('close', () => {
            const index = sockets.indexOf(entry);
            if (index >= 0) sockets.splice(index, 1);
          });
          const nativeSend = ws.send;
          ws.send = function (data) {
            try {
              observeOutgoing(entry, data);
            } catch (e) { /* ignore */ }
            return nativeSend.apply(this, arguments);
          };
        } catch (e) { /* ignore */ }
        return ws;
      },
    });
  }

  /** Classic `app.send(data, room)` and Preact `PS.send(msg, roomid)` both send "ROOM|DATA". */
  function wrapSend(owner, label) {
    if (!owner || typeof owner.send !== 'function' || owner.send.__numbrion) return;
    const original = owner.send;
    const wrapped = function (data, room) {
      try {
        let message = String(data);
        if (label === 'classic') {
          if (room && room !== true) message = room + '|' + message;
          else if (room !== true) message = '|' + message;
        } else {
          message = (room || '') + '|' + message;
        }
        captureOutgoing(message, 'client');
      } catch (e) { /* ignore */ }
      return original.apply(this, arguments);
    };
    wrapped.__numbrion = label;
    owner.send = wrapped;
  }

  function wrapReceive(owner, label) {
    if (!owner || typeof owner.receive !== 'function' || owner.receive.__numbrion) return;
    const original = owner.receive;
    const wrapped = function (data) {
      try {
        capture(String(data), 'client');
      } catch (e) { /* ignore */ }
      return original.apply(this, arguments);
    };
    wrapped.__numbrion = label;
    owner.receive = wrapped;
  }

  // ------------------------------------------------------------- client state

  function classicApp() {
    const app = window.app;
    return app && app.rooms && typeof app.receive === 'function' ? app : null;
  }

  function preactPS() {
    const ps = window.PS;
    return ps && ps.rooms && typeof ps.receive === 'function' ? ps : null;
  }

  function clientBattleRooms() {
    const result = [];
    const add = (roomid, room) => {
      if (!isBattleRoomId(roomid) || !room || !room.battle || !Array.isArray(room.battle.stepQueue)) return;
      result.push({ roomid, room });
    };
    const app = classicApp();
    if (app) for (const roomid of Object.keys(app.rooms)) add(roomid, app.rooms[roomid]);
    const ps = preactPS();
    if (ps) for (const roomid of Object.keys(ps.rooms)) add(roomid, ps.rooms[roomid]);
    return result;
  }

  function requestText(request) {
    if (!request) return null;
    const copy = Object.assign({}, request);
    delete copy.requestType; // added by the clients
    return JSON.stringify(copy);
  }

  /** Rebuilds the history of battle rooms whose |init| this hook never saw (it was injected late). */
  function backfill() {
    for (const { roomid, room } of clientBattleRooms()) {
      const buffer = roomBuffers.get(roomid);
      if (buffer && buffer.some((msg) => typeof msg === 'string' && messageHead(msg).first === '|init|battle')) continue;
      const steps = room.battle.stepQueue.filter((line) => typeof line === 'string' && line);
      if (!steps.length) continue;
      const lines = ['|init|battle'];
      if (room.title && !steps.some((line) => line.startsWith('|title|'))) lines.push('|title|' + room.title);
      lines.push(...steps);
      const messages = ['>' + roomid + '\n' + lines.join('\n')];
      const request = requestText(room.request);
      if (request) messages.push('>' + roomid + '\n|request|' + request);
      // (Choices captured since the late injection stay; their battle lines are in the client's queue.)
      roomBuffers.set(roomid, messages.concat(buffer ? buffer.filter((msg) => typeof msg !== 'string') : []));
      for (const msg of messages) {
        if (sessions.size) post({ type: 'msg', data: msg, backfill: true });
      }
    }
  }

  function activeRoom() {
    try {
      const app = classicApp();
      if (app && app.curRoom) {
        const main = app.curRoom.id || '';
        if (isBattleRoomId(main)) return main;
        const side = (app.curSideRoom && app.curSideRoom.id) || '';
        if (isBattleRoomId(side)) return side;
        return '';
      }
      const ps = preactPS();
      if (ps) {
        for (const room of [ps.room, ps.panel, ps.leftPanel, ps.rightPanel]) {
          if (room && isBattleRoomId(room.id)) return room.id;
        }
        return '';
      }
    } catch (e) { /* fall back to the URL */ }
    const segment = window.location.pathname.split('/').filter(Boolean).pop() || '';
    return isBattleRoomId(segment) ? segment : '';
  }

  function tick() {
    try {
      wrapReceive(classicApp(), 'classic');
      wrapReceive(preactPS(), 'preact');
      wrapSend(classicApp(), 'classic');
      wrapSend(preactPS(), 'preact');
      if (sessions.size) {
        backfill();
        const focus = activeRoom();
        if (focus !== lastFocus) {
          lastFocus = focus;
          post({ type: 'focus', roomid: focus });
        }
      }
    } catch (e) { /* ignore */ }
  }

  // Fast polling while the client starts (so its receive hook is in place before it connects), then slower.
  let ticks = 0;
  const fast = setInterval(() => {
    tick();
    if (++ticks >= 200) {
      clearInterval(fast);
      setInterval(tick, 500);
    }
  }, 50);

  // ------------------------------------------------------------- play

  function showdownSocket() {
    for (let i = sockets.length - 1; i >= 0; i--) {
      const entry = sockets[i];
      if (entry.showdown && entry.ws.readyState === 1) return entry;
    }
    return null;
  }

  function updateClassicControls(room, classicChoices) {
    try {
      room.choice = { choices: classicChoices, switchFlags: {}, switchOutFlags: {}, waiting: true };
      if (typeof room.closeNotification === 'function') room.closeNotification('choice');
      room.updateControls();
    } catch (e) {
      try {
        room.choice = { waiting: true };
        room.updateControls();
      } catch (e2) { /* the next request resets the controls anyway */ }
    }
  }

  /**
   * Submits a choice for request `rqid` in room `roomid`.
   * Classic client: room.send("/choose CHOICE|RQID"), then its controls switch to "waiting" with its own summary
   * of the choice (`classicChoices`: its per-slot form, "move 1 terastallize 2"); Cancel still works.
   * Preact client: its own "/choose" command with `preactChoice` (no "pass" parts), which validates the choice
   * and updates its controls; if it refuses, PS.send. Otherwise straight to the Showdown socket.
   */
  function play(cmd) {
    const { roomid, choice, rqid } = cmd;
    const command = '/choose ' + choice + (rqid !== null && rqid !== undefined ? '|' + rqid : '');
    const stale = (request) =>
      request && request.rqid !== undefined && rqid !== null && rqid !== undefined && request.rqid !== rqid;

    const app = classicApp();
    const classicRoom = app && app.rooms[roomid];
    if (classicRoom && typeof classicRoom.send === 'function' && classicRoom.battle) {
      if (stale(classicRoom.request)) {
        return { ok: false, error: 'the client is on request ' + classicRoom.request.rqid + ', the advice is for ' + rqid };
      }
      classicRoom.send(command);
      updateClassicControls(classicRoom, cmd.classicChoices || [choice]);
      return { ok: true, method: 'classic client' };
    }

    const ps = preactPS();
    const preactRoom = ps && ps.rooms[roomid];
    if (preactRoom && typeof preactRoom.send === 'function' && preactRoom.battle) {
      if (stale(preactRoom.request)) {
        return { ok: false, error: 'the client is on request ' + preactRoom.request.rqid + ', the advice is for ' + rqid };
      }
      try {
        preactRoom.send('/choose ' + (cmd.preactChoice || choice));
        if (preactRoom.choices && typeof preactRoom.choices.isDone === 'function' && preactRoom.choices.isDone()) {
          return { ok: true, method: 'Preact client' };
        }
      } catch (e) { /* fall through to a direct send */ }
      ps.send(command, roomid);
      return { ok: true, method: 'Preact client (direct)' };
    }

    const entry = showdownSocket();
    if (entry) {
      entry.ws.send(entry.decoder.encodeOutgoing(roomid + '|' + command));
      return { ok: true, method: 'socket' };
    }
    return { ok: false, error: 'no Showdown connection found in this page' };
  }

  // ------------------------------------------------------------- messages from the content script

  window.addEventListener('message', (event) => {
    if (event.source !== window || event.origin !== ORIGIN) return;
    const data = event.data;
    if (!data || data.__numbrion !== TAG || data.dir !== 'ext->page') return;
    if (data.type === 'hello') {
      const known = sessions.has(data.session);
      sessions.add(data.session);
      post({ type: 'hello-ack', session: data.session, version: 1 });
      if (known) return; // a repeated hello: the history was sent already
      // History for the new content script, addressed to it only.
      if (lastUserMessage) post({ type: 'msg', data: lastUserMessage, replayFor: data.session });
      for (const buffer of roomBuffers.values()) {
        for (const msg of buffer) {
          if (typeof msg === 'string') post({ type: 'msg', data: msg, replayFor: data.session });
          else post({ type: 'sent', data: msg.out, source: 'replay', replayFor: data.session });
        }
      }
      backfill();
      lastFocus = activeRoom();
      post({ type: 'focus', roomid: lastFocus });
    } else if (data.type === 'bye') {
      sessions.delete(data.session);
    } else if (data.type === 'play') {
      let result;
      try {
        result = play({
          roomid: String(data.roomid),
          choice: String(data.choice),
          rqid: data.rqid,
          classicChoices: Array.isArray(data.classicChoices) ? data.classicChoices.map(String) : null,
          preactChoice: typeof data.preactChoice === 'string' ? data.preactChoice : null,
        });
      } catch (e) {
        result = { ok: false, error: String((e && e.message) || e) };
      }
      // (The choice itself reaches the content script through the outgoing capture, as sent.)
      post({ type: 'play-result', id: data.id, roomid: data.roomid, rqid: data.rqid, choice: data.choice, result });
    } else if (data.type === 'focus?') {
      lastFocus = activeRoom();
      post({ type: 'focus', roomid: lastFocus });
    }
  });
})();
