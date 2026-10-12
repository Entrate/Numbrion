#!/usr/bin/env node
/*
 * A minimal Showdown opponent for testing the extension against a local server: logs in with /trn (the local
 * server's noguestsecurity), accepts every challenge and answers every request with "/choose default".
 *
 *     node tools/chrome-extension/dev/opponent.js [--name extopp] [--server ws://127.0.0.1:8000/showdown/websocket]
 *                                               [--delay-ms 1500] [--challenge USER]
 *
 * --challenge USER challenges USER to gen9randomdoublesbattle (repeated until a battle starts). On SIGINT/SIGTERM
 * it forfeits its open battles and exits. Needs Node 22+ (global WebSocket).
 */
'use strict';

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf('--' + name);
  return i >= 0 ? args[i + 1] : fallback;
};
const NAME = opt('name', 'extopp');
const SERVER = opt('server', 'ws://127.0.0.1:8000/showdown/websocket');
const DELAY = parseInt(opt('delay-ms', '1500'), 10);
const CHALLENGE = opt('challenge', null);
const FORMAT = 'gen9randomdoublesbattle';
const toId = (s) => String(s).toLowerCase().replace(/[^a-z0-9]/g, '');

const battles = new Set();
let challengeTimer = null;
const log = (...parts) => console.log(new Date().toISOString().slice(11, 23), ...parts);

const ws = new WebSocket(SERVER);
const send = (text) => {
  log('>>', text.slice(0, 120));
  ws.send(text);
};

ws.addEventListener('open', () => log('connected to', SERVER));
ws.addEventListener('close', () => {
  log('disconnected');
  process.exit(0);
});
ws.addEventListener('error', (event) => {
  log('socket error', event.message || '');
  process.exit(1);
});

ws.addEventListener('message', (event) => {
  const text = String(event.data);
  let roomid = '';
  let lines = text.split('\n');
  if (text[0] === '>') {
    roomid = lines[0].slice(1);
    lines = lines.slice(1);
  }
  for (const line of lines) {
    const parts = line.split('|');
    const type = parts[1];
    if (type === 'challstr') {
      send('|/trn ' + NAME + ',0,');
    } else if (type === 'updateuser' && toId(parts[2]) === toId(NAME)) {
      log('logged in as', parts[2].trim());
      if (CHALLENGE && !challengeTimer) {
        const challenge = () => {
          if (battles.size) return;
          send('|/utm null');
          send('|/challenge ' + CHALLENGE + ', ' + FORMAT);
        };
        challenge();
        challengeTimer = setInterval(challenge, 10000);
      }
    } else if (type === 'pm' && parts[4] && parts[4].startsWith('/challenge ' + FORMAT) && toId(parts[2]) !== toId(NAME)) {
      log('challenge from', parts[2].trim());
      send('|/utm null');
      send('|/accept ' + toId(parts[2]));
    } else if (type === 'init' && roomid.startsWith('battle-')) {
      battles.add(roomid);
      log('battle', roomid);
    } else if (type === 'request' && roomid) {
      const json = line.slice(9);
      if (!json || json === 'null') continue;
      const request = JSON.parse(json);
      if (request.wait) continue;
      setTimeout(() => {
        if (battles.has(roomid)) send(roomid + '|/choose default|' + request.rqid);
      }, DELAY);
    } else if (type === 'error' && roomid) {
      log('error in', roomid, line);
    } else if ((type === 'win' || type === 'tie') && roomid) {
      log('battle over', roomid, line);
      battles.delete(roomid);
      setTimeout(() => send('|/leave ' + roomid), 2000);
    } else if (type === 'popup') {
      log('popup', parts.slice(2).join('|').slice(0, 200));
    }
  }
});

function stop() {
  for (const roomid of battles) send(roomid + '|/forfeit');
  setTimeout(() => process.exit(0), 300);
}
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
