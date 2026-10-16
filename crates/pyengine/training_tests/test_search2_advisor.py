"""The second layer as the advisor sees it: the layer-1 pick published for two-phase advice (``on_layer1``) and the
``depth2.changed`` flag measured against that pick, also when layer 1 samples its choice (``play: "sample"``).

Reuses test_search2's random-weight model and battles (mechanics, not strength)."""
from __future__ import annotations

from dataclasses import replace

import numpy as np
import pytest

from test_search2 import checkpoint, make_bot, play  # noqa: F401 - checkpoint is a fixture

from training import search2


def run(checkpoint, play_mode, seeds, monkeypatch=None, force=None):
    """Decisions of a bot whose layer 1 plays ``play_mode``, with the layer-1 pick that ``on_layer1`` published for
    each. ``force`` ("keep" or "argmax"): where the sampled pick differs from argmax(x), layer 2's choice is set to
    the sampled pick or to the argmax pick (the cases where comparing with argmax(x) gets ``changed`` wrong)."""
    bot = make_bot(checkpoint)
    bot.search = replace(bot.search, play=play_mode)
    published, forced = {}, set()  # decision number -> the pick on_layer1 published; numbers forced
    if force:
        real = search2.deepen

        def deepen(bot_, cfg, cfg2, layer1, **kw):
            out = real(bot_, cfg, cfg2, layer1, **kw)
            if out is None or out["chosen"] is None:
                return out
            names = [kw["describe"](int(a), int(b)) for a, b in layer1.ours]
            number = counter[0] - 1  # the decision in progress
            best, top = names.index(published[number]), int(np.argmax(layer1.x))
            if best != top:
                out["chosen"] = best if force == "keep" else top
                out["changed"] = out["chosen"] != top  # what deepen reports: against argmax(x)
                out["analysis"].update(chosen=names[out["chosen"]], changed=out["changed"])
                forced.add(number)
            return out

        monkeypatch.setattr(search2, "deepen", deepen)

    counter = [0]

    def hook(player):  # before each decide: a fresh number
        number = counter[0]
        counter[0] += 1
        player.on_layer1 = lambda decision: published.setdefault(number, decision.choice)

    rows = []
    for seed in seeds:
        start = counter[0]
        decisions, _ = play(bot, seed=seed, decisions=6, hook=hook)
        for i, d in enumerate(decisions):
            block = d.analysis.get("depth2")
            if block and block.get("rounds"):
                rows.append((start + i, d, block))
    return rows, published, forced


def check(rows, published):
    for number, d, block in rows:
        assert block["layer1_choice"] == published[number]  # the pick phase 1 showed
        assert block["changed"] == (d.choice != block["layer1_choice"])


def test_sampled_layer1_changed_is_measured_against_the_published_pick(checkpoint):
    rows, published, _ = run(checkpoint, "sample", seeds=(1, 2, 3))
    assert rows
    check(rows, published)


@pytest.mark.parametrize("force", ["keep", "argmax"])
def test_sampled_pick_kept_or_replaced_by_layer2(checkpoint, monkeypatch, force):
    rows, published, forced = run(checkpoint, "sample", seeds=(1, 2, 3, 4), monkeypatch=monkeypatch, force=force)
    assert forced, "no decision sampled a pick other than argmax(x)"
    check(rows, published)
    hit = [(d, block) for number, d, block in rows if number in forced]
    assert hit
    for d, block in hit:
        if force == "keep":  # layer 2 confirms the sampled pick; deepen alone would call it changed
            assert d.choice == block["layer1_choice"] and block["changed"] is False
        else:  # layer 2 moves to argmax(x); deepen alone would call it unchanged
            assert d.choice != block["layer1_choice"] and block["changed"] is True


def test_argmax_play_publishes_argmax_and_keeps_deepens_flag(checkpoint):
    rows, published, _ = run(checkpoint, "argmax", seeds=(5,))
    assert rows
    check(rows, published)
    for _, d, block in rows:
        top = max(block["candidates"], key=lambda c: c["x"])["choice"]
        assert block["layer1_choice"] == top
        assert block["changed"] == (block["chosen"] != top)
