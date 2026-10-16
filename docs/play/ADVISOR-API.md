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

## Two-phase advice (two-ply search)

A bot whose `search.depth2.enabled` is true runs a second search layer after the one-turn search (layer 1). Layer 1
takes about as long as a one-ply bot's whole decision; layer 2 adds up to a few seconds (the bot's `budget`). A
client can show the layer-1 pick at once and update it when layer 2 is done.

### Detecting support

`GET /api/status` has `"two_phase_advise": true` at the top level when the server speaks this protocol, and
every bot has `"two_phase": true|false`: true when its search has a second layer (its `search.depth2.enabled`).
A client that never sends `two_phase` gets the single-phase behaviour below from every server and every bot.

### `POST /api/advise` with `"two_phase": true`

The body is the one above plus `"two_phase": true`. The server answers as soon as the first of these exists:

* **the layer-1 result** while layer 2 still runs: a phase-1 response with `"pending": true`;
* **the final decision**: a phase-2 response with `"pending": false`. This is the only answer for a bot without a
  second layer, a policy decision (no search, no worlds, incomplete history, only one legal choice), a cached
  answer, a request that needs no choice, or a decision where the bot's budget planned no second layer.

Phase-1 response (layer 1 done, layer 2 running):

```json
{
  "ok": true, "room": "battle-gen9randomdoublesbattle-123", "rqid": 7, "bot": "Numbrion Spark 2-ply",
  "phase": 1, "pending": true, "advise_id": "9f3c2a1b7d4e5f60", "elapsed_ms": 742.5,
  "decision": {
    "choice": "move 1 +2, move 3", "a0": 13, "a1": 24,
    "analysis": {"source": "search", "value": 0.24, "win_probability": 0.62, "time_ms": 690.2,
                 "worlds": 32, "cells": 6144, "candidates": ["..."], "opponent": ["..."], "policy": {"...": "..."},
                 "slots": ["..."], "observed": {}, "inferred": {}, "timing": {}}
  }
}
```

* `decision.analysis` is the layer-1 analysis with the same keys as a one-ply search decision (table below):
  `candidates` (with layer-1 `value` and `weight`), `opponent`, `policy`, `win_probability`, `slots`, ...
  It has no `depth2` key and no `note`. `time_ms` is the decision time up to the end of layer 1.
* `advise_id`: fetch the final decision with `GET /api/advise/<advise_id>`.
* `elapsed_ms`: server time from receiving the POST until this result was ready (includes waiting for the room
  and replaying its history).
* No `replay` key (it comes with the final decision).

Phase-2 (final) response, to the POST or to `GET /api/advise/<advise_id>`:

```json
{
  "ok": true, "room": "battle-gen9randomdoublesbattle-123", "rqid": 7, "bot": "Numbrion Spark 2-ply",
  "phase": 2, "pending": false, "advise_id": "9f3c2a1b7d4e5f60", "elapsed_ms": 3105.0,
  "decision": {"choice": "move 2 +1, move 3", "a0": 14, "a1": 24,
               "analysis": {"source": "search", "value": 0.19, "win_probability": 0.595, "time_ms": 3050.7,
                            "candidates": ["..."], "opponent": ["..."], "depth2": {"...": "see below"}}},
  "replay": {"rebuilt": false, "replayed": 1, "skipped": 4}
}
```

`decision` is the final choice. Its analysis is the layer-1 analysis plus the `depth2` block (absent when no
layer 2 was attempted, e.g. a policy decision). `value` and `win_probability` are the **layer-1** estimates of
the final choice; the layer-2 values are in `depth2.candidates[].q2`. `candidates` stays the layer-1 list, ordered
by layer-1 weight; the final choice is the candidate whose `choice` equals `decision.choice`. `time_ms` is the
whole decision. `advise_id` is present when the request had `two_phase` (it may also answer at once, without a
phase 1). `elapsed_ms` counts from the POST until the final decision was ready.

### `analysis.depth2`

| Key | Meaning |
|---|---|
| `layer1_choice` | the layer-1 pick (the phase-1 `decision.choice`) |
| `chosen` | the layer-2 pick as choice text, or `null` when layer 2 completed no round (layer 1 stands) |
| `changed` | true when the final choice differs from `layer1_choice` |
| `skipped` | why layer 2 did not run or stopped early (`"the budget planned no second layer"`, `"nothing to deepen"`, `"cancelled: superseded by a newer request"`, `"error: ..."`, a refused stage), else `null` |
| `rounds`, `rounds_planned` | halving rounds completed / planned |
| `candidates` | the deepened shortlist, one entry per candidate: `{"choice", "x", "prior", "q", "q2", "score"}`. `x` = layer-1 weight, `q` = layer-1 value, `q2` = list of layer-2 values per round (`null` for a round the candidate did not survive into; the last non-null one is the most refined), `score` = per-round score used for halving |
| `replies` | per shortlisted candidate (same order), the opponent replies deepened: `{"text", "weight", "worlds", "danger"}` |
| `total_ms` | layer-2 wall time (ms) |
| `layer1_ms` | decision time when layer 1 finished (ms from the start of `decide`) |
| `timing`, `units`, `cells`, `leaves`, `sizes`, `config`, `difficulty` | layer-2 diagnostics |

When layer 2 is skipped the block is short: `{"skipped": "...", "rounds": 0, "changed": false, ...}`.
Values are in [-1, 1] like `value`; `(q2 + 1) / 2` is a win probability.

### `GET /api/advise/<advise_id>?timeout=<seconds>`

Long-poll for the final decision of a two-phase request. `timeout` defaults to 20 and is clamped to [0, 60].

| Status | Body | Meaning |
|---|---|---|
| 200 | the phase-2 response above | the final decision (returned at once if it is already done) |
| 200 | `{"ok": true, "phase": 1, "pending": true, "advise_id", "room", "rqid"}` (no `decision`) | still running after `timeout`: poll again |
| 409 | `{"ok": false, "superseded": true, "error": "superseded by a newer request for this room", "advise_id", "room", "rqid"}` | a newer `POST /api/advise` for the same bot and room came; never show this result |
| 404 | `{"ok": false, "error": "unknown or expired advise id"}` | never existed, or finished more than 5 minutes ago |
| 500 | `{"ok": false, "error": "..."}` | the decision failed |

The server keeps a finished result for 5 minutes. It can be fetched more than once.

### Stale and cancelled requests

A newer `POST /api/advise` for the same (bot, room) supersedes every earlier request of that room:

* An earlier request still waiting for the room gets 409 (as before), whether or not it asked for two phases.
* An earlier request that is running is cancelled cooperatively: its layer 2 stops before its next chunk (a
  few hundred ms at most). Layer 1 is never interrupted. Its result is not cached; its
  `GET /api/advise/<id>` answers 409. A two-phase POST that was superseded before its phase 1 was ready gets 409.
* The newer request waits for the room (one decision at a time per bot) and then runs normally.

The server only knows about rooms, not about which request the client shows. Clients must still drop any
answer whose `rqid` (and their own request sequence) is no longer the room's current request, and must never
let a phase-2 answer overwrite the display of a newer request.

### Single phase (no `two_phase`)

Unchanged: the POST waits for the final decision and returns the original body (`ok`, `room`, `rqid`, `bot`,
`decision`, `replay`), with no `phase`, `pending` or `advise_id`. For a two-ply bot that is the layer-2 decision,
including `analysis.depth2`. A running request superseded by a newer one still returns its (cut-short) result,
with `"superseded": true` added.

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
 "two_phase_advise": true,
 "bots": [{"name": "NumbrionSearch", "checkpoint": "...", "search": {"budget_ms": 1500}, "connected": true,
           "battles": 1, "two_phase": false}]}
```

`two_phase_advise`: the server supports two-phase advice; `two_phase` per bot: its search has a second layer
(see "Two-phase advice" above).

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
| `depth2` | the second search layer of a two-ply bot (see "`analysis.depth2`" above) |
