# Chrome extension

`tools/chrome-extension` is a Manifest V3 extension for the Showdown web client. While you play a battle it shows
what the advisor bot would choose in your position, and a "Play this" button submits that choice. In battles
against the play server's bots it also lists what those bots chose on earlier turns. It talks to the play
server's HTTP API, described in [ADVISOR-API.md](ADVISOR-API.md).

## Install

1. Open `chrome://extensions`, turn on Developer mode, click **Load unpacked** and pick `tools/chrome-extension`.
2. Click the extension's toolbar icon. The popup holds the settings: the API URL, `http://127.0.0.1:8765` by
   default, the search budget in ms, the advisor bot and three switches. **Check API** calls `/api/status`.
   The same page opens from the extension's Options entry.

Open the client at `https://localhost.psim.us/`. It connects to `ws://localhost:8000`. The first time, Chrome asks
whether `localhost.psim.us` may "access apps and services on this device", its local network permission. Allow it.
Without it Chrome blocks the connection to the local server and the client never logs in. A plain `http://` page
cannot get the permission at all. For a server on another port add `?port=8010` to the URL.

Keep the extension enabled before you start a battle. The advisor's search replays your own earlier choices to
guess the opponent's team, and the extension only knows the choices it saw you send. A battle it joins later,
or a battle tab reloaded after turn 1, gets the policy's advice instead, and the panel says so. Reload the tab
before turn 1 if the note appears at the start of a battle.

The extension also runs on `play.pokemonshowdown.com`, `*.psim.us`, `localhost` and `127.0.0.1` pages.
Installing it while a battle is open works without a page reload. The service worker attaches to open Showdown
tabs and rebuilds their battles from the client's state. After reloading the extension in `chrome://extensions`,
reload the battle tab if the panel does not come back; the server then resends the battle.

## Use

The panel appears in the battle tab you are looking at and hides elsewhere. Drag it by its title bar. It keeps
its place and its collapsed state. For each request it shows:

* the advised action per active pokemon, the raw choice text and the win probability,
* where the advice came from, search or policy, with its time, worlds and cells,
* the alternatives with their prior, searched value and final weight, and the opponent's likely replies,
* a status line: thinking, API offline, advisor errors, waiting for the opponent, battle over.

**Play this** submits the advised choice. The classic client then shows its usual "waiting for opponent" box
with its own summary of the choice, and its Cancel button still works. A choice the server rejects shows the
server's error in the panel.

The footer switches auto-advise and the bot log. With auto-advise off, **Advise now** asks for the current
request. The bot log lists `GET /api/bot-log` entries for the battle, newest first. Only battles that the play
server's bots take part in have entries. While a battle runs, the server hides the bots' choices and analysis,
and the panel shows only the turn, side, win estimate and search time. The choices appear when the battle ends.

## How it works

`src/page-hook.js` runs in the page's own JavaScript world at `document_start`, before the client's scripts. It
wraps `window.WebSocket` so each socket gets a message listener of its own, and it decodes SockJS frames
`o`, `h`, `a[...]`, `m"..."` and `c[...]`. It also wraps the client's receive function, `app.receive` in the
classic client and `PS.receive` in the Preact client. That catches transports that never touch a page
WebSocket, such as the Preact client's connection worker, and it is the only source when the extension arrives
after the page loaded. The hook skips messages it already saw on a socket. It also watches what the page sends:
`send()` on the wrapped sockets and the client's own `app.send` and `PS.send`. It keeps the battle commands
`ROOMID|/choose CHOICE|RQID`, `/team`, `/undo` and `/cancel`, which include the choices "Play this" makes.
Per battle room it keeps the messages and commands in order from the room's first `|init|`, and replays them
to the content script when that script says hello. When a battle room's start is missing, it rebuilds the room
from `battle.stepQueue` and the client's request; the choices made before then stay unknown.

`src/content.js` runs in the extension's isolated world. It keeps each battle room's lines without the
`|request|` and `|sentchoice|` lines, every request with the number of lines before it, and the last choice
sent for each rqid. An `/undo` or `/cancel` drops the current request's choice; a `/choose` without an rqid
counts for the current request. `src/protocol.js` holds that bookkeeping. A room counts as settled 150 ms after
its last message, because the server sends a request and the lines that lead to it back to back, in either
order. If the settled room is the active one and its request needs a choice, the script sends
`POST /api/advise` through the service worker, with `history`: every request so far with its choice, the
current one last. When the server resends a battle's log after a reconnect, the requests from before it move to
the same places in the new lines, counted in battle lines; chat, joins and timestamps may differ. If the log does
not match, the earlier history is dropped. It drops an answer whose rqid is no
longer the room's request. The two scripts talk through `window.postMessage` with a fixed tag and an origin
check.

`src/background.js` is the service worker and the only part that fetches the API. Its host permissions cover
`127.0.0.1` and `localhost`, so the page's CORS rules do not apply.

Chrome injects an extension file into a frame only once, even when two worlds list it. So the hook carries
its own transport code and `protocol.js` serves only the isolated world.

## Development

```sh
node --test tools/chrome-extension/test/*.test.js
python3 tools/chrome-extension/dev/mock_api.py --port 8766
node tools/chrome-extension/dev/opponent.js --name extopp --server ws://127.0.0.1:8000/showdown/websocket
```

The unit tests replay a recorded local-server battle, once as plain WebSocket frames and once re-framed as SockJS,
and every play-server recording in `scratch/showdown-logs` when there are any.
They also run the page hook in a sandbox with a fake socket and a fake classic client. The mock API answers
with a legal choice for each request and invented analysis. It checks every advise body against the API
contract, and `GET /api/_last?room=...` returns the last body. The opponent accepts challenges and plays
`/choose default`.

`dev/e2e.js` drives Chrome for Testing with the extension loaded against a local Showdown server:

```sh
cd tools/chrome-extension/dev && npm install && npm run install-chrome
node e2e.js --api http://127.0.0.1:8766 --mock --turns 4 --sockjs http://127.0.0.1:8000/showdown
node e2e.js --api http://127.0.0.1:8766 --mock --late-install
node e2e.js --api http://127.0.0.1:8765 --opponent NumbrionDefault
node e2e.js --full-battle --opponent NumbrionSearch --human-turns 2,5 --screenshot ../../../scratch/real.png
```

The first run plays four turns, one of them with the client's own buttons and the rest with "Play this". It
checks the advise bodies against the client's state, including `history` and the choices sent, then the bot
log, dragging, a page reload, a dead API URL and a bare page that speaks SockJS. The second starts the
battle before the extension exists and installs it into the running browser, so the panel must show the
"search needs the battle from its start" note. The script loads a temporary copy of the extension whose default
API URL is `--api`, so a fresh install never calls a play server on the default port. The third uses the real play
server. `--full-battle` plays one whole battle against the real play server. It uses "Play this", except on the
`--human-turns`, where it clicks the client's own buttons. It prints each request's advice source, search time
and round trip, any policy note, and whether the bot log had the opponent's previous turn. Branded Chrome ignores `--load-extension`, so the script uses Chrome for Testing.

## Limitations

* Only the classic client was tested in a browser. The Preact client paths, its receive hook and its `/choose`
  command, follow its source and fall back to sending through the socket.
* Team preview is not advised. Random Doubles has none.
* A battle joined without its start, with no client state to rebuild it from, is not advised. The panel asks
  for a page reload, and the server then resends the whole log.
* The choices live in the page. Reloading the tab mid-battle loses them, since the server resends the lines
  but not the past requests, so search falls back to the policy for the rest of that battle.
* When "Play this" cannot go through the client's own choice code, it sends `/choose` straight to the server.
  The client's buttons then stay up until the next request resets them. That happens when the Preact client
  rejects the choice text, or on a page without a known client.
* Only installing into open tabs was tested. Reloading the extension relies on the same `onInstalled` code,
  but the test harness could not drive `chrome.runtime.reload()` reliably.
