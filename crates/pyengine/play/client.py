"""One Showdown websocket connection per bot account: login, challenges, battle rooms, raw logs.

Protocol notes (pinned server 7332b60e2, ``noguestsecurity = true``):

* Login: after ``|challstr|`` send ``|/trn NAME,0,`` (no token); ``|updateuser| NAME|1|...`` confirms it.
* Challenges arrive as ``|pm| FROM| TO|/challenge FORMAT|TEAMFORMAT|MESSAGE|...``; a bare ``/challenge`` means it
  was cancelled or resolved. Accept with ``|/utm null`` + ``|/accept FROM``, refuse with ``|/reject FROM``.
* The server joins both players to the battle room (``>battle-...`` + ``|init|battle``) and lists running games
  in ``|updatesearch|``. After a reconnect the bot rejoins those rooms; the ``|init|`` message carries the whole
  log, so the room starts over with a fresh player.
"""
from __future__ import annotations

import asyncio
import json
import logging
import time
from pathlib import Path

import websockets

from play.driver import FORMAT, BattleRoom, BotLog, BotWorker, to_id

log = logging.getLogger("play")

FORMAT_NAME = "[Gen 9] Random Doubles Battle"
HELP = (f"Hi! I play {FORMAT_NAME} only. Challenge me to that format, or PM me \"challenge\" and I will "
        f"challenge you.")


class ShowdownClient:
    """A bot account on the Showdown server. ``run`` keeps it connected until ``stop``."""

    def __init__(self, worker: BotWorker, url: str, bot_log: BotLog, log_dir: Path | None, timer: bool = False):
        self.worker = worker
        self.name = worker.name
        self.userid = to_id(worker.name)
        self.url = url
        self.bot_log = bot_log
        self.log_dir = log_dir
        self.timer = timer
        self.ws = None
        self.send_lock = asyncio.Lock()
        self.logged_in = asyncio.Event()
        self.stopping = False
        self.rooms: dict[str, BattleRoom] = {}
        self.games: set[str] = set()
        self.finished: asyncio.Queue[BattleRoom] = asyncio.Queue()
        self.helped: set[str] = set()
        self.done_rooms: set[str] = set()  # finished battles: never rejoined
        self.battles_played = 0

    @property
    def connected(self) -> bool:
        return self.logged_in.is_set()

    async def send(self, text: str) -> None:
        ws = self.ws
        if ws is None:
            log.warning("%s: not connected, dropped %r", self.name, text)
            return
        async with self.send_lock:
            try:
                await ws.send(text)
            except websockets.ConnectionClosed:
                log.warning("%s: connection closed, dropped %r", self.name, text)

    # -- connection ------------------------------------------------------------------------------------------

    async def run(self) -> None:
        await self.worker.ready.wait()  # nobody can challenge a bot that is still loading
        backoff = 1.0
        while not self.stopping:
            try:
                async with websockets.connect(self.url, max_size=None, ping_interval=20, ping_timeout=60,
                                              open_timeout=10) as ws:
                    self.ws = ws
                    backoff = 1.0
                    log.info("%s: connected to %s", self.name, self.url)
                    async for message in ws:
                        if isinstance(message, bytes):
                            message = message.decode("utf-8", "replace")
                        self.on_message(message)
            except asyncio.CancelledError:
                raise
            except (OSError, TimeoutError, websockets.WebSocketException) as error:
                if not self.stopping:
                    log.warning("%s: connection to %s failed: %s", self.name, self.url, error)
            finally:
                self.ws = None
                self.logged_in.clear()
                self._abandon_rooms()
            if not self.stopping:
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, 30.0)

    async def stop(self) -> None:
        self.stopping = True
        tasks = [room.task for room in self.rooms.values() if room.task is not None]
        self._abandon_rooms()
        if tasks:
            await asyncio.wait(tasks, timeout=10)
        if self.ws is not None:
            await self.ws.close()

    def _abandon_rooms(self) -> None:
        for room in list(self.rooms.values()):
            room.stop()
        # on_end removes them; until their tasks finish they no longer receive messages
        self.rooms.clear()

    # -- messages --------------------------------------------------------------------------------------------

    def on_message(self, message: str) -> None:
        received_at = time.time()
        if message.startswith(">"):
            roomid, _, rest = message[1:].partition("\n")
            if roomid.startswith("battle-"):
                self._on_battle_message(roomid, rest, received_at, message)
            return
        for line in message.split("\n"):
            if line.startswith("|challstr|"):
                asyncio.create_task(self.send(f"|/trn {self.name},0,"))
            elif line.startswith("|updateuser|"):
                self._on_updateuser(line)
            elif line.startswith("|nametaken|"):
                log.error("%s: login refused: %s", self.name, line)
            elif line.startswith("|pm|"):
                self._on_pm(line)
            elif line.startswith("|updatesearch|"):
                self._on_updatesearch(line)
            elif line.startswith("|popup|"):
                log.warning("%s: popup: %s", self.name, line[len("|popup|"):].replace("||", " "))

    def _on_updateuser(self, line: str) -> None:
        parts = line.split("|")
        if len(parts) < 4:
            return
        name, named = parts[2], parts[3]
        if named == "1" and to_id(name) == self.userid:
            if not self.logged_in.is_set():
                log.info("%s: logged in, accepting %s challenges", self.name, FORMAT)
                self.logged_in.set()
                asyncio.get_running_loop().call_later(2.0, self._rejoin_games)
        elif named == "1":
            log.error("%s: logged in as %r instead (name taken?)", self.name, name.strip())

    def _on_updatesearch(self, line: str) -> None:
        try:
            games = json.loads(line[len("|updatesearch|"):]).get("games") or {}
        except (json.JSONDecodeError, AttributeError):
            return
        self.games = {roomid for roomid in games if roomid.startswith("battle-")}
        if self.connected:
            # A battle that just started is joined by the server itself; rejoin only what is still missing later.
            asyncio.get_running_loop().call_later(2.0, self._rejoin_games)

    def _rejoin_games(self) -> None:
        if not self.connected:
            return
        for roomid in self.games - set(self.rooms) - self.done_rooms:
            log.info("%s: joining running battle %s", self.name, roomid)
            asyncio.create_task(self.send(f"|/join {roomid}"))

    def _on_pm(self, line: str) -> None:
        parts = line.split("|", 4)
        if len(parts) < 5:
            return
        sender, receiver, text = parts[2], parts[3], parts[4]
        sender_id = to_id(sender)
        if to_id(receiver) != self.userid or sender_id == self.userid:
            return
        if text.startswith("/challenge "):
            challenge_format = to_id(text[len("/challenge "):].split("|", 1)[0])
            if challenge_format == FORMAT:
                log.info("%s: accepting %s challenge from %s", self.name, FORMAT, sender_id)
                asyncio.create_task(self._accept(sender_id))
            else:
                log.info("%s: rejecting %s challenge from %s", self.name, challenge_format, sender_id)
                asyncio.create_task(self._reject(sender_id, challenge_format))
        elif text.startswith("/"):
            return  # cancelled challenges, /log, /nonotify, /text, /raw ...
        elif "challenge" in text.lower():
            asyncio.create_task(self.challenge(sender_id))
        elif sender_id not in self.helped:
            self.helped.add(sender_id)
            asyncio.create_task(self.send(f"|/pm {sender_id}, {HELP}"))

    async def _accept(self, userid: str) -> None:
        await self.send("|/utm null")
        await self.send(f"|/accept {userid}")

    async def _reject(self, userid: str, challenge_format: str) -> None:
        await self.send(f"|/reject {userid}")
        await self.send(f"|/pm {userid}, Sorry, I only play {FORMAT_NAME} ({FORMAT}), not {challenge_format}.")

    async def challenge(self, user: str) -> None:
        log.info("%s: challenging %s to %s", self.name, user, FORMAT)
        await self.send("|/utm null")
        await self.send(f"|/challenge {to_id(user)}, {FORMAT}")

    # -- battle rooms ------------------------------------------------------------------------------------------

    def _on_battle_message(self, roomid: str, rest: str, received_at: float, message: str) -> None:
        room = self.rooms.get(roomid)
        if room is None:
            if not rest.startswith("|init|battle"):
                return  # a room we already left (|deinit|, late chat)
            if roomid in self.done_rooms:
                asyncio.create_task(self.send(f"{roomid}|/leave"))
                return
            room = self._open_room(roomid)
        room.push(received_at, message)

    def _open_room(self, roomid: str) -> BattleRoom:
        log_path = None
        if self.log_dir is not None:
            log_path = self.log_dir / f"{roomid}-{self.userid}.json"
            n = 2
            while log_path.exists():  # room numbers restart when the Showdown server loses its logs/ directory
                log_path = self.log_dir / f"{roomid}-{self.userid}-{n}.json"
                n += 1
        room = BattleRoom(roomid, self.worker, self.send, self.bot_log, log_path, self.url, on_end=self._room_ended)
        self.rooms[roomid] = room
        room.start()
        log.info("%s: joined %s", self.name, roomid)
        if self.timer:
            asyncio.create_task(self.send(f"{roomid}|/timer on"))
        return room

    def _room_ended(self, room: BattleRoom) -> None:
        if self.rooms.get(room.room) is room:
            del self.rooms[room.room]
        if room.ended:
            self.done_rooms.add(room.room)
            self.battles_played += 1
            result = room.result or {}
            outcome = "tie" if result.get("tie") else f"winner {result.get('winner')}"
            log.info("%s: %s ended after %d turns, %s; stats %s", self.name, room.room, room.turn, outcome,
                     room.stats)
            if self.ws is not None:
                asyncio.create_task(self.send(f"{room.room}|/leave"))
        self.finished.put_nowait(room)
