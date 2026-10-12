#!/usr/bin/env python3
"""Mock of the play server's HTTP API (docs/play/ADVISOR-API.md) for developing the Chrome extension.

    python3 tools/chrome-extension/dev/mock_api.py [--port 8765] [--delay-ms 300] [--log advise.jsonl]

POST /api/advise answers with a legal choice for the request (first usable move per active slot, a target when
the move needs one, Tera on the first slot when available; first healthy bench pokemon for forced switches) and
made-up analysis (win probability, candidates, opponent replies). Each request is checked against the API
contract, including ``history``, and summarised to --log as one JSON line. Like the real server, the analysis
says "policy" instead of "search" when the history misses past choices or does not reach back to turn 1. Every advised decision of a room moves to that room's bot log
once a newer request of the room is advised, so GET /api/bot-log has entries too. GET /api/_last?room=...
returns the last advise body (for end-to-end checks). Standard library only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

LOCK = threading.Lock()
ROOMS: dict[str, dict] = {}  # room -> {"last": entry | None, "log": [entries], "body": last body}
ARGS = None

TARGETED = {"normal", "any", "adjacentFoe"}


def rand(*parts) -> float:
    digest = hashlib.sha256("|".join(map(str, parts)).encode()).digest()
    return int.from_bytes(digest[:4], "big") / 2**32


def actives_from_lines(lines: list[str]) -> dict[str, str]:
    actives = {}
    for line in lines:
        parts = line.split("|")
        if len(parts) > 2 and parts[1] in ("switch", "drag", "replace"):
            m = re.match(r"^(p\d[a-z]): (.*)$", parts[2])
            if m:
                actives[m.group(1)] = m.group(2)
    return actives


def name_of(mon: dict) -> str:
    return re.sub(r"^p\d[a-z]?: ", "", mon.get("ident", "?"))


def fainted(mon: dict) -> bool:
    return str(mon.get("condition", "")).endswith(" fnt")


def legal_choice(req: dict, actives: dict[str, str]):
    """(choice, slots, alternatives) or None when no choice is needed."""
    if not req or req.get("wait") or req.get("teamPreview"):
        return None
    side = req.get("side") or {}
    me = side.get("id", "p1")
    foe = "p2" if me == "p1" else "p1"
    team = side.get("pokemon") or []
    if req.get("forceSwitch"):
        bench = [i for i, mon in enumerate(team) if not mon.get("active") and not fainted(mon)]
        parts, slots = [], []
        for slot, must in enumerate(req["forceSwitch"]):
            if must and bench:
                i = bench.pop(0)
                parts.append(f"switch {i + 1}")
                slots.append({"pokemon": name_of(team[slot]) if slot < len(team) else "?",
                              "text": f"switch to {name_of(team[i])}", "code": 20 + i})
            else:
                parts.append("pass")
        if all(p == "pass" for p in parts):
            return None
        return ", ".join(parts), slots, []
    parts, slots, alternatives = [], [], []
    tera_used = False
    for slot, act in enumerate(req.get("active") or []):
        mon = team[slot] if slot < len(team) else {}
        if not act or fainted(mon) or mon.get("commanding"):
            parts.append("pass")
            continue
        moves = act.get("moves") or []
        usable = [i for i, m in enumerate(moves) if not m.get("disabled")] or [0]
        options = []
        for i in usable:
            move = moves[i] if i < len(moves) else {"move": "Struggle", "target": "randomNormal"}
            target = move.get("target")
            text, choice = move.get("move", "?"), f"move {i + 1}"
            if target in TARGETED:
                foe_pos = foe + "a"
                choice += " +1"
                text += " → " + actives.get(foe_pos, "foe 1")
            elif target == "adjacentAlly":
                choice += f" -{2 if slot == 0 else 1}"
                text += " → " + actives.get(me + ("b" if slot == 0 else "a"), "ally")
            elif target == "adjacentAllyOrSelf":
                choice += f" -{slot + 1}"
            options.append((choice, text))
        choice, text = options[0]
        if act.get("canTerastallize") and not tera_used and slot == 0:
            choice += " terastallize"
            text += " (Tera " + str(act["canTerastallize"]) + ")"
            tera_used = True
        parts.append(choice)
        slots.append({"pokemon": name_of(mon), "text": text, "code": usable[0] + 1})
        alternatives.append(options)
    return ", ".join(parts), slots, alternatives


def make_decision(room: str, req: dict, lines: list[str]):
    picked = legal_choice(req, actives_from_lines(lines))
    if picked is None:
        return None
    choice, slots, alternatives = picked
    rqid = req.get("rqid")
    value = round(rand(room, rqid, "v") * 1.2 - 0.5, 3)
    candidates = [{"choice": choice, "text": " / ".join(s["text"] for s in slots), "prior": 0.0, "value": value, "weight": 0.0}]
    # A few variants: other moves for the first slot.
    if alternatives and len(alternatives[0]) > 1:
        rest = choice.split(", ")[1:]
        for alt_choice, alt_text in alternatives[0][1:4]:
            text = " / ".join([alt_text] + [s["text"] for s in slots[1:]])
            candidates.append({"choice": ", ".join([alt_choice] + rest), "text": text, "prior": 0.0,
                               "value": round(value - 0.05 - 0.4 * rand(room, rqid, alt_choice), 3), "weight": 0.0})
    priors = [0.25 + rand(room, rqid, "p", i) for i in range(len(candidates))]
    weights = [0.5 + rand(room, rqid, "w", i) for i in range(len(candidates))]
    weights[0] = max(weights) + 0.4
    for cand, prior, weight in zip(candidates, priors, weights):
        cand["prior"] = round(prior / sum(priors), 3)
        cand["weight"] = round(weight / sum(weights), 3)
    me = (req.get("side") or {}).get("id", "p1")
    foe = "p2" if me == "p1" else "p1"
    actives = actives_from_lines(lines)
    foes = [actives.get(foe + "a", "Foe A"), actives.get(foe + "b", "Foe B")]
    opponent = []
    for i, verb in enumerate(["Protect", "attacks", "switches"]):
        opponent.append({"text": f"{foes[i % 2]}: {verb}" + (f" → {slots[0]['pokemon']}" if verb == "attacks" and slots else ""),
                         "prior": round(0.5 / (i + 1), 3), "weight": round(0.55 / (i + 1.2), 3)})
    analysis = {
        "source": "search",
        "value": value,
        "win_probability": round((value + 1) / 2, 3),
        "time_ms": ARGS.delay_ms,
        "worlds": 32,
        "cells": 2048,
        "slots": slots,
        "candidates": candidates,
        "opponent": opponent,
        "note": "mock advisor: first usable move per slot",
    }
    return {"choice": choice, "a0": slots[0]["code"] if slots else -1, "a1": slots[1]["code"] if len(slots) > 1 else -1,
            "analysis": analysis}


def check_history(body: dict, lines: list[str]) -> tuple[list[str], dict]:
    """Problems with ``history`` and a summary: entries, known choices, missing choices, starts at turn <= 1."""
    history = body.get("history")
    info = {"entries": 0, "choices": 0, "missing": 0, "from_start": False}
    if history is None:
        return [], info
    problems = []
    if not isinstance(history, list) or not history:
        return ["history is not a non-empty list"], info
    info["entries"] = len(history)
    last_index = 0
    for i, entry in enumerate(history):
        if not isinstance(entry, dict) or set(entry) != {"index", "rqid", "request", "choice"}:
            problems.append(f"history[{i}] keys are not index, rqid, request, choice")
            continue
        if not isinstance(entry["index"], int) or not last_index <= entry["index"] <= len(lines):
            problems.append(f"history[{i}].index {entry['index']} out of order or past the lines")
        else:
            last_index = entry["index"]
        try:
            req = json.loads(entry["request"])
        except (TypeError, ValueError):
            problems.append(f"history[{i}].request is not JSON")
            continue
        if req and req.get("rqid") != entry["rqid"]:
            problems.append(f"history[{i}].rqid {entry['rqid']} differs from its request's {req.get('rqid')}")
        if entry["choice"] is not None and not isinstance(entry["choice"], str):
            problems.append(f"history[{i}].choice is not a string or null")
        if i < len(history) - 1:
            if entry["choice"]:
                info["choices"] += 1
            elif req and not req.get("wait") and not req.get("teamPreview"):
                info["missing"] += 1
    last = history[-1]
    if isinstance(last, dict):
        if last.get("request") != body.get("request"):
            problems.append("the last history entry is not the current request")
        if last.get("choice") is not None:
            problems.append("the last history entry has a choice")
    first = history[0] if isinstance(history[0], dict) else {}
    before = lines[: first.get("index", 0)] if isinstance(first.get("index"), int) else []
    turns = [int(line[6:]) for line in before if line.startswith("|turn|") and line[6:].isdigit()]
    info["from_start"] = max(turns, default=0) <= 1
    return problems, info


def check_body(body: dict) -> tuple[list[str], dict | None]:
    problems = []
    if not isinstance(body.get("room"), str) or not body["room"].startswith("battle-"):
        problems.append("room is not a battle room id")
    lines = body.get("lines")
    if not isinstance(lines, list) or not all(isinstance(line, str) for line in lines):
        problems.append("lines is not a list of strings")
        lines = []
    if any(line.startswith("|request|") for line in lines):
        problems.append("lines contain |request| lines")
    if lines and lines[0] != "|init|battle":
        problems.append(f"first line is {lines[0][:40]!r}, not |init|battle")
    req = None
    if not isinstance(body.get("request"), str):
        problems.append("request is not a string")
    else:
        try:
            req = json.loads(body["request"])
        except ValueError:
            problems.append("request is not JSON")
    if req is not None and "rqid" not in req:
        problems.append("request has no rqid")
    history_problems, _ = check_history(body, lines)
    return problems + history_problems, req


class Handler(BaseHTTPRequestHandler):
    server_version = "NumbrionMockAPI/1"

    def log_message(self, fmt, *args):  # quieter than the default
        if ARGS.verbose:
            super().log_message(fmt, *args)

    def send_json(self, status: int, payload: dict):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(data)

    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    def do_GET(self):
        url = urlparse(self.path)
        query = parse_qs(url.query)
        room = (query.get("room") or [""])[0]
        if url.path == "/api/status":
            with LOCK:
                battles = len(ROOMS)
            self.send_json(200, {"ok": True, "server": "ws://localhost:8000/showdown/websocket", "advisor": "MockBot",
                                 "bots": [{"name": "MockBot", "checkpoint": "(mock)", "search": {"budget_ms": 1500},
                                           "connected": True, "battles": battles}]})
        elif url.path == "/api/bot-log":
            if not room:
                return self.send_json(400, {"ok": False, "error": "room is required"})
            with LOCK:
                entries = list(ROOMS.get(room, {}).get("log", []))
            self.send_json(200, {"ok": True, "room": room, "entries": entries})
        elif url.path == "/api/_last":
            with LOCK:
                state = ROOMS.get(room) or {}
                self.send_json(200, {"ok": True, "room": room, "body": state.get("body"), "count": state.get("count", 0),
                                     "problems": state.get("problems")})
        else:
            self.send_json(404, {"ok": False, "error": "not found"})

    def do_POST(self):
        if urlparse(self.path).path != "/api/advise":
            return self.send_json(404, {"ok": False, "error": "not found"})
        try:
            body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))) or b"{}")
        except ValueError:
            return self.send_json(400, {"ok": False, "error": "body is not JSON"})
        problems, req = check_body(body)
        room = body.get("room") or ""
        lines = body.get("lines") or []
        turns = [int(line[6:]) for line in lines if isinstance(line, str) and line.startswith("|turn|") and line[6:].isdigit()]
        _, history = check_history(body, lines)
        summary = {"time": round(time.time(), 3), "room": room, "rqid": req.get("rqid") if req else None,
                   "lines": len(lines), "turn": max(turns) if turns else 0, "budget_ms": body.get("budget_ms"),
                   "bot": body.get("bot"), "history": history, "problems": problems}
        print("advise", json.dumps(summary), flush=True)
        if ARGS.log:
            with open(ARGS.log, "a") as fh:
                fh.write(json.dumps(summary) + "\n")
        if problems and req is None:
            return self.send_json(400, {"ok": False, "error": "; ".join(problems)})
        time.sleep(ARGS.delay_ms / 1000)
        decision = make_decision(room, req, lines)
        if decision:
            note = decision["analysis"]["note"]
            if not history["entries"]:
                decision["analysis"].update(source="policy", note=note + "; no history: policy only")
            elif history["missing"] or not history["from_start"]:
                decision["analysis"].update(source="policy", note=note + f"; history incomplete ({history['missing']} past "
                                            f"choices missing, from turn 1: {history['from_start']}): policy only")
            else:
                decision["analysis"]["note"] = note + f"; history: {history['entries']} requests, {history['choices']} choices"

        with LOCK:
            state = ROOMS.setdefault(room, {"last": None, "log": [], "count": 0})
            state["body"] = body
            state["count"] += 1
            state["problems"] = problems
            last = state["last"]
            if last and last["rqid"] != req.get("rqid"):
                state["log"].append(last)
            if decision:
                state["last"] = {"bot": "MockBot", "side": (req.get("side") or {}).get("id"), "turn": summary["turn"],
                                 "rqid": req.get("rqid"), "time": time.time(), "decision": decision}
        self.send_json(200, {"ok": True, "room": room, "rqid": req.get("rqid"), "decision": decision})


def main():
    global ARGS
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--delay-ms", type=int, default=300, help="simulated thinking time")
    parser.add_argument("--log", help="append one JSON line per advise request")
    parser.add_argument("--verbose", action="store_true")
    ARGS = parser.parse_args()
    server = ThreadingHTTPServer((ARGS.host, ARGS.port), Handler)
    print(f"mock advisor API on http://{ARGS.host}:{ARGS.port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
