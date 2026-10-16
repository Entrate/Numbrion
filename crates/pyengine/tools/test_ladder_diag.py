"""Recorder-shaped fixtures, independent of torch, the native engine and the ladder."""
import gzip
import json
from pathlib import Path

import pytest

from tools import ladder_diag as diag

ROOM = "battle-gen9randomdoublesbattle-test"


def write_records(tmp_path, events):
    path = tmp_path / (ROOM + ".jsonl.gz")
    for event in events:
        with path.open("ab") as handle:
            handle.write(gzip.compress((json.dumps(dict(event, room=ROOM)) + "\n").encode()))
    return path


def message(text):
    return dict(kind="message", text=text)


def request(turn, *, grass=False, alive=2, forced=False):
    party = [dict(ident="p1: Pikachu", details="Pikachu, L80", condition="100/100", active=True),
             dict(ident="p1: Snorlax", details="Snorlax, L80", condition="100/100", active=True)]
    if grass:
        party.append(dict(ident="p1: Amoonguss", details="Amoonguss, L80", condition="100/100",
                          ability="regenerator", active=False))
    while len(party) < 6:
        party.append(dict(ident=f"p1: Dead{len(party)}", details="Ditto", condition="0 fnt", active=False))
    if alive == 1:
        party[1]["condition"] = "0 fnt"
    req = dict(rqid=turn, side=dict(id="p1", name="Numbrion", pokemon=party),
               active=[dict(moves=[dict(move="Protect", id="protect"), dict(move="Thunderbolt", id="thunderbolt")]),
                       dict(moves=[dict(move="Recover", id="recover"), dict(move="Body Slam", id="bodyslam")])])
    if forced:
        req["forceSwitch"] = [True, False]
    return dict(kind="request", seq=turn, rqid=turn, text=json.dumps(req))


def decision(turn, *, sleep="Spore", in_set=True, in_replies=True, choice="move 1, move 1", value=-.5, attack=-.501):
    choices = ["move 1, move 1", "move 2 +1, move 2 +1", "switch 3, move 2 +1"]
    texts = [[sleep + " -> Pikachu", "pass"], ["Pollen Puff -> Pikachu", "pass"]] if in_replies else [["Pollen Puff -> Pikachu", "pass"]]
    n = len(texts)
    world = dict(weight=1., team=[dict(species="amoonguss", moves=[diag.ident(sleep)] if in_set else ["pollenpuff"])],
                 text=texts, choices=[f"reply{i}" for i in range(n)], prior=[1 / n] * n, y=[1 / n] * n,
                 payoffs=[[-.5] * n, [-.501] * n, [-.4] * n], counts=[[2] * n for _ in choices],
                 sampled_valid=[True] * 3, valid=[True] * 3)
    snapshot = dict(version=1, side=0, choices=choices, q=[value, attack, -.6],
                    prior=[.5, .4, .1], x=[.6, .3, .1], worlds=[world])
    return dict(kind="decision", seq=turn, rqid=turn, turn=turn, choice=choice, source="search",
                analysis=dict(value=value, agrees_with_policy=False, search_record=snapshot,
                              observed=[dict(species="Amoonguss", moves=[diag.ident(sleep)])],
                              candidates=[dict(choice=c, text=t) for c, t in zip(choices, ["Protect; Recover", "Thunderbolt -> Amoonguss; Body Slam -> Amoonguss", "switch to Amoonguss; Body Slam -> Amoonguss"])]),
                timing=dict(total_ms=2000, compute_ms=1500))


def initial():
    return message("|player|p1|Numbrion\n|player|p2|Foe\n|teamsize|p1|6\n|teamsize|p2|2\n"
                   "|switch|p1a: Pikachu|Pikachu, L80|100/100\n|switch|p1b: Snorlax|Snorlax, L80|100/100\n"
                   "|switch|p2a: Amoonguss|Amoonguss, L80|100/100\n|turn|1")


def test_spore_case_and_provenance(tmp_path):
    d = decision(1)
    path = write_records(tmp_path, [initial(), request(1, grass=True), d,
        message("|move|p2a: Amoonguss|Spore|p1a: Pikachu\n|-status|p1a: Pikachu|slp\n|turn|2"),
        request(2), decision(2, value=-.9), dict(kind="end", won=False, result=dict(winner="Foe"), turns=2)])
    summary, cases = diag.analyze([path])
    c = next(c for c in cases if c["kind"] == "sleep")
    ev = c["decisions"][0]
    assert c["classification"] == "H3"
    assert c["incident"]["slept"]
    assert ev["set_fraction"] == 1
    assert ev["observed"] is True
    assert ev["reply_worlds"] == 1
    assert ev["mean_solved_mass"] == .5
    assert c["value_delta"] == pytest.approx(-.4)
    assert c["probability_delta"] == pytest.approx(-.2)
    assert c["request"]["side"]["id"] == "p1"
    assert c["snapshot_ref"]["event_index"] == 2
    assert c["snapshot_ref"]["path"] == str(path)
    assert ev["worlds_detail"][0]["replies"][0]["chosen_payoff"] == -.5
    assert any("powder immunity" in h for row in ev["candidates"] for h in row["prevention_hints"])
    report = diag.render_report(summary, cases, partial=True)
    assert "PARTIAL" in report
    assert "Spore" in report
    assert "https://replay.pokemonshowdown.com/gen9randomdoublesbattle-test" in report


@pytest.mark.parametrize("in_set,in_replies,label", [(False, False, "H2"), (True, False, "H1"), (True, True, "H3")])
def test_sleep_classification(tmp_path, in_set, in_replies, label):
    path = write_records(tmp_path, [initial(), request(1), decision(1, in_set=in_set, in_replies=in_replies),
        message("|move|p2a: Amoonguss|Spore|p1a: Pikachu\n|-miss|p2a: Amoonguss|p1a: Pikachu\n|turn|2")])
    summary, cases = diag.analyze([path])
    c = next(c for c in cases if c["kind"] == "sleep")
    assert c["classification"] == label
    assert not c["incident"]["slept"]
    assert summary["sleep_landed"] == 0


def test_yawn_includes_escape_decision_and_no_protect_hint(tmp_path):
    path = write_records(tmp_path, [initial(), request(1, grass=True), decision(1, sleep="Yawn"),
        message("|move|p2a: Amoonguss|Yawn|p1a: Pikachu\n|-start|p1a: Pikachu|move: Yawn\n|turn|2"),
        request(2, grass=True), decision(2, sleep="Yawn"),
        message("|-status|p1a: Pikachu|slp\n|turn|3"), request(3), decision(3)])
    _, cases = diag.analyze([path])
    sleeps = [c for c in cases if c["kind"] == "sleep"]
    assert len(sleeps) == 1
    c = sleeps[0]
    assert c["incident"]["sleep_turn"] == 2
    assert [d["turn"] for d in c["decisions"]] == [1, 2]
    assert c["decisions"][1]["phase"] == "yawn_escape"
    hints = [h for row in c["decisions"][1]["candidates"] for h in row["prevention_hints"]]
    assert "switch clears existing Yawn drowsiness" in hints
    assert not any("Protect" in h or "powder immunity" in h for h in hints)


def test_endgame_stall_discount_and_repeat(tmp_path):
    path = write_records(tmp_path, [initial(), request(1), decision(1),
        message("|move|p1a: Pikachu|Protect|p1a: Pikachu\n|move|p1b: Snorlax|Recover|p1b: Snorlax\n|turn|2"),
        request(2), decision(2), message("|faint|p1a: Pikachu\n|turn|3"),
        dict(kind="end", won=False, end_reason="inactivity", result=dict(winner="Foe"), turns=3)])
    summary, cases = diag.analyze([path])
    ends = [c for c in cases if c["kind"] == "endgame"]
    assert len(ends) == 2
    assert ends[0]["q_gap"] == pytest.approx(.001)
    assert ends[0]["discount_effect_signed"]["1"] == pytest.approx(-.0025)
    assert ends[0]["discount_comparable_horizons"] == ["1", "3", "5", "10"]
    assert ends[1]["repeated_stall"]
    assert ends[1]["next_turn"]["faints"] == ["|faint|p1a: Pikachu"]
    assert summary["endgame"]["stall"] == 2
    assert summary["battles"][0]["timer"]
    assert summary["search_policy_disagreed"] == 2
    assert summary["timing_ms"]["total_ms"]["p95"] == 2000


def test_unknown_bench_counts_as_alive_and_configurable(tmp_path):
    init = initial()
    init["text"] = init["text"].replace("|teamsize|p2|2", "|teamsize|p2|6")
    path = write_records(tmp_path, [init, request(1), decision(1)])
    assert diag.analyze([path])[0]["endgame"]["decisions"] == 0
    assert diag.analyze([path], per_side=0, total=8)[0]["endgame"]["decisions"] == 1


def test_truncated_last_member_keeps_previous_events(tmp_path):
    path = write_records(tmp_path, [initial(), request(1), decision(1)])
    with path.open("ab") as handle:
        # Full JSON may decompress, but the gzip trailer is incomplete.
        handle.write(gzip.compress(b'{"kind":"end","won":true}\n')[:-5])
    b = diag.Battle.read(path)
    assert len(b.decisions) == 1
    assert b.end is None
    assert "truncated final gzip member" in b.warnings[0]
    summary, cases = diag.main([str(tmp_path), "--out", str(tmp_path / "report.md"),
                               "--cases", str(tmp_path / "cases.jsonl"), "--partial"])
    assert summary["outcomes"] == {"unfinished": 1}
    assert len([json.loads(l) for l in (tmp_path / "cases.jsonl").read_text().splitlines()]) == len(cases)


def test_refused_and_forced_are_excluded_and_late_not_substituted(tmp_path):
    d = decision(1)
    d["source"] = "fallback"
    del d["analysis"]["search_record"]
    path = write_records(tmp_path, [initial(), request(1), d,
        dict(kind="late_decision", seq=1, analysis=decision(1)["analysis"]),
        message("|move|p2a: Amoonguss|Spore|p1a: Pikachu\n|-status|p1a: Pikachu|slp\n|turn|2"),
        request(2, forced=True), decision(2), request(3), decision(3), dict(kind="refused", rqid=3)])
    summary, cases = diag.analyze([path])
    sleep = next(c for c in cases if c["kind"] == "sleep")
    assert sleep["classification"] == "unavoidable/other"
    assert not sleep["snapshot_ref"]["present"]
    assert summary["late_decisions"] == 1
    assert summary["refused_decisions"] == 1
    assert summary["endgame"]["decisions"] == 1


def test_missing_or_wrong_species_is_not_h2(tmp_path):
    d = decision(1)
    d["analysis"]["search_record"]["worlds"][0]["team"][0]["species"] = "ditto"
    path = write_records(tmp_path, [initial(), request(1), d,
        message("|move|p2a: Amoonguss|Spore|p1a: Pikachu\n|-status|p1a: Pikachu|slp")])
    _, cases = diag.analyze([path])
    assert next(c for c in cases if c["kind"] == "sleep")["classification"] == "unavoidable/other"


def test_wrong_foe_slot_and_substituted_payoffs(tmp_path):
    d = decision(1)
    w = d["analysis"]["search_record"]["worlds"][0]
    w["text"] = [["Pollen Puff", "Spore -> Pikachu"]]
    path = write_records(tmp_path, [initial(), request(1), d,
        message("|move|p2a: Amoonguss|Spore|p1a: Pikachu")])
    _, cases = diag.analyze([path])
    assert next(c for c in cases if c["kind"] == "sleep")["classification"] == "H1"
    w["text"] = [["Spore -> Pikachu", "pass"]]
    w["counts"][0][0] = 0
    b = diag.Battle.read(path)
    b.decisions[0]["event"] = d
    incident = diag.sleep_incidents(b)[0]
    evidence = diag.sleep_evidence(b.decisions[0], incident, .25)
    assert evidence["worlds_detail"][0]["replies"][0]["chosen_payoff"] is None


def test_two_events_in_one_member_and_corrupt_member(tmp_path):
    path = write_records(tmp_path, [initial()])
    payload = json.dumps(request(1)) + "\n" + json.dumps(decision(1)) + "\n"
    with path.open("ab") as handle:
        handle.write(gzip.compress(payload.encode()))
        corrupt = bytearray(gzip.compress(b'{"kind":"end"}\n'))
        corrupt[-8] ^= 1  # Bad CRC must not admit this event.
        handle.write(corrupt)
    b = diag.Battle.read(path)
    assert len(b.decisions) == 1
    assert b.end is None
    assert "corrupt gzip member" in b.warnings[0]


def test_self_rest_is_other_and_not_a_successful_foe_sleep(tmp_path):
    path = write_records(tmp_path, [initial(), request(1), decision(1),
        message("|move|p2a: Amoonguss|Spore|p1a: Pikachu\n|-miss|p2a: Amoonguss|p1a: Pikachu\n"
                "|move|p1a: Pikachu|Rest|p1a: Pikachu\n|-status|p1a: Pikachu|slp")])
    _, cases = diag.analyze([path])
    sleeps = [c for c in cases if c["kind"] == "sleep"]
    assert len(sleeps) == 2
    assert sleeps[0]["incident"]["slept"] is False
    assert sleeps[1]["classification"] == "unavoidable/other"


def test_winning_stall_not_a_loss_discount_flag(tmp_path):
    path = write_records(tmp_path, [initial(), request(1), decision(1, value=.5, attack=.499)])
    _, cases = diag.analyze([path])
    c = next(c for c in cases if c["kind"] == "endgame")
    assert c["close_values"]
    assert c["discount_effect_signed"]["1"] > 0
    assert not c["discount_comparable_horizons"]


def test_yawn_switch_clears_it_and_retains_escape_decision(tmp_path):
    path = write_records(tmp_path, [initial(), request(1, grass=True), decision(1, sleep="Yawn"),
        message("|move|p2a: Amoonguss|Yawn|p1a: Pikachu\n|-start|p1a: Pikachu|move: Yawn\n|turn|2"),
        request(2, grass=True), decision(2, sleep="Yawn", choice="switch 3, move 2 +1"),
        message("|switch|p1a: Amoonguss|Amoonguss, L80|100/100\n|turn|3"), request(3), decision(3)])
    _, cases = diag.analyze([path])
    c = next(c for c in cases if c["kind"] == "sleep")
    assert not c["incident"]["slept"]
    assert [d["turn"] for d in c["decisions"]] == [1, 2]


def test_yawn_expires_after_its_resolution_turn(tmp_path):
    path = write_records(tmp_path, [initial(), request(1), decision(1, sleep="Yawn"),
        message("|move|p2a: Amoonguss|Yawn|p1a: Pikachu\n|-start|p1a: Pikachu|move: Yawn\n|turn|2"),
        request(2), decision(2), message("|turn|3"), request(3), decision(3),
        message("|-status|p1a: Pikachu|slp")])
    _, cases = diag.analyze([path])
    sleeps = [c for c in cases if c["kind"] == "sleep"]
    assert len(sleeps) == 2
    assert not sleeps[0]["incident"]["slept"]
    assert [d["turn"] for d in sleeps[0]["decisions"]] == [1, 2]
    assert sleeps[1]["classification"] == "unavoidable/other"
