"""ShowdownClient on synthetic messages: registered login and the battle cap of a public bot.

    cd crates/pyengine && ../../.venv/bin/python -m pytest play/test_client.py -q
"""
from __future__ import annotations

import asyncio
from types import SimpleNamespace

import pytest

from play import client as client_module
from play.auth import AuthError, Credentials
from play.client import ShowdownClient
from play.driver import BotLog

CHALLENGE = "|pm| Someone| Numbrion Spark|/challenge gen9randomdoublesbattle|gen9randomdoublesbattle||"
ROOM = "battle-gen9randomdoublesbattle-1"


class FakeWs:
    def __init__(self):
        self.closed = False

    async def close(self):
        self.closed = True


def make_client(**kwargs) -> tuple[ShowdownClient, list[str]]:
    client = ShowdownClient(SimpleNamespace(name="Numbrion Spark"), "ws://test", BotLog(), None, **kwargs)
    sent: list[str] = []

    async def send(text: str) -> None:
        sent.append(text)

    client.send = send
    client.ws = FakeWs()
    return client, sent


async def settle():
    for _ in range(5):
        await asyncio.sleep(0)


def test_challenge_accepted_below_the_cap():
    async def body():
        client, sent = make_client(max_battles=2)
        client.games = {"battle-gen9randomdoublesbattle-1"}
        client.on_message(CHALLENGE)
        await settle()
        assert sent == ["|/utm null", "|/accept someone"]
    asyncio.run(body())


def test_challenge_rejected_at_the_cap():
    async def body():
        client, sent = make_client(max_battles=1)
        client.games = {"battle-gen9randomdoublesbattle-1"}
        client.on_message(CHALLENGE)
        client.on_message("|pm| Other| Numbrion Spark|challenge me")
        await settle()
        assert sent[0] == "|/reject someone"
        assert sent[1].startswith("|/pm someone, Sorry, I am playing 1 battles")
        assert sent[2].startswith("|/pm other, Sorry")
        assert not any("/accept" in text or "/challenge" in text for text in sent)
    asyncio.run(body())


def test_registered_login_sends_the_assertion(monkeypatch):
    async def fake_assertion(session, username, password, challstr, url=None):
        assert (username, password, challstr) == ("Numbrion Spark", "secret", "4|abc")
        return "ASSERTION"

    monkeypatch.setattr(client_module, "login_assertion", fake_assertion)

    async def body():
        client, sent = make_client(credentials=Credentials("Numbrion Spark", "secret"))
        await client.login("4|abc")
        await client.http.close()
        assert sent == ["|/trn Numbrion Spark,0,ASSERTION"]
    asyncio.run(body())


def test_refused_login_closes_and_counts(monkeypatch):
    async def refuse(*args, **kwargs):
        raise AuthError("wrong password")

    monkeypatch.setattr(client_module, "login_assertion", refuse)

    async def body():
        client, sent = make_client(credentials=Credentials("Numbrion Spark", "secret"))
        ws = client.ws
        await client.login("4|abc")
        await client.http.close()
        assert sent == [] and ws.closed and client.auth_failures == 1
    asyncio.run(body())


def test_finished_battle_frees_capacity_without_updatesearch():
    async def body():
        client, sent = make_client(max_battles=1)
        room = SimpleNamespace(room=ROOM, ended=True, result={"tie": True}, turn=10, stats={})
        client.rooms[ROOM] = room
        client.on_message(f'|updatesearch|{{"games":{{"{ROOM}":"A vs B"}}}}')
        assert client.busy()
        client._room_ended(room)
        assert ROOM not in client.games and not client.busy()
        # A stale server list must not make the bot busy again.
        client.on_message(f'|updatesearch|{{"games":{{"{ROOM}":"A vs B"}}}}')
        assert not client.busy()
        client.on_message(CHALLENGE)
        await settle()
        assert "|/accept someone" in sent
    asyncio.run(body())


def test_concurrent_challenges_reserve_capacity_before_accept():
    async def body():
        client, sent = make_client(max_battles=1)
        client.on_message(CHALLENGE)
        client.on_message(CHALLENGE)
        client.on_message(CHALLENGE.replace("Someone", "Other"))
        assert client.busy() and set(client.pending_challenges) == {"someone"}
        await settle()
        assert sent.count("|/accept someone") == 1
        assert "|/reject other" in sent and "|/accept other" not in sent
    asyncio.run(body())


@pytest.mark.parametrize("from_pm", [False, True])
def test_outgoing_challenges_reserve_capacity(from_pm):
    async def body():
        client, sent = make_client(max_battles=1)
        if from_pm:
            client.on_message("|pm| Other| Numbrion Spark|challenge me")
        else:
            await client.challenge("Other")
        client.on_message(CHALLENGE)
        await settle()
        assert "|/challenge other, gen9randomdoublesbattle" in sent
        assert "|/reject someone" in sent and "|/accept someone" not in sent
    asyncio.run(body())


def test_challenge_reservations_release_on_cancel_timeout_and_disconnect(monkeypatch):
    now = [100.0]
    monkeypatch.setattr(client_module.time, "monotonic", lambda: now[0])

    async def body():
        client, sent = make_client(max_battles=1)
        client.on_message(CHALLENGE)
        client.on_message("|pm| Someone| Numbrion Spark|/challenge")
        await settle()
        assert not client.busy() and "|/accept someone" not in sent
        await client.challenge("Someone")
        client.on_message("|pm| Someone| Numbrion Spark|/challenge")
        assert not client.busy()
        client.on_message(CHALLENGE)
        await settle()
        now[0] += 61.0
        client.on_message(CHALLENGE)
        await settle()
        assert sent.count("|/accept someone") == 2
        client._abandon_rooms()
        assert not client.pending_challenges and not client.busy()
        await client.challenge("Other")
        await client.stop()
        assert not client.pending_challenges
    asyncio.run(body())


@pytest.mark.parametrize("split_header", [False, True])
def test_battle_header_replaces_only_opponents_reservation(monkeypatch, split_header):
    class FakeRoom:
        def __init__(self, *args, **kwargs):
            pass

        def start(self):
            pass

        def push(self, *args):
            pass

    monkeypatch.setattr(client_module, "BattleRoom", FakeRoom)

    async def body():
        client, _ = make_client(max_battles=3)
        await client.challenge("Someone")
        await client.challenge("Other")
        header = "|player|p1|Numbrion Spark|1|\n|player|p2|Someone|1|"
        client.on_message(f">{ROOM}\n|init|battle" + ("" if split_header else "\n" + header))
        if split_header:
            client.on_message(f">{ROOM}\n{header}")
        assert set(client.pending_challenges) == {"other"}
        assert not client.busy()
        client.on_message(CHALLENGE.replace("Someone", "Third"))
        assert client.busy()
        await settle()
    asyncio.run(body())


@pytest.mark.parametrize("closed", [False, True])
def test_dropped_login_commands_are_redacted(caplog, closed):
    class ClosedWs(FakeWs):
        async def send(self, text):
            raise client_module.websockets.ConnectionClosed(None, None)

    async def body():
        client, _ = make_client()
        client.ws = ClosedWs() if closed else None
        await ShowdownClient.send(client, "|/trn Numbrion Spark,0,SECRET-ASSERTION")
        assert "SECRET-ASSERTION" not in caplog.text
        assert "|/trn [redacted]" in caplog.text
    asyncio.run(body())


@pytest.mark.parametrize("refused", [False, True])
def test_old_login_cannot_send_or_close_replacement_socket(monkeypatch, refused):
    async def body():
        started, release = asyncio.Event(), asyncio.Event()

        async def assertion(*args):
            started.set()
            await release.wait()
            if refused:
                raise AuthError("wrong password")
            return "OLD-ASSERTION"

        monkeypatch.setattr(client_module, "login_assertion", assertion)
        client, sent = make_client(credentials=Credentials("Numbrion Spark", "secret"))
        client.auth_failures = 2
        client.on_message("|challstr|4|old")
        task = client.login_task
        await started.wait()
        replacement = client.ws = FakeWs()
        release.set()
        await task
        assert sent == [] and not replacement.closed and client.auth_failures == 2
        await client.stop()
        assert client.http.closed
    asyncio.run(body())


def test_queued_login_is_bound_to_original_socket(monkeypatch):
    def unexpected_session():
        pytest.fail("a stale login created an HTTP session")

    monkeypatch.setattr(client_module.aiohttp, "ClientSession", unexpected_session)

    async def body():
        client, sent = make_client(credentials=Credentials("Numbrion Spark", "secret"))
        client.on_message("|challstr|4|old")
        task = client.login_task
        client.ws = FakeWs()
        await task
        assert sent == [] and client.http is None
    asyncio.run(body())


def test_stop_cancels_queued_login_and_prevents_late_sessions(monkeypatch):
    def unexpected_session():
        pytest.fail("a stopped client created an HTTP session")

    monkeypatch.setattr(client_module.aiohttp, "ClientSession", unexpected_session)

    async def body():
        client, sent = make_client(credentials=Credentials("Numbrion Spark", "secret"))
        client.on_message("|challstr|4|abc")
        task = client.login_task
        await client.stop()
        client.on_message("|challstr|4|late")
        await client.login("4|late")
        await settle()
        assert task.cancelled() and client.login_task is None
        assert sent == [] and client.http is None
    asyncio.run(body())


@pytest.mark.parametrize("disconnect", [False, True])
def test_login_task_cancelled_on_stop_or_disconnect(monkeypatch, disconnect):
    async def body():
        started, cancelled = asyncio.Event(), asyncio.Event()

        async def assertion(*args):
            started.set()
            try:
                await asyncio.Future()
            finally:
                cancelled.set()

        monkeypatch.setattr(client_module, "login_assertion", assertion)
        client, sent = make_client(credentials=Credentials("Numbrion Spark", "secret"), max_battles=1)
        client._reserve_challenge("someone")
        if disconnect:
            class Connection(FakeWs):
                async def __aenter__(self):
                    return self

                async def __aexit__(self, *args):
                    pass

                async def __aiter__(self):
                    yield "|challstr|4|abc"
                    await started.wait()
                    client.stopping = True

            monkeypatch.setattr(client_module.websockets, "connect", lambda *args, **kwargs: Connection())
            client.worker.ready = asyncio.Event()
            client.worker.ready.set()
            await client.run()
            assert client.ws is None
        else:
            client.on_message("|challstr|4|abc")
            task = client.login_task
            client.on_message("|challstr|4|duplicate")
            assert client.login_task is task
            await started.wait()
        await client.stop()
        assert cancelled.is_set() and client.login_task is None
        assert not client.pending_challenges and sent == [] and client.http.closed
    asyncio.run(body())


@pytest.mark.parametrize("busy", [False, True])
def test_challenge_pm_cooldown(monkeypatch, busy):
    now = [100.0]
    monkeypatch.setattr(client_module.time, "monotonic", lambda: now[0])

    async def body():
        client, sent = make_client(max_battles=1)
        if busy:
            client.games.add(ROOM)
        for _ in range(20):
            client.on_message("|pm| Someone| Numbrion Spark|challenge me")
        await settle()
        prefix = "|/pm someone," if busy else "|/challenge someone,"
        assert sum(text.startswith(prefix) for text in sent) == 1
        client.on_message("|pm| Someone| Numbrion Spark|/challenge")
        now[0] += 31.0
        client.on_message("|pm| Someone| Numbrion Spark|challenge me")
        await settle()
        assert sum(text.startswith(prefix) for text in sent) == 2
    asyncio.run(body())


def test_optional_pm_global_limit_preserves_required_rejects(monkeypatch):
    now = [100.0]
    monkeypatch.setattr(client_module.time, "monotonic", lambda: now[0])

    async def body():
        client, sent = make_client(max_battles=1)
        client.games.add(ROOM)
        for i in range(20):
            client.on_message(CHALLENGE.replace("Someone", f"User{i}"))
        await settle()
        assert sum(text.startswith("|/reject ") for text in sent) == 20
        assert sum(text.startswith("|/pm ") for text in sent) == 5
        for i in range(20):
            client.on_message(f"|pm| User{i}| Numbrion Spark|challenge me")
            client.on_message(f"|pm| Help{i}| Numbrion Spark|hello")
        await settle()
        assert sum(text.startswith("|/pm ") for text in sent) == 5
        now[0] += 31.0
        client.on_message("|pm| User0| Numbrion Spark|challenge me")
        await settle()
        assert sum(text.startswith("|/pm ") for text in sent) == 6
    asyncio.run(body())


def test_challenge_backs_share_global_optional_pm_limit():
    async def body():
        client, sent = make_client()
        for i in range(20):
            client.on_message(f"|pm| User{i}| Numbrion Spark|challenge me")
        await settle()
        assert sum(text.startswith("|/challenge ") for text in sent) == 5
    asyncio.run(body())
