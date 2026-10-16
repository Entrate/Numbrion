"""Oracle labels at real BatchEnv boundaries, backwards compatibility and complete student checkpoints."""
from __future__ import annotations

import gzip
import json
import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace

import numpy as np
import pytest
import torch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import numbrion as nb
import distill_train as trainer
import search_labels as labels
from train_selfplay import partner_view
from training.dex import load_dex
from training.distill import DistillData, DistillStream, VALUE_KEYS, label_losses, value_target
from training.encoder import RustEncoder
from training.model import Model
from training.ppo import gae
from training.protocol import team_from_request
from training.search import Net, SearchBot
from test_distill import synthetic_labels

HERE = Path(__file__).resolve().parents[1]
POOL = HERE / 'testdata/pool-400.txt.gz'
SEARCH = dict(worlds=1, ours=3, theirs=2, seeds=1, threads=1)


@pytest.fixture(scope='module')
def bot(tmp_path_factory):
    torch.set_num_threads(1)
    torch.manual_seed(9)
    path = tmp_path_factory.mktemp('value-teacher') / 'teacher.pt'
    torch.save(dict(model=Model(load_dex(), 32, 1).state_dict(), width=32, layers=1), path)
    return SearchBot(checkpoint=str(path), search=SEARCH, pools=[str(POOL)], device='cpu')


def test_oracle_and_terminal_return_match_real_batchenv(bot, monkeypatch):
    env = nb.BatchEnv(2, [str(POOL)], seed=19, threads=1, log=True)
    env.reset()
    battle = env.clone_battle(1)
    encoder = RustEncoder(4)
    expected, rewards, steps, pending, active = [], [], 0, {}, []
    current = None
    current_logs = None
    players = []
    new_player = bot.new_player
    def remember(room):
        player = new_player(room)
        players.append(player)
        return player
    monkeypatch.setattr(bot, 'new_player', remember)
    flags = dict(wait=False, switch=False)

    class Bridge:
        """Replay every successful choice in env 1; env 0 proves the 2e / 2e+1 row offset."""
        def start(self):
            pass  # clone_battle is already at the first decision

        def __getattr__(self, name):
            return getattr(battle, name)

        def drain_player_log(self, side):
            nonlocal current, active, current_logs
            if side == 0:
                current_logs = encoder.encode_env(env, keep_logs=True)
                current = {k: v.copy() for k, v in encoder.buffers.arrays().items()}
                active = [s for s in (0, 1) if battle.needs_action(s)]
                flags['wait'] |= len(active) == 1
                flags['switch'] |= any('forceSwitch' in json.loads(battle.request_json(s)) for s in active)
            battle.drain_player_log(side)
            return current_logs[2 + side][0]

        def choose_action(self, side, a0, a1):
            nonlocal steps
            battle.choose_action(side, a0, a1)
            pending[side] = (a0, a1)
            if len(pending) == len(active):
                actions = env.random_actions()
                actions[1] = -1
                for s, pair in pending.items():
                    actions[1, s] = pair
                result = env.step(actions)
                rewards.append(result['reward'][1].copy())
                assert result['done'][1] == battle.ended
                if not battle.ended:
                    assert result['turn'][1] == battle.turn
                steps += 1
                pending.clear()

    original = labels.make_label

    def capture(search, k):
        row = original(search, k)
        side = search['side']
        ids, floats = partner_view(current, np.array([2 + (side ^ 1)]), torch.device('cpu'))
        expected.append((side, steps, ids[0].numpy().copy(), floats[0].numpy().copy(),
                         current['foe_match'][2 + side].copy()))
        np.testing.assert_array_equal(row['ids'], current['ids'][2 + side])
        np.testing.assert_allclose(row['floats'], current['floats'][2 + side], atol=1e-6)
        return row

    teams = [team_from_request(json.loads(battle.request_json(s)), []) for s in (0, 1)]
    monkeypatch.setattr(labels, 'numbrion', SimpleNamespace(Battle=lambda *a, **k: Bridge(), ChoiceError=nb.ChoiceError))
    monkeypatch.setattr(labels, 'make_label', capture)
    rows, stats = labels.play_game([bot, bot], *teams, (1, 2, 3, 4), 0, np.random.default_rng(2), 3)
    assert stats['ended'] and len(rows) > 10 and stats['rejected'] == 0, (stats, [p.builder_failed for p in players])
    assert stats['steps'] == len(rewards) == steps
    assert flags['wait'] and flags['switch']
    returns = gae(np.asarray(rewards), np.zeros((steps, 2), np.float32),
                  np.broadcast_to(np.arange(steps)[:, None] == steps - 1, (steps, 2)),
                  np.zeros(2, np.float32), .995, 1.0)[1]
    for row, (side, step, ids, floats, match) in zip(rows, expected):
        np.testing.assert_array_equal(row['partner_ids'], ids)
        np.testing.assert_allclose(row['partner_floats'], floats, atol=1e-6)
        np.testing.assert_array_equal(row['foe_match'], match)
        assert row['outcome'] == rewards[-1][side]
        assert row['remaining_steps'] == steps - step - 1
        assert row['outcome'] * .995 ** row['remaining_steps'] == pytest.approx(returns[step, side], abs=1e-6)
    # Exact same critic computation as PPO after round-tripping the stored arrays.
    arrays = {k: np.stack([r[k] for r in rows]) for k in rows[0]}
    loader = object.__new__(DistillData)
    loader.device, loader.pairs = torch.device('cpu'), 3
    batch = loader._prepare([arrays])
    batch.pop('_reference')
    with torch.no_grad():
        model = bot.net.model
        state = model.trunk(*(batch[k] for k in trainer.OBS_KEYS))
        stored = model.value(state, batch['partner_ids'], batch['partner_floats'])
        reference = model.value(state, torch.from_numpy(np.stack([e[2] for e in expected])),
                                torch.from_numpy(np.stack([e[3] for e in expected])))
        torch.testing.assert_close(stored, reference)


def test_capped_game_has_no_terminal_value_target(bot):
    with gzip.open(POOL, 'rt') as stream:
        teams = [next(stream).strip(), next(stream).strip()]
    rows, stats = labels.play_game([bot, bot], *teams, (1, 2, 3, 4), 0, np.random.default_rng(0), 3, max_turns=1)
    assert rows and not stats['ended'] and stats['winner'] is None
    assert all(np.isnan(r['outcome']) and r['remaining_steps'] == -1 for r in rows)


def add_value_fields(raw):
    n = len(raw['x'])
    raw.update(partner_ids=raw['ids'].copy(), partner_floats=raw['floats'][:, :6].copy(),
               outcome=np.where(np.arange(n) % 2, -1, 1).astype(np.float32),
               remaining_steps=np.arange(n, dtype=np.int32), foe_match=np.full((n, 6), -1, np.int64))
    raw['foe_match'][:, :2] = [0, 1]
    raw['outcome'][-1] = np.nan
    raw['remaining_steps'][-1] = -1
    return raw


def test_old_new_roundtrip_stream_and_value_mask(tmp_path):
    synthetic_labels(tmp_path, shards=2)
    old = DistillStream._read(tmp_path / 'labels-0000.npz')
    new = add_value_fields(DistillStream._read(tmp_path / 'labels-0001.npz'))
    np.savez_compressed(tmp_path / 'labels-0001.npz', **new)
    read = DistillStream._read(tmp_path / 'labels-0001.npz')
    for key in VALUE_KEYS:
        np.testing.assert_array_equal(read[key], new[key])
    data = DistillData([tmp_path], torch.device('cpu'))
    n = len(old['x'])
    assert not data.train['value_mask'][:n].any()
    assert data.train['value_mask'][n:-1].all() and not data.train['value_mask'][-1]
    batch = data.batch(np.arange(data.size))
    model = Model(load_dex(), 32, 1)
    policy, value, aux, _ = label_losses(model, batch)
    selected = batch['value_mask'].bool()
    with torch.no_grad():
        state = model.trunk(*(batch[k] for k in trainer.OBS_KEYS))
        prediction = model.value(state, batch['partner_ids'], batch['partner_floats'])
        expected = (prediction[selected] - value_target(batch)[selected]).square().mean()
        assert value == pytest.approx(float(expected), abs=1e-6)
        for mix in (0, 1):
            target = value_target(batch, mix)
            torch.testing.assert_close(target, batch['search_value'] if mix == 0 else batch['outcome'])
            torch.testing.assert_close(value_target(batch, 1, 0.995), batch['outcome'] * .995 ** batch['remaining_steps'])
    masked = {k: v.clone() for k, v in batch.items()}
    masked['partner_floats'][~selected] = 9
    masked['outcome'][~selected] = 100
    masked['search_value'][~selected] = -100
    assert float(label_losses(model, masked)[1]) == pytest.approx(float(value))
    legacy = data.batch(np.arange(n))
    _, v, a, _ = label_losses(model, legacy)
    assert float(v) == float(a) == 0
    model.zero_grad()
    (v + a).backward()
    assert all(p.grad is None for name, p in model.named_parameters() if name.startswith(('value_head.', 'partner_proj.', 'aux_')))
    assert torch.isfinite(policy + value + aux)
    model.zero_grad()
    (value + aux).backward()
    assert any(p.grad is not None and p.grad.abs().sum() > 0 for p in model.value_head.parameters())
    assert any(p.grad is not None and p.grad.abs().sum() > 0 for p in model.aux_item.parameters())
    stream = DistillStream(tmp_path, torch.device('cpu'), holdout_rows=0, max_age_minutes=float('inf'))
    stream.begin_update(np.random.default_rng(3), 8, 1, 0, 0)
    assert len(stream.shards) == 2 and not list((tmp_path / 'rejected').glob('*'))


def test_bounded_epochs_use_every_row_once_and_keep_holdouts_separate(tmp_path):
    synthetic_labels(tmp_path, shards=3)
    heldout = tmp_path / 'heldout'
    heldout.mkdir()
    (tmp_path / 'labels-0002.npz').rename(heldout / 'labels-0002.npz')
    train, held = trainer.split_shards([tmp_path])
    assert {p.name for p, _ in train} == {'labels-0000.npz', 'labels-0001.npz'}
    assert [p.name for p, _ in held] == ['labels-0002.npz']
    metrics = trainer.score(Model(load_dex(), 32, 1), held, torch.device('cpu'))
    assert metrics['value_rows'] == 0 and metrics['value_mse'] is metrics['value_explained_variance'] is None
    for resident in (0, 10000):
        stream = trainer.Epochs(train, 4, resident, buffer_rows=10)
        batches = list(stream.batches(np.random.default_rng(0), 7, torch.device('cpu')))
        assert sum(len(b['weight']) for b in batches) == stream.rows
        expected = trainer.concat([trainer.compact(p, i) for p, i in train])
        # Compare a multiset of whole observations, so duplicating a row and dropping another fails too.
        actual = np.concatenate([b['floats'].numpy().astype(np.float16) for b in batches])
        assert sorted(row.tobytes() for row in actual) == sorted(row.tobytes() for row in expected['floats'])


def test_invalid_optional_fields_are_rejected(tmp_path):
    synthetic_labels(tmp_path, shards=1)
    path = tmp_path / 'labels-0000.npz'
    raw = add_value_fields(DistillStream._read(path))
    for key, wrong in [('outcome', np.full(len(raw['x']), 2, np.float32)),
                       ('remaining_steps', np.full(len(raw['x']), -2, np.int32))]:
        np.savez_compressed(path, **dict(raw, **{key: wrong}))
        with pytest.raises(ValueError, match='outcome or remaining_steps'):
            DistillStream._read(path)
    np.savez_compressed(path, **{k: v for k, v in raw.items() if k != 'partner_ids'})
    with pytest.raises(ValueError, match='incomplete value fields'):
        DistillStream._inspect(path)


def test_trainer_smoke_checkpoint_and_search_pair(tmp_path):
    dataset = tmp_path / 'labels'
    dataset.mkdir()
    synthetic_labels(dataset, shards=2)
    for path in dataset.glob('*.npz'):
        raw = add_value_fields(DistillStream._read(path))
        np.savez_compressed(path, **raw)
    output = tmp_path / 'student'
    args = trainer.parse_args(['train', '--labels', str(dataset), '--heldout', str(dataset / 'labels-0001.npz'),
                              '--output', str(output), '--device', 'cpu', '--width', '32', '--layers', '1',
                              '--epochs', '8', '--batch', '16', '--aux-coef', '0'])
    # Keep these actual CLI smoke artifacts for debugging when pytest retains tmp_path.
    result = trainer.train(args)
    state = torch.load(output / 'latest.pt', weights_only=False)
    assert result['epoch'] == 8 and result['heldout']['value_rows'] > 0
    net = Net(output / 'latest.pt', load_dex(), 'cpu')
    assert net.width == 32 and len(net.model.blocks) == 1
    log = [json.loads(line) for line in (output / 'metrics.jsonl').read_text().splitlines()]
    assert log[-1]['train']['policy_loss'] < log[0]['train']['policy_loss']
    assert log[-1]['train']['value_mse'] < log[0]['train']['value_mse']
    assert log[-1]['lr'] == 0
    assert all(key in state for key in ('optimizer', 'pool', 'elapsed', 'update', 'battles', 'samples', 'numpy_rng'))
    resume = trainer.parse_args(['train', '--resume', str(output / 'latest.pt'), '--output', str(output),
                                '--device', 'cpu', '--epochs', '9', '--batch', '16', '--aux-coef', '0'])
    assert trainer.train(resume)['rows_seen'] == result['rows_seen'] + trainer.DistillStream._inspect(dataset / 'labels-0000.npz')['size']
    team_pool = tmp_path / 'teams.txt'
    with gzip.open(POOL, 'rt') as stream:
        team_pool.write_text(stream.read())
    command = [sys.executable, str(HERE / 'search_eval.py'), '--checkpoint', str(output / 'latest.pt'),
               '--pool', str(team_pool), '--set-pool', str(POOL), '--pairs', '1', '--concurrency', '1',
               '--device', 'cpu', '--torch-threads', '1', '--search', json.dumps(SEARCH), '--out', str(tmp_path / 'eval')]
    completed = subprocess.run(command, cwd=HERE, capture_output=True, text=True, timeout=60)
    assert completed.returncode == 0, completed.stdout + completed.stderr
