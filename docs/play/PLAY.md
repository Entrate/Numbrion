# Play against the bots

You play [Gen 9] Random Doubles Battle (`gen9randomdoublesbattle`) in the normal Showdown web client, against
bot accounts on a Showdown server on your machine. You need Node 22 or later, the repo's `.venv` with the
`numbrion` wheel, and internet access. The client page and the name check come from play.pokemonshowdown.com.
The battles run locally.

For the bot on the public ladder, see [LADDER.md](LADDER.md).

## Once: set up the Showdown server

```sh
tools/play/setup-showdown.sh
```

The script puts a Showdown checkout at the pinned commit in `scratch/showdown-play`. It clones from a local
checkout when one exists, otherwise from GitHub, then runs `npm ci` and the build. It also writes
`config/config.js`:

* port 8000, listening on 127.0.0.1 only,
* login without a password (`noguestsecurity`), since nobody here has a registered account,
* no throttles or IP checks,
* PMs allowed between unregistered names.

`tools/play/setup-showdown.sh --config` rewrites only the config.

The search bots sample the opponent's sets from a large team pool, `data/teams/pool-s1-200k.txt.gz` (git-ignored,
about 2 minutes to make):

```sh
node tools/oracle/gen-teams.mjs scratch/showdown-play --count 200000 --seed 1 --out data/teams/pool-s1-200k.txt
gzip -k data/teams/pool-s1-200k.txt
```

## Every time

1. Start Showdown in one terminal. Ctrl-C stops it.

   ```sh
   tools/play/start-showdown.sh
   ```

2. Start the bots in a second terminal. Ctrl-C stops them.

   ```sh
   .venv/bin/python crates/pyengine/play/server.py
   ```

   It loads the bots of `crates/pyengine/play/bots.json`, which takes about 4 seconds, and serves the HTTP API
   on `http://127.0.0.1:8765`. Wait for the line `bots ready after ...`. Two bots log in to Showdown:
   `NumbrionSearch` searches each decision, and `NumbrionPolicy` plays the network's policy alone. A third,
   `NumbrionAdvisor`, has no account and answers the advisor API.

3. In Chrome, open **http://localhost:8000/**. It redirects to `https://localhost.psim.us/`, the official client
   connected to `ws://localhost:8000`. `https://play.pokemonshowdown.com/~~localhost:8000` works too. Use
   `localhost`, not `127.0.0.1`.
   Chrome asks once whether the page may "access apps and services on this device". Allow it, or Chrome blocks
   the connection to the local server.
4. Click **Choose name** at the top right and enter any name, for example `Amin`.
5. Click **Find a user**, enter `NumbrionSearch`, click **Open**, then **Challenge**. Set **Format** to
   "Random Doubles Battle" in the Gen 9 list and click **Challenge**. The bot accepts and the battle tab opens.
   A search decision takes 0.1 to 1 second.

The bot refuses other formats and tells you why in a PM. You can also PM the bot `challenge` and accept the
challenge it sends back.

The battle timer is off. Turn it on with the **Timer** button in the battle, or start the bots with `--timer`.

## Bots

Each entry of `bots.json` is a `training.player.load_bot` spec. The callable named by `factory` gets the whole
entry as keyword arguments, so the bot's settings, such as a checkpoint or a search budget, go in the entry:

```json
{"advisor": "NumbrionAdvisor",
 "bots": [{"name": "NumbrionSearch", "factory": "training.search:SearchBot", "checkpoint": ".../m060.pt",
           "search": {}, "device": "auto", "torch_threads": 4},
          {"name": "NumbrionPolicy", "factory": "training.search:SearchBot", "checkpoint": ".../m060.pt",
           "search": false},
          {"name": "NumbrionAdvisor", "factory": "training.search:SearchBot", "showdown": false, "...": "..."}]}
```

`search` takes the fields of `training.search.SearchConfig` (`{}` for the defaults, see [SEARCH.md](SEARCH.md));
`false` turns search off. The checkpoint in
`bots.json` is an absolute path on this Mac, so change it on another machine.

* `name` is the Showdown user name, at most 18 characters.
* `"showdown": false` keeps a bot off Showdown. It then only answers `/api/advise`.
* `advisor` names the bot that `/api/advise` uses by default. Without it, the first bot advises.
* `play.testbots:RandomBot` makes random choices and checks the driver's event order. It is for testing.
* `bots-test.json` has two instant `DefaultBot`s (`--config crates/pyengine/play/bots-test.json`) for testing the
  server without a model.

Each bot thinks on its own worker thread, one decision at a time. Other options of `server.py` are `--config`,
`--server ws://...`, `--api-port`, `--challenge USER` (the first bot challenges USER), `--log-dir`, `--no-logs`
and `-v`. `--reveal-bot-log` shows the bots' choices and analysis in `/api/bot-log` while battles run. Without
it they appear only after the battle, so an opponent can't read them. Use it for bot-against-bot battles only.

## Advisor API and Chrome extension

The Chrome extension lives in `tools/chrome-extension` and is built separately. It calls the API to show what
the advisor bot would choose in your battle and what the bots chose in theirs. [ADVISOR-API.md](ADVISOR-API.md)
describes the API. The server has one more endpoint: `POST /api/challenge` with
`{"bot": "NumbrionSearch", "user": "Amin"}` makes the bot challenge you.

The advisor searches only when the extension sends `history`, the room's past requests with your choices. The
search needs your past choices to sample the opponent's team. Without `history` the advice comes from the policy
after the first turn.

## Logs and testing

* The server saves every battle of every bot to `scratch/showdown-logs/<room>-<bot>.json`. A file holds the raw
  websocket messages with receive times, the commands the bot sent, its decisions with their analysis, the
  player's own counters (for a search bot, `no_worlds` and the time per search step), the result and counters.
* Bot against bot: `.venv/bin/python crates/pyengine/play/server.py --selfplay 4`. The first two bots play 4
  battles and take turns as p1. The server then prints a summary and exits: wins, decision sources, notes, and
  decision times from the analysis `time_ms`. The exit status is 0 only if every battle finished without errors.
* Unit tests: `cd crates/pyengine && ../../.venv/bin/python -m pytest play -q`.

Restarting only the bot server is safe: the bots rejoin their running battles. Stopping Showdown ends all
battles.
