"""Before a ladder run: check that the frozen bot really searches, on this machine, within its time rules.

``run_preflight(bot, timing)`` plays a short in-process battle: the bot's protocol-driven player on side 1, fed only
its player view and requests (as on Showdown), against Showdown's ``default`` choices on side 2, with the run's
search allowance and deadline handed to the player the way play.driver does. It passes when the search extension is
loaded, the battle runs without errors, most decisions come from search, and none needed the deadline fallback.

Call it on the bot's worker thread (``await worker.run(run_preflight, bot, timing)``); it takes a few seconds.
"""
from __future__ import annotations

import gzip
import time

import numpy as np

MAX_DECISIONS = 24
MIN_SEARCH_SHARE = 0.8


def _teams(bot) -> list[str]:
    """Two teams for the preflight battle: the first lines of the bot's own team pool."""
    teams = []
    with gzip.open(bot.pool_paths[0], "rt") as pool:
        for line in pool:
            if line.strip():
                teams.append(line.strip())
            if len(teams) == 2:
                break
    return teams


def run_preflight(bot, timing=None) -> dict:
    """Play one short battle with ``bot`` (a training.search.SearchBot) and report whether its search works."""
    import numbrion

    result = {"ok": False, "search_extension": bool(getattr(numbrion, "SEARCH", False)),
              "fused_grid": bool(getattr(numbrion, "FUSED_GRID", False))}
    problems = []
    if getattr(bot, "search", None) is None:
        problems.append("search is off in the bot's config")
    if not result["search_extension"] or not result["fused_grid"]:
        problems.append("numbrion was built without the search feature (maturin --features python,search)")
    if problems:
        result["summary"] = "; ".join(problems)
        return result

    search_s = getattr(timing, "search_s", None)
    deadline_s = getattr(timing, "deadline_s", None)
    reserve_s = getattr(timing, "reserve_s", 0.5) or 0.0
    teams = _teams(bot)
    battle = numbrion.Battle((11, 23, 5, 8), p1=teams[0], p2=teams[1], log=True)
    battle.start()
    player = bot.new_player("preflight")
    sources, notes, times, late = {}, {}, [], 0
    started = time.perf_counter()
    try:
        decisions = 0
        while not battle.ended and decisions < MAX_DECISIONS:
            player.feed(battle.drain_player_log(0))
            battle.drain_player_log(1)
            if battle.needs_action(0):
                player.set_request(battle.request_json(0))
                begin = time.monotonic()
                deadline = None if deadline_s is None else begin + deadline_s - reserve_s
                player.budget_ms = search_s * 1000.0 if search_s else None
                if deadline is not None:
                    left = (deadline - time.monotonic()) * 1000.0 - 250.0
                    player.budget_ms = left if player.budget_ms is None else min(player.budget_ms, left)
                player.deadline = deadline
                player.decision_token = ("preflight", decisions)
                decision = player.decide()
                elapsed = time.monotonic() - begin
                times.append(elapsed * 1000.0)
                if deadline is not None and time.monotonic() > deadline:
                    late += 1
                analysis = decision.analysis
                sources[analysis.get("source", "?")] = sources.get(analysis.get("source", "?"), 0) + 1
                if analysis.get("note"):
                    notes[analysis["note"][:80]] = notes.get(analysis["note"][:80], 0) + 1
                try:
                    battle.choose(0, decision.choice)
                except numbrion.ChoiceError as error:  # a hidden trap the request could not show
                    player.rejected(str(error.args[0]))
                    player.set_request(error.args[1])
                    battle.choose(0, player.decide().choice)
                decisions += 1
            if battle.needs_action(1):
                battle.choose(1, "default")
    except Exception as error:  # noqa: BLE001 - reported, the run does not start
        problems.append(f"the preflight battle failed: {type(error).__name__}: {error}")
    finally:
        player.close()
    forced = notes.get("only one legal choice", 0)
    searched = sources.get("search", 0)
    chosen = sum(sources.values()) - forced
    share = searched / chosen if chosen else 0.0
    result.update(decisions=sum(sources.values()), sources=sources, notes=notes, search_share=round(share, 3),
                  late=late, seconds=round(time.perf_counter() - started, 1),
                  time_ms={"median": round(float(np.median(times)), 1), "max": round(max(times), 1)} if times
                  else None, player_stats={k: round(v, 1) if isinstance(v, float) else v
                                           for k, v in (getattr(player, "stats", {}) or {}).items()})
    if chosen < 4:
        problems.append(f"only {chosen} decisions to check")
    elif share < MIN_SEARCH_SHARE:
        problems.append(f"only {searched} of {chosen} decisions searched (notes: {notes})")
    if late:
        problems.append(f"{late} decisions missed the {deadline_s} s deadline")
    result["ok"] = not problems
    result["summary"] = "; ".join(problems) if problems else (
        f"search ran on {searched} of {chosen} decisions, median {result['time_ms']['median']:.0f} ms, "
        f"max {result['time_ms']['max']:.0f} ms")
    return result
