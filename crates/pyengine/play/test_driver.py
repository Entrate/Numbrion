"""BattleRoom event order on synthetic Showdown messages (both request orders, refusals, hold timeout).

    cd crates/pyengine && ../../.venv/bin/python -m pytest play/test_driver.py -q
"""
from __future__ import annotations

import asyncio
import json

import pytest

from play import driver
from play.driver import BattleRoom, BotLog, BotWorker
from training.player import Bot, Decision, Player

ROOM = "battle-gen9randomdoublesbattle-1"


def request(rqid: int, **extra) -> str:
    body = {"active": [{"moves": [{"move": "Tackle", "id": "tackle", "target": "normal"}]}] * 2,
            "side": {"name": "A", "id": "p1", "pokemon": []}, "rqid": rqid, **extra}
    return json.dumps(body)


def msg(*lines: str) -> str:
    return "\n".join([f">{ROOM}", *lines])


START = [
    msg("|init|battle", "|title|A vs. B", "|j|☆A"),
    msg("|t:|1", "|gametype|doubles"),
    msg("|player|p1|A|1|"),
    msg("|player|p2|B|2|", "|gen|9", "|tier|[Gen 9] Random Doubles Battle", "|", "|start",
        "|switch|p1a: X|X, L80|100/100", "|switch|p1b: Y|Y, L80|100/100", "|turn|1"),
]
TURN_1 = msg("|", "|t:|2", "|move|p1a: X|Tackle|p2a: Z", "|-damage|p2a: Z|50/100", "|upkeep", "|turn|2")


class RecordingPlayer(Player):
    def __init__(self, calls, choices):
        super().__init__()
        self.calls, self.choices = calls, choices

    def feed(self, lines):
        self.calls.append(("feed", tuple(lines)))

    def set_request(self, request_json):
        self.calls.append(("set_request", json.loads(request_json)["rqid"]))

    def rejected(self, message):
        self.calls.append(("rejected", message))

    def record_choice(self, choice):
        self.calls.append(("record_choice", choice))

    def decide(self):
        self.calls.append(("decide",))
        return Decision(self.choices.pop(0) if self.choices else "move 1 1, move 1 2")

    def close(self):
        self.calls.append(("close",))


class RecordingBot(Bot):
    def __init__(self, choices=()):
        self.calls, self.choices = [], list(choices)

    def new_player(self, room):
        return RecordingPlayer(self.calls, self.choices)


async def play(messages, choices=(), wait=0.0):
    bot = RecordingBot(choices)
    worker = BotWorker({"name": "A"}, bot)
    sent = []

    async def send(text):
        sent.append(text)

    bot_log = BotLog()
    room = BattleRoom(ROOM, worker, send, bot_log, None, "ws://test")
    room.start()
    for message in messages:
        room.push(0.0, message)
    if wait:
        await asyncio.sleep(wait)
    room.stop()
    await room.task
    worker.shutdown()
    return bot.calls, sent, bot_log, room


def feeds(calls):
    return [line for call in calls if call[0] == "feed" for line in call[1]]


def test_lines_first_and_request_first_give_the_same_calls():
    lines_first = [*START, msg(f"|request|{request(1)}"), TURN_1, msg(f"|request|{request(3)}")]
    request_first = [*START[:3], msg(f"|request|{request(1)}"), START[3], msg(f"|request|{request(3)}"), TURN_1]
    calls_a, sent_a, _, _ = asyncio.run(play(lines_first))
    calls_b, sent_b, _, _ = asyncio.run(play(request_first))
    assert calls_a == calls_b
    assert [c[0] for c in calls_a] == ["feed", "set_request", "decide", "feed", "set_request", "decide", "close"]
    # every line before a request is fed before set_request
    first_feed = calls_a[0][1]
    assert first_feed[0] == "|init|battle" and first_feed[-1] == "|turn|1"
    assert calls_a[3][1][-1] == "|turn|2"
    assert sent_a == sent_b == [f"{ROOM}|/choose move 1 1, move 1 2|1", f"{ROOM}|/choose move 1 1, move 1 2|3"]


def test_unavailable_choice_resend_then_invalid_then_default():
    messages = [
        *START, msg(f"|request|{request(1)}"),
        msg("|error|[Unavailable choice] Can't move: X's Tackle is disabled"),
        msg(f"|request|{request(2, update=True)}"),
        msg("|error|[Invalid choice] Can't move: bad"),
        TURN_1, msg(f"|request|{request(4)}"),
    ]
    calls, sent, bot_log, room = asyncio.run(play(messages, choices=["move 2", "move 3"]))
    kinds = [c[0] if c[0] != "set_request" else f"set_request {c[1]}" for c in calls]
    # refused once: rejected, re-sent request, decide again; refused twice: rejected, server default, which the
    # player learns as someone else's choice
    assert kinds == ["feed", "set_request 1", "decide", "rejected", "set_request 2", "decide", "rejected",
                     "record_choice", "feed", "set_request 4", "decide", "close"]
    assert ("record_choice", "default") in calls
    assert sent == [f"{ROOM}|/choose move 2|1", f"{ROOM}|/choose move 3|2", f"{ROOM}|/choose default|2",
                    f"{ROOM}|/choose move 1 1, move 1 2|4"]
    # only the carried-out choice (default for rqid 2, turn 1 is over) is in the bot log; rqid 4 is still pending
    entries = bot_log.entries(ROOM, reveal=True)
    assert [(e["rqid"], e["decision"]["choice"]) for e in entries] == [(2, "default")]
    assert room.stats["refused"] == 2 and room.stats["defaults"] == 1


def test_invalid_choice_decides_again_on_the_same_request():
    messages = [*START, msg(f"|request|{request(1)}"), msg("|error|[Invalid choice] Can't move: bad"),
                TURN_1, msg(f"|request|{request(3)}")]
    calls, sent, bot_log, _ = asyncio.run(play(messages, choices=["move 9", "move 1 2, move 1 1"]))
    assert [c[0] for c in calls] == ["feed", "set_request", "decide", "rejected", "decide", "feed",
                                     "set_request", "decide", "close"]
    assert sent[:2] == [f"{ROOM}|/choose move 9|1", f"{ROOM}|/choose move 1 2, move 1 1|1"]
    assert [e["decision"]["choice"] for e in bot_log.entries(ROOM, reveal=True)] == ["move 1 2, move 1 1"]


def test_stale_errors_are_ignored():
    messages = [*START, msg(f"|request|{request(1)}"),
                msg("|error|[Invalid choice] Sorry, too late to make a different move; the next turn has already "
                    "started")]
    calls, sent, _, _ = asyncio.run(play(messages))
    assert [c[0] for c in calls] == ["feed", "set_request", "decide", "close"]
    assert len(sent) == 1


def test_request_without_lines_is_dispatched_after_the_hold(monkeypatch):
    monkeypatch.setattr(driver, "HOLD_SECONDS", 0.05)
    messages = [*START, msg(f"|request|{request(1)}"), msg(f"|request|{request(2)}")]
    calls, sent, _, room = asyncio.run(play(messages, wait=0.3))
    assert [c[0] for c in calls] == ["feed", "set_request", "decide", "set_request", "decide", "close"]
    assert room.stats["held"] == 1


def test_win_resolves_the_last_choice_and_closes():
    win = msg("|", "|move|p1a: X|Tackle|p2a: Z", "|faint|p2a: Z", "|win|A")
    calls, sent, bot_log, room = asyncio.run(play([*START, msg(f"|request|{request(1)}"), win]))
    assert calls[-2][0] == "feed" and calls[-2][1][-1] == "|win|A"
    assert calls[-1] == ("close",)
    assert [e["rqid"] for e in bot_log.entries(ROOM)] == [1]
    assert room.result == {"winner": "A", "tie": False}


def test_wait_request_sends_nothing():
    wait = json.dumps({"wait": True, "side": {"id": "p1", "pokemon": []}, "rqid": 1})
    calls, sent, _, _ = asyncio.run(play([*START, msg(f"|request|{wait}")]))
    assert [c[0] for c in calls] == ["feed", "set_request", "decide", "close"]
    assert sent == []


def test_decide_failure_sends_default_and_records_it():
    class Failing(RecordingPlayer):
        def decide(self):
            self.calls.append(("decide",))
            raise RuntimeError("network down")

    class FailingBot(RecordingBot):
        def new_player(self, room):
            return Failing(self.calls, self.choices)

    async def run():
        bot = FailingBot()
        worker = BotWorker({"name": "A"}, bot)
        sent = []

        async def send(text):
            sent.append(text)

        room = BattleRoom(ROOM, worker, send, BotLog(), None, "ws://test")
        room.start()
        for message in [*START, msg(f"|request|{request(1)}")]:
            room.push(0.0, message)
        room.stop()
        await room.task
        worker.shutdown()
        return bot.calls, sent

    calls, sent = asyncio.run(run())
    assert [c[0] for c in calls] == ["feed", "set_request", "decide", "record_choice", "close"]
    assert calls[3] == ("record_choice", "default") and sent == [f"{ROOM}|/choose default|1"]


PIVOT = msg("|", "|t:|2", "|move|p1a: X|U-turn|p2a: Z", "|-damage|p2a: Z|50/100")
AFTER_PIVOT = msg("|", "|switch|p1a: W|W, L80|100/100", "|move|p1b: Y|Tackle|p2b: V", "|-damage|p2b: V|80/100",
                  "|upkeep", "|turn|2")


def test_choices_are_published_only_when_their_turn_is_over():
    switch = json.dumps({"forceSwitch": [True, False], "side": {"id": "p1", "pokemon": []}, "rqid": 2})

    async def run():
        bot = RecordingBot(["move 1 1, move 1 2", "switch 3, pass"])
        worker = BotWorker({"name": "A"}, bot)
        bot_log = BotLog()
        sent = []

        async def send(text):
            sent.append(text)

        room = BattleRoom(ROOM, worker, send, bot_log, None, "ws://test")
        room.start()
        snapshots = []
        for message in [*START, msg(f"|request|{request(1)}"), PIVOT, msg(f"|request|{switch}")]:
            room.push(0.0, message)
        await asyncio.sleep(0.2)
        # U-turn replacement request: the turn-1 choice was accepted, but Y's Tackle is still queued
        snapshots.append(bot_log.entries(ROOM, reveal=True))
        for message in [AFTER_PIVOT, msg(f"|request|{request(3)}")]:
            room.push(0.0, message)
        await asyncio.sleep(0.2)
        snapshots.append(bot_log.entries(ROOM, reveal=True))
        room.stop()
        await room.task
        worker.shutdown()
        return snapshots

    during, after = asyncio.run(run())
    assert during == []
    assert [(e["rqid"], e["decision"]["choice"]) for e in after] == [(1, "move 1 1, move 1 2"), (2, "switch 3, pass")]


def test_bot_log_is_redacted_while_the_battle_runs():
    analysis = {"source": "search", "value": 0.2, "win_probability": 0.6, "time_ms": 120.0, "worlds": 4,
                "cells": 300, "slots": [{"pokemon": "X", "text": "Tackle -> Z", "code": 3}],
                "candidates": [{"choice": "switch 4, move 1", "text": "switch to Secret"}],
                "opponent": [{"text": "..."}], "note": "a note", "timing": {"grid_ms": 1}}
    entry = {"bot": "A", "side": "p1", "turn": 3, "rqid": 7, "time": 1.0,
             "decision": {"choice": "move 1 1, move 2", "a0": 1, "a1": 2, "analysis": analysis}}
    log = BotLog()
    log.add(ROOM, entry)
    (live,) = log.entries(ROOM)
    assert live == {"bot": "A", "side": "p1", "turn": 3, "rqid": 7, "time": 1.0, "redacted": True,
                    "decision": {"analysis": {"source": "search", "value": 0.2, "win_probability": 0.6,
                                              "time_ms": 120.0, "worlds": 4, "cells": 300}}}
    assert log.entries(ROOM, reveal=True) == [entry]
    log.end(ROOM)
    assert log.entries(ROOM) == [entry]


def test_win_ends_the_room_in_the_bot_log():
    win = msg("|", "|move|p1a: X|Tackle|p2a: Z", "|faint|p2a: Z", "|win|A")
    _, _, bot_log, _ = asyncio.run(play([*START, msg(f"|request|{request(1)}"), win]))
    assert not bot_log.is_live(ROOM)
    assert bot_log.entries(ROOM)[0]["decision"]["choice"] == "move 1 1, move 1 2"


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-q"]))
