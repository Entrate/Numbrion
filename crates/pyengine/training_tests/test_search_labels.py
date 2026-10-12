"""Atomic labels and teacher changes, including games completing across a reload and STOP."""
from __future__ import annotations

import argparse
import os
import sys
import threading
import time
from pathlib import Path

import numpy as np
import pytest
import torch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import search_labels as labels


def checkpoint(path, minutes=None):
    torch.save({} if minutes is None else {"minutes": minutes}, path)
    return labels.Teacher.load(path)


def test_teacher_selection_is_numeric_then_mtime(tmp_path):
    assert labels.newest_teacher(tmp_path) is None
    a, b = tmp_path / 'other.pt', tmp_path / 'recent.pt'
    a.touch()
    b.touch()
    os.utime(a, ns=(1, 1))
    os.utime(b, ns=(2, 2))
    assert labels.newest_teacher(tmp_path) == b
    (tmp_path / 'm9999.pt.part').touch()
    (tmp_path / 'm9998.pt.tmp').touch()
    (tmp_path / 'm1000.pt').touch()
    (tmp_path / 'm999.pt').touch()
    assert labels.newest_teacher(tmp_path).name == 'm1000.pt'


def test_atomic_shards_metadata_holdout_and_late_old_game(tmp_path, monkeypatch):
    old = checkpoint(tmp_path / 'm115.pt', 115.2)
    new = checkpoint(tmp_path / 'm120.pt', 120.3)
    missing = checkpoint(tmp_path / 'unknown.pt')
    writer = labels.ShardWriter(tmp_path, 4, '20261012T000000Z')
    replace = labels.os.replace
    seen = []

    def inspect(source, destination):
        assert source.name.endswith('.npz.tmp') and not destination.exists()
        with np.load(source) as shard:
            assert shard['teacher'].shape == ()
            assert shard['teacher_minutes'].dtype == np.float64
            assert shard['created'].dtype == np.float64
            assert shard['holdout'].dtype == bool
        seen.append(destination.name)
        replace(source, destination)

    monkeypatch.setattr(labels.os, 'replace', inspect)
    row = {'value': np.float32(1), 'pairs': np.array([[1, 2]], dtype=np.int16)}
    writer.add([row] * 3, old)
    writer.add([row] * 2, new)
    writer.add([row], old)
    writer.flush(final=True)
    assert len(seen) == 3
    for index, (name, count, minutes) in enumerate([('m115.pt', 3, 115.2), ('m120.pt', 2, 120.3),
                                                 ('m115.pt', 1, 115.2)]):
        with np.load(tmp_path / seen[index]) as shard:
            assert shard['teacher'].item() == name
            assert shard['teacher_minutes'].item() == minutes
            assert len(shard['value']) == count
    writer.add([row] * (17 * 4), missing)
    with np.load(tmp_path / seen[19]) as shard:
        assert shard['holdout'].item() is True
        assert np.isnan(shard['teacher_minutes'].item())
    assert not list(tmp_path.glob('*.tmp'))
    with pytest.raises(FileExistsError):
        labels.ShardWriter(tmp_path, 4, '20261012T000000Z')


def args(out, follow=None, checkpoint=None):
    return argparse.Namespace(out=out, follow=follow, checkpoint=checkpoint, torch_threads=1,
                              search='{}', pool=str(out / 'teams'), set_pool='unused', seed=11,
                              device='cpu', policy_temperature=0.5, explore=0.25, games=0,
                              concurrency=2, shard=64, minutes=float('inf'), reload_minutes=0.001)


def test_follow_rebuilds_bots_keeps_inflight_teacher_and_drains_stop(tmp_path, monkeypatch):
    teacher = tmp_path / 'teacher'
    teacher.mkdir()
    checkpoint(teacher / 'm115.pt', 115)
    (tmp_path / 'teams').write_text('team1\nteam2\n')
    settings = args(tmp_path, follow=teacher)
    slow_started, new_started, release_old = threading.Event(), threading.Event(), threading.Event()
    starts, built = [], []
    start_lock = threading.Lock()

    class Bot:
        def __init__(self, **kwargs):
            self.teacher = Path(kwargs['checkpoint']).name
            built.append((threading.get_ident(), self.teacher))

    def game(bots, *unused):
        name = bots[0].teacher
        with start_lock:
            index = len(starts)
            starts.append(name)
        if index == 0:
            slow_started.set()
            assert release_old.wait(5)
        elif name == 'm115.pt':
            time.sleep(0.02)
        else:
            (tmp_path / 'STOP').touch()
            new_started.set()
        return [{'value': np.float32(115 if name == 'm115.pt' else 120)}], {'labels': 1}

    monkeypatch.setattr(labels, 'SearchBot', Bot)
    monkeypatch.setattr(labels, 'play_game', game)
    errors = []

    def generate():
        try:
            labels.generate(settings)
        except Exception as error:
            errors.append(error)

    thread = threading.Thread(target=generate)
    thread.start()
    try:
        assert slow_started.wait(5)
        checkpoint(teacher / 'm120.pt', 120)
        assert new_started.wait(5)
        count = len(starts)
        assert thread.is_alive()
        release_old.set()
        thread.join(5)
        assert not thread.is_alive() and not errors
        assert len(starts) == count
        assert any(name == 'm120.pt' for _, name in built)
        shards = sorted(tmp_path.glob('labels-*.npz'))
        assert shards
        names = set()
        for path in shards:
            with np.load(path) as shard:
                name = shard['teacher'].item()
                names.add(name)
                assert np.all(shard['value'] == (115 if name == 'm115.pt' else 120))
                assert shard['teacher_minutes'].item() == (115 if name == 'm115.pt' else 120)
        assert names == {'m115.pt', 'm120.pt'}
    finally:
        release_old.set()
        (tmp_path / 'STOP').touch()
        thread.join(5)


def test_checkpoint_mode_and_worker_errors(tmp_path, monkeypatch):
    teacher = checkpoint(tmp_path / 'one.pt', 5)
    (tmp_path / 'teams').write_text('team1\nteam2\n')
    settings = args(tmp_path, checkpoint=teacher.path)
    settings.games = 1
    settings.concurrency = 1
    monkeypatch.setattr(labels, 'SearchBot', lambda **kwargs: argparse.Namespace())
    monkeypatch.setattr(labels, 'play_game', lambda *a: ([{'value': np.float32(1)}], {'labels': 1}))
    assert labels.generate(settings)['games'] == 1
    with np.load(next(tmp_path.glob('labels-*.npz'))) as shard:
        assert shard['teacher'].item() == 'one.pt'
    for path in tmp_path.glob('labels-*.npz'):
        path.unlink()

    def fail(*a):
        raise RuntimeError('game failed')

    monkeypatch.setattr(labels, 'play_game', fail)
    with pytest.raises(RuntimeError, match='game failed'):
        labels.generate(settings)
