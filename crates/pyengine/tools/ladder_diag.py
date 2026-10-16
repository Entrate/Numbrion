"""Offline diagnostics for BattleRecorder v1 gzip JSONL. No engine/model imports.

See docs/play/LADDER_DIAG.md for attribution rules and limits. Values are signed
search values, never calibrated win probabilities. Original snapshots stay in the
input files and are referenced by absolute path and zero-based event index.
"""
from __future__ import annotations

import argparse
import copy
import json
import math
import re
import statistics
import zlib
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

SLEEP = {"spore", "sleeppowder", "hypnosis", "yawn", "lovelykiss", "sing",
         "grasswhistle", "darkvoid", "relicsong"}
POWDER = {"spore", "sleeppowder"}
STALL = {"protect", "detect", "wideguard", "quickguard", "roost", "recover", "slackoff",
         "moonlight", "synthesis", "wish", "substitute", "softboiled", "milkdrink",
         "shoreup", "morningsun", "rest", "lifedew", "junglehealing", "lunarblessing",
         "healorder", "healpulse", "floralhealing", "strengthsap", "swallow",
         "kingsshield", "spikyshield", "banefulbunker", "silktrap", "burningbulwark",
         "obstruct", "endure", "matblock"}
SLEEP_IMMUNE = {"insomnia", "vitalspirit", "sweetveil", "comatose", "purifyingsalt"}


def ident(text):
    return re.sub(r"[^a-z0-9]", "", str(text).lower())


def mon_key(text):
    """A party identity survives slot changes; nicknames are unique in this format."""
    return text[:2] + ":" + text.partition(":")[2].strip()


def move_id(text):
    return ident(text.split(" -> ")[0].split(" + Tera")[0])


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def probability(value):
    return (value + 1) / 2 if number(value) and -1 <= value <= 1 else None


def load_events(path, warnings):
    """Validate each concatenated gzip member before yielding its JSON lines.

    An unfinished final member is discarded, even if it already decompressed a
    full line. Earlier members survive. CRC/data errors are reported, not hidden.
    """
    def lines(payload):
        for line in payload.splitlines():
            if line.strip():
                try:
                    event = json.loads(line)
                    if not isinstance(event, dict):
                        raise ValueError("event is not an object")
                    yield event
                except (ValueError, UnicodeError) as exc:
                    warnings.append(f"{path.name}: invalid JSON event: {exc}")

    if path.suffix != ".gz":
        yield from lines(path.read_bytes())
        return
    decoder, pieces, pending = None, [], b""
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(65536)
            if not chunk:
                break
            pending += chunk
            while pending:
                if decoder is None:
                    decoder = zlib.decompressobj(31)
                    pieces = []
                try:
                    pieces.append(decoder.decompress(pending))
                except zlib.error as exc:
                    warnings.append(f"{path.name}: corrupt gzip member: {exc}")
                    return
                pending = decoder.unused_data
                if decoder.eof:
                    yield from lines(b"".join(pieces))
                    decoder = None
                else:
                    break
    if decoder is not None:
        warnings.append(f"{path.name}: truncated final gzip member ignored")


@dataclass
class Battle:
    path: Path
    room: str = ""
    side: str | None = None
    decisions: list = field(default_factory=list)
    lines: list = field(default_factory=list)
    end: dict | None = None
    ratings: list = field(default_factory=list)
    warnings: list = field(default_factory=list)
    late: int = 0
    max_turn: int = 0
    first_time: float | None = None
    last_time: float | None = None

    @classmethod
    def read(cls, path):
        b = cls(Path(path).resolve())
        requests, current_request, active, fainted = {}, {}, {}, {"p1": set(), "p2": set()}
        sizes, players, turn = {"p1": 6, "p2": 6}, {}, 0
        for ei, event in enumerate(load_events(b.path, b.warnings)):
            timestamp = event.get("t", event.get("time"))
            if number(timestamp):
                b.first_time = timestamp if b.first_time is None else min(b.first_time, timestamp)
                b.last_time = timestamp if b.last_time is None else max(b.last_time, timestamp)
            b.room = event.get("room", b.room) or path.name.removesuffix(".jsonl.gz")
            kind = event.get("kind")
            if kind in {"request", "request_received"}:
                req = event.get("request")
                if req is None:
                    try:
                        req = json.loads(event.get("text", "{}"))
                    except ValueError:
                        b.warnings.append(f"event {ei}: invalid request")
                        continue
                current_request = req
                b.side = req.get("side", {}).get("id", b.side)
                if kind == "request":
                    requests[event.get("seq")] = req
            elif kind == "message":
                for li, text in enumerate(event.get("text", "").splitlines()):
                    p = text.split("|")
                    if len(p) < 2:
                        continue
                    cmd = p[1]
                    if cmd == "turn" and len(p) > 2:
                        turn = int(p[2])
                        b.max_turn = max(turn, b.max_turn)
                    if cmd == "player" and len(p) > 3:
                        players[p[2]] = p[3]
                    if cmd == "teamsize" and len(p) > 3:
                        sizes[p[2]] = int(p[3])
                    if cmd in {"switch", "drag", "replace"} and len(p) > 3:
                        active[p[2][:3]] = p[3].split(",")[0]
                    if cmd == "faint" and len(p) > 2:
                        fainted[p[2][:2]].add(mon_key(p[2]))
                    if cmd in {"switch", "drag", "-heal"} and len(p) > 3:
                        # Revival is rare, but do not keep a revived mon counted as fainted.
                        cond = p[4] if cmd in {"switch", "drag"} and len(p) > 4 else p[3]
                        if not cond.startswith("0"):
                            fainted.get(p[2][:2], set()).discard(mon_key(p[2]))
                    b.lines.append(dict(event_index=ei, line_index=li, turn=turn, text=text,
                                        actor_species=active.get(p[2][:3]) if len(p) > 2 else None,
                                        target_species=active.get(p[4][:3]) if len(p) > 4 else None))
            elif kind == "decision":
                req = requests.get(event.get("seq"), current_request)
                b.side = req.get("side", {}).get("id", b.side)
                snapshot = (event.get("analysis") or {}).get("search_record")
                if not b.side and snapshot:
                    b.side = ("p1", "p2")[snapshot["side"]]
                alive = {s: max(0, sizes[s] - len(fainted[s])) for s in sizes}
                if b.side and req.get("side", {}).get("pokemon"):
                    alive[b.side] = sum("fnt" not in m.get("condition", "")
                                        for m in req["side"]["pokemon"])
                b.decisions.append(dict(event=event, event_index=ei, request=copy.deepcopy(req),
                                        active=active.copy(), alive=alive, refused=False,
                                        forced=any(req.get("forceSwitch", []))))
            elif kind == "refused":
                for d in reversed(b.decisions):
                    if d["event"].get("rqid") == event.get("rqid"):
                        d["refused"] = True
                        break
            elif kind == "late_decision":
                b.late += 1
            elif kind == "rating":
                b.ratings.append(event)
            elif kind == "end":
                b.end = event
        for d in b.decisions:
            d["side"] = b.side
        name = players.get(b.side)
        b.ratings = [r for r in b.ratings if not name or ident(r.get("user", r.get("userid", ""))) == ident(name)]
        return b


def snapshot(d):
    return (d["event"].get("analysis") or {}).get("search_record") or {}


def snapshot_ref(b, d):
    return dict(path=str(b.path), event_index=d["event_index"], seq=d["event"].get("seq"),
                rqid=d["event"].get("rqid"), field="analysis.search_record",
                present=bool(snapshot(d)))


def action_slots(choice, request, moves):
    result = []
    for slot, part in enumerate(choice.split(",")):
        part = part.strip()
        match = re.match(r"move (\d+)", part)
        if match:
            ms = (request.get("active") or [])
            options = ms[slot].get("moves", []) if slot < len(ms) else []
            idx = int(match[1]) - 1
            m = options[idx] if idx < len(options) else {}
            mid = ident(m.get("id", m.get("move", "")))
            category = moves.get(mid, {}).get("category")
            label = "stall" if mid in STALL else "attack" if category in {"Physical", "Special"} else "other"
            result.append(dict(slot=slot, move=mid or None, label=label))
        else:
            result.append(dict(slot=slot, move=None, label="switch" if part.startswith("switch") else "other"))
    return result


def candidates(d, moves):
    a, sr = d["event"].get("analysis") or {}, snapshot(d)
    texts = {c["choice"]: c.get("text", "") for c in a.get("candidates", [])}
    policy_priors = {c["choice"]: c.get("prior") for c in a.get("candidates", [])}
    if sr:
        rows = [dict(index=i, choice=c, text=texts.get(c, c), value=sr["q"][i],
                     prior=sr["prior"][i], weight=sr["x"][i]) for i, c in enumerate(sr["choices"])]
    else:
        rows = [dict(c, index=i) for i, c in enumerate(a.get("candidates", []))]
    for c in rows:
        c["policy_prior"] = policy_priors.get(c["choice"])
        c["chosen"] = c["choice"] == d["event"].get("choice")
        c["slots"] = action_slots(c["choice"], d["request"], moves)
        c["stall"] = any(s["label"] == "stall" for s in c["slots"])
        c["attack"] = any(s["label"] == "attack" for s in c["slots"])
    return rows


def decision_value(d):
    sr = snapshot(d)
    choice = d["event"].get("choice")
    if choice in sr.get("choices", []):
        return sr["q"][sr["choices"].index(choice)]
    return (d["event"].get("analysis") or {}).get("value")


def common_case(b, d, moves):
    e = d["event"]
    return dict(room=b.room, turn=e.get("turn"), our_side=b.side, seq=e.get("seq"),
                replay_url="https://replay.pokemonshowdown.com/" + b.room.removeprefix("battle-"),
                replay_saved=False, request=d["request"], snapshot_ref=snapshot_ref(b, d),
                choice=e.get("choice"), source=e.get("source"), candidates=candidates(d, moves),
                value=decision_value(d), win_probability_proxy=probability(decision_value(d)),
                alive=d["alive"], battle_result=b.end, rating_changes=b.ratings)


def excerpt(b, turns):
    return [line["text"] for line in b.lines if line["turn"] in turns and
            not line["text"].startswith(("|request|", "|c|", "|c:|", "|j|", "|l|", "|t:|"))]


def sleep_incidents(b):
    """Group use, drowsiness and status into one exposure, keeping misses/failed uses."""
    incidents, pending_yawn, last_move = [], {}, None
    for line in b.lines:
        p = line["text"].split("|")
        if len(p) < 3:
            continue
        cmd, actor = p[1], p[2]
        if cmd == "turn":
            last_move = None
            for key, pending in list(pending_yawn.items()):
                if line["turn"] > pending["exposure_turn"] + 1:
                    pending["events"].append(line)
                    del pending_yawn[key]
        if cmd == "move" and len(p) > 3:
            last_move = (actor, ident(p[3]))
        if cmd in {"switch", "drag"}:
            # The switch line names the incoming mon, not the drowsy outgoing mon.
            for key, pending in list(pending_yawn.items()):
                if pending["victim"][:3] == actor[:3]:
                    pending["events"].append(line)
                    del pending_yawn[key]
        if cmd == "faint":
            pending = pending_yawn.pop(mon_key(actor), None)
            if pending:
                pending["events"].append(line)
        if cmd == "move" and len(p) > 4 and ident(p[3]) in SLEEP and actor[:2] != b.side and p[4][:2] == b.side:
            incidents.append(dict(move=p[3], foe=actor, foe_species=line["actor_species"],
                                  victim=p[4], exposure_turn=line["turn"], sleep_turn=None,
                                  start=line, events=[line], drowsy=False, slept=False))
        elif cmd in {"-start", "-status"} and actor[:2] == b.side and len(p) > 3:
            drowsy = cmd == "-start" and ident(p[3]) in {"moveyawn", "yawn"}
            slept = cmd == "-status" and p[3] == "slp"
            if not (drowsy or slept):
                continue
            incident = None
            self_sleep = slept and last_move == (actor, "rest")
            if slept and not self_sleep:
                incident = pending_yawn.pop(mon_key(actor), None)
            if incident is None and not self_sleep:
                for candidate in reversed(incidents):
                    if candidate["exposure_turn"] != line["turn"]:
                        break
                    if mon_key(candidate["victim"]) == mon_key(actor) and not candidate["slept"]:
                        if not drowsy or ident(candidate["move"]) == "yawn":
                            incident = candidate
                            break
            if incident is None:
                incident = dict(move="Yawn" if drowsy else "unknown/self-induced", foe=None,
                                foe_species=None, victim=actor, exposure_turn=line["turn"],
                                sleep_turn=None, start=line, events=[], drowsy=False, slept=False)
                incidents.append(incident)
            incident["events"].append(line)
            incident["drowsy"] |= drowsy
            incident["slept"] |= slept
            if drowsy:
                pending_yawn[mon_key(actor)] = incident
            if slept:
                incident["sleep_turn"] = line["turn"]
        elif cmd in {"-miss", "-fail", "-immune", "-block"}:
            for incident in reversed(incidents):
                if incident["exposure_turn"] != line["turn"]:
                    break
                if actor in {incident["foe"], incident["victim"]} or incident["victim"] in p:
                    incident["events"].append(line)
                    break
    return incidents


def sleep_evidence(d, incident, rare_fraction):
    sr, mid = snapshot(d), ident(incident["move"])
    worlds = sr.get("worlds", [])
    species = ident(incident["foe_species"])
    foe_slot = incident["foe"][2:3] if incident["foe"] else ""
    slot = {"a": 0, "b": 1}.get(foe_slot)
    our_choice = d["event"].get("choice")
    chosen_i = sr.get("choices", []).index(our_choice) if our_choice in sr.get("choices", []) else None
    details, set_n, reply_n, known_n = [], 0, 0, 0
    for wi, world in enumerate(worlds):
        sets = [m for m in world.get("team", []) if ident(m.get("species")) == species]
        has = any(mid in {ident(m) for m in s.get("moves", [])} for s in sets)
        known_n += bool(sets)
        set_n += has
        hits = []
        for j, texts in enumerate(world.get("text", [])):
            # Move name and acting slot must both match. Another foe having Spore is insufficient.
            if slot is None or slot >= len(texts) or move_id(texts[slot]) != mid:
                continue
            pay = world.get("payoffs", [])
            counts = world.get("counts", [])
            sampled = world.get("sampled_valid", [])
            measured = (chosen_i is not None and chosen_i < len(counts) and
                        j < len(counts[chosen_i]) and counts[chosen_i][j] > 0 and
                        chosen_i < len(sampled) and sampled[chosen_i])
            hits.append(dict(column=j, text=texts, prior=world["prior"][j], weight=world["y"][j],
                             chosen_payoff=pay[chosen_i][j] if measured else None,
                             candidate_payoffs=[row[j] if i < len(counts) and j < len(counts[i]) and
                                                counts[i][j] > 0 and i < len(sampled) and sampled[i] else None
                                                for i, row in enumerate(pay)]))
        reply_n += bool(hits)
        details.append(dict(world_index=wi, world_weight=world.get("weight"),
                            foe_set_found=bool(sets), set_has_move=has, replies=hits,
                            prior_mass=sum(h["prior"] for h in hits),
                            solved_mass=sum(h["weight"] for h in hits)))
    n = len(worlds)
    observed = (d["event"].get("analysis") or {}).get("observed", [])
    revealed = any(ident(m.get("species")) == species and mid in {ident(v) for v in m.get("moves", [])}
                   for m in observed if isinstance(m, dict)) if isinstance(observed, list) else None
    fraction = set_n / n if n and known_n == n else None
    active_species = d["active"].get(incident["foe"][:3]) if incident["foe"] else None
    same_actor = ident(active_species) == species if active_species and species else False
    phase = "yawn_escape" if incident["drowsy"] and d["event"].get("turn", 0) > incident["exposure_turn"] else "exposure"
    if phase == "yawn_escape":
        label, reason = "unavoidable/other", "Yawn already applied; a new opponent Yawn reply is not required for sleep."
    elif not n or not species or known_n != n or not same_actor:
        label, reason = "unavoidable/other", "Missing search snapshot or unresolved foe identity in sampled sets."
    elif fraction < rare_fraction:
        label, reason = "H2", f"Sleep move occurs in fewer than {rare_fraction:.0%} of sampled worlds."
    elif reply_n == 0:
        label, reason = "H1", "Move exists in sampled sets but no shortlisted pair uses it from the acting foe slot."
    else:
        label, reason = "H3", "Move sampled and shortlisted; inspect reply mass and conditional payoffs. This does not prove head error."
    matching = [r for w in details for r in w["replies"] if number(r["chosen_payoff"])]
    mass = sum(r["weight"] for r in matching)
    conditional = sum(r["chosen_payoff"] * r["weight"] for r in matching) / mass if mass else None
    return dict(classification=label, reason=reason, phase=phase, worlds=n, set_worlds=set_n,
                foe_active_at_decision=same_actor, conditional_sleep_reply_value=conditional,
                foe_identity_worlds=known_n, set_fraction=fraction, reply_worlds=reply_n,
                reply_fraction=reply_n / n if n else None, observed=revealed,
                mean_prior_mass=sum(w["prior_mass"] for w in details) / n if n else None,
                mean_solved_mass=sum(w["solved_mass"] for w in details) / n if n else None,
                worlds_detail=details)


def prevention_hints(c, d, incident, species_data, moves):
    """Conditional alternatives, not counterfactual proof. No speed/damage simulation."""
    hints = []
    victim_slot = {"a": 0, "b": 1}.get(incident["victim"][2:3])
    delayed = incident["drowsy"] and d["event"].get("turn", 0) > incident["exposure_turn"]
    party = d["request"].get("side", {}).get("pokemon", [])
    for slot, part in enumerate(c["choice"].split(",")):
        match = re.match(r"\s*switch (\d+)", part)
        if match and slot == victim_slot:
            idx = int(match[1]) - 1
            mon = party[idx] if idx < len(party) else {}
            species = species_data.get(ident(mon.get("details", "").split(",")[0]), {})
            types = [mon["terastallized"]] if mon.get("terastallized") else species.get("types", [])
            ability = ident(mon.get("ability", mon.get("baseAbility", "")))
            if delayed:
                hints.append("switch clears existing Yawn drowsiness")
            if ability in SLEEP_IMMUNE:
                hints.append("switch to sleep-immune ability, unless suppressed or bypassed")
            if ident(incident["move"]) in POWDER and ("Grass" in types or ability == "overcoat" or ident(mon.get("item")) == "safetygoggles"):
                hints.append("switch to powder immunity, subject to ability/item suppression")
        actions = c.get("slots", [])
        if slot < len(actions):
            mid = actions[slot]["move"]
            if not delayed and slot == victim_slot and mid in {"protect", "detect", "kingsshield", "spikyshield", "banefulbunker", "silktrap", "burningbulwark"}:
                hints.append("Protect may block the targeted sleep move; accuracy/bypass not simulated")
            if actions[slot]["label"] == "attack" and incident["foe"]:
                target = re.search(r"\+([12])", part)
                attack_targets_foe = target and int(target[1]) == {"a": 1, "b": 2}.get(incident["foe"][2:3])
                spread = moves.get(mid, {}).get("target") in {"allAdjacent", "allAdjacentFoes", "all"}
                if attack_targets_foe or spread:
                    hints.append("attack targets sleeper; prevention requires a KO before it moves")
    return sorted(set(hints))


def extract_sleep(b, moves, species_data, rare_fraction):
    cases = []
    eligible = [d for d in b.decisions if not d["refused"] and not d["forced"]]
    for incident in sleep_incidents(b):
        start = incident["start"]["event_index"]
        before = [d for d in eligible if d["event_index"] < start and
                  d["event"].get("turn") == incident["exposure_turn"]]
        if not before:
            # Preserve exposures even when the record begins after the relevant decision.
            cases.append(dict(kind="sleep", room=b.room, turn=incident["exposure_turn"], our_side=b.side,
                              incident=incident, classification="unavoidable/other", reason="No preceding turn decision",
                              request=None, snapshot_ref=None, seq=None,
                              decisions=[], replay_url="https://replay.pokemonshowdown.com/" + b.room.removeprefix("battle-")))
            continue
        selected = [before[-1]]
        if incident["drowsy"]:
            finish = incident["events"][-1]["event_index"]
            selected += [d for d in eligible if start < d["event_index"] < finish and
                         d["event"].get("turn", 0) > incident["exposure_turn"]]
        root = common_case(b, selected[0], moves)
        root.update(kind="sleep", incident=incident, decisions=[])
        for d in selected:
            detail = common_case(b, d, moves)
            detail.update(sleep_evidence(d, incident, rare_fraction))
            detail["opponent_replies"] = opponent_summary(d)
            for c in detail["candidates"]:
                c["prevention_hints"] = prevention_hints(c, d, incident, species_data, moves)
            root["decisions"].append(detail)
        root["classification"] = root["decisions"][0]["classification"]
        root["reason"] = root["decisions"][0]["reason"]
        # First actionable boundary after sleep, not the arbitrary next replacement.
        end_index = incident["events"][-1]["event_index"]
        after = next((d for d in eligible if d["event_index"] > end_index), None)
        end_value = decision_value(after) if after else None
        if after is None and b.end and b.end.get("won") is not None:
            end_value = 1 if b.end["won"] else -1
        root["after"] = dict(value=end_value, win_probability_proxy=probability(end_value),
                             snapshot_ref=snapshot_ref(b, after) if after else None,
                             terminal=after is None and b.end is not None)
        root["value_delta"] = end_value - root["value"] if number(end_value) and number(root["value"]) else None
        root["probability_delta"] = root["value_delta"] / 2 if root["value_delta"] is not None else None
        root["excerpt"] = excerpt(b, set(range(incident["exposure_turn"], (incident["sleep_turn"] or incident["exposure_turn"]) + 2)))
        cases.append(root)
    return cases


def extract_endgame(b, moves, per_side, total, discount, horizons):
    cases, streak, last_turn = [], 0, None
    decisions = [d for d in b.decisions if not d["refused"] and not d["forced"]]
    for d in decisions:
        counts, turn = d["alive"], d["event"].get("turn", 0)
        if not ((per_side and max(counts.values()) <= per_side) or (total and sum(counts.values()) <= total)):
            streak, last_turn = 0, None
            continue
        c = common_case(b, d, moves)
        chosen_slots = action_slots(c["choice"] or "", d["request"], moves)
        is_stall = any(s["label"] == "stall" for s in chosen_slots)
        streak = (streak + 1 if last_turn is not None and turn == last_turn + 1 else 1) if is_stall else 0
        last_turn = turn
        stalls = [r for r in c["candidates"] if r["stall"] and number(r.get("value")) and r["value"] >= -1]
        attacks = [r for r in c["candidates"] if r["attack"] and not r["stall"] and number(r.get("value")) and r["value"] >= -1]
        bs = max(stalls, key=lambda r: r["value"], default=None)
        ba = max(attacks, key=lambda r: r["value"], default=None)
        gap = bs["value"] - ba["value"] if bs and ba else None
        value = c["value"]
        effects = {str(k): value * (1 - discount ** k) if number(value) and -1 <= value <= 1 else None for k in horizons}
        chosen = next((r for r in c["candidates"] if r["chosen"]), None)
        chosen_gap = chosen["value"] - ba["value"] if chosen and number(chosen.get("value")) and ba else None
        flags = [k for k, effect in effects.items() if is_stall and gap is not None and
                 value is not None and value < 0 and 0 <= gap <= abs(effect)]
        after = next((x for x in decisions if x["event_index"] > d["event_index"] and x["event"].get("turn", 0) > turn), None)
        next_lines = excerpt(b, {turn})
        c.update(kind="endgame", chosen_slots=chosen_slots, stall=is_stall,
                 mixed_stall_attack=is_stall and any(s["label"] == "attack" for s in chosen_slots),
                 stall_streak=streak, repeated_stall=streak >= 2,
                 best_stall=bs["choice"] if bs else None, best_attack=ba["choice"] if ba else None,
                 q_gap=gap, chosen_vs_attack_gap=chosen_gap, discount_effect_signed=effects,
                 discount_comparable_horizons=flags, classification="H4" if flags else "unavoidable/other",
                 close_values=gap is not None and abs(gap) <= .02,
                 confidently_losing=number(value) and value <= -.8,
                 next_turn=dict(lines=next_lines, faints=[s for s in next_lines if s.startswith("|faint|")],
                                next_value=decision_value(after) if after else None,
                                terminal=b.end if after is None else None),
                 excerpt=excerpt(b, {max(1, turn - 1), turn, turn + 1}),
                 opponent_replies=opponent_summary(d))
        cases.append(c)
    return cases


def opponent_summary(d):
    """Full per-world text merged for display, without the six-entry summary cap."""
    merged = {}
    worlds = snapshot(d).get("worlds", [])
    for w in worlds:
        for texts, prior, y in zip(w.get("text", []), w.get("prior", []), w.get("y", [])):
            text = "; ".join(texts)
            row = merged.setdefault(text, dict(text=text, prior=0, weight=0, worlds=0))
            row["prior"] += prior / len(worlds)
            row["weight"] += y / len(worlds)
            row["worlds"] += 1
    return sorted(merged.values(), key=lambda x: -x["weight"])


def percentile(values, percent):
    if not values:
        return None
    xs = sorted(values)
    pos = (len(xs) - 1) * percent / 100
    lo, hi = math.floor(pos), math.ceil(pos)
    return xs[lo] * (hi - pos) + xs[hi] * (pos - lo) if hi != lo else xs[lo]


def analyze(paths, *, per_side=2, total=3, rare_fraction=.25, discount=.995, horizons=(1, 3, 5, 10), dex_path=None):
    dex_path = dex_path or Path(__file__).resolve().parents[3] / "data/training/dex.json"
    dex = json.loads(Path(dex_path).read_text())
    moves = {m["id"]: m for m in dex["moves"]}
    species = {s["id"]: s for s in dex["species"]}
    cases, battles, warnings, times = [], [], [], defaultdict(list)
    sources = Counter(dict(search=0, policy=0, fallback=0, default=0))
    notes, outcomes, length_groups = Counter(), Counter(), defaultdict(Counter)
    disagreement = compared = refused = missing_snapshot = late = 0
    for path in paths:
        b = Battle.read(path)
        warnings.extend(b.warnings)
        sleeps = extract_sleep(b, moves, species, rare_fraction)
        ends = extract_endgame(b, moves, per_side, total, discount, horizons)
        cases.extend(sleeps + ends)
        late += b.late
        for d in b.decisions:
            e, a = d["event"], d["event"].get("analysis") or {}
            sources[e.get("source", a.get("source", "unknown"))] += 1
            if a.get("note"):
                notes[a["note"]] += 1
            refused += d["refused"]
            if e.get("source") == "search":
                missing_snapshot += not bool(snapshot(d))
                agrees = a.get("agrees_with_policy")
                if agrees is None and a.get("policy", {}).get("choice"):
                    agrees = a["policy"]["choice"] == e.get("choice")
                if agrees is not None:
                    compared += 1
                    disagreement += not agrees
            for key, value in (e.get("timing") or {}).items():
                if key.endswith("_ms") and number(value):
                    times[key].append(value)
        outcome = "unfinished"
        if b.end:
            won = b.end.get("won")
            outcome = "win" if won is True else "loss" if won is False else "tie" if b.end.get("result", {}).get("tie") else "unknown"
        turns = b.end.get("turns", b.max_turn) if b.end else b.max_turn
        bucket = "1-20" if turns <= 20 else "21-40" if turns <= 40 else "41-80" if turns <= 80 else "81+"
        outcomes[outcome] += 1
        length_groups[bucket][outcome] += 1
        timer = bool(b.end and b.end.get("end_reason") == "inactivity")
        limit = any("turn limit" in l["text"].lower() or "turn-limit" in l["text"].lower() for l in b.lines)
        battles.append(dict(room=b.room, our_side=b.side, outcome=outcome, turns=turns,
                            first_event_time=b.first_time, last_event_time=b.last_time,
                            decisions=len(b.decisions), end_reason=b.end.get("end_reason") if b.end else None,
                            timer=timer, turn_limit=limit, reached_1000=turns >= 1000,
                            rating_changes=b.ratings, sleep_cases=len(sleeps), endgame_decisions=len(ends)))
    endcases = [c for c in cases if c["kind"] == "endgame"]
    sleepcases = [c for c in cases if c["kind"] == "sleep"]
    summary = dict(battles=battles, outcomes=dict(outcomes), sources=dict(sources), policy_notes=dict(notes),
                   search_policy_compared=compared, search_policy_disagreed=disagreement,
                   refused_decisions=refused, late_decisions=late, missing_search_snapshots=missing_snapshot,
                   timing_ms={k: dict(n=len(v), p50=percentile(v, 50), p95=percentile(v, 95),
                                      p99=percentile(v, 99), maximum=max(v), mean=statistics.mean(v)) for k, v in times.items()},
                   by_length={k: dict(v) for k, v in length_groups.items()},
                   sleep_classifications=dict(Counter(c["classification"] for c in sleepcases)),
                   sleep_uses=len(sleepcases), sleep_landed=sum(c["incident"]["slept"] for c in sleepcases),
                   endgame=dict(decisions=len(endcases), stall=sum(c["stall"] for c in endcases),
                                mixed=sum(c["mixed_stall_attack"] for c in endcases),
                                repeated=sum(c["repeated_stall"] for c in endcases),
                                h4=sum(bool(c["discount_comparable_horizons"]) for c in endcases),
                                close=sum(c["close_values"] for c in endcases),
                                losing=sum(c["confidently_losing"] for c in endcases),
                                comparable=sum(c["q_gap"] is not None for c in endcases)),
                   warnings=warnings, config=dict(per_side=per_side, total=total, rare_fraction=rare_fraction,
                                                  discount=discount, horizons=list(horizons)))
    return summary, cases


def fmt(value):
    return f"{value:.4f}" if number(value) else "n/a"


def table(headers, rows):
    def cell(x):
        return str(x).replace("|", "\\|").replace("\n", " ")
    return "\n".join(["| " + " | ".join(map(cell, headers)) + " |",
                      "| " + " | ".join("---" for _ in headers) + " |"] +
                     ["| " + " | ".join(map(cell, row)) + " |" for row in rows])


def winrate(counts):
    n = sum(counts.get(k, 0) for k in ("win", "loss", "tie"))
    return f"{counts.get('win', 0) / n:.1%} ({n} finished, ties in denominator)" if n else "n/a"


def informative_score(c):
    if c["kind"] == "sleep":
        return 20 + (10 if c["classification"] in {"H1", "H2"} else 0) + abs(c.get("value_delta") or 0) * 10
    return (15 if c["discount_comparable_horizons"] else 0) + (10 if c["repeated_stall"] else 0) + (5 if c["confidently_losing"] else 0)


def render_report(summary, cases, *, partial=False, top=15):
    s, end = summary, summary["endgame"]
    parts = ["# Numbrion ladder diagnostics", "PARTIAL snapshot of a live run." if partial else "Analysis of the supplied record snapshot.",
             f"{len(s['battles'])} battles; outcomes {s['outcomes']}. Win rate {winrate(s['outcomes'])}.",
             "## Hypotheses", table(["Hypothesis", "Cases", "Meaning"], [
                 ["H1", s["sleep_classifications"].get("H1", 0), "Sleep sampled, absent from opponent shortlist"],
                 ["H2", s["sleep_classifications"].get("H2", 0), "Sleep rarely sampled"],
                 ["H3", s["sleep_classifications"].get("H3", 0), "Sleep sampled and shortlisted; weighting/head investigation"],
                 ["Sleep other", s["sleep_classifications"].get("unavoidable/other", 0), "Missing evidence, unresolved identity or non-opponent cause"],
                 ["H4", end["h4"], "Chosen stall, losing value, best-stall q advantage within discount magnitude"]]),
             f"{s['sleep_uses']} sleep exposures including misses/failed uses; {s['sleep_landed']} led to recorded sleep. "
             "H1-H3 are mutually exclusive exposure labels, not counts of individual moves or proof of causality.",
             "## Operations and results", table(["Source", "Decisions emitted"], sorted(s["sources"].items())),
             f"Search disagreed with policy {s['search_policy_disagreed']}/{s['search_policy_compared']} comparable search decisions. "
             f"Refused decisions {s['refused_decisions']}; late decisions {s['late_decisions']}; search snapshots missing {s['missing_search_snapshots']}.",
             table(["Timing", "N", "p50 ms", "p95 ms", "p99 ms", "max ms"],
                   [[k, v["n"], fmt(v["p50"]), fmt(v["p95"]), fmt(v["p99"]), fmt(v["maximum"])] for k, v in s["timing_ms"].items()]),
             table(["Game length", "Outcomes", "Win rate"], [[k, v, winrate(v)] for k, v in sorted(s["by_length"].items())]),
             "Policy notes: " + json.dumps(s["policy_notes"], ensure_ascii=False),
             "## Endgame", f"Rule: both sides have <= {s['config']['per_side']} left OR total alive <= {s['config']['total']}; zero disables a branch. "
             "Unrevealed foe bench members count as alive. Forced replacements and refused choices are excluded from cases.",
             f"Stall {end['stall']}/{end['decisions']} decisions " + (f"({end['stall']/end['decisions']:.1%})" if end["decisions"] else "") +
             f"; mixed attack+stall {end['mixed']}; repeated stalls {end['repeated']}; q-comparable {end['comparable']}; "
             f"close values {end['close']}; confidently losing {end['losing']}; discount-comparable {end['h4']}.",
             "Stall means at least one slot uses a listed stall move. The attacking comparator has an attack and no stall move. "
             "Repeated means consecutive non-replacement turn decisions containing a stall, possibly from different Pokemon. "
             "Close is |best stall q - best attack q| <= 0.02; confidently losing is chosen value <= -0.8.",
             table(["Termination", "Battles"], [["Timer/inactivity", sum(b["timer"] for b in s["battles"])],
                 ["Explicit turn limit", sum(b["turn_limit"] for b in s["battles"])], ["Reached turn 1000", sum(b["reached_1000"] for b in s["battles"])]]),
             "## Classification and limits", f"H2 takes precedence when the move is in < {s['config']['rare_fraction']:.0%} of worlds. "
             "H1 requires resolved foe sets in every world, sufficient set coverage, and zero replies from the acting foe slot. "
             "H3 requires sufficient set coverage and at least one such reply. Yawn escape decisions are labelled other because the earlier Yawn already applied. "
             "No snapshot or unresolved foe identity is other, never evidence of absence.",
             f"Discount comparisons use signed value v × (1 - {s['config']['discount']}^k), k={s['config']['horizons']}. "
             "For losses its negative sign means delaying the target raises value by its magnitude. H4 flags nonnegative best-stall q advantages no larger than that magnitude. "
             "The chosen-vs-attack gap is also retained because the KL-anchored mixed strategy need not choose the greatest q. "
             "These comparisons are a scale estimate; the recorded q already contains discounted training effects.",
             "Values are in [-1,1]; (v+1)/2 is an uncalibrated probability proxy. Probability changes compare different later search states and are not a causal sleep penalty. "
             "Misses and failed sleep uses are exposures too. Prevention hints are conditional: no speed, KO, targeting redirection, ability suppression or Protect success is simulated. "
             "Protect does not stop existing Yawn drowsiness; Grass blocks powder moves, not Hypnosis or Yawn.",
             "Snapshots preserve averaged leaf payoffs, not leaf status, terminal flags per cell, per-seed values or the root critic value. "
             "H3 cannot separate a head failing to punish sleep from a leaf that never fell asleep. Add those fields to the recorder to test that claim. "
             "Opponent request/active identities are also absent per world; this tool joins actor slots from the live protocol and species from team sets. "
             "Illusion, Transform and ambiguous identity can prevent attribution. Add per-world active species/party indices and requests. "
             "Late search snapshots are counted but never substituted for an action actually sent as fallback. "
             "Replay URLs are conventional room URLs; recording does not imply the replay was uploaded.",
             "Candidate priors come from search_record and are normalized over the solver shortlist. "
             "The original policy probability is also retained as policy_prior in JSONL. "
             "Input references count decoded events, including request_received events stored in the same member as a message.",
             "## Battle inventory", table(["Room", "Side", "Turns", "Result", "Rating delta", "End reason"],
                 [[b["room"], b["our_side"], b["turns"], b["outcome"], [r.get("delta") for r in b["rating_changes"]], b["end_reason"]] for b in s["battles"]]),
             "## Input warnings", "\n".join("- " + w for w in s["warnings"]) or "None.",
             "## Informative cases"]
    # Reserve space for both kinds when both exist, then rank within the selection.
    sleeps = sorted((c for c in cases if c["kind"] == "sleep"), key=informative_score, reverse=True)
    stalls = sorted((c for c in cases if c["kind"] == "endgame"), key=informative_score, reverse=True)
    picked = sleeps[:min(len(sleeps), max(1, top // 2))] + stalls[:min(len(stalls), max(1, top // 2))] if sleeps and stalls else []
    picked_ids = {id(c) for c in picked}
    picked += [c for c in sorted(cases, key=informative_score, reverse=True) if id(c) not in picked_ids][:max(0, top - len(picked))]
    for i, c in enumerate(sorted(picked[:top], key=informative_score, reverse=True), 1):
        parts += [f"### {i}. {c['kind']} {c['classification']}, turn {c['turn']}",
                  f"[{c['room']}]({c['replay_url']}) | side {c['our_side']} | choice `{c.get('choice')}` | value {fmt(c.get('value'))}"]
        if c["kind"] == "sleep":
            inc = c["incident"]
            parts += [f"{inc['move']} from {inc['foe']} against {inc['victim']}; drowsy {inc['drowsy']}, slept {inc['slept']}. "
                      f"Value change {fmt(c.get('value_delta'))}; probability-proxy change {fmt(c.get('probability_delta'))}."]
            details = c["decisions"]
        else:
            parts += [f"Best-stall minus best-attack q {fmt(c['q_gap'])}; chosen minus best-attack {fmt(c['chosen_vs_attack_gap'])}; "
                      f"stall streak {c['stall_streak']}; discount effects {json.dumps(c['discount_effect_signed'])}; flagged k {c['discount_comparable_horizons']}."]
            details = [c]
        for d in details:
            parts += [f"Decision turn {d['turn']}, seq {d['seq']}; reference `{json.dumps(d['snapshot_ref'])}`."]
            if c["kind"] == "sleep":
                parts += [f"{d['classification']}: {d['reason']} Sets {d['set_worlds']}/{d['worlds']}, replies {d['reply_worlds']}/{d['worlds']}, "
                          f"observed {d['observed']}; prior mass {fmt(d['mean_prior_mass'])}, solved mass {fmt(d['mean_solved_mass'])}; "
                          f"chosen payoff conditional on sleep replies {fmt(d['conditional_sleep_reply_value'])}."]
            parts += [table(["Chosen", "Our candidate", "q", "Prior", "Solved weight", "Prevention hints"],
                            [["yes" if r['chosen'] else "", r.get("text", r["choice"]), fmt(r.get("value")), fmt(r.get("prior")),
                              fmt(r.get("weight")), "; ".join(r.get("prevention_hints", []))] for r in d["candidates"]])]
            if c["kind"] == "sleep":
                rows = []
                for w in d["worlds_detail"]:
                    for r in w["replies"]:
                        rows.append([w["world_index"], "; ".join(r["text"]), fmt(r["prior"]), fmt(r["weight"]), fmt(r["chosen_payoff"])])
                parts += ["Sleep replies, first 24 world/pair entries; JSONL retains every matching entry.",
                          table(["World", "Opponent pair", "Prior", "Solved weight", "Chosen conditional payoff"], rows[:24])]
            parts += ["Opponent replies, highest solved weights; full matrix remains at the snapshot reference.",
                      table(["Opponent pair", "Prior", "Solved weight", "World pair occurrences"],
                            [[r["text"], fmt(r["prior"]), fmt(r["weight"]), r["worlds"]] for r in d["opponent_replies"][:8]])]
        parts += ["```text\n" + "\n".join(c.get("excerpt", [e["text"] for e in c.get("incident", {}).get("events", [])])) + "\n```"]
    return "\n\n".join(parts) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("records", nargs="+", type=Path, help="Room files or directories, recursively scanned")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--summary", type=Path, help="Optional machine-readable aggregate JSON")
    parser.add_argument("--partial", action="store_true", help="Mark report as a partial live-run snapshot")
    parser.add_argument("--endgame-per-side", type=int, default=2)
    parser.add_argument("--endgame-total", type=int, default=3)
    parser.add_argument("--rare-fraction", type=float, default=.25)
    parser.add_argument("--discount", type=float, default=.995)
    parser.add_argument("--horizons", default="1,3,5,10")
    parser.add_argument("--top", type=int, default=15)
    parser.add_argument("--dex", type=Path)
    args = parser.parse_args(argv)
    if not 0 < args.rare_fraction <= 1 or not 0 < args.discount <= 1 or min(args.endgame_per_side, args.endgame_total, args.top) < 0:
        parser.error("thresholds must be nonnegative; rare-fraction and discount must be in (0,1]")
    try:
        horizons = tuple(int(k) for k in args.horizons.split(","))
        if not horizons or min(horizons) < 1:
            raise ValueError
    except ValueError:
        parser.error("horizons must be comma-separated positive integers")
    paths = sorted({p.resolve() for source in args.records for p in
                    (source.rglob("*.jsonl.gz") if source.is_dir() else [source])})
    if not paths:
        parser.error("no record files found")
    summary, cases = analyze(paths, per_side=args.endgame_per_side, total=args.endgame_total,
                             rare_fraction=args.rare_fraction, discount=args.discount, horizons=horizons, dex_path=args.dex)
    for path in (args.out, args.cases, args.summary):
        if path:
            if path.resolve() in paths:
                parser.error("output must not overwrite an input record")
            path.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(render_report(summary, cases, partial=args.partial, top=args.top), encoding="utf-8")
    with args.cases.open("w", encoding="utf-8") as handle:
        for c in cases:
            handle.write(json.dumps(c, ensure_ascii=False, allow_nan=False) + "\n")
    if args.summary:
        args.summary.write_text(json.dumps(summary, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in summary.items() if k not in {"battles", "policy_notes", "timing_ms", "by_length"}}, indent=2))
    print(f"Wrote {len(cases)} cases from {len(paths)} battles to {args.cases}; report {args.out}")
    return summary, cases


if __name__ == "__main__":
    main()
