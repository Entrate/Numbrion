"""BattleRoom time rules and ladder facts: deadline fallbacks, pacing, the battle clock, ratings, observer events.

    cd crates/pyengine && ../../.venv/bin/python -m pytest play/test_deadline.py -q
"""
from __future__ import annotations

import asyncio
import time

from play.driver import BattleRoom, BotLog, BotWorker, DecisionTiming
from play.test_driver import ROOM, TURN_1, RecordingBot, RecordingPlayer, msg, request
from training.player import Decision

RATED_START = [
    msg("|init|battle", "|title|A vs. B", "|j|☆A"),
    msg("|t:|1", "|gametype|doubles"),
    msg("|player|p1|A|1|1500"),
    msg("|player|p2|B|2|1450", "|gen|9", "|tier|[Gen 9] Random Doubles Battle", "|rated|", "|", "|start",
        "|switch|p1a: X|X, L80|100/100", "|switch|p1b: Y|Y, L80|100/100", "|turn|1"),
]
SEARCH_CHOICE = "move 2 1, move 2 2"
POLICY_CHOICE = "move 1 1, move 1 2"


class SlowPlayer(RecordingPlayer):
    """Posts a policy fallback at once (unless ``early`` is false), then thinks ``think`` seconds."""

    def __init__(self, calls, choices, think=0.0, early=True):
        super().__init__(calls, choices)
        self.think, self.early = think, early
        self.budgets = []

    def decide(self):
        self.calls.append(("decide",))
        self.budgets.append((getattr(self, "budget_ms", None), getattr(self, "deadline", None)))
        if self.early:
            self.fallback = (self.decision_token, Decision(POLICY_CHOICE, 1, 2, {"source": "policy"}))
        time.sleep(self.think)
        return Decision(SEARCH_CHOICE, 3, 4, {"source": "search", "win_probability": 0.6})


class SlowBot(RecordingBot):
    def __init__(self, think=0.0, early=True):
        super().__init__()
        self.think, self.early = think, early
        self.players = []

    def new_player(self, room):
        player = SlowPlayer(self.calls, self.choices, self.think, self.early)
        self.players.append(player)
        return player


class Events:
    def __init__(self):
        self.events = []

    def room_event(self, room, kind, data):
        self.events.append((kind, data))

    def kinds(self):
        return [kind for kind, _ in self.events if kind not in ("message", "sent")]

    def of(self, kind):
        return [data for k, data in self.events if k == kind]


async def play(messages, bot, timing=None, gaps=(), wait=0.0, rating_wait_s=0.0, stop=True):
    """Push ``messages`` (``gaps[i]`` seconds before message i, default none), wait, stop; return the pieces."""
    worker = BotWorker({"name": "A"}, bot)
    sent = []

    async def send(text):
        sent.append((time.monotonic(), text))

    observer = Events()
    room = BattleRoom(ROOM, worker, send, BotLog(), None, "ws://test", observer=observer, timing=timing,
                      rating_wait_s=rating_wait_s)
    room.start()
    for i, message in enumerate(messages):
        if i < len(gaps) and gaps[i]:
            await asyncio.sleep(gaps[i])
        room.push(time.time(), message)
    if wait:
        await asyncio.sleep(wait)
    if stop:
        room.stop()
    await room.task
    worker.shutdown()
    return sent, observer, room


def test_deadline_sends_the_early_policy_choice_and_the_player_learns_it():
    bot = SlowBot(think=0.5)
    timing = DecisionTiming(deadline_s=0.3, reserve_s=0.05)
    began = time.monotonic()
    sent, events, room = asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}"), TURN_1,
                                           msg(f"|request|{request(3)}")], bot, timing, gaps=[0, 0, 0, 0, 0, 0.6]))
    first = sent[0]
    assert first[1] == f"{ROOM}|/choose {POLICY_CHOICE}|1"
    assert first[0] - began < 0.45  # sent at the deadline, not after the 0.5 s decision
    # The late decision finished before the next request; the player then learned the choice actually sent,
    # before it saw the next turn.
    kinds = [c[0] for c in bot.calls]
    assert kinds[:4] == ["feed", "set_request", "decide", "record_choice"]
    assert bot.calls[3] == ("record_choice", POLICY_CHOICE)
    decision = events.of("decision")[0]
    assert decision["source"] == "fallback" and decision["fallback"] == "deadline"
    assert decision["timing"]["decided"] is None and decision["timing"]["total_ms"] < 450
    late = events.of("late_decision")[0]
    assert late["seq"] == 1 and late["choice"] == SEARCH_CHOICE and late["compute_ms"] >= 450
    assert room.stats["fallbacks"] == 2 and room.stats["late"] >= 1  # the second request is slow too


def test_deadline_without_an_early_decision_sends_default():
    bot = SlowBot(think=0.4, early=False)
    sent, events, room = asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}")], bot,
                                          DecisionTiming(deadline_s=0.2, reserve_s=0.05), wait=0.6))
    assert sent[0][1] == f"{ROOM}|/choose default|1"
    assert ("record_choice", "default") in bot.calls
    decision = events.of("decision")[0]
    assert decision["source"] == "fallback" and decision["fallback"].startswith("deadline, no policy")


def test_budget_and_deadline_are_handed_to_the_player():
    bot = SlowBot()
    asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}")], bot,
                     DecisionTiming(search_s=3.0, deadline_s=6.0, reserve_s=0.5), wait=0.1))
    budget, deadline = bot.players[0].budgets[0]
    assert 2900 <= budget <= 3000
    assert 5.0 < deadline - time.monotonic() + 0.2 < 6.0


def test_budget_shrinks_to_the_time_left():
    bot = SlowBot()
    asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}")], bot,
                     DecisionTiming(search_s=3.0, deadline_s=1.0, reserve_s=0.2), wait=0.1))
    budget, _ = bot.players[0].budgets[0]
    assert 400 <= budget <= 800  # 0.8 s to the deadline less the 250 ms margin


def test_min_response_paces_computed_decisions():
    bot = SlowBot()
    sent, events, _ = asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}")], bot,
                                       DecisionTiming(deadline_s=2.0, min_response_s=0.3), wait=0.5))
    decision = events.of("decision")[0]
    assert decision["source"] == "search" and decision["fallback"] is None
    assert decision["timing"]["paced_ms"] > 200 and 290 <= decision["timing"]["total_ms"] < 600
    assert decision["timing"]["compute_ms"] is not None and decision["timing"]["queue_ms"] is not None


def test_the_battle_clock_brings_the_deadline_forward():
    bot = SlowBot(think=0.5)
    clock = msg("|inactive|Time left: 12 sec this turn | 140 sec total")
    sent, events, room = asyncio.run(play([*RATED_START, clock, msg(f"|request|{request(1)}")], bot,
                                          DecisionTiming(deadline_s=6.0, clock_margin_s=11.8), wait=0.7))
    decision = events.of("decision")[0]
    assert decision["source"] == "fallback" and decision["timing"]["total_ms"] < 450
    assert events.of("request")[0]["clock"]["turn_left"] == 12
    assert events.of("clock")[0]["total_left"] == 140


def test_a_stuck_decision_is_reported_unhealthy():
    bot = SlowBot(think=0.6)
    _, events, room = asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}")], bot,
                                       DecisionTiming(deadline_s=0.1, reserve_s=0.0, hang_s=0.2), wait=0.8))
    assert events.of("unhealthy") and room.stats["unhealthy"] == 1


def test_ratings_are_read_before_the_room_closes():
    bot = SlowBot()
    end = msg("|", "|-message|B lost due to inactivity.", "|win|A")
    rating = msg("|raw|A's rating: 1500 &rarr; <strong>1525</strong><br />(+25 for winning)",
                 "|raw|B's rating: 1450 &rarr; <strong>1426</strong><br />(-24 for losing)")
    began = time.monotonic()
    _, events, room = asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}"), end, rating], bot,
                                       gaps=[0, 0, 0, 0, 0, 0, 0.2], rating_wait_s=5.0, stop=False))
    assert time.monotonic() - began < 2.0  # closed when our rating came, not after the whole wait
    assert room.players["p1"] == {"name": "A", "userid": "a", "rating": 1500}
    assert room.players["p2"]["rating"] == 1450 and room.rated is True
    assert room.ratings["a"]["new"] == 1525 and room.ratings["a"]["delta"] == 25
    assert room.ratings["b"]["delta"] == -24
    end_event = events.of("end")[0]
    assert end_event["won"] is True and end_event["end_reason"] == "inactivity"
    kinds = events.kinds()
    assert kinds.index("end") < kinds.index("rating") < kinds.index("closed")
    closed = events.of("closed")[0]
    assert closed["ended"] and not closed["abandoned"] and closed["ratings"]["a"]["new"] == 1525


def test_rating_wait_ends_without_a_rating_line():
    bot = SlowBot()
    began = time.monotonic()
    _, events, room = asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}"), msg("|", "|win|B")], bot,
                                       rating_wait_s=0.3, stop=False))
    assert 0.25 < time.monotonic() - began < 2.0
    assert events.of("end")[0]["won"] is False and not room.ratings


def test_timer_confirmation_and_events():
    bot = SlowBot()
    timer = msg("|inactive|Battle timer is ON: inactive players will automatically lose when time's up. "
                "(requested by A)")
    _, events, room = asyncio.run(play([*RATED_START, timer, msg(f"|request|{request(1)}")], bot, wait=0.1))
    assert room.timer_on is not None and events.of("timer")[0]["state"] == "on"
    request_event = events.of("request")[0]
    assert request_event["seq"] == 1 and request_event["rqid"] == 1 and request_event["lines"]
    assert events.of("sent")[0]["text"] == f"{ROOM}|/choose {SEARCH_CHOICE}|1"
    decision = events.of("decision")[0]
    assert decision["a0"] == 3 and decision["analysis"]["win_probability"] == 0.6


def test_a_room_opened_mid_battle_is_marked_rejoined():
    bot = SlowBot()
    init = msg("|init|battle", "|title|A vs. B", "|player|p1|A|1|", "|player|p2|B|2|", "|start", "|turn|1",
               "|move|p1a: X|Tackle|p2a: Z", "|turn|2")
    _, _, room = asyncio.run(play([init, msg(f"|request|{request(5)}")], bot, wait=0.1))
    assert room.rejoined
    _, _, fresh = asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}")], bot, wait=0.1))
    assert not fresh.rejoined


def test_deciding_again_after_a_refusal_is_timed_from_the_request():
    bot = SlowBot()
    refused = msg("|error|[Invalid choice] Can't move: bad")
    _, events, _ = asyncio.run(play([*RATED_START, msg(f"|request|{request(1)}"), refused], bot,
                                    DecisionTiming(deadline_s=2.0), gaps=[0, 0, 0, 0, 0, 0.3], wait=0.2))
    first, again = events.of("decision")[:2]
    assert again["seq"] == first["seq"] + 1 and again["timing"]["received"] == first["timing"]["received"]
    assert again["timing"]["total_ms"] >= 300 and again["fallback"] is None
