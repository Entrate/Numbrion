import json
import re

import numpy as np
import pytest

import numbrion as nb
from conftest import n_battles


def make(teams, i=0, j=1, seed=(1, 2, 3, 4), log=True):
    b = nb.Battle(seed, teams[i], teams[j], names=("Alice", "Bob"), log=log)
    b.start()
    return b


def first_pair(b, side):
    m0, joint = b.legal_mask(side)
    a0 = int(np.flatnonzero(m0)[0])
    a1 = int(np.flatnonzero(joint[a0])[0])
    return a0, a1


def rng_step(b, rng):
    """Random legal choice for every side that has to act."""
    for side in (0, 1) if rng.integers(2) == 0 else (1, 0):
        if not b.needs_action(side):
            continue
        m0, joint = b.legal_mask(side)
        a0 = int(rng.choice(np.flatnonzero(m0)))
        a1 = int(rng.choice(np.flatnonzero(joint[a0])))
        b.choose_action(side, a0, a1)


def signature(b):
    return (b.turn, b.ended, b.prng_seed, b.request_json(0), b.request_json(1))


def resolve(raw, player):
    """Independent implementation of Showdown's extractChannelMessages for one player (1 or 2)."""
    out, i = [], 0
    while i < len(raw):
        line = raw[i]
        if line.startswith("|split|p"):
            owner = int(line[len("|split|p")])
            secret, shared = raw[i + 1], raw[i + 2]
            chosen = secret if owner == player else shared
            if chosen:
                out.append(chosen)
            i += 3
        else:
            if line:
                out.append(line)
            i += 1
    return out


def test_constants():
    assert nb.N_ACTIONS == 47
    assert nb.PASS == 46 and nb.SWITCH_BASE == 40 and nb.FORCED_MOVE == 4
    assert nb.move_action(1, -1, True) == 1 * 10 + 1 * 2 + 1
    assert nb.describe_action(nb.move_action(2, 1, False)) == "move 3 target +1"
    assert nb.describe_action(nb.switch_action(3)) == "switch 4"
    assert nb.decode_action(nb.PASS) == {"kind": "pass"}
    with pytest.raises(ValueError):
        nb.describe_action(47)


def test_start_request_and_text_choices(teams):
    b = make(teams)
    assert (b.turn, b.ended, b.winner, b.tie) == (1, False, None, False)
    for side in (0, 1):
        req = json.loads(b.request_json(side))
        assert len(req["active"]) == 2 and len(req["side"]["pokemon"]) == 6
        assert b.request_kind(side) == "move" and b.needs_action(side)
    # A garbage choice is rejected with Showdown's error text.
    with pytest.raises(nb.ChoiceError) as e:
        b.choose(0, "move 9")
    assert e.value.args[0].startswith("[Invalid choice]")
    assert "doesn't have a move 9" in e.value.args[0]
    with pytest.raises(ValueError):  # ChoiceError is a ValueError
        b.choose(0, "switch 1")
    # A legal text choice is accepted; the turn advances when both sides are in.
    a0, a1 = first_pair(b, 0)
    b.choose(0, b.action_text(0, a0, a1))
    assert b.turn == 1
    b0, b1 = first_pair(b, 1)
    b.choose(1, b.action_text(1, b0, b1))
    assert b.turn == 2


def test_choose_action_rejects_masked_out_pairs(teams):
    b = make(teams)
    m0, joint = b.legal_mask(0)
    assert m0.dtype == bool and joint.shape == (nb.N_ACTIONS, nb.N_ACTIONS)
    bad = [(a0, a1) for a0 in range(nb.N_ACTIONS) for a1 in range(nb.N_ACTIONS) if not joint[a0, a1]]
    assert bad
    for a0, a1 in bad[:50] + bad[-50:]:
        with pytest.raises(nb.ChoiceError):
            b.clone().choose_action(0, a0, a1)
    with pytest.raises(ValueError):
        b.choose_action(0, 99, 0)
    with pytest.raises(ValueError):
        b.choose_action(2, 0, 0)


def test_masks_exact_on_clones(teams):
    """Every mask-legal pair is accepted (typed and text path agree), every other pair is rejected."""
    rng = np.random.default_rng(5)
    scratch = [None, None]
    checked = 0
    for k in range(n_battles(3)):
        b = make(teams, 3 * k, 3 * k + 1, seed=(k, 2, 3, 4), log=False)
        while not b.ended:
            for side in (0, 1):
                if not b.needs_action(side):
                    continue
                m0, joint = b.legal_mask(side)
                assert (m0 == joint.any(axis=1)).all()
                if scratch[0] is None or scratch[0].teams != b.teams:
                    scratch = [b.clone(), b.clone()]
                typed, text = scratch
                for a0 in range(nb.N_ACTIONS):
                    assert (b.legal_mask_slot1(side, a0) == joint[a0]).all()
                    for a1 in range(nb.N_ACTIONS):
                        typed.copy_from(b)
                        if joint[a0, a1]:
                            typed.choose_action(side, a0, a1)
                            text.copy_from(b)
                            text.choose(side, b.action_text(side, a0, a1))
                            if not b.needs_action(1 - side):  # committed: compare resulting battles
                                assert signature(typed) == signature(text)
                            checked += 1
                        else:
                            assert b.action_text(side, a0, a1) is None
                            with pytest.raises(nb.ChoiceError):
                                typed.choose_action(side, a0, a1)
            rng_step(b, rng)
    assert checked > 200


def test_player_logs_resolve_split_lines(teams):
    rng = np.random.default_rng(1)
    for k in range(n_battles(5)):
        b = make(teams, k, k + 7, seed=(9, k, 8, 7))
        raw, p1, p2 = [], [], []
        while not b.ended:
            rng_step(b, rng)
            raw += b.drain_log()
            p1 += b.drain_player_log(0)
            p2 += b.drain_player_log(1)
        raw_rest = b.drain_log()
        assert raw_rest == []
        assert p1 == resolve(raw, 1)
        assert p2 == resolve(raw, 2)
        assert not any(l.startswith("|split|") or l == "" for l in p1 + p2)
        assert raw[0] == "|t:|" and any(l.startswith("|win|") or l == "|tie" for l in raw)
        # Own side shows exact HP ("123/123"), the opponent only percentages ("100/100"); the omniscient
        # log contains both forms for the same switch-in.
        hp = lambda l: re.search(r"\|(\d+/\d+)(?: \w+)?(?:\||$)", l).group(1)  # noqa: E731
        own = [hp(l) for l in p1 if l.startswith("|switch|p1")]
        seen = [hp(l) for l in p2 if l.startswith("|switch|p1")]
        assert own and seen
        assert any(not h.endswith("/100") for h in own)  # exact HP for the owner
        assert all(h.endswith("/100") for h in seen)  # percentages only for the opponent


def test_log_disabled_battle_refuses_logs(teams):
    b = make(teams, log=False)
    assert not b.log_enabled
    with pytest.raises(RuntimeError):
        b.drain_log()
    with pytest.raises(RuntimeError):
        b.drain_player_log(0)


def test_clone_reseed_copy_from(teams):
    rng = np.random.default_rng(3)
    b = make(teams)
    for _ in range(3):
        rng_step(b, rng)
    c = b.clone()
    assert signature(c) == signature(b)
    assert c.prng_seed == b.prng_seed  # the PRNG is copied

    # Same decisions on both copies give the same battle; the original is untouched by the copy.
    before = signature(b)
    r1, r2 = np.random.default_rng(10), np.random.default_rng(10)
    while not b.ended:
        rng_step(b, r1)
        rng_step(c, r2)
        assert signature(b) == signature(c)
    d = make(teams)
    assert before != signature(b)

    # reseed: a different stream changes the future randomness, the same state changes nothing.
    e = d.clone()
    e.reseed(d.prng_seed)
    assert signature(e) == signature(d)
    f = d.clone()
    f.reseed((1, 1, 1, 1))
    assert f.prng_seed == (1, 1, 1, 1)
    # copy_from restores a snapshot (and refuses a different battle)
    snap = d.clone()
    rng_step(d, np.random.default_rng(0))
    assert signature(d) != signature(snap)
    d.copy_from(snap)
    assert signature(d) == signature(snap)
    other = make(teams, 5, 6, seed=(4, 4, 4, 4))
    with pytest.raises(ValueError):
        d.copy_from(other)


def test_reseed_changes_outcomes(teams):
    """Playing the same decisions under different seeds must diverge somewhere."""
    seen = set()
    for s in range(6):
        b = make(teams, 2, 3, log=False)
        b.reseed((s, s + 1, s + 2, s + 3))
        r = np.random.default_rng(0)
        while not b.ended:
            try:
                rng_step(b, r)
            except (nb.ChoiceError, ValueError):
                break
        seen.add((b.turn, b.winner, b.prng_seed))
    assert len(seen) > 1


def test_search_style_rollout_from_clone(teams):
    """Typical use: clone at a decision, reseed, roll out, original unaffected."""
    b = make(teams, 11, 12, log=False)
    start = signature(b)
    outcomes = []
    for s in range(5):
        c = b.clone()
        c.reseed((s, 0, 0, 1))
        r = np.random.default_rng(s)
        while not c.ended:
            rng_step(c, r)
        outcomes.append((c.winner, c.turn))
    assert signature(b) == start
    assert len({o for o in outcomes}) >= 1
