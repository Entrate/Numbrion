"""Regression tests for scored-game diagnostics and Protect opportunities."""
import sys
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest

import evaluate


DEX = SimpleNamespace(moves=[{"id": "tackle", "category": "Physical"}])


def test_play_logs_choices_and_values_match_scored_games(monkeypatch, tmp_path):
    class StubVecEnv:
        def __init__(self, n, pools, **kwargs):
            assert n == 2 and kwargs["mirror"] and kwargs["log_games"]
            self.obs = dict(needs_action=np.ones((n, 2), dtype=bool),
                            mask0=np.ones((n, 2, evaluate.N_ACTIONS), dtype=bool),
                            battle_id=np.ones(n, dtype=np.int64), turn=np.ones(n, dtype=np.int32),
                            done=np.zeros(n, dtype=bool), winner=np.full(n, -1))
            self.logs = []
            self.steps = 0

        def __enter__(self):
            return self

        def __exit__(self, *args):
            pass

        def reset(self):
            return self.obs

        def mask_slot1(self, a0):
            return self.obs["mask0"]

        def step(self, actions):
            self.steps += 1
            # Env 0 finishes twice while its mirrored partner finishes once.
            self.obs["done"][:] = (True, self.steps == 2)
            self.obs["winner"][:] = 0
            for e in np.flatnonzero(self.obs["done"]):
                battle = int(self.obs["battle_id"][e])
                for side in (0, 1):
                    own, foe = f"p{side + 1}", f"p{2 - side}"
                    self.logs.append(dict(env=int(e), side=side, battle=battle, winner=0, log=[
                        "|turn|1",
                        f"|move|{own}a: Attacker|Tackle|{foe}a: Target",
                        f"|faint|{foe}a: Target",
                        f"|move|{own}b: Partner|Tackle|{foe}b: Other",
                    ]))
                self.obs["battle_id"][e] += 1
                self.obs["turn"][e] = 1
            self.obs["turn"][~self.obs["done"]] += 1
            return self.obs  # Mutates the same arrays, like VecEnv's shared memory.

        def finished_logs(self):
            return self.logs

    class StubAgent:
        record_values = True
        name = "stub"

        def first(self, obs, rows, mask):
            self.last_values = obs["battle_id"].copy()
            return np.full(len(rows), 6, dtype=np.int32)  # Single-target attack at foe slot a.

        def second(self, rows, mask):
            return np.full(len(rows), 6, dtype=np.int32)

    monkeypatch.setattr(evaluate, "VecEnv", StubVecEnv)
    result, values, logs, a_side = evaluate.play(
        StubAgent(), StubAgent(), tmp_path / "unused.txt", 1, 1, 7, 2, log_games=True)

    assert result["games"] == 2
    assert result["wins"] == 1 and result["score"] == 0.5
    assert result["pair_splits"] == 1 and result["steps"] == 2
    assert {(x["env"], x["battle"], x["side"]) for x in logs} == {
        (0, 1, 0), (0, 1, 1), (1, 1, 0), (1, 1, 1)}
    assert len(logs) == 4  # Both player views of each scored game.
    own_logs = [x for x in logs if x["side"] == a_side[x["env"]]]
    assert [x["choices"] for x in own_logs] == [{1: (6, 6)}, {1: (6, 6), 2: (6, 6)}]
    assert all("choices" not in x for x in logs if x not in own_logs)
    counts = evaluate.mistakes(own_logs, DEX)
    assert counts["battles"] == result["games"]
    assert counts["chosen_attacks"] == 4
    assert counts["focus_fire"] == 2 and counts["overkill_retarget"] == 2
    assert values == [(1.0, 1.0), (1.0, 0.0), (1.0, 0.0)]
    replays = evaluate.write_replays(logs, a_side, tmp_path / "replays", "stub", "stub", 10)
    assert len(replays) == 1  # Only env 0's scored player-1 view is replay eligible.


@pytest.mark.parametrize("side", [0, 1])
@pytest.mark.parametrize("protect_move", ["Protect", "Spiky Shield"])
def test_protector_opportunities_use_history_at_start_of_turn(side, protect_move):
    own, foe = f"p{side + 1}", f"p{2 - side}"
    lines = []
    # First revelation, consecutive use, turn after use, rested protector, rested use, repeat.
    for turn in range(1, 7):
        lines.append(f"|turn|{turn}")
        if turn in (1, 2, 5, 6):
            lines.append(f"|move|{foe}a: Defender|{protect_move}|{foe}a: Defender")
            lines.append(f"|move|{own}b: Partner|Protect|{own}b: Partner")
        lines.append(f"|move|{own}a: Attacker|Tackle|{foe}a: Defender")
        if turn in (1, 2, 5, 6):
            lines.append(f"|-activate|{foe}a: Defender|move: Protect")
            lines.append(f"|-activate|{own}b: Partner|move: Protect")
        lines.append("|")

    counts = evaluate.mistakes([dict(side=side, log=lines)], DEX)

    assert counts["attacks"] == 6 and counts["into_protect"] == 4
    assert counts["attacks_vs_protector"] == 2  # Turns 4 and 5 only.
    assert counts["into_protect_vs_protector"] == 1  # Turn 5 only.
    assert counts["into_protect_per_attack_vs_protector"] == 0.5
    assert counts["protects"] == 4 and counts["protect_repeats"] == 2
    assert counts["protect_blocks"] == 4
