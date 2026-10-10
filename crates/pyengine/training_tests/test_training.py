import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import torch

from train import advantages, sample
from training.observations import PublicEncoder
from training.policy import masked_logprobs


def test_terminal_reward_never_bootstraps_from_auto_reset_battle():
    rewards = np.array([[1, -1]], dtype=np.float32)
    values = np.array([[0.2, -0.2]], dtype=np.float32)
    adv, target = advantages(rewards, values, np.array([[True, True]]), np.array([100, -100]))
    np.testing.assert_allclose(target, rewards)
    np.testing.assert_allclose(adv, [[0.8, -0.8]])


def test_gae_bootstraps_truncated_rollout_and_stops_at_episode_end():
    rewards = np.array([[0], [1], [0]], dtype=np.float32)
    _, target = advantages(rewards, np.zeros_like(rewards), np.array([[False], [True], [False]]), np.array([2]), gamma=1, gae_lambda=1)
    np.testing.assert_allclose(target, [[1], [1], [2]])


def test_masked_sampling_and_gradient_exclude_illegal_actions():
    logits = torch.zeros((100, 47), requires_grad=True)
    mask = torch.zeros_like(logits)
    mask[:, [3, 22]] = 1
    logp = masked_logprobs(logits, mask)
    sampled = sample(logp, np.random.default_rng(3))
    assert set(sampled) == {3, 22}
    loss = -logp[:, 3].mean()
    loss.backward()
    assert torch.equal(logits.grad[:, mask[0] == 0], torch.zeros((100, 45)))


def observation(battle_id, lines, prev_log=None):
    return {"battle_id": battle_id, "turn": 1, "request": '{"side":{"pokemon":[]}}', "log": lines, "prev_log": prev_log or []}


def test_public_encoder_tracks_only_revealed_foes_and_clears_on_auto_reset():
    encoder = PublicEncoder(1)
    empty = encoder.encode(observation(0, []), 0, 0)
    assert not empty[120:180].any()
    revealed = encoder.encode(observation(0, ["|switch|p2a: Dragonite|Dragonite, L80|100/100", "|-damage|p2a: Dragonite|50/100"]), 0, 0)
    assert revealed[120] == 0.5 and revealed[123] == 1
    reset = encoder.encode(observation(1, [], ["|switch|p2a: Secret|Secret|1/100"]), 0, 0)
    np.testing.assert_array_equal(reset, empty)


def test_player_trackers_are_independent_and_hashing_is_deterministic():
    lines = ["|switch|p2a: Pikachu|Pikachu, L80|100/100"]
    first, second = PublicEncoder(1), PublicEncoder(1)
    x = first.encode(observation(0, lines), 0, 0)
    y = second.encode(observation(0, lines), 0, 0)
    np.testing.assert_array_equal(x, y)
    own_view = first.encode(observation(0, lines), 1, 1)
    assert not own_view[120:180].any()
