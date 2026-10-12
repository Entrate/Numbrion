#!/usr/bin/env node
/*
 * End-to-end test of the extension in Chrome for Testing against a local Showdown server.
 *
 * Prerequisites (docs/play/EXTENSION.md): a local Showdown server (tools/play/start-showdown.sh), an advisor API
 * (dev/mock_api.py or the play server), an opponent that accepts challenges (the play server's bots or
 * dev/opponent.js), and `npm install && npm run install-chrome` in this directory.
 *
 *     node e2e.js --api http://127.0.0.1:8766 [--client https://localhost.psim.us/?port=8000] [--user numbrionext]
 *                 [--opponent extopp] [--turns 3] [--screenshot ../../../scratch/extension-screenshot.png]
 *                 [--mock] [--sockjs http://127.0.0.1:8000/showdown [--sockjs-opponent BOT]]
 *                 [--late-install] [--headful] [--keep-open]
 *
 * Default run: the options page reaches the API; the panel appears in the battle; the advise request carries the
 * room's lines (from |init|battle on, no |request| lines, every line the client queued) and the client's current
 * request (--mock reads it back from mock_api.py's /api/_last); "Play this" submits a choice the server accepts
 * (the client shows its "waiting" controls, the next request arrives without an |error|) for --turns turns; the
 * bot log fills; the panel can be dragged and keeps its place across a page reload, after which the rejoined
 * battle is advised again; a dead API URL shows "API offline"; the end of the battle is shown. With --sockjs, a
 * bare page also opens a SockJS connection itself (the transport the client uses for non-localhost servers): the
 * hook must decode its frames, and "play" must answer through it.
 *
 * --late-install: the battle starts (and a turn is played) before the extension exists; then it is installed
 * into the running browser. The service worker injects it into the open tab, the hook wraps the client's
 * receive function (the socket already exists), the battle's history is rebuilt from the client's state, and
 * advice and "Play this" work from there on.
 */
'use strict';
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const puppeteer = require('puppeteer-core');
const { computeExecutablePath, Browser, detectBrowserPlatform } = require('@puppeteer/browsers');

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf('--' + name);
  return i >= 0 ? args[i + 1] : fallback;
};
const flag = (name) => args.includes('--' + name);
const EXT = path.resolve(__dirname, '..');
const API = opt('api', 'http://127.0.0.1:8765');
const CLIENT = opt('client', 'https://localhost.psim.us/');
const USER = opt('user', 'numbrionext');
const OPPONENT = opt('opponent', 'extopp');
const TURNS = parseInt(opt('turns', '3'), 10);
const SCREENSHOT = path.resolve(opt('screenshot', path.join(EXT, '..', '..', 'scratch', 'extension-screenshot.png')));
const MOCK = flag('mock');
const SOCKJS = opt('sockjs', null);
const SOCKJS_OPPONENT = opt('sockjs-opponent', OPPONENT);
const TAG = 'numbrion-advisor-v1';
const BUDGET = opt('budget', MOCK ? '1000' : '');
// --full-battle: play one whole battle; --human-turns 2,5 plays those turns with the client's own buttons.
const HUMAN_TURNS = new Set(String(opt('human-turns', '')).split(',').filter(Boolean).map(Number));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const log = (...parts) => console.log(new Date().toISOString().slice(11, 23), ...parts);
const results = [];
function check(ok, what, detail) {
  results.push({ ok: !!ok, what, detail });
  log(ok ? 'PASS' : 'FAIL', what, detail !== undefined ? JSON.stringify(detail).slice(0, 300) : '');
  return ok;
}

// ---------------------------------------------------------------------- helpers

/**
 * A copy of the extension whose default API URL is the test API, so a fresh install never calls another API
 * (such as a play server already running on the default port) before the options page is set.
 */
function extensionCopy(dir) {
  fs.cpSync(path.join(EXT, 'manifest.json'), path.join(dir, 'manifest.json'));
  fs.cpSync(path.join(EXT, 'options.html'), path.join(dir, 'options.html'));
  fs.cpSync(path.join(EXT, 'src'), path.join(dir, 'src'), { recursive: true });
  const settings = path.join(dir, 'src', 'settings.js');
  const text = fs.readFileSync(settings, 'utf8');
  const patched = text.replace(/apiUrl: '[^']*'/, 'apiUrl: ' + JSON.stringify(API));
  if (patched === text) throw new Error('could not set the default API URL in the extension copy');
  fs.writeFileSync(settings, patched);
  return dir;
}

function chromePath() {
  const cacheDir = path.join(__dirname, '.browsers');
  const builds = fs.existsSync(path.join(cacheDir, 'chrome')) ? fs.readdirSync(path.join(cacheDir, 'chrome')) : [];
  if (!builds.length) throw new Error('Chrome for Testing not found: run `npm run install-chrome` in ' + __dirname);
  const buildId = builds.sort().pop().split('-').pop();
  return computeExecutablePath({ browser: Browser.CHROME, buildId, cacheDir, platform: detectBrowserPlatform() });
}

async function panelState(page) {
  return page.evaluate(() => {
    const host = document.getElementById('numbrion-advisor-host');
    const root = host && host.shadowRoot;
    if (!root || host.style.display === 'none') return null;
    const panel = root.querySelector('.panel');
    const play = root.querySelector('.play');
    const rect = panel.getBoundingClientRect();
    const center = (el) => {
      const r = el.getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    };
    return {
      text: panel.innerText,
      status: (root.querySelector('.status .txt') || {}).textContent || '',
      right: (root.querySelector('.status .muted') || {}).textContent || '',
      choice: (root.querySelector('.choice code') || {}).textContent || null,
      playEnabled: !!play && !play.disabled,
      playBox: play ? center(play) : null,
      head: (() => { const r = root.querySelector('.head').getBoundingClientRect(); return { x: r.x + 60, y: r.y + r.height / 2 }; })(),
      rect: { left: rect.left, top: rect.top, width: rect.width },
      logEntries: root.querySelectorAll('.log').length,
    };
  });
}

async function clientRoom(page, roomid) {
  return page.evaluate((roomid) => {
    const room = window.app && app.rooms[roomid];
    if (!room) return null;
    return {
      rqid: room.request ? room.request.rqid : null,
      requestType: room.request ? room.request.requestType : null,
      side: room.request && room.request.side ? room.request.side.id : null,
      ended: !!room.battle.ended,
      waiting: !!(room.choice && room.choice.waiting),
      controls: room.$controls ? room.$controls.text() : '',
      steps: room.battle.stepQueue.slice(),
      // battle.turn follows the animation; the last |turn| line is the server's turn.
      lastTurn: Math.max(0, ...room.battle.stepQueue.filter((l) => l.startsWith('|turn|')).map((l) => parseInt(l.slice(6), 10))),
    };
  }, roomid);
}

/** Lines of `steps` (the classic client's queue) missing from `lines`, which must keep their order. */
function missingInOrder(lines, steps) {
  let i = 0;
  const missing = [];
  for (const step of steps) {
    if (!step) continue; // the client keeps empty lines, the extension drops them
    if (step.startsWith('|message|') && step.includes('is trapped')) continue; // synthesized by the client
    const at = lines.indexOf(step, i);
    if (at < 0) missing.push(step);
    else i = at + 1;
  }
  return missing;
}

/** localhost.psim.us is a public origin: Chrome's Local Network Access checks block its WebSocket to
 * ws://localhost unless the user allows it (a prompt; the "loopbackNetwork" permission in recent Chrome). */
async function grantLoopback(browser) {
  const cdp = await browser.target().createCDPSession();
  const origin = new URL(CLIENT).origin;
  // grantPermissions revokes what it does not list: one call with every name this Chrome knows.
  for (const permissions of [['loopbackNetwork', 'localNetwork'], ['loopbackNetwork'], ['localNetworkAccess']]) {
    try {
      await cdp.send('Browser.grantPermissions', { origin, permissions });
      return;
    } catch (e) { /* not known to this Chrome */ }
  }
}

async function login(page) {
  try {
    await page.waitForFunction(() => window.app && app.socket && app.socket.readyState === 1, { timeout: 60000, polling: 250 });
  } catch (e) {
    const state = await page.evaluate(() => ({ href: location.href, url: window.app && app.socket && app.socket.url, state: window.app && app.socket && app.socket.readyState }));
    throw new Error('the client did not connect: ' + JSON.stringify(state));
  }
  await page.evaluate((user) => {
    Storage.prefs('noanim', true); // no battle animations: the controls update at once
    app.send('/trn ' + user + ',0,');
  }, USER);
  await page.waitForFunction((user) => app.user && app.user.get('userid') === user, { timeout: 20000, polling: 250 }, USER);
}

async function openClient(browser) {
  const page = await browser.newPage();
  page.on('pageerror', (err) => {
    if (!/vntsm|ccgateway|script\.ac|queue|Uncaught \(in promise\) false/.test(String(err))) log('page error:', String(err).slice(0, 200));
  });
  page.on('console', (m) => {
    const text = m.text();
    if (!/^(<<|>>) /.test(text) && /WebSocket|LOCAL_NETWORK|numbrion/i.test(text)) log('console:', text.slice(0, 200));
  });
  await page.goto(CLIENT, { waitUntil: 'domcontentloaded', timeout: 90000 });
  await login(page);
  check(true, 'logged in on the local server as ' + USER);
  // Forfeit battles left over from an earlier run (the server rejoins them on login).
  await sleep(1500);
  const leftovers = await page.evaluate(() => Object.keys(app.rooms).filter((id) => id.startsWith('battle-')));
  for (const id of leftovers) {
    await page.evaluate((id) => {
      app.rooms[id].send('/forfeit');
      app.removeRoom(id);
      app.send('/leave ' + id);
    }, id);
  }
  if (leftovers.length) log('forfeited leftover battles', leftovers.join(', '));
  return page;
}

async function startBattle(page) {
  await page.evaluate((opponent) => {
    app.send('/utm null');
    app.send('/challenge ' + opponent + ', gen9randomdoublesbattle');
  }, OPPONENT);
  await page.waitForFunction(() => app.curRoom && app.curRoom.id.startsWith('battle-') && app.curRoom.request, { timeout: 30000, polling: 250 });
  const roomid = await page.evaluate(() => app.curRoom.id);
  check(true, 'battle started', roomid);
  return roomid;
}

async function waitForAdvice(page, rqid, timeout) {
  return page.waitForFunction((rqid) => {
    const root = document.getElementById('numbrion-advisor-host')?.shadowRoot;
    const play = root && root.querySelector('.play');
    const right = root && root.querySelector('.status .muted');
    return play && !play.disabled && right && right.textContent === 'rqid ' + rqid;
  }, { timeout: timeout || 30000, polling: 200 }, rqid);
}

/** Choices sent per room and rqid during this run (by "Play this" or the client's buttons). */
const played = new Map();

/** --mock: the advise body the mock API received last for the room matches the client's state. */
async function checkAdviseBody(roomid, before, label, expectComplete) {
  const last = await (await fetch(API + '/api/_last?room=' + roomid)).json();
  const body = last.body || {};
  const request = JSON.parse(body.request || 'null');
  const lines = body.lines || [];
  const missing = missingInOrder(lines, before.steps);
  const turnLines = lines.filter((l) => l.startsWith('|turn|'));
  check(request && request.rqid === before.rqid && request.side.id === before.side, label + 'advise request carries the client’s current request', { rqid: request && request.rqid, side: request && request.side.id });
  check(lines[0] === '|init|battle' && !lines.some((l) => l.startsWith('|request|')), label + 'advise lines start with |init|battle and exclude |request|', { n: lines.length });
  check(missing.length === 0, label + 'advise lines contain every battle line the client queued, in order', { missing: missing.slice(0, 5) });
  check(turnLines[turnLines.length - 1] === '|turn|' + before.lastTurn, label + 'advise lines reach the request’s turn', turnLines.slice(-1));
  check((last.problems || []).length === 0, label + 'mock API found no contract problems', last.problems);
  // history: every request so far, the last one the current request without a choice, earlier ones with the
  // choice the client sent.
  const history = body.history || [];
  const current = history[history.length - 1] || {};
  check(current.rqid === before.rqid && current.request === body.request && current.choice === null, label + 'history ends with the current request', { entries: history.length, last: current.rqid });
  const ours = played.get(roomid) || new Map();
  const wrong = history.slice(0, -1).filter((h) => ours.has(h.rqid) && h.choice !== ours.get(h.rqid)).map((h) => [h.rqid, h.choice, ours.get(h.rqid)]);
  const known = history.slice(0, -1).filter((h) => h.choice).length;
  check(wrong.length === 0 && [...ours.keys()].filter((rqid) => rqid < before.rqid).every((rqid) => history.some((h) => h.rqid === rqid)), label + 'history carries the choices sent for earlier requests', { known, wrong });
  check(history.every((h, i) => h.index <= lines.length && (i === 0 || h.index >= history[i - 1].index)), label + 'history indexes are in order and inside the lines');
  if (expectComplete !== undefined) {
    check(expectComplete ? known === history.length - 1 - history.slice(0, -1).filter((h) => JSON.parse(h.request).wait).length : true, label + (expectComplete ? 'every earlier move request has its choice' : 'history is partial, as expected'), { entries: history.length, known });
  }
}

/** Plays the turn by clicking the classic client's own buttons (first usable move, target or switch). */
async function playWithClientButtons(page, roomid, before) {
  for (let click = 0; click < 8; click++) {
    const state = await page.evaluate((roomid, rqid) => {
      const room = app.rooms[roomid];
      if ((room.choice && room.choice.waiting) || (room.request && room.request.rqid !== rqid)) return { done: true, choices: room.choice && room.choice.choices };
      const controls = room.$controls[0];
      const button = ['chooseMoveTarget', 'chooseMove', 'chooseSwitch']
        .map((name) => [...controls.querySelectorAll('button[name="' + name + '"]')].find((b) => !b.disabled && b.offsetParent))
        .find(Boolean);
      if (!button) return { done: false, none: true };
      button.setAttribute('data-e2e-click', String(Date.now()));
      return { done: false };
    }, roomid, before.rqid);
    if (state.done) return state.choices ? state.choices.filter(Boolean).join(',') : null;
    if (state.none) await sleep(200);
    else {
      await page.click('[data-e2e-click]');
      await page.evaluate(() => document.querySelectorAll('[data-e2e-click]').forEach((b) => b.removeAttribute('data-e2e-click')));
    }
  }
  return null;
}

/** Clicks "Play this" and waits for the server's next request. Returns the room after it. */
async function playTurn(page, roomid, before, panel) {
  await page.mouse.click(panel.playBox.x, panel.playBox.y);
  // The client waits for the opponent, unless the server answered at once (a forced switch nobody else makes).
  await page.waitForFunction((roomid, rqid) => {
    const room = app.rooms[roomid];
    return (room.choice && room.choice.waiting) || (room.request && room.request.rqid > rqid) || room.battle.ended;
  }, { timeout: 5000, polling: 50 }, roomid, before.rqid);
  const waiting = await clientRoom(page, roomid);
  if (waiting.rqid === before.rqid) {
    // (The client summarises move and switch choices; "default" has no summary.)
    const summary = panel.choice === 'default' || /will (use|switch|Terastallize)/.test(waiting.controls);
    check(waiting.waiting && /Waiting for opponent/.test(waiting.controls) && summary, 'classic client shows its "waiting" controls with the choice summary', waiting.controls.replace(/\s+/g, ' ').slice(0, 200));
    await page.waitForFunction(() => /Sent via/.test(document.getElementById('numbrion-advisor-host').shadowRoot.querySelector('.panel').innerText), { timeout: 5000, polling: 100 }).catch(() => {});
    const afterClick = await panelState(page);
    check(/Sent via classic client/.test(afterClick.text) && !afterClick.playEnabled, 'panel reports the submission and disables Play', afterClick.text.split('\n').find((l) => /Sent via/.test(l)));
  } else {
    log('  (the server answered before the client could show "waiting")');
  }
  const stepsBefore = before.steps.length;
  await page.waitForFunction((roomid, rqid) => {
    const room = app.rooms[roomid];
    return room.battle.ended || (room.request && room.request.rqid > rqid && !room.request.wait);
  }, { timeout: 60000, polling: 250 }, roomid, before.rqid);
  const after = await clientRoom(page, roomid);
  const errors = after.steps.slice(stepsBefore).filter((l) => l.startsWith('|error|'));
  check(errors.length === 0 && (after.ended || after.rqid > before.rqid), 'server accepted the choice (next request, no error)', { rqid: after.rqid, turn: after.lastTurn, errors });
  log('  played', panel.choice, '| moves:', after.steps.slice(stepsBefore).filter((l) => l.startsWith('|move|')).slice(0, 4).join('  '));
  return after;
}

async function configureApi(browser, extId) {
  const options = await browser.newPage();
  await options.goto('chrome-extension://' + extId + '/options.html');
  await options.waitForSelector('#apiUrl');
  await options.$eval('#apiUrl', (input, api) => {
    input.value = api;
    input.dispatchEvent(new Event('change'));
  }, API);
  await options.$eval('#budgetMs', (input, budget) => {
    input.value = budget; // '' = the server's default budget
    input.dispatchEvent(new Event('change'));
  }, BUDGET);
  await options.click('#check');
  await options.waitForFunction(() => /API online|offline|error/i.test(document.getElementById('status').textContent) && !/Checking/.test(document.getElementById('status').textContent), { timeout: 10000 });
  const statusText = await options.$eval('#status', (el) => el.textContent);
  check(/API online/.test(statusText), 'options page: API status check', statusText);
  await options.close();
}

async function forfeit(page, roomid) {
  await page.evaluate((roomid) => app.rooms[roomid] && app.rooms[roomid].send('/forfeit'), roomid);
  await sleep(1000);
  const ended = await panelState(page);
  check(/Battle over/.test(ended ? ended.status : ''), 'panel shows the end of the battle', ended && ended.status);
}

/**
 * A bare page (no Showdown client) on 127.0.0.1 opens its own SockJS connection to the server, logs in and starts
 * a battle; the hook must decode the SockJS frames into battle messages and send "play" through the socket.
 */
async function sockjsCheck(browser) {
  const http = require('node:http');
  const html = '<!DOCTYPE html><title>SockJS check</title><script src="https://play.pokemonshowdown.com/js/lib/sockjs-1.4.0-nwjsfix.min.js"></script>';
  const server = http.createServer((req, res) => {
    res.writeHead(200, { 'Content-Type': 'text/html' });
    res.end(html);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const page = await browser.newPage();
  try {
    await page.goto('http://127.0.0.1:' + server.address().port + '/sockjs-check.html', { waitUntil: 'load' });
    await page.waitForFunction(() => window.SockJS, { timeout: 30000 });
    const result = await page.evaluate((url, opponent, tag) => new Promise((resolve) => {
      const out = { transport: null, msgs: 0, battleMsgs: 0, requests: [], play: null, rqidAfter: null, errors: [] };
      const name = 'sockjscheck' + Math.floor(Math.random() * 1000);
      let played = false;
      const finish = () => resolve(out);
      setTimeout(finish, 45000);
      const sock = new SockJS(url);
      // What the page hook relays to the content script.
      window.addEventListener('message', (event) => {
        const d = event.data;
        if (!d || d.__numbrion !== tag || d.dir !== 'page->ext') return;
        if (d.type === 'msg') {
          out.msgs++;
          if (d.data.startsWith('>battle-')) out.battleMsgs++;
          const m = /^>(battle-[^\n]+)\n\|request\|(.+)$/s.exec(d.data);
          if (m) {
            const request = JSON.parse(m[2]);
            out.requests.push(request.rqid);
            if (!played && !request.wait) {
              played = true;
              window.postMessage({ __numbrion: tag, dir: 'ext->page', type: 'play', id: 1, roomid: m[1], choice: 'default', rqid: request.rqid }, location.origin);
            } else if (played && out.play && !request.wait && request.rqid > out.play.rqid) {
              out.rqidAfter = request.rqid;
              sock.send(m[1] + '|/forfeit');
              setTimeout(finish, 500);
            }
          }
          if (/\n\|error\|/.test(d.data)) out.errors.push(d.data.slice(0, 120));
        } else if (d.type === 'play-result') {
          out.play = { rqid: d.rqid, result: d.result };
        }
      });
      sock.onopen = () => { out.transport = sock.transport; };
      sock.onmessage = (event) => {
        const msg = event.data;
        if (msg.startsWith('|challstr|')) sock.send('|/trn ' + name + ',0,');
        if (msg.startsWith('|updateuser| ' + name + '|1|')) {
          sock.send('|/utm null');
          sock.send('|/challenge ' + opponent + ', gen9randomdoublesbattle');
        }
      };
    }), SOCKJS, SOCKJS_OPPONENT, TAG);
    check(result.transport === 'websocket' && result.battleMsgs > 0, 'SockJS page: the hook decodes SockJS frames into battle messages', { transport: result.transport, msgs: result.msgs, battleMsgs: result.battleMsgs });
    check(result.play && result.play.result.ok && result.play.result.method === 'socket', '"play" without a Showdown client goes through the socket', result.play);
    check(result.rqidAfter && !result.errors.length, 'the server accepted the SockJS-framed choice', { requests: result.requests, errors: result.errors });
  } finally {
    await page.close();
    server.close();
  }
}

// ---------------------------------------------------------------------- runs

async function normalRun(browser) {
  const worker = await browser.waitForTarget((t) => t.type() === 'service_worker' && t.url().endsWith('/src/background.js'), { timeout: 15000 });
  const extId = new URL(worker.url()).host;
  check(true, 'extension loaded', extId);
  await configureApi(browser, extId);

  await grantLoopback(browser);
  const page = await openClient(browser);
  const hooked = await page.evaluate(() => !!window.__numbrionAdvisorHook && WebSocket.toString().includes('native code'));
  check(hooked, 'page hook installed, WebSocket still looks native');
  const roomid = await startBattle(page);

  for (let turn = 1; turn <= TURNS; turn++) {
    const before = await clientRoom(page, roomid);
    if (before.ended) break;
    await waitForAdvice(page, before.rqid);
    const panel = await panelState(page);
    check(panel && panel.choice, `turn ${before.lastTurn}: panel shows advice for rqid ${before.rqid}`, { choice: panel.choice, status: panel.status });
    if (MOCK) await checkAdviseBody(roomid, before, '', true);
    if (!played.has(roomid)) played.set(roomid, new Map());
    check(!/Search needs the battle/.test(panel.text), 'no "search needs the battle from its start" note when the extension saw the whole battle');
    let after;
    if (turn === 2) {
      // This turn the user clicks the client's own buttons; the extension must record that choice too.
      const sent = await playWithClientButtons(page, roomid, before);
      check(!!sent, 'chose through the client\u2019s own buttons', sent);
      if (sent) played.get(roomid).set(before.rqid, sent);
      await page.waitForFunction((roomid, rqid) => {
        const room = app.rooms[roomid];
        return room.battle.ended || (room.request && room.request.rqid > rqid && !room.request.wait);
      }, { timeout: 60000, polling: 250 }, roomid, before.rqid);
      after = await clientRoom(page, roomid);
    } else {
      played.get(roomid).set(before.rqid, panel.choice);
      after = await playTurn(page, roomid, before, panel);
    }
    if (after.ended) break;
  }
  if (MOCK) {
    const now = await clientRoom(page, roomid);
    if (!now.ended) {
      await waitForAdvice(page, now.rqid);
      await checkAdviseBody(roomid, now, 'final: ', true);
      const meta = (await panelState(page)).text;
      check(/search/.test(meta) && /history: \d+ requests/.test(meta), 'with the whole history the (mock) advisor reports search', meta.split('\n').find((l) => /history/.test(l)));
    }
  }

  // Bot log: the opponent bot's decisions (the mock instead lists each advised decision once the next one is).
  await page.waitForFunction(() => document.getElementById('numbrion-advisor-host').shadowRoot.querySelectorAll('.log').length > 0, { timeout: 10000, polling: 250 }).catch(() => {});
  const logState = await panelState(page);
  const logLines = logState.text.split(/BOT LOG/i)[1];
  check(logState.logEntries > 0, 'bot log section lists entries', logLines ? logLines.trim().split('\n').slice(0, 4) : logState.logEntries);

  // Screenshot once the current request's advice is shown.
  const now = await clientRoom(page, roomid);
  if (!now.ended) await waitForAdvice(page, now.rqid);
  await sleep(1500);
  fs.mkdirSync(path.dirname(SCREENSHOT), { recursive: true });
  await page.screenshot({ path: SCREENSHOT });
  check(true, 'screenshot saved', SCREENSHOT);

  // Drag the panel, then reload: the position is kept and the rejoined battle is advised again.
  const p0 = await panelState(page);
  await page.mouse.move(p0.head.x, p0.head.y);
  await page.mouse.down();
  await page.mouse.move(p0.head.x - 300, p0.head.y + 60, { steps: 8 });
  await page.mouse.up();
  const p1 = await panelState(page);
  check(Math.abs(p1.rect.left - (p0.rect.left - 300)) < 3 && Math.abs(p1.rect.top - (p0.rect.top + 60)) < 3, 'panel can be dragged', { from: p0.rect, to: p1.rect });
  const reloadUrl = new URL(CLIENT);
  reloadUrl.pathname = '/' + roomid;
  await page.goto(reloadUrl.href, { waitUntil: 'domcontentloaded' });
  await login(page);
  await page.waitForFunction((roomid) => app.rooms[roomid] && app.rooms[roomid].request, { timeout: 30000, polling: 250 }, roomid);
  await page.evaluate((roomid) => app.focusRoom(roomid), roomid);
  const rejoined = await clientRoom(page, roomid);
  const readvised = await waitForAdvice(page, rejoined.rqid).then(() => true, () => false);
  const p2 = await panelState(page);
  check(p2 && Math.abs(p2.rect.left - p1.rect.left) < 3 && Math.abs(p2.rect.top - p1.rect.top) < 3, 'panel position restored after reload', p2 && p2.rect);
  check(readvised, 'rejoined battle (the server resends the whole log) is advised again', { rqid: rejoined.rqid, status: p2 && p2.status });

  // A dead API URL: "Advise now" reports the API as offline; then back.
  const sw = await worker.worker();
  const setApi = (url) => sw.evaluate(async (url) => {
    const { settings } = await chrome.storage.local.get('settings');
    await chrome.storage.local.set({ settings: Object.assign({}, settings, { apiUrl: url }) });
  }, url);
  await setApi('http://127.0.0.1:9');
  await page.evaluate(() => {
    const root = document.getElementById('numbrion-advisor-host').shadowRoot;
    [...root.querySelectorAll('.foot button')].find((b) => /Advise now/.test(b.textContent)).click();
  });
  const offline = await page.waitForFunction(() => {
    const text = document.getElementById('numbrion-advisor-host').shadowRoot.querySelector('.status .txt').textContent;
    return /API offline/.test(text) && text;
  }, { timeout: 15000, polling: 250 }).then((h) => h.jsonValue(), () => null);
  check(!!offline, 'a dead API URL shows "API offline"', offline);
  await setApi(API);
  const back = await waitForAdvice(page, rejoined.rqid, 15000).then(() => true, () => false);
  check(back, 'after fixing the URL the panel asks again at once');

  if (!flag('keep-open')) await forfeit(page, roomid);
  if (SOCKJS) await sockjsCheck(browser);
}

async function lateInstallRun(browser) {
  await grantLoopback(browser);
  const page = await openClient(browser);
  const roomid = await startBattle(page);
  // A turn without the extension.
  const first = await clientRoom(page, roomid);
  await page.evaluate((roomid, rqid) => app.rooms[roomid].send('/choose default|' + rqid), roomid, first.rqid);
  await page.waitForFunction((roomid, rqid) => {
    const room = app.rooms[roomid];
    return room.battle.ended || (room.request && room.request.rqid > rqid && !room.request.wait);
  }, { timeout: 60000, polling: 250 }, roomid, first.rqid);
  check(!(await page.evaluate(() => !!window.__numbrionAdvisorHook)), 'battle running before the extension exists');

  const extId = await browser.installExtension(EXT_UNDER_TEST);
  check(true, 'extension installed into the running browser', extId);
  await configureApi(browser, extId);
  await page.bringToFront();
  const hook = await page.waitForFunction(() => window.__numbrionAdvisorHook && app.receive.__numbrion, { timeout: 15000, polling: 100 }).then((h) => h.jsonValue(), () => null);
  check(hook === 'classic', 'late hook wraps the client’s receive function (the socket existed before)', hook);
  const before = await clientRoom(page, roomid);
  const advised = await waitForAdvice(page, before.rqid).then(() => true, () => false);
  const panel = await panelState(page);
  check(advised, 'panel shows advice for the battle that was already running', panel && { choice: panel.choice, status: panel.status });
  if (MOCK) await checkAdviseBody(roomid, before, 'backfilled: ', false);
  check(panel && /Search needs the battle from its start/.test(panel.text), 'panel notes that search needs the battle from its start', panel && panel.text.split('\n').find((l) => /Search needs/.test(l)));
  if (MOCK) {
    const meta = await page.evaluate(() => (document.getElementById('numbrion-advisor-host').shadowRoot.querySelector('.win .meta') || {}).textContent || '');
    check(/^policy/.test(meta), 'the (mock) advisor falls back to the policy', meta);
  }
  if (!advised) return;
  played.set(roomid, new Map([[before.rqid, panel.choice]]));
  const after = await playTurn(page, roomid, before, panel);
  if (!after.ended) {
    const next = await waitForAdvice(page, after.rqid).then(() => true, () => false);
    check(next, 'messages after the late install are captured (advice for the next request)', { rqid: after.rqid });
    if (MOCK && next) await checkAdviseBody(roomid, after, 'after late install: ', false);
  }
  if (!flag('keep-open')) await forfeit(page, roomid);
}

/** Waits until the panel answers request `rqid`: advice, no choice needed, an advisor error, or a timeout. */
async function waitForPanelAnswer(page, rqid, timeout) {
  return page.waitForFunction((rqid) => {
    const root = document.getElementById('numbrion-advisor-host')?.shadowRoot;
    if (!root) return false;
    const right = root.querySelector('.status .muted');
    if (!right || right.textContent !== 'rqid ' + rqid) return false;
    const play = root.querySelector('.play');
    if (play && !play.disabled) return 'advice';
    const text = root.querySelector('.status .txt').textContent;
    if (/no choice to make/.test(text)) return 'none';
    if (/Advisor error|API offline|timed out/.test(text)) return 'error: ' + text;
    return false;
  }, { timeout, polling: 200 }, rqid).then((h) => h.jsonValue(), () => 'timeout');
}

/** The advice the panel shows: source and search time from the analysis line, round trip, note. */
async function panelAdvice(page) {
  return page.evaluate(() => {
    const root = document.getElementById('numbrion-advisor-host').shadowRoot;
    const meta = (root.querySelector('.win .meta') || root.querySelector('.body > .muted') || {}).textContent || '';
    const status = root.querySelector('.status .txt').textContent;
    const note = root.querySelector('.note:not(.history)');
    const round = /round trip ([\d.]+) s/.exec(status);
    const time = /^\w+ \u00B7 ([\d.]+) s/.exec(meta);
    return {
      meta,
      source: meta.split(' \u00B7 ')[0] || '?',
      searchMs: time ? Math.round(parseFloat(time[1]) * 1000) : null,
      roundTripMs: round ? Math.round(parseFloat(round[1]) * 1000) : null,
      note: note ? note.textContent : '',
      historyNote: !!root.querySelector('.note.history'),
      choice: (root.querySelector('.choice code') || {}).textContent || null,
      slots: [...root.querySelectorAll('.slot')].map((el) => el.textContent),
      candidates: root.querySelectorAll('.cand').length,
      logHeads: [...root.querySelectorAll('.log .lh')].map((el) => el.textContent),
    };
  });
}

/**
 * --full-battle: one whole battle against --opponent with the real play server. Each actionable request: the
 * panel's answer (source, times, note) is recorded, then "Play this" (or the client's own buttons on
 * --human-turns) submits a choice the server must accept; after each turn the bot log must list the opponent
 * bot's decision with its analysis.
 */
async function fullBattleRun(browser) {
  const worker = await browser.waitForTarget((t) => t.type() === 'service_worker' && t.url().endsWith('/src/background.js'), { timeout: 15000 });
  await configureApi(browser, new URL(worker.url()).host);
  await grantLoopback(browser);
  const page = await openClient(browser);
  const roomid = await startBattle(page);
  const records = [];
  const errors = [];
  let screenshotTaken = false;
  let lastLogTurn = 0;
  for (let step = 0; step < 400; step++) {
    const room = await clientRoom(page, roomid);
    if (room.ended) break;
    if (room.requestType === 'wait' || room.requestType === 'team' || room.rqid === null) {
      await page.waitForFunction((roomid, rqid) => {
        const r = app.rooms[roomid];
        return r.battle.ended || (r.request && r.request.rqid !== rqid && r.request.requestType !== 'wait');
      }, { timeout: 120000, polling: 250 }, roomid, room.rqid);
      continue;
    }
    const answer = await waitForPanelAnswer(page, room.rqid, 180000);
    const advice = answer === 'advice' ? await panelAdvice(page) : null;
    const record = { turn: room.lastTurn, rqid: room.rqid, kind: room.requestType, answer, source: advice ? advice.source : answer, searchMs: advice && advice.searchMs, roundTripMs: advice && advice.roundTripMs, note: advice ? advice.note : '', choice: advice && advice.choice, by: 'advice' };
    if (advice && advice.historyNote) errors.push('turn ' + room.lastTurn + ': history note shown');
    records.push(record);
    log(`  T${record.turn} rqid ${record.rqid} ${record.kind}: ${record.source} ${record.searchMs ?? '-'} ms (round trip ${record.roundTripMs ?? '-'} ms) ${record.choice || ''}${record.note ? ' | note: ' + record.note : ''}`);

    // Bot log: the opponent bot's decisions up to the previous turn, with their analysis.
    const botLog = await (await fetch(API + '/api/bot-log?room=' + roomid)).json();
    const entries = botLog.entries || [];
    const newest = entries[entries.length - 1];
    if (room.lastTurn > 1 && room.requestType !== 'switch') {
      const ok = newest && newest.turn >= room.lastTurn - 1 && newest.decision && newest.decision.analysis && newest.decision.analysis.source;
      if (!ok) errors.push('turn ' + room.lastTurn + ': bot log lacks the previous turn (' + JSON.stringify(newest && { turn: newest.turn, source: newest.decision && newest.decision.analysis && newest.decision.analysis.source }) + ')');
      if (newest) lastLogTurn = newest.turn;
    }
    if (advice && advice.logHeads.length) record.panelLog = advice.logHeads[0];

    if (!screenshotTaken && advice && advice.source === 'search' && room.lastTurn >= 3 && advice.logHeads.length) {
      await sleep(800);
      fs.mkdirSync(path.dirname(SCREENSHOT), { recursive: true });
      await page.screenshot({ path: SCREENSHOT });
      screenshotTaken = true;
      log('  screenshot saved', SCREENSHOT);
      // And one scrolled down to the bot log.
      await page.evaluate(() => {
        const body = document.getElementById('numbrion-advisor-host').shadowRoot.querySelector('.body');
        body.scrollTop = body.scrollHeight;
      });
      const botLogShot = SCREENSHOT.replace(/\.png$/, '-botlog.png');
      await page.screenshot({ path: botLogShot });
      await page.evaluate(() => { document.getElementById('numbrion-advisor-host').shadowRoot.querySelector('.body').scrollTop = 0; });
      log('  screenshot saved', botLogShot);
    }

    const stepsBefore = room.steps.length;
    if (answer === 'advice' && !(room.requestType === 'move' && HUMAN_TURNS.has(room.lastTurn))) {
      await page.mouse.click((await panelState(page)).playBox.x, (await panelState(page)).playBox.y);
    } else {
      const sent = await playWithClientButtons(page, roomid, room);
      record.by = 'client buttons';
      record.human = sent;
      record.differs = advice ? sent !== advice.choice : null;
      log(`    played with the client's buttons: ${sent}${advice ? (record.differs ? ' (differs from the advice)' : ' (same as the advice)') : ''}`);
    }
    await page.waitForFunction((roomid, rqid) => {
      const r = app.rooms[roomid];
      return r.battle.ended || (r.request && r.request.rqid > rqid);
    }, { timeout: 180000, polling: 250 }, roomid, room.rqid);
    const after = await clientRoom(page, roomid);
    const errorLines = after.steps.slice(stepsBefore).filter((l) => l.startsWith('|error|'));
    if (errorLines.length) errors.push('rqid ' + room.rqid + ': ' + errorLines.join(' '));
  }
  const final = await clientRoom(page, roomid);
  const winner = await page.evaluate((roomid) => (app.rooms[roomid].battle.stepQueue.find((l) => l.startsWith('|win|')) || '|tie').slice(5), roomid);
  const finalLog = (await (await fetch(API + '/api/bot-log?room=' + roomid)).json()).entries || [];

  // Summary.
  const bySource = {};
  for (const r of records) {
    const key = r.kind + ':' + r.source;
    bySource[key] = (bySource[key] || 0) + 1;
  }
  const search = records.filter((r) => r.source === 'search');
  const stat = (values) => {
    const v = values.filter((x) => typeof x === 'number').sort((a, b) => a - b);
    if (!v.length) return null;
    return { n: v.length, min: v[0], median: v[Math.floor(v.length / 2)], p90: v[Math.floor(v.length * 0.9)], max: v[v.length - 1] };
  };
  const afterHuman = records.filter((r, i) => i > 0 && records.slice(0, i).some((x) => x.by === 'client buttons'));
  const summary = {
    room: roomid,
    ended: final.ended,
    winner,
    turns: final.lastTurn,
    requests: records.length,
    bySource,
    searchMs: stat(search.map((r) => r.searchMs)),
    roundTripMs: stat(records.map((r) => r.roundTripMs)),
    policyNotes: records.filter((r) => r.source !== 'search' && r.note).map((r) => `T${r.turn} ${r.kind} ${r.source}: ${r.note}`),
    humanTurns: records.filter((r) => r.by === 'client buttons').map((r) => ({ turn: r.turn, human: r.human, advice: r.choice, differs: r.differs })),
    sourcesAfterHumanTurns: afterHuman.reduce((acc, r) => ((acc[r.source] = (acc[r.source] || 0) + 1), acc), {}),
    panelBotLog: { requestsWithEntries: records.filter((r) => r.panelLog).length, newest: records.filter((r) => r.panelLog).slice(-3).map((r) => `at T${r.turn}: ${r.panelLog}`) },
    botLog: { entries: finalLog.length, sources: finalLog.reduce((acc, e) => { const s = (e.decision && e.decision.analysis && e.decision.analysis.source) || '?'; acc[s] = (acc[s] || 0) + 1; return acc; }, {}), lastTurn: lastLogTurn },
    errors,
  };
  console.log('SUMMARY ' + JSON.stringify(summary, null, 1));
  check(final.ended, 'the battle finished', { winner, turns: final.lastTurn });
  check(errors.length === 0, 'no server errors, history notes or missing bot log entries', errors.slice(0, 5));
  check(search.length > 0 && records.filter((r) => r.kind === 'move' && r.source !== 'search').length <= Math.max(1, Math.floor(records.length * 0.1)), 'move requests are advised by search', bySource);
  if (HUMAN_TURNS.size) check(afterHuman.length > 0 && afterHuman.filter((r) => r.kind === 'move').every((r) => r.source === 'search'), 'advice stays "search" after turns played with the client\u2019s buttons', summary.sourcesAfterHumanTurns);
  check(finalLog.length > 0 && finalLog.some((e) => e.decision && e.decision.analysis && e.decision.analysis.source === 'search'), 'bot log has the opponent bot\u2019s search analysis', summary.botLog);
}

let EXT_UNDER_TEST = EXT;

async function main() {
  const late = flag('late-install');
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'numbrion-ext-e2e-'));
  EXT_UNDER_TEST = extensionCopy(fs.mkdtempSync(path.join(os.tmpdir(), 'numbrion-ext-copy-')));
  const launchArgs = ['--window-size=1440,960', '--no-first-run', '--no-default-browser-check', '--mute-audio'];
  const browser = await puppeteer.launch({
    executablePath: chromePath(),
    headless: !flag('headful'),
    userDataDir: profile,
    defaultViewport: { width: 1440, height: 900 },
    ...(late
      ? { pipe: true, enableExtensions: true, args: [...launchArgs, '--enable-unsafe-extension-debugging'] }
      : { ignoreDefaultArgs: ['--disable-extensions'], args: [...launchArgs, '--disable-extensions-except=' + EXT_UNDER_TEST, '--load-extension=' + EXT_UNDER_TEST] }),
  });
  try {
    if (late) await lateInstallRun(browser);
    else if (flag('full-battle')) await fullBattleRun(browser);
    else await normalRun(browser);
  } finally {
    if (!flag('keep-open')) await browser.close();
    fs.rmSync(profile, { recursive: true, force: true });
    fs.rmSync(EXT_UNDER_TEST, { recursive: true, force: true });
  }
  const failed = results.filter((r) => !r.ok);
  log(`${results.length - failed.length}/${results.length} checks passed`);
  process.exit(failed.length ? 1 : 0);
}

main().catch((err) => {
  log('E2E aborted:', err && err.stack ? err.stack : err);
  process.exit(2);
});
