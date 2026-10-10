import numpy as np
import pytest

import numbrion as nb
from conftest import n_battles


def rng_step(b, rng):
    """Random legal codes for every side that has to act; returns them for replay."""
    codes = []
    for side in (0, 1):
        if b.needs_action(side):
            m0, joint = b.legal_mask(side)
            a0 = int(rng.choice(np.flatnonzero(m0)))
            a1 = int(rng.choice(np.flatnonzero(joint[a0])))
            b.choose_action(side, a0, a1)
            codes.append((side, a0, a1))
    return codes


def replay(b, codes):
    for side, a0, a1 in codes:
        b.choose_action(side, a0, a1)


def signature(b):
    return (b.turn, b.ended, b.prng_seed, b.request_json(0), b.request_json(1))


def test_hidden_set_world_equals_replayed_construction(teams):
    """A world with swapped never-revealed opponent sets is the battle built from the swapped packed teams
    (pool genders are explicit, so the original seed applies) after replaying the same decisions."""
    rng = np.random.default_rng(11)
    swaps = 0
    for n in range(n_battles(12)):
        seed = (n, 7, 7, 7)
        b = nb.Battle(seed, teams[2 * n], teams[2 * n + 1], log=True)
        b.start()
        history = []
        for _ in range(int(rng.integers(6))):
            if not b.ended:
                history += rng_step(b, rng)
        if b.ended:
            continue
        viewer = int(rng.integers(2))
        world = b.clone()
        hidden = world.hidden_team_indices(viewer)
        for i in set(range(6)) - set(hidden):
            with pytest.raises(ValueError):
                world.replace_hidden_set(viewer, i, teams[0].split("]")[0])
        for i in hidden:
            donor = teams[int(rng.integers(len(teams)))].split("]")[int(rng.integers(6))]
            try:
                world.replace_hidden_set(viewer, i, donor)
                swaps += 1
            except ValueError as e:
                assert "BattleStart" in str(e)
        ref = nb.Battle(seed, *world.teams, log=True)
        ref.start()
        replay(ref, history)
        assert signature(ref) == signature(world)
        ref.drain_player_log(0), ref.drain_player_log(1)
        ref.reseed((5, 6, 7, 8))
        world.reseed((5, 6, 7, 8))
        while not world.ended:
            replay(ref, rng_step(world, rng))
            assert signature(ref) == signature(world)
            for side in (0, 1):
                assert ref.drain_player_log(side) == world.drain_player_log(side)
    assert swaps > 5
