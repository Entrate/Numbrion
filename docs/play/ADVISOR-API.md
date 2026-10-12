# Play server HTTP API (advisor and bot log)

The play server (`crates/pyengine/play/server.py`) runs the bot accounts on a Showdown server and an HTTP API on
`http://127.0.0.1:8765` (`--api-port`). The Chrome extension (`tools/chrome-extension`) uses it to show what a bot
would choose in the user's position and what the bots chose in their own battles.

All bodies are JSON. Every response has `"ok": true` or `"ok": false, "error": "<text>"`. The server sends
`Access-Control-Allow-Origin: *` and answers `OPTIONS` preflights, but the extension calls it from its service
worker (host permission for `http://127.0.0.1/*`), not from the page.

## `POST /api/advise`

What the advisor bot would choose for the user's current request.

```json
{
  "room": "battle-gen9randomdoublesbattle-123",
  "lines": ["|init|battle", "|title|...", "|j|...", "|player|p1|Alice|1", "...", "|turn|3"],
  "request": "{\"active\": [...], \"side\": {...}, \"rqid\": 7}",
  "budget_ms": 1500,
  "bot": null
}
```

* `lines`: every protocol line the client received for this room, from the first, in order, **without** the
  `|request|` lines (ladder-only lines are fine: chat, timestamps, `|j|`, `|raw|`, ...). The server keeps a
  player per room and feeds only the new suffix; if the stored prefix differs it rebuilds the room.
* `request`: the latest `|request|` JSON text for the user's side, verbatim.
* `budget_ms` (optional): search time budget; `bot` (optional): bot name from `/api/status`, default the
  server's advisor bot.
* `history` (needed for search; without it the advice is the policy alone): every `|request|` the client
  received for this room, in order, each with the user's final choice for it:

  ```json
  "history": [
    {"index": 41, "rqid": 2, "request": "{...}", "choice": "move 1 +1, move 3"},
    {"index": 77, "rqid": 3, "request": "{...}", "choice": "switch 4, pass"},
    {"index": 120, "rqid": 4, "request": "{...}", "choice": null}
  ]
  ```

  `index` is the number of entries of `lines` received before that request arrived (its lines are
  `lines[:index]`). `choice` is the text of the last `/choose` the user sent for that rqid (without
  `/choose` and `|rqid`), `null` when none was seen (the current request, a `wait` request, a choice made by
  the timer or by `default`). The last entry is the current request. The search's opponent-team sampler replays
  our own past choices, so it needs them; when a past choice is missing the advice falls back to the policy.

  The server replays the history through the room's player: for each earlier entry, feed `lines[:index]` not
  yet fed, `set_request(request)`, then `record_choice(choice)` if there is a choice; for the last entry,
  `set_request` and `decide`. Entries already replayed (same `index`, `rqid` and `choice`) are skipped; a
  changed earlier entry rebuilds the room.

Response:

```json
{
  "ok": true,
  "room": "battle-gen9randomdoublesbattle-123",
  "rqid": 7,
  "decision": {
    "choice": "move 1 +2 terastallize, move 3",
    "a0": 13, "a1": 24,
    "analysis": { "...": "see below" }
  }
}
```

`decision` is `null` when the request needs no choice (`wait`, or a replacement request that has no choice to
make). Requests are answered one at a time per room; a newer request for the same room supersedes an older one
that is still waiting.

## `GET /api/bot-log?room=<roomid>`

Decisions the server's own bots made in that room, oldest first. A decision is listed only once the turn it
belongs to is over: after the next `|turn|` line, or when the battle ends. A choice made at the start of a turn
can still be queued after a U-turn or Eject Button replacement request, so it stays hidden until then.

While the battle runs, entries are **redacted**. They keep `bot`, `side`, `turn`, `rqid`, `time`, and from the
analysis only `source`, `win_probability`, `value`, `time_ms`, `worlds` and `cells`. The choice, its action
codes, `slots`, `candidates`, `opponent` and `note` come from the bot's own request: unused moves, bench
Pokemon, a move that never ran. Once the battle ended (`|win|`, `|tie|` or the room closed), entries are full.
`--reveal-bot-log` on the server returns full entries at all times, for watching bot-against-bot battles.

```json
{"ok": true, "room": "battle-...", "live": true, "redacted": true, "entries": [
  {"bot": "NumbrionSearch", "side": "p2", "turn": 3, "rqid": 7, "time": 1760000000.0, "redacted": true,
   "decision": {"analysis": {"source": "search", "win_probability": 0.62, "value": 0.24, "time_ms": 151.0,
                             "worlds": 4, "cells": 320}}}
]}
```

A full entry has `"decision": {"choice": "...", "a0": 1, "a1": 2, "analysis": {...}}` and no `redacted` key.
Without `room` the endpoint lists the rooms that have entries: `{"ok": true, "rooms": [{"room", "entries"}]}`.

## `GET /api/status`

```json
{"ok": true, "server": "ws://localhost:8000/showdown/websocket", "advisor": "NumbrionSearch",
 "bots": [{"name": "NumbrionSearch", "checkpoint": "...", "search": {"budget_ms": 1500}, "connected": true,
           "battles": 1}]}
```

## `analysis`

Keys a decision may carry (all optional except `source`):

| Key | Meaning |
|---|---|
| `source` | `"search"`, `"policy"` (no world could be built, or search is off), or `"default"` |
| `value` | estimated outcome in [-1, 1] for the deciding side after this choice |
| `win_probability` | `(value + 1) / 2`, rounded |
| `time_ms`, `worlds`, `cells` | search cost: wall time, sampled opponent teams, simulated (world, our pair, their pair, seed) cells |
| `slots` | per active slot: `{"pokemon": "Amoonguss", "text": "Spore -> Incineroar", "code": 7}` |
| `candidates` | our candidate pairs, best first: `{"choice", "text", "prior", "value", "weight"}`: policy probability, searched value, final weight (the choice is the highest weight) |
| `opponent` | the opponent's most likely replies over the sampled worlds: `{"text", "prior", "weight"}` |
| `note` | a short human-readable remark (e.g. why search fell back to the policy) |
