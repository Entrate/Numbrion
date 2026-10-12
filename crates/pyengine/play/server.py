"""Play server: the bots as Showdown accounts, plus the HTTP API of docs/play/ADVISOR-API.md (docs/play/PLAY.md).

    .venv/bin/python crates/pyengine/play/server.py                  # bots of play/bots.json on localhost:8000
    .venv/bin/python crates/pyengine/play/server.py --selfplay 4     # the first two bots play 4 battles, then exit
    .venv/bin/python crates/pyengine/play/server.py --config crates/pyengine/play/bots-test.json   # DefaultBots

Config (``--config``, default ``crates/pyengine/play/bots.json``)::

    {"advisor": "NumbrionAdvisor",
     "bots": [{"name": "NumbrionSearch", "factory": "training.search:SearchBot", "checkpoint": "...", "search": {}},
              {"name": "NumbrionAdvisor", "factory": "...", "showdown": false, "checkpoint": "..."}]}

Each entry of ``bots`` is a ``training.player.load_bot`` spec: ``factory`` is called with the whole entry as keyword
arguments, except the driver's own key ``showdown`` (default true; false = no Showdown account, only for
``/api/advise``). ``name`` is the Showdown user name (at most 18 characters). ``advisor`` names the bot that
``/api/advise`` uses by default (default: the first bot).

Startup: every bot loads and warms up on its own worker thread (play.driver.BotWorker.load: the model, then the
team pool of a search bot), so the event loop and the API stay responsive. The log says "<bot>: ready" for each and
"bots ready after ..." at the end; a bot's Showdown account logs in only once the bot is ready.
"""
from __future__ import annotations

import argparse
import asyncio
import json
import logging
import signal
import sys
import time
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Run as a script, sys.path[0] is play/ (whose modules would shadow top-level names): use crates/pyengine instead,
# like evaluate.py, so `training` and `play` import as packages.
if sys.path and Path(sys.path[0] or ".").resolve() == HERE:
    sys.path[0] = str(HERE.parent)
elif str(HERE.parent) not in sys.path:
    sys.path.insert(0, str(HERE.parent))
REPO = HERE.parents[2]

from aiohttp import web  # noqa: E402

from play.api import Api  # noqa: E402
from play.client import ShowdownClient  # noqa: E402
from play.driver import BattleRoom, BotLog, BotWorker, to_id  # noqa: E402

log = logging.getLogger("play")


def load_config(path: Path) -> tuple[list[dict], str | None]:
    config = json.loads(path.read_text())
    if isinstance(config, list):
        config = {"bots": config}
    specs = config.get("bots") or []
    if not specs:
        raise SystemExit(f"{path}: no bots configured")
    names = set()
    for spec in specs:
        if "name" not in spec or "factory" not in spec:
            raise SystemExit(f"{path}: every bot needs a name and a factory: {spec}")
        if to_id(spec["name"]) in names:
            raise SystemExit(f"{path}: duplicate bot name {spec['name']!r}")
        names.add(to_id(spec["name"]))
        if spec.get("showdown", True) and len(spec["name"]) > 18:
            raise SystemExit(f"{path}: Showdown names have at most 18 characters: {spec['name']!r}")
    advisor = config.get("advisor") or specs[0]["name"]
    if advisor not in {spec["name"] for spec in specs}:
        raise SystemExit(f"{path}: advisor {advisor!r} is not one of the bots")
    return specs, advisor


def decision_summary(rooms: list[BattleRoom]) -> dict:
    """Sources, notes and decision times (analysis ``time_ms``) of one bot's resolved decisions."""
    sources, notes, times, player_stats = Counter(), Counter(), [], {}
    for room in rooms:
        for entry in room.decisions:
            analysis = (entry.get("decision") or {}).get("analysis") or {}
            sources[analysis.get("source", "?")] += 1
            if analysis.get("note"):
                notes[str(analysis["note"])[:80]] += 1
            if isinstance(analysis.get("time_ms"), (int, float)):
                times.append(float(analysis["time_ms"]))
        for key, value in (room.player_stats or {}).items():
            if isinstance(value, (int, float)):
                player_stats[key] = player_stats.get(key, 0) + value
    times.sort()

    def quantile(q):
        return round(times[min(len(times) - 1, int(q * len(times)))], 1)

    return {
        "decisions": sum(sources.values()), "sources": dict(sources), "notes": dict(notes.most_common(10)),
        "time_ms": {"mean": round(sum(times) / len(times), 1), "median": quantile(0.5), "p90": quantile(0.9),
                    "max": round(times[-1], 1)} if times else None,
        "player_stats": {key: round(value, 1) for key, value in player_stats.items()},
    }


async def selfplay(clients: list[ShowdownClient], battles: int, timeout: float) -> dict:
    """The first two clients play ``battles`` battles, one at a time, taking turns as challenger (= p1)."""
    a, b = clients[0], clients[1]
    for client in (a, b):
        await client.worker.ready.wait()
    await asyncio.wait_for(asyncio.gather(a.logged_in.wait(), b.logged_in.wait()), 120)
    summary = {"battles": 0, "wins": {a.name: 0, b.name: 0}, "ties": 0, "turns": [], "rooms": [], "p1": [],
               "refused": 0, "defaults": 0, "errors": 0, "held": 0, "unfinished": 0}
    rooms: dict[str, list[BattleRoom]] = {a.name: [], b.name: []}
    for index in range(battles):
        challenger, opponent = (a, b) if index % 2 == 0 else (b, a)
        for client in (a, b):
            while not client.finished.empty():
                client.finished.get_nowait()
        await challenger.challenge(opponent.name)
        try:
            room_a = await asyncio.wait_for(a.finished.get(), timeout)
            room_b = await asyncio.wait_for(b.finished.get(), 30)
        except TimeoutError:
            log.error("self-play battle %d did not finish within %.0f s", index + 1, timeout)
            summary["unfinished"] += 1
            break
        if not (room_a.ended and room_b.ended and room_a.room == room_b.room):
            log.error("self-play battle %d ended abnormally (%s / %s)", index + 1, room_a.room, room_b.room)
            summary["unfinished"] += 1
            break
        summary["battles"] += 1
        summary["rooms"].append(room_a.room)
        summary["turns"].append(room_a.turn)
        summary["p1"].append(a.name if room_a.side == "p1" else b.name)
        rooms[a.name].append(room_a)
        rooms[b.name].append(room_b)
        result = room_a.result or {}
        if result.get("tie"):
            summary["ties"] += 1
        else:
            winner = {to_id(a.name): a.name, to_id(b.name): b.name}.get(to_id(result.get("winner") or ""))
            if winner:
                summary["wins"][winner] += 1
        for room in (room_a, room_b):
            for key in ("refused", "defaults", "errors", "held"):
                summary[key] += room.stats[key]
        log.info("self-play %d/%d: %s, %d turns, p1 %s, result %s", index + 1, battles, room_a.room, room_a.turn,
                 summary["p1"][-1], result)
    summary["decisions"] = {name: decision_summary(bot_rooms) for name, bot_rooms in rooms.items()}
    return summary


async def announce_ready(workers: dict[str, BotWorker], tasks: list[asyncio.Task]) -> None:
    started = time.perf_counter()
    await asyncio.gather(*tasks)
    ready = [name for name, worker in workers.items() if worker.ready.is_set()]
    failed = [name for name, worker in workers.items() if worker.failed]
    log.info("bots ready after %.1f s: %s%s", time.perf_counter() - started, ", ".join(ready) or "none",
             f"; FAILED: {', '.join(failed)}" if failed else "")


async def main(args: argparse.Namespace) -> int:
    specs, advisor = load_config(Path(args.config))
    workers = {spec["name"]: BotWorker(spec) for spec in specs}
    bot_log = BotLog()
    log_dir = None if args.no_logs else Path(args.log_dir)
    clients = {spec["name"]: ShowdownClient(workers[spec["name"]], args.server, bot_log, log_dir, args.timer)
               for spec in specs if spec.get("showdown", True)}
    api = Api(workers, advisor, clients, bot_log, args.server, reveal_bot_log=args.reveal_bot_log)

    runner = None
    if not args.no_api:
        runner = web.AppRunner(api.app(), access_log=None)
        await runner.setup()
        await web.TCPSite(runner, args.api_host, args.api_port).start()
        log.info("API on http://%s:%d (advisor %s)", args.api_host, args.api_port, advisor)

    # Each bot loads and warms up on its own worker thread; its account logs in once it is ready.
    loads = [asyncio.create_task(worker.load()) for worker in workers.values()]
    announcer = asyncio.create_task(announce_ready(workers, loads))

    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    tasks = [asyncio.create_task(client.run(), name=f"client:{name}") for name, client in clients.items()]

    status = 0
    try:
        if args.selfplay:
            if len(clients) < 2:
                raise SystemExit("--selfplay needs two bots with Showdown accounts in the config")
            play = asyncio.create_task(selfplay(list(clients.values()), args.selfplay, args.battle_timeout))
            stopped = asyncio.create_task(stop.wait())
            await asyncio.wait([play, stopped], return_when=asyncio.FIRST_COMPLETED)
            if play.done():
                summary = play.result()
                summary["bots"] = {name: worker.describe() for name, worker in workers.items()}
                print(json.dumps(summary, indent=1))
                status = 0 if summary["battles"] == args.selfplay and not summary["errors"] else 1
            else:
                play.cancel()
                status = 1
            stopped.cancel()
        else:
            if args.challenge:
                client = next(iter(clients.values()))
                await client.worker.ready.wait()
                await asyncio.wait_for(client.logged_in.wait(), 60)
                await client.challenge(args.challenge)
            await stop.wait()
    finally:
        log.info("shutting down")
        announcer.cancel()
        await asyncio.gather(*(client.stop() for client in clients.values()), return_exceptions=True)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        await api.close()
        if runner is not None:
            await runner.cleanup()
        for worker in workers.values():
            worker.shutdown()
    return status


def parse_args(argv=None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--config", default=str(HERE / "bots.json"), help="bot config JSON (default: %(default)s)")
    parser.add_argument("--server", default="ws://localhost:8000/showdown/websocket",
                        help="Showdown websocket URL (default: %(default)s)")
    parser.add_argument("--api-host", default="127.0.0.1")
    parser.add_argument("--api-port", type=int, default=8765)
    parser.add_argument("--no-api", action="store_true", help="do not serve the HTTP API")
    parser.add_argument("--log-dir", default=str(REPO / "scratch" / "showdown-logs"),
                        help="raw battle logs, one JSON per battle and bot (default: %(default)s)")
    parser.add_argument("--no-logs", action="store_true", help="do not save battle logs")
    parser.add_argument("--timer", action="store_true", help="turn the battle timer on in the bots' battles")
    parser.add_argument("--reveal-bot-log", action="store_true",
                        help="/api/bot-log returns full entries (choices, candidates) while battles run, not only "
                             "after they end; for watching bot-vs-bot battles, never when playing a bot yourself")
    parser.add_argument("--challenge", metavar="USER", help="the first bot challenges USER once logged in")
    parser.add_argument("--selfplay", type=int, default=0, metavar="N",
                        help="the first two bots play N battles against each other, then the server exits")
    parser.add_argument("--battle-timeout", type=float, default=900, help="self-play: seconds per battle")
    parser.add_argument("-v", "--verbose", action="store_true")
    return parser.parse_args(argv)


if __name__ == "__main__":
    arguments = parse_args()
    logging.basicConfig(level=logging.DEBUG if arguments.verbose else logging.INFO,
                        format="%(asctime)s %(levelname)s %(message)s", datefmt="%H:%M:%S")
    logging.getLogger("websockets").setLevel(logging.INFO)
    logging.getLogger("aiohttp").setLevel(logging.WARNING)
    sys.exit(asyncio.run(main(arguments)))
