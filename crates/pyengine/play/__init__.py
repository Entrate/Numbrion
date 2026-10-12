"""Play against the bots on a local Showdown server (docs/play/PLAY.md).

* ``play.server``: entry point; bot accounts on Showdown, the HTTP API of docs/play/ADVISOR-API.md, self-play.
* ``play.driver``: one bot's side of one battle room (the ``training.player`` event-order contract) and the
  per-bot worker thread.
* ``play.client``: one websocket connection per bot account (login, challenges, rooms, raw logs).
* ``play.api``: the aiohttp app (``/api/status``, ``/api/advise``, ``/api/bot-log``).
* ``play.advise``: the advisor's rooms: feeding lines, or replaying the request history, into a player.
* ``play.testbots``: a random bot that checks the event order, for tests.
"""
