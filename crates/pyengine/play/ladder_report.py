"""Markdown strength and operations reports, plus full decision inspection from the complete battle logs."""
from __future__ import annotations

import argparse
import json
import math
import sqlite3
import sys
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
if sys.path and Path(sys.path[0] or ".").resolve() == HERE:
    sys.path[0] = str(HERE.parent)
elif str(HERE.parent) not in sys.path:
    sys.path.insert(0, str(HERE.parent))

from play.ladder import absolute_path, percentile  # noqa: E402
from play.runstore import read_battle  # noqa: E402


def stamp(value) -> str:
    return datetime.fromtimestamp(value, timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC") if value else "pending"


def cell(value) -> str:
    if value is None:
        return "—"
    return str(value).replace("|", "\\|").replace("\n", " ")


def table(headers, rows) -> str:
    return "\n".join(["| " + " | ".join(map(cell, headers)) + " |",
                      "| " + " | ".join("---" for _ in headers) + " |",
                      *("| " + " | ".join(map(cell, row)) + " |" for row in rows)])


def win_rate(battles) -> str:
    return f"{sum(b['result'] == 'win' for b in battles) / len(battles):.1%}" if battles else "—"


def interval(wins: int, n: int) -> str:
    """Wilson score interval: finite at 0 or 100%, including a one-game run."""
    if not n:
        return "—"
    z = 1.959963984540054
    p, denominator = wins / n, 1 + z * z / n
    center = (p + z * z / (2 * n)) / denominator
    radius = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denominator
    return f"{center - radius:.1%}–{center + radius:.1%}"


def fetch(conn, sql, values=()):
    return [dict(row) for row in conn.execute(sql, values)]


def battle_events(battle):
    try:
        return list(read_battle(Path(battle["log_path"])))
    except (OSError, ValueError, TypeError):
        return []


def report(conn: sqlite3.Connection, run_id: str | None = None) -> str:
    where, args = (" WHERE run_id=?", (run_id,)) if run_id else ("", ())
    runs = fetch(conn, "SELECT * FROM runs" + where + " ORDER BY started", args)
    if not runs:
        raise ValueError("No matching runs")
    battles = fetch(conn, "SELECT * FROM battles" + where + " ORDER BY ended,started", args)
    decisions = fetch(conn, "SELECT * FROM decisions" + where + " ORDER BY run_id,room,seq", args)
    ratings = fetch(conn, "SELECT * FROM ratings" + where + " ORDER BY time", args)
    events = fetch(conn, "SELECT * FROM events" + where + " ORDER BY time", args)
    resources = fetch(conn, "SELECT * FROM resources" + where + " ORDER BY time", args)
    resolved = [b for b in battles if b["result"] in {"win", "loss", "tie"}]
    rated = [b for b in resolved if b["rated"]]
    counts = Counter(b["result"] for b in rated)
    event_counts = Counter(e["kind"] for e in events)
    lines = [f"# Ladder report: {run_id or 'all runs'}", "",
             table(["Run", "Account", "Started", "Status"],
                   [(r["run_id"], r["account"], stamp(r["started"]), r["status"]) for r in runs]), "",
             "## Strength", "",
             f"Rated games: **{len(rated)}**; W/L/T: **{counts['win']}/{counts['loss']}/{counts['tie']}**. "
             f"Win rate: **{win_rate(rated)}** (95% Wilson interval {interval(counts['win'], len(rated))}). "
             "Ties are included in games and are not wins; technical failures remain in these figures.", "",
             f"Unrated finished games: {len(resolved) - len(rated)}; pending: "
             f"{sum(b['result'] is None for b in battles)}; "
             f"unknown: {sum(b['result'] == 'unknown' for b in battles)}.", "",
             f"Rolling win rate, last 20 rated games ({min(20, len(rated))} available): "
             f"**{win_rate(rated[-20:])}**.", ""]
    elo = [b["our_rating_after_room"] for b in rated if b["our_rating_after_room"] is not None]
    lines += [f"Room Elo: current **{elo[-1] if elo else 'unknown'}**, peak **{max(elo) if elo else 'unknown'}**.", "",
              table(["Game", "Run", "Room", "Time", "Result", "Opponent", "Opponent Elo", "Elo before", "Elo after"],
                    [(i, b["run_id"], b["room"], stamp(b["ended"]), b["result"], b["opponent"], b["opp_rating_before"],
                      b["our_rating_before"], b["our_rating_after_room"]) for i, b in enumerate(rated, 1)]), ""]
    latest = {}
    for r in ratings:
        if r["source"] == "users_json":
            latest[r["run_id"]] = r
    lines += ["### Latest official snapshots", "",
              table(["Run", "Time", "Elo", "GXE", "Glicko rpr ± rprd", "W/L"],
                    [(rid, stamp(r["time"]), r["elo"], r["gxe"], f"{r['rpr']} ± {r['rprd']}",
                      f"{r['w']}/{r['l']}") for rid, r in latest.items()]), ""]
    for rid, snapshot in latest.items():
        last_rooms = [b for b in rated if b["run_id"] == rid and b["our_rating_after_room"] is not None]
        if last_rooms and snapshot["elo"] != last_rooms[-1]["our_rating_after_room"]:
            lines += [f"Rating mismatch for {rid}: last room Elo {last_rooms[-1]['our_rating_after_room']}, "
                      f"official snapshot Elo {snapshot['elo']} at {stamp(snapshot['time'])}.", ""]
    if not latest:
        lines += ["No official snapshot captured.", ""]
    pending_snapshots = []
    # Pending snapshot flags belong to the complete run log rather than the derived ratings table.
    for r in runs:
        manifest = json.loads(r["manifest"])
        run_dir = Path(manifest.get("runs_dir", absolute_path("scratch/ladder/runs"))) / r["run_id"]
        try:
            states = {}
            for line in (run_dir / "events.jsonl").read_text(encoding="utf-8").splitlines():
                e = json.loads(line)
                if e["kind"] == "ratings_snapshot" and e.get("battle_room"):
                    states[e["battle_room"]] = e.get("pending", False)
                elif e["kind"] == "ratings_pending":
                    states[e.get("battle_room")] = True
            pending_snapshots += [room for room, pending in states.items() if pending]
        except (OSError, ValueError):
            pass
    if pending_snapshots:
        lines += ["Official ratings still pending: " + ", ".join(pending_snapshots) + ".", ""]
    opponent_ratings = [b["opp_rating_before"] for b in rated if b["opp_rating_before"] is not None]
    if opponent_ratings:
        lines += [f"Opponent Elo: mean {sum(opponent_ratings) / len(opponent_ratings):.1f}; "
                  f"range {min(opponent_ratings):g}–{max(opponent_ratings):g}.", ""]
    bands = defaultdict(list)
    for b in rated:
        rating = b["opp_rating_before"]
        band = f"{int(rating // 200) * 200}–{int(rating // 200) * 200 + 199}" if rating is not None else "unknown"
        bands[band].append(b)
    lines += [table(["Opponent rating band", "Games", "Wins", "Win rate"],
                    [(band, len(group), sum(b["result"] == "win" for b in group), win_rate(group))
                     for band, group in sorted(bands.items())]), "", "## Operations", ""]
    timing_rows = []
    for key in ("total_ms", "compute_ms"):
        values = [d[key] for d in decisions if d[key] is not None]
        timing_rows.append([key, len(values), *(f"{percentile(values, q):.1f}" if values else "—"
                                              for q in (.5, .9, .95, .99, 1))])
    lines += [table(["Timing (ms)", "Samples", "p50", "p90", "p95", "p99", "max"], timing_rows), ""]
    sources = Counter(d["source"] for d in decisions)
    lines += [table(["Decision source", "Count", "Share"],
                    [(source, sources[source], f"{sources[source] / len(decisions):.1%}" if decisions else "—")
                     for source in ("search", "policy", "fallback", "default")]), ""]
    # "no world: ... (last failure: <line>)" differs per decision; count the reasons, not the lines.
    notes = Counter(d["note"].split(" (last failure")[0] for d in decisions if d["note"] and d["source"] != "search")
    fallback_reasons = Counter(d["fallback"] or d["note"] or "unspecified" for d in decisions
                               if d["fallback"] or d["source"] == "fallback")
    lines += ["Policy notes (why search did not run):", "", table(["Note", "Count"], notes.most_common()), "",
              "Fallback reasons:", "", table(["Reason", "Count"], fallback_reasons.most_common()), ""]
    technical_rooms, disconnected_rooms = set(), set()
    for e in events:
        detail = json.loads(e["detail"])
        rooms = [e["room"]] if e["room"] else detail.get("pending_rooms", [])
        if e["kind"] in {"refused", "server_error", "fallback", "late_decision", "deadline_miss", "unhealthy",
                         "disconnect", "rejoin"}:
            technical_rooms.update((e["run_id"], room) for room in rooms)
        if e["kind"] == "disconnect":
            disconnected_rooms.update((e["run_id"], room) for room in rooms)
    technical_rooms.update((b["run_id"], b["room"]) for b in battles if b["errors"] or b["fallback"] or b["refused"])
    timer_losses = sum(b["result"] == "loss" and b["end_reason"] == "inactivity" for b in resolved)
    disconnect_losses = sum(b["result"] == "loss" and (b["run_id"], b["room"]) in disconnected_rooms for b in resolved)
    lines += [table(["Reliability", "Count"],
                    [("Deadline misses", event_counts["deadline_miss"]),
                     ("Refused choices", sum(b["refused"] for b in battles)),
                     ("Late decisions", event_counts["late_decision"]),
                     ("Disconnects", event_counts["disconnect"]),
                     ("Rejoins", sum(b["rejoins"] for b in battles)), ("Timer losses", timer_losses),
                     ("Losses with a disconnect", disconnect_losses),
                     ("Driver errors", sum(b["errors"] for b in battles)),
                     ("Unhealthy events", event_counts["unhealthy"]),
                     ("Authentication failures", event_counts["auth_fail"])]),
              "", "A loss with a disconnect means a disconnect was recorded during that battle; "
              "causality is unknown.", ""]
    comparison = []
    for name, problem in (("Technical problem", True), ("Clean", False)):
        group = [b for b in rated if ((b["run_id"], b["room"]) in technical_rooms) == problem]
        c = Counter(b["result"] for b in group)
        comparison.append([name, len(group), f"{c['win']}/{c['loss']}/{c['tie']}", win_rate(group)])
    lines += [table(["Rated battle group", "Games", "W/L/T", "Win rate"], comparison), ""]
    lines += ["### Resources", "", table(["Metric", "Mean", "Peak"],
              [(key, f"{sum(vals) / len(vals):.2f}", f"{max(vals):.2f}") for key in
               ("cpu", "proc_cpu", "rss_mb", "mem_percent", "worker_queue", "rooms")
               if (vals := [r[key] for r in resources if r[key] is not None])]), "", "## Deeper analysis", "",
              "Species figures cover only opponent species actually seen, with at least three rated games.", ""]
    species_groups = defaultdict(list)
    for b in rated:
        for species in set(json.loads(b["opp_species"])):
            species_groups[species].append(b)
    lines += [table(["Opponent species", "Games", "Losses", "Win rate"],
                    [(s, len(group), sum(b["result"] == "loss" for b in group), win_rate(group))
                     for s, group in sorted(species_groups.items()) if len(group) >= 3]), ""]
    tera = [b for b in rated if b["tera_used"]]
    turns = [b["tera_turn"] for b in tera if b["tera_turn"] is not None]
    lines += [f"Our Tera use: {len(tera)}/{len(rated)} rated games "
              f"({len(tera) / len(rated):.1%})." if rated else "Our Tera use: no rated games.", "",
              table(["Our Tera", "Games", "Win rate", "Mean turn", "Turn range"],
                    [("Used", len(tera), win_rate(tera), f"{sum(turns) / len(turns):.1f}" if turns else "—",
                      f"{min(turns)}–{max(turns)}" if turns else "—"),
                     ("Unused", len(rated) - len(tera),
                      win_rate([b for b in rated if not b["tera_used"]]), "—", "—")]), ""]
    outcomes = {(b["run_id"], b["room"]): 1 if b["result"] == "win" else .5 if b["result"] == "tie" else 0
                for b in rated}
    calibration = defaultdict(list)
    squared = []
    for d in decisions:
        key, p = (d["run_id"], d["room"]), d["win_probability"]
        if d["source"] == "search" and p is not None and 0 <= p <= 1 and key in outcomes:
            y = outcomes[key]
            calibration[min(9, int(p * 10))].append((p, y))
            squared.append((p - y) ** 2)
    lines += ["### Win-probability calibration", "",
              "Each search decision is one sample, against its rated battle result (ties = 0.5). "
              "Multiple decisions from the same battle are correlated.", "",
              f"Brier score: **{sum(squared) / len(squared):.4f}** ({len(squared)} decisions)." if squared else
              "Brier score: unavailable (no search probabilities with known rated results).", "",
              table(["Probability bin", "Decisions", "Mean prediction", "Observed result"],
                    [(f"{i / 10:.1f}–{(i + 1) / 10:.1f}", len(group),
                      f"{sum(p for p, y in group) / len(group):.3f}", f"{sum(y for p, y in group) / len(group):.3f}")
                     for i, group in sorted(calibration.items())]), ""]
    compared = [d for d in decisions if d["source"] == "search" and d["agrees_with_policy"] is not None]
    disagreements = Counter((d["run_id"], d["room"]) for d in compared if not d["agrees_with_policy"])
    lines += [f"Search vs policy disagreement: {sum(disagreements.values())}/{len(compared)} "
              f"({sum(disagreements.values()) / len(compared):.1%})." if compared else
              "Search vs policy disagreement: no policy comparison available.", ""]
    disagreement_groups = defaultdict(list)
    for b in rated:
        disagreement_groups[disagreements[(b["run_id"], b["room"])]].append(b)
    lines += [table(["Disagreements in battle", "Games", "Win rate"],
                    [(n, len(group), win_rate(group)) for n, group in sorted(disagreement_groups.items())]), "",
              "### Suspicious decisions", ""]
    grouped = defaultdict(list)
    for d in decisions:
        grouped[(d["run_id"], d["room"])].append(d)
    log_decisions = {}
    for b in battles:
        for e in battle_events(b):
            if e["kind"] == "decision":
                log_decisions[(b["run_id"], b["room"], e.get("seq"))] = e
    suspicious = []
    for key, group in grouped.items():
        for i, d in enumerate(group):
            reasons = []
            if d["fallback"] or d["source"] == "fallback":
                reasons.append("fallback: " + (d["fallback"] or d["note"] or "unspecified"))
            if i + 1 < len(group) and d["win_probability"] is not None and group[i + 1]["win_probability"] is not None:
                drop = d["win_probability"] - group[i + 1]["win_probability"]
                if drop >= .35:
                    reasons.append(f"next decision probability dropped {drop:.2f}")
            if d["source"] == "search" and d["agrees_with_policy"] == 0:
                e = log_decisions.get((*key, d["seq"]), {})
                candidates = (e.get("analysis") or {}).get("candidates") or []
                choices = {c.get("choice"): c.get("value") for c in candidates}
                chosen, policy = choices.get(d["choice"]), choices.get(d["policy_choice"])
                if chosen is not None and policy is not None and 0 <= chosen - policy < .02:
                    reasons.append(f"disagreement, value gain only {chosen - policy:.4f}")
            if reasons:
                suspicious.append([d["run_id"], f"{d['room']}:{d['seq']}", d["turn"], "; ".join(reasons)])
    lines += [table(["Run", "Decision id", "Turn", "Reason"], suspicious), ""]
    return "\n".join(lines)


def inspect_decision(conn, decision_id: str, run_id: str | None = None) -> str:
    try:
        room, seq_text = decision_id.rsplit(":", 1)
        seq = int(seq_text)
    except ValueError:
        raise ValueError("Decision id must be ROOM:SEQ") from None
    sql, args = "SELECT * FROM battles WHERE room=?", [room]
    if run_id:
        sql += " AND run_id=?"
        args.append(run_id)
    battles = fetch(conn, sql, args)
    if len(battles) != 1:
        raise ValueError("Battle missing or ambiguous; specify --run ID")
    events = battle_events(battles[0])
    selected, request, messages = None, None, []
    for e in events:
        if e["kind"] == "message":
            messages.extend(line for line in e.get("text", "").splitlines() if not line.startswith("|request|"))
        elif e["kind"] == "request":
            request = e
            request_messages = list(messages)
        elif e["kind"] == "decision" and e.get("seq") == seq:
            selected = e
            break
    if selected is None:
        raise ValueError("Decision not found in the complete battle log")
    analysis, timing = selected.get("analysis") or {}, selected.get("timing") or {}
    lines = [f"# Decision {decision_id}", "", "## Battle lines up to the request", "", "```text",
             "\n".join(request_messages if request else messages), "```", "", "## Request", ""]
    if request:
        body = request.get("text", request.get("request", {}))
        try:
            body = json.loads(body) if isinstance(body, str) else body
        except ValueError:
            body = {"unparsed": body}
        # Keep the body readable while putting each of our Pokemon on exactly one line.
        pretty = json.dumps(body, indent=2, ensure_ascii=False)
        pokemon = (body.get("side") or {}).get("pokemon") if isinstance(body, dict) else None
        if pokemon:
            compact = "[\n" + ",\n".join("      " + json.dumps(p, ensure_ascii=False) for p in pokemon) + "\n    ]"
            body = dict(body)
            side = dict(body["side"])
            side["pokemon"] = "__POKEMON_ONE_PER_LINE__"
            body["side"] = side
            pretty = json.dumps(body, indent=2, ensure_ascii=False).replace('"__POKEMON_ONE_PER_LINE__"', compact)
        lines += ["```json", pretty, "```", ""]
    else:
        lines += ["No request observer event was recorded.", ""]
    lines += ["## Candidates", "", table(["Choice", "Text", "Prior", "Value", "Weight"],
              [(c.get("choice"), c.get("text"), c.get("prior"), c.get("value"), c.get("weight"))
               for c in analysis.get("candidates", [])]), "", "## Opponent replies", "",
              table(["Choice", "Text", "Prior", "Value", "Weight"],
                    [(c.get("choice"), c.get("text"), c.get("prior"), c.get("value"), c.get("weight"))
                     for c in analysis.get("opponent", [])]), "", "## Policy choice", "", "```json",
              json.dumps(analysis.get("policy"), indent=2, ensure_ascii=False), "```", "",
              "## Observed opponent information", "", "```json", json.dumps(analysis.get("observed"), indent=2),
              "```", "", "## Inferred opponent information", "", "```json",
              json.dumps(analysis.get("inferred"), indent=2), "```", "", "## Timing", "", "```json",
              json.dumps(timing, indent=2), "```", "", "## Full decision event", "", "```json",
              json.dumps(selected, indent=2, ensure_ascii=False), "```", ""]
    return "\n".join(lines)


def inspect_battle(conn, room: str, run_id: str | None = None) -> str:
    sql, args = "SELECT * FROM battles WHERE room=?", [room]
    if run_id:
        sql += " AND run_id=?"
        args.append(run_id)
    rows = fetch(conn, sql, args)
    if len(rows) != 1:
        raise ValueError("Battle missing or ambiguous; specify --run ID")
    battle = rows[0]
    events = battle_events(battle)
    return "\n".join([f"# Battle {room}", "", "```json", json.dumps(battle, indent=2), "```", "",
                      table(["Decision id", "Turn", "Source", "Choice", "Total ms"],
                            [(f"{room}:{e['seq']}", e.get("turn"), e.get("source"), e.get("choice"),
                              (e.get("timing") or {}).get("total_ms")) for e in events if e["kind"] == "decision"]),
                      "", table(["Reliability event", "Time", "Detail"],
                                [(e["kind"], stamp(e["t"]), e) for e in events if e["kind"] in
                                 {"refused", "server_error", "late_decision", "unhealthy", "rejoin"}]), ""])


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--run")
    group.add_argument("--all", action="store_true")
    parser.add_argument("--db", default="scratch/ladder/ladder.sqlite")
    parser.add_argument("--decision")
    parser.add_argument("--battle")
    parser.add_argument("--plots", action="store_true")
    args = parser.parse_args(argv)
    try:
        db = absolute_path(args.db)
        with sqlite3.connect(db.as_uri() + "?mode=ro", uri=True) as conn:
            conn.row_factory = sqlite3.Row
            if args.decision:
                output = inspect_decision(conn, args.decision, args.run)
            elif args.battle:
                output = inspect_battle(conn, args.battle, args.run)
            else:
                output = report(conn, args.run)
                if args.run:
                    row = conn.execute("SELECT manifest FROM runs WHERE run_id=?", (args.run,)).fetchone()
                    manifest = json.loads(row[0])
                    run_dir = Path(manifest.get("runs_dir", absolute_path("scratch/ladder/runs"))) / args.run
                    run_dir.mkdir(parents=True, exist_ok=True)
                    (run_dir / "report.md").write_text(output, encoding="utf-8")
                    if args.plots:
                        try:
                            import matplotlib
                            matplotlib.use("Agg")
                            import matplotlib.pyplot as plt
                        except ImportError:
                            print("Matplotlib unavailable; skipping plots.", file=sys.stderr)
                        else:
                            games = fetch(conn, "SELECT our_rating_after_room FROM battles WHERE run_id=? AND "
                                          "rated=1 AND result IN ('win','loss','tie') ORDER BY ended", (args.run,))
                            fig, ax = plt.subplots()
                            ax.plot(range(1, len(games) + 1), [b["our_rating_after_room"]
                                                             if b["our_rating_after_room"] is not None else math.nan
                                                             for b in games], marker=".")
                            ax.set(xlabel="Rated game", ylabel="Elo", title=args.run)
                            fig.tight_layout()
                            fig.savefig(run_dir / "elo.png")
                            plt.close(fig)
            print(output)
        return 0
    except (OSError, ValueError, sqlite3.Error) as error:
        print(f"Report failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):  # user names can hold characters a Windows console cannot show
        stream.reconfigure(errors="replace")
    sys.exit(main())
