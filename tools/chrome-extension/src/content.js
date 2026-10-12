/*
 * ISOLATED-world content script: receives the server messages captured by page-hook.js, keeps the battle rooms
 * (protocol.js BattleStore), asks the service worker for advice and the bot log, and draws the panel.
 *
 * Advice timing: Showdown sends a request and the battle lines that lead to it back to back (in either order),
 * so a room is "settled" 150 ms after its last message; then, if its latest request is actionable and was not
 * asked about yet, POST /api/advise goes out with every line so far and that request. An answer whose rqid is no
 * longer the room's current request is dropped.
 */
(function () {
  'use strict';
  const P = globalThis.NumbrionProtocol;
  const Settings = globalThis.NumbrionSettings;
  const TAG = 'numbrion-advisor-v1';
  const ORIGIN = window.location.origin;
  const SETTLE_MS = 150;
  const session = Math.random().toString(36).slice(2) + Date.now().toString(36);

  const store = new P.BattleStore();
  let settings = Settings.normalize(null);
  let activeRoomId = '';
  let hookReady = false;
  let alive = true;
  let apiState = { kind: 'unknown', text: '', checkedAt: 0 };
  const advice = new Map(); // roomid -> {rqid, seq, status, result, error, startedAt, finishedAt, roundTripMs}
  const botLogs = new Map(); // roomid -> {entries, error, fetchedAt, inflight, turn, seq}
  const playState = new Map(); // roomid -> {rqid, status, message}
  const settleTimers = new Map();
  let renderQueued = false;
  let tickTimer = null;
  let statusTimer = null;

  const panel = new globalThis.NumbrionPanel({
    onPlay: () => playActive(),
    onAdviseNow: () => adviseActive(true),
    onToggle: (key, value) => saveSettings({ [key]: value }),
    onCollapse: (collapsed) => storageSet({ panelCollapsed: collapsed }),
  });

  // ------------------------------------------------------------- extension plumbing

  function extensionAlive() {
    try {
      return !!(chrome.runtime && chrome.runtime.id);
    } catch (e) {
      return false;
    }
  }

  /** The extension was reloaded or removed (this copy is orphaned), or a newer copy took over. */
  function shutdown() {
    if (!alive) return;
    alive = false;
    window.removeEventListener('message', onPageMessage);
    for (const timer of settleTimers.values()) clearTimeout(timer);
    if (tickTimer) clearInterval(tickTimer);
    if (statusTimer) clearInterval(statusTimer);
    postToPage({ type: 'bye', session });
    panel.destroy();
  }

  // One copy per page: the service worker injects this script into open tabs after an install or update, which
  // can race with the manifest's own injection. The newest copy wins; the page hook replays the history to it.
  const previous = globalThis.__numbrionAdvisorContent;
  if (previous && typeof previous.shutdown === 'function') {
    try {
      previous.shutdown();
    } catch (e) { /* orphaned copy */ }
  }
  globalThis.__numbrionAdvisorContent = { session, shutdown };

  function sendToWorker(message) {
    if (!extensionAlive()) {
      shutdown();
      return Promise.resolve({ ok: false, error: 'extension reloaded; reload this page' });
    }
    return new Promise((resolve) => {
      try {
        chrome.runtime.sendMessage(message, (response) => {
          const error = chrome.runtime.lastError;
          if (error) resolve({ ok: false, error: error.message });
          else resolve(response || { ok: false, error: 'no response' });
        });
      } catch (e) {
        shutdown();
        resolve({ ok: false, error: String((e && e.message) || e) });
      }
    });
  }

  function storageSet(values) {
    try {
      chrome.storage.local.set(values);
    } catch (e) { /* orphaned */ }
  }

  /** Saved to storage; storage.onChanged (below) applies it here and in other tabs. */
  async function saveSettings(patch) {
    try {
      await Settings.save(patch);
    } catch (e) { /* orphaned */ }
  }

  function afterSettingsChange() {
    // A new API URL or bot: ask again where the last ask failed.
    for (const [roomid, entry] of advice) {
      if (entry.status === 'offline' || entry.status === 'error') advice.delete(roomid);
    }
    apiState = { kind: 'unknown', text: '', checkedAt: 0 };
    if (settings.enabled) {
      checkApi();
      adviseActive(false);
      refreshBotLog(activeRoomId);
    }
    queueRender();
  }

  function postToPage(payload) {
    payload.__numbrion = TAG;
    payload.dir = 'ext->page';
    window.postMessage(payload, ORIGIN);
  }

  // ------------------------------------------------------------- page messages

  function onPageMessage(event) {
    if (event.source !== window || event.origin !== ORIGIN) return;
    const data = event.data;
    if (!data || data.__numbrion !== TAG || data.dir !== 'page->ext') return;
    if (!extensionAlive()) {
      shutdown();
      return;
    }
    switch (data.type) {
      case 'hello-ack':
        if (data.session === session) hookReady = true;
        break;
      case 'msg':
        if (data.replayFor && data.replayFor !== session) return;
        onServerMessage(String(data.data));
        break;
      case 'sent': {
        // A battle command the page sent: the user's choices make up the advise "history".
        if (data.replayFor && data.replayFor !== session) return;
        const room = store.noteOutgoing(String(data.data), data.source);
        if (room) queueRender();
        break;
      }
      case 'focus':
        setActiveRoom(String(data.roomid || ''));
        break;
      case 'play-result':
        onPlayResult(data);
        break;
      default:
        break;
    }
  }

  function onServerMessage(msg) {
    const result = store.receive(msg);
    if (!result) return;
    const { roomid } = result;
    if (result.deinit) {
      advice.delete(roomid);
      botLogs.delete(roomid);
      playState.delete(roomid);
      clearTimeout(settleTimers.get(roomid));
      settleTimers.delete(roomid);
      queueRender();
      return;
    }
    if (result.error && result.room) {
      const play = playState.get(roomid);
      if (play && play.status === 'sent') {
        playState.set(roomid, Object.assign({}, play, { status: 'rejected', message: 'Server: ' + result.error }));
      }
    }
    clearTimeout(settleTimers.get(roomid));
    settleTimers.set(roomid, setTimeout(() => settled(roomid), SETTLE_MS));
  }

  /** The room got no message for SETTLE_MS: its request and lines belong together. */
  function settled(roomid) {
    settleTimers.delete(roomid);
    if (!alive) return;
    const room = store.get(roomid);
    if (!room) return;
    if (roomid === activeRoomId) {
      adviseActive(false);
      const log = botLogs.get(roomid);
      const requestSeq = room.request ? room.request.seq : 0;
      if (!log || log.turn !== room.turn || log.seq !== requestSeq || (room.ended && !log.ended)) {
        refreshBotLog(roomid);
        // Bot decisions are listed once the server sent the bot its next request; look again shortly.
        setTimeout(() => refreshBotLog(roomid), 1500);
      }
    }
    queueRender();
  }

  function setActiveRoom(roomid) {
    if (roomid === activeRoomId) return;
    activeRoomId = roomid;
    if (roomid && store.get(roomid)) {
      adviseActive(false);
      refreshBotLog(roomid);
    }
    queueRender();
  }

  // ------------------------------------------------------------- advice

  function adviseActive(force) {
    if (!alive) return;
    const roomid = activeRoomId;
    if (!roomid || !settings.enabled) return;
    const room = store.get(roomid);
    if (!room || !room.request || !P.isActionable(room.request) || room.ended) return;
    if (!room.complete) {
      advice.set(roomid, { rqid: room.request.rqid, seq: room.request.seq, status: 'incomplete' });
      queueRender();
      return;
    }
    const current = advice.get(roomid);
    const sameRequest = current && current.seq === room.request.seq && current.rqid === room.request.rqid;
    if (!force) {
      if (!settings.autoAdvise) return;
      if (sameRequest && current.status !== 'offline') return;
      // API offline: retry on later messages, at most every 5 s.
      if (sameRequest && Date.now() - (current.finishedAt || 0) < 5000) return;
    }
    if (current && current.status === 'thinking' && sameRequest && !force) return;
    const body = P.buildAdviseBody(room, {}, store);
    if (!body) return;
    const entry = { rqid: room.request.rqid, seq: room.request.seq, status: 'thinking', startedAt: Date.now() };
    advice.set(roomid, entry);
    ensureTicking();
    queueRender();
    sendToWorker({ type: 'advise', body }).then((response) => {
      const latest = store.get(roomid);
      if (advice.get(roomid) !== entry) return; // superseded by a newer ask
      if (!latest || !latest.request || latest.request.rqid !== entry.rqid || latest.request.seq !== entry.seq) {
        advice.delete(roomid); // stale: the room moved on to another request
        queueRender();
        return;
      }
      entry.finishedAt = Date.now();
      entry.roundTripMs = entry.finishedAt - entry.startedAt;
      if (response && response.ok) {
        const data = response.data || {};
        if (data.rqid !== undefined && data.rqid !== null && data.rqid !== entry.rqid) {
          entry.status = 'error';
          entry.error = 'answer for request ' + data.rqid + ', expected ' + entry.rqid;
        } else {
          entry.status = data.decision ? 'done' : 'nochoice';
          entry.result = data;
        }
        apiState = { kind: 'ok', text: 'API online', checkedAt: Date.now() };
      } else {
        entry.status = response && response.offline ? 'offline' : 'error';
        entry.error = (response && response.error) || 'no answer';
        if (response && response.offline) apiState = { kind: 'offline', text: entry.error, checkedAt: Date.now() };
      }
      queueRender();
    });
  }

  async function refreshBotLog(roomid) {
    if (!alive || !roomid || !settings.enabled || !settings.showBotLog) return;
    const room = store.get(roomid);
    if (!room) return;
    const log = botLogs.get(roomid) || { entries: [], error: null };
    if (log.inflight) {
      log.again = true;
      return;
    }
    log.inflight = true;
    log.turn = room.turn;
    log.seq = room.request ? room.request.seq : 0;
    log.ended = room.ended;
    botLogs.set(roomid, log);
    const response = await sendToWorker({ type: 'botLog', room: roomid });
    log.inflight = false;
    log.fetchedAt = Date.now();
    if (response.ok) {
      log.entries = Array.isArray(response.data.entries) ? response.data.entries : [];
      log.error = null;
    } else {
      log.error = response.offline ? 'Bot log unavailable (API offline)' : 'Bot log: ' + response.error;
    }
    if (log.again) {
      log.again = false;
      refreshBotLog(roomid);
    }
    queueRender();
  }

  async function checkApi() {
    if (!alive || !settings.enabled) return;
    const response = await sendToWorker({ type: 'status' });
    if (response.ok) {
      const d = response.data || {};
      apiState = { kind: 'ok', text: 'API online' + (d.advisor ? ' · advisor ' + d.advisor : ''), checkedAt: Date.now() };
    } else {
      apiState = { kind: response.offline ? 'offline' : 'error', text: response.error, checkedAt: Date.now() };
    }
    queueRender();
  }

  // ------------------------------------------------------------- play

  function playActive() {
    const roomid = activeRoomId;
    const room = store.get(roomid);
    const entry = advice.get(roomid);
    if (!room || !entry || entry.status !== 'done' || !room.request) return;
    if (room.request.rqid !== entry.rqid) return;
    const choice = entry.result.decision.choice;
    playState.set(roomid, { rqid: entry.rqid, status: 'pending', message: 'Submitting…' });
    postToPage({
      type: 'play',
      id: Date.now(),
      roomid,
      choice,
      rqid: entry.rqid,
      // The forms the clients keep their own choices in (see page-hook.js play()).
      classicChoices: P.splitChoice(choice).map(P.toClassicChoice),
      preactChoice: P.splitChoice(choice).filter((part) => part !== 'pass').join(', '),
    });
    queueRender();
  }

  function onPlayResult(data) {
    const roomid = String(data.roomid);
    const result = data.result || {};
    playState.set(roomid, {
      rqid: data.rqid,
      status: result.ok ? 'sent' : 'failed',
      message: result.ok ? 'Sent via ' + result.method + '; waiting for the server…' : 'Not sent: ' + result.error,
    });
    queueRender();
  }

  // ------------------------------------------------------------- view

  function ensureTicking() {
    if (tickTimer) return;
    tickTimer = setInterval(() => {
      const thinking = [...advice.values()].some((entry) => entry.status === 'thinking');
      if (!thinking) {
        clearInterval(tickTimer);
        tickTimer = null;
      }
      queueRender();
    }, 250);
  }

  function queueRender() {
    if (renderQueued || !alive) return;
    renderQueued = true;
    requestAnimationFrame(() => {
      renderQueued = false;
      if (!alive) return;
      try {
        panel.render(buildView());
      } catch (e) {
        console.warn('[numbrion advisor] render failed', e);
      }
    });
  }

  const num = (x) => (typeof x === 'number' && Number.isFinite(x) ? x : null);
  const seconds = (ms) => (ms / 1000).toFixed(ms < 10000 ? 2 : 1) + ' s';

  function adviceView(room, entry) {
    const data = entry.result || {};
    const decision = data.decision;
    if (!decision) return null;
    const analysis = decision.analysis || {};
    let slots = Array.isArray(analysis.slots) ? analysis.slots.map((slot) => ({ pokemon: slot.pokemon, text: slot.text })) : [];
    if (!slots.length) slots = P.describeChoice(decision.choice, room.request && room.request.data, room);
    let win = num(analysis.win_probability);
    if (win === null && num(analysis.value) !== null) win = (analysis.value + 1) / 2;
    const meta = [];
    if (analysis.source) meta.push(analysis.source);
    if (num(analysis.time_ms) !== null) meta.push(seconds(analysis.time_ms));
    if (num(analysis.worlds) !== null) meta.push(analysis.worlds + ' worlds');
    if (num(analysis.cells) !== null) meta.push(analysis.cells + ' cells');
    if (num(analysis.value) !== null) meta.push('value ' + (analysis.value >= 0 ? '+' : '') + analysis.value.toFixed(2));
    const candidates = (Array.isArray(analysis.candidates) ? analysis.candidates : []).slice(0, 6).map((cand, index) => ({
      text: cand.text || cand.choice,
      choice: cand.choice,
      prior: num(cand.prior),
      value: num(cand.value),
      weight: num(cand.weight),
      best: cand.choice ? cand.choice === decision.choice : index === 0,
    }));
    const opponent = (Array.isArray(analysis.opponent) ? analysis.opponent : []).map((reply) => ({
      text: reply.text,
      prior: num(reply.prior),
      weight: num(reply.weight),
    }));
    const summary = slots.map((slot) => slot.text).join(' / ') + (win !== null ? ' · ' + Math.round(win * 100) + '%' : '');
    return {
      choice: decision.choice,
      slots,
      winProbability: win,
      meta: meta.join(' · '),
      candidates,
      opponent,
      note: analysis.note || '',
      summary,
    };
  }

  function botLogView(roomid) {
    if (!settings.showBotLog) return { enabled: false };
    const log = botLogs.get(roomid);
    if (!log) return { enabled: true, entries: [], emptyText: 'Loading…' };
    const entries = (log.entries || []).slice().reverse().map((entry) => {
      const decision = entry.decision || {};
      const analysis = decision.analysis || {};
      const head = ['T' + (entry.turn !== undefined ? entry.turn : '?'), entry.bot || 'bot', entry.side || ''];
      const win = num(analysis.win_probability) !== null ? analysis.win_probability : num(analysis.value) !== null ? (analysis.value + 1) / 2 : null;
      if (win !== null) head.push(Math.round(win * 100) + '%');
      if (analysis.source) head.push(analysis.source + (num(analysis.time_ms) !== null ? ' ' + seconds(analysis.time_ms) : ''));
      const slots = Array.isArray(analysis.slots) ? analysis.slots : [];
      let text = slots.length ? slots.map((slot) => (slot.pokemon ? slot.pokemon + ': ' : '') + slot.text).join(' / ') : decision.choice || (entry.redacted ? 'choice hidden until the battle ends' : '(no choice)');
      if (analysis.note) text += ' — ' + analysis.note;
      return { head: head.filter(Boolean).join(' · '), text, choice: decision.choice };
    });
    return { enabled: true, entries, error: log.error, emptyText: 'No bot decisions in this room (only battles of the play server’s bots have them).' };
  }

  function buildView() {
    const roomid = activeRoomId;
    const room = roomid ? store.get(roomid) : null;
    if (!settings.enabled || !room) return { visible: false };
    const request = room.request;
    const entry = advice.get(roomid);
    const view = {
      visible: true,
      roomid,
      turn: room.turn,
      settings,
      header: '',
      status: { kind: 'idle', text: '' },
      advice: null,
      play: null,
      sent: null,
      historyNote: null,
      botLog: botLogView(roomid),
    };
    if (room.complete && !room.ended && request && P.isActionable(request) && !store.historyComplete(room)) {
      // The search replays the user's past choices; the extension only knows those it saw being sent.
      view.historyNote = 'Search needs the battle from its start: reload the tab before turn 1. In this battle the advice falls back to the policy.';
    }
    const players = Object.values(room.players).join(' vs ') || room.title || roomid;
    const side = room.mySide;
    view.header = (room.turn ? 'T' + room.turn + ' · ' : '') + players + (side ? ' · you: ' + side : '');

    const fresh = entry && request && entry.seq === request.seq && entry.rqid === request.rqid;
    const right = request && request.rqid !== null ? 'rqid ' + request.rqid : '';
    if (room.ended) {
      view.status = { kind: 'idle', text: room.winner ? 'Battle over: ' + room.winner + ' won' : 'Battle over: tie' };
    } else if (!room.complete) {
      view.status = { kind: 'warn', text: 'Joined mid-battle without its history; reload the page to advise.' };
    } else if (!request) {
      view.status = { kind: 'idle', text: 'Spectating (no request for you in this battle)' };
    } else if (request.kind === 'wait') {
      view.status = { kind: 'idle', text: 'Waiting for the opponent', right };
    } else if (request.kind === 'team') {
      view.status = { kind: 'idle', text: 'Team preview (not advised)', right };
    } else if (!P.isActionable(request)) {
      view.status = { kind: 'idle', text: 'No choice needed', right };
    } else if (!fresh) {
      view.status = settings.autoAdvise
        ? { kind: 'busy', text: 'Waiting for the turn’s messages…', right }
        : { kind: 'idle', text: 'Auto-advise is off: click “Advise now”', right };
    } else if (entry.status === 'thinking') {
      view.status = { kind: 'busy', text: 'Thinking… ' + seconds(Date.now() - entry.startedAt), right };
    } else if (entry.status === 'offline') {
      view.status = { kind: 'error', text: entry.error + ' — start the play server or fix the URL in the options', right };
    } else if (entry.status === 'error') {
      view.status = { kind: 'error', text: 'Advisor error: ' + entry.error, right };
    } else if (entry.status === 'nochoice') {
      view.status = { kind: 'idle', text: 'The advisor has no choice to make for this request', right };
    } else if (entry.status === 'done') {
      view.status = { kind: 'ok', text: 'Advice for this request · round trip ' + seconds(entry.roundTripMs || 0), right };
    }
    if (apiState.kind === 'offline' && (!fresh || entry.status !== 'thinking') && view.status.kind !== 'error' && !room.ended) {
      view.status = { kind: 'error', text: apiState.text, right };
    }

    if (fresh && entry.status === 'done') {
      view.advice = adviceView(room, entry);
      const play = playState.get(roomid);
      const samePlay = play && play.rqid === entry.rqid;
      const sent = room.sentChoice && room.sentChoice.rqid === entry.rqid;
      view.play = {
        enabled: !room.ended && !(samePlay && (play.status === 'pending' || play.status === 'sent')) && !sent,
        label: samePlay && play.status === 'sent' ? 'Sent' : samePlay && play.status === 'pending' ? 'Sending…' : 'Play recommended',
        message: samePlay ? play.message : '',
        messageKind: samePlay ? (play.status === 'sent' ? 'ok' : play.status === 'pending' ? 'info' : 'error') : 'info',
      };
      if (sent && !samePlay) view.play.message = 'A choice was already sent for this request.';
      if (room.lastError && room.lastError.rqid === entry.rqid) {
        view.play.message = 'Server: ' + room.lastError.text;
        view.play.messageKind = 'error';
        view.play.enabled = !room.ended;
      }
    }
    if (room.sentChoice && request && room.sentChoice.rqid === request.rqid) {
      const matches = view.advice && room.sentChoice.choice.replace(/\s+/g, ' ') === view.advice.choice.replace(/\s+/g, ' ');
      view.sent = 'Sent: ' + room.sentChoice.choice + (view.advice ? (matches ? ' (the advice)' : ' (differs from the advice)') : '');
    }
    return view;
  }

  // ------------------------------------------------------------- start

  async function start() {
    try {
      const stored = await chrome.storage.local.get([Settings.KEY, 'panelCollapsed']);
      settings = Settings.normalize(stored[Settings.KEY]);
      panel.collapsed = !!stored.panelCollapsed;
    } catch (e) { /* defaults */ }
    chrome.storage.onChanged.addListener((changes, area) => {
      if (area !== 'local' || !alive) return;
      if (changes[Settings.KEY]) {
        settings = Settings.normalize(changes[Settings.KEY].newValue);
        afterSettingsChange();
      }
    });
    checkApi();
    statusTimer = setInterval(() => {
      if (alive && activeRoomId && apiState.kind !== 'ok' && Date.now() - apiState.checkedAt > 10000) checkApi();
    }, 5000);
    queueRender();
  }

  window.addEventListener('message', onPageMessage);
  // Say hello (once the settings are loaded) until the page hook answers; it may be injected after this script.
  // The hook keeps the battle history and replays it on hello, so nothing is lost meanwhile.
  const hello = () => {
    if (!alive || hookReady) return;
    postToPage({ type: 'hello', session });
    setTimeout(hello, 1000);
  };
  start().then(hello, hello);
})();
