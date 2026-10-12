"""/api/advise replay (play.advise) with a fake player that records the call sequence.

    cd crates/pyengine && ../../.venv/bin/python -m pytest play/test_advise.py -q
"""
from __future__ import annotations

import asyncio
import json

import pytest
from aiohttp.test_utils import TestClient, TestServer

from play.advise import ASKED_AGAIN, UNCONFIRMED, USER_CHOSE, AdviseRoom, Entry, parse_history
from play.api import Api
from play.driver import BotLog, BotWorker
from training.player import Bot, Decision, Player

LINES = [f"|line|{i}" for i in range(12)]


def request(rqid: int, wait: bool = False) -> str:
    body = {"side": {"id": "p1", "pokemon": []}, "rqid": rqid}
    body.update({"wait": True} if wait else {"active": [{"moves": []}]})
    return json.dumps(body)


class FakePlayer(Player):
    def __init__(self, calls: list, number: int):
        super().__init__()
        self.calls, self.number = calls, number
        self.calls.append(("new", number))

    def feed(self, lines):
        self.calls.append(("feed", tuple(lines)))

    def set_request(self, request_json):
        self.calls.append(("set_request", json.loads(request_json)["rqid"]))
        self.request = json.loads(request_json)

    def record_choice(self, choice):
        self.calls.append(("record_choice", choice))

    def rejected(self, message):
        self.calls.append(("rejected", message))

    def decide(self):
        self.calls.append(("decide",))
        if self.request.get("wait"):
            return None
        return Decision(f"advice {self.request['rqid']}", analysis={"source": "default"})

    def close(self):
        self.calls.append(("close", self.number))


class FakeBot(Bot):
    name = "Fake"

    def __init__(self):
        self.calls: list = []
        self.players = 0

    def new_player(self, room):
        self.players += 1
        return FakePlayer(self.calls, self.players)


def entry(index, rqid, choice=None, wait=False):
    return Entry(index, rqid, request(rqid, wait), choice)


def take(bot):
    calls, bot.calls[:] = list(bot.calls), []
    return calls


def test_history_replays_earlier_entries_then_decides_on_the_last():
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    decision, info = room.advise_history(LINES[:5], [entry(2, 1, "move 1, move 2"), entry(5, 2)])
    assert decision.choice == "advice 2"
    assert take(bot) == [
        ("new", 1),
        ("feed", tuple(LINES[:2])), ("set_request", 1), ("record_choice", "move 1, move 2"),
        ("feed", tuple(LINES[2:5])), ("set_request", 2), ("decide",),
    ]
    assert info == {"rebuilt": True, "reason": "new room", "replayed": 1, "skipped": 0}


def test_next_turn_replays_only_the_new_entries_and_drops_the_old_advice():
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    room.advise_history(LINES[:5], [entry(2, 1, "move 1, move 2"), entry(5, 2)])
    take(bot)
    # the user chose for rqid 2 (not necessarily our advice); a wait request and rqid 4 followed
    history = [entry(2, 1, "move 1, move 2"), entry(5, 2, "switch 3, move 1 1"), entry(7, 3, wait=True),
               entry(10, 4)]
    decision, info = room.advise_history(LINES[:11], history)
    assert decision.choice == "advice 4"
    assert take(bot) == [
        ("rejected", USER_CHOSE), ("set_request", 2), ("record_choice", "switch 3, move 1 1"),
        ("feed", tuple(LINES[5:7])), ("set_request", 3),
        ("feed", tuple(LINES[7:10])), ("set_request", 4), ("decide",),
    ]
    assert info == {"rebuilt": False, "reason": None, "replayed": 2, "skipped": 1}
    # lines after the current request (chat while the user thinks) wait for the next request
    assert room.fed == LINES[:10]


def test_same_request_again_is_cached_and_more_lines_redecide():
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    history = [entry(2, 1, "move 1"), entry(5, 2)]
    room.advise_history(LINES[:5], history)
    take(bot)
    decision, info = room.advise_history(LINES[:5], history)
    assert info == {"cached": True} and decision.choice == "advice 2" and take(bot) == []
    # another budget: decide again on the same request, after dropping the previous suggestion
    room.advise_history(LINES[:6], history, budget_ms=3000)
    assert take(bot) == [("rejected", ASKED_AGAIN), ("set_request", 2), ("decide",)]


def test_a_changed_replayed_entry_rebuilds_the_player():
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    room.advise_history(LINES[:5], [entry(2, 1, "move 1"), entry(5, 2)])
    take(bot)
    decision, info = room.advise_history(LINES[:8], [entry(2, 1, "move 2"), entry(5, 2, "move 3"), entry(8, 3)])
    assert info["rebuilt"] and info["reason"] == "a replayed history entry changed"
    assert take(bot) == [
        ("close", 1), ("new", 2),
        ("feed", tuple(LINES[:2])), ("set_request", 1), ("record_choice", "move 2"),
        ("feed", tuple(LINES[2:5])), ("set_request", 2), ("record_choice", "move 3"),
        ("feed", tuple(LINES[5:8])), ("set_request", 3), ("decide",),
    ]


def test_changed_lines_rebuild_the_player():
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    room.advise_history(LINES[:5], [entry(5, 1)])
    take(bot)
    other = ["|other|0"] + LINES[1:6]
    _, info = room.advise_history(other, [entry(5, 1, "move 1"), entry(6, 2)])
    assert info["reason"] == "lines differ from the ones already fed"
    calls = take(bot)
    assert calls[:2] == [("close", 1), ("new", 2)] and calls[2] == ("feed", tuple(other[:5]))


def test_no_rejected_after_a_wait_request():
    # decide returned None on the wait request: nothing to drop, and rejected() would also drop the user's
    # previous choice that a SearchPlayer still holds until its next update
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    room.advise_history(LINES[:5], [entry(2, 1, "move 1"), entry(5, 2)])
    room.advise_history(LINES[:7], [entry(2, 1, "move 1"), entry(5, 2, "move 2"), entry(7, 3, wait=True)])
    take(bot)
    history = [entry(2, 1, "move 1"), entry(5, 2, "move 2"), entry(7, 3, wait=True), entry(9, 4)]
    room.advise_history(LINES[:9], history)
    assert take(bot) == [("set_request", 3), ("feed", tuple(LINES[7:9])), ("set_request", 4), ("decide",)]
    room.advise_history(LINES[:9], history, budget_ms=1)  # asked again after a real suggestion: dropped
    assert take(bot)[0] == ("rejected", ASKED_AGAIN)


def test_missing_choice_is_not_recorded():
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    room.advise_history(LINES[:6], [entry(3, 1), entry(6, 2)])  # rqid 1: choice unknown (timer, default)
    assert ("record_choice", None) not in take(bot)


def test_without_history_feeds_suffixes_and_rebuilds_on_a_changed_prefix():
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    room.advise_lines(LINES[:4], request(1))
    assert take(bot) == [("new", 1), ("feed", tuple(LINES[:4])), ("set_request", 1), ("decide",)]
    _, info = room.advise_lines(LINES[:4], request(1))
    assert info == {"cached": True} and take(bot) == []
    room.advise_lines(LINES[:7], request(2))
    assert take(bot) == [("rejected", UNCONFIRMED), ("feed", tuple(LINES[4:7])), ("set_request", 2), ("decide",)]
    room.advise_lines(["|x|"] + LINES[1:8], request(3))
    assert take(bot)[:3] == [("close", 1), ("new", 2), ("feed", tuple(["|x|"] + LINES[1:8]))]


def test_switching_between_modes_rebuilds():
    bot = FakeBot()
    room = AdviseRoom(bot, "battle-1")
    room.advise_lines(LINES[:4], request(1))
    take(bot)
    _, info = room.advise_history(LINES[:4], [entry(4, 1)])
    assert info["rebuilt"] and take(bot)[:2] == [("close", 1), ("new", 2)]


def test_parse_history_validation():
    good = [{"index": 2, "rqid": 1, "request": request(1), "choice": "move 1"},
            {"index": 5, "rqid": 2, "request": request(2), "choice": None}]
    assert [e.key() for e in parse_history(good, 5, request(2))] == [(2, 1, "move 1"), (5, 2, None)]
    with pytest.raises(ValueError, match="current request"):
        parse_history(good, 5, request(3))
    with pytest.raises(ValueError, match="smaller"):
        parse_history(list(reversed(good)), 5, None)
    with pytest.raises(ValueError, match="index"):
        parse_history(good, 4, None)
    with pytest.raises(ValueError, match="non-empty"):
        parse_history([], 5, None)


def test_http_advise_with_history():
    async def run():
        bot = FakeBot()
        worker = BotWorker({"name": "Fake"}, bot)
        api = Api({"Fake": worker}, "Fake", {}, BotLog(), "ws://test")
        async with TestClient(TestServer(api.app())) as client:
            history = [{"index": 2, "rqid": 1, "request": request(1), "choice": "move 1"},
                       {"index": 5, "rqid": 2, "request": request(2), "choice": None}]
            body = {"room": "battle-1", "lines": LINES[:5], "request": request(2), "history": history}
            response = await client.post("/api/advise", json=body)
            data = await response.json()
            assert response.status == 200 and data["ok"] and data["rqid"] == 2
            assert data["decision"]["choice"] == "advice 2" and data["replay"]["replayed"] == 1
            assert response.headers["Access-Control-Allow-Origin"] == "*"
            bad = dict(body, history=[dict(history[0], index=9)])
            response = await client.post("/api/advise", json=bad)
            assert response.status == 400 and not (await response.json())["ok"]
        await api.close()
        worker.shutdown()
        return bot.calls

    calls = asyncio.run(run())
    assert ("record_choice", "move 1") in calls and calls[-1] == ("close", 1)


def test_http_bot_log_redacts_live_battles_unless_revealed():
    entry = {"bot": "A", "side": "p2", "turn": 2, "rqid": 5, "time": 1.0,
             "decision": {"choice": "switch 4, move 1", "a0": 40, "a1": 1,
                          "analysis": {"source": "search", "time_ms": 99.0, "candidates": [{"text": "switch to X"}]}}}

    async def fetch(reveal: bool, ended: bool):
        bot_log = BotLog()
        bot_log.add("battle-9", entry)
        if ended:
            bot_log.end("battle-9")
        api = Api({}, None, {}, bot_log, "ws://test", reveal_bot_log=reveal)
        async with TestClient(TestServer(api.app())) as client:
            response = await client.get("/api/bot-log", params={"room": "battle-9"})
            return await response.json()

    live = asyncio.run(fetch(False, False))
    assert live["redacted"] and live["live"]
    assert live["entries"] == [{"bot": "A", "side": "p2", "turn": 2, "rqid": 5, "time": 1.0, "redacted": True,
                                "decision": {"analysis": {"source": "search", "time_ms": 99.0}}}]
    revealed = asyncio.run(fetch(True, False))
    assert not revealed["redacted"] and revealed["entries"] == [entry]
    ended = asyncio.run(fetch(False, True))
    assert not ended["redacted"] and not ended["live"] and ended["entries"] == [entry]


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
