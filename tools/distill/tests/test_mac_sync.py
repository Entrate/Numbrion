"""Local relay tests never invoke ssh/scp, including failures before and after Windows publication."""
from __future__ import annotations

import base64
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import pytest

PATH = Path(__file__).resolve().parents[1] / 'mac_sync.py'
spec = importlib.util.spec_from_file_location('mac_sync', PATH)
sync = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = sync
spec.loader.exec_module(sync)


@pytest.fixture
def config(tmp_path, monkeypatch):
    def no_remote(*a, **kw):
        pytest.fail('LOCAL must never start ssh/scp')
    monkeypatch.setattr(subprocess, 'run', no_remote)
    return sync.Config(**{**sync.CONFIG, 'local': True, 'labels': tmp_path / 'mac/labels',
                          'teacher': tmp_path / 'mac/teacher', 'state': tmp_path / 'mac/sync',
                          'local_laptop': tmp_path / 'laptop', 'local_windows': tmp_path / 'windows'})


def test_finished_only_atomic_arrival_and_persistent_ledger(config, monkeypatch):
    loop = sync.Sync(config)
    final = config.labels / 'labels-20261012T000000Z-000000.npz'
    final.write_bytes(b'finished labels')
    for name in ('labels-x.npz.part', 'labels-y.npz.tmp', 'other.npz'):
        (config.labels / name).write_bytes(b'unfinished')
    replace, arrivals = sync.os.replace, []

    def inspect(source, destination):
        source, destination = Path(source), Path(destination)
        if destination.parent == loop.remote.labels:
            assert source.name == destination.name + '.part'
            assert not destination.exists()
            assert not loop.ledger_path.exists()
            arrivals.append(destination.name)
        replace(source, destination)

    monkeypatch.setattr(sync.os, 'replace', inspect)
    assert loop.round(force=True)
    assert arrivals == [final.name]
    assert json.loads(loop.ledger_path.read_text()) == [final.name]
    assert (loop.remote.labels / final.name).read_bytes() == b'finished labels'
    resumed = sync.Sync(config)
    assert resumed.backlog() == []
    resumed.round(force=True)
    assert len(arrivals) == 1


def test_transfer_failure_backoff_then_retry(config):
    loop = sync.Sync(config)
    final = config.labels / 'labels-run-000000.npz'
    final.write_bytes(b'labels')
    failure = config.local_windows / 'FAIL_TRANSFER'
    failure.touch()
    assert not loop.round(force=True)
    assert not loop.ledger_path.exists()
    assert not (loop.remote.labels / final.name).exists()
    assert (loop.remote.labels / (final.name + '.part')).exists()
    assert loop.failures['shards'] == 1 and loop.due['shards'] > 0
    assert loop.backlog() == [final]
    failure.unlink()
    assert loop.round(force=True)
    assert not list(loop.remote.labels.glob('*.part'))
    assert loop.sent == {final.name} and loop.last_error == ''


@pytest.mark.parametrize('destination', ['', 'used', 'heldout'])
def test_rename_succeeded_ledger_failed_retry_does_not_republish(config, monkeypatch, destination):
    loop = sync.Sync(config)
    final = config.labels / 'labels-run-000000.npz'
    final.write_bytes(b'labels')
    write = sync.atomic_json

    def fail_ledger(path, value):
        if path == loop.ledger_path:
            raise OSError('ledger disk failure')
        write(path, value)

    monkeypatch.setattr(sync, 'atomic_json', fail_ledger)
    assert not loop.round(force=True)
    published = loop.remote.labels / final.name
    if destination:
        consumed = loop.remote.labels / destination
        consumed.mkdir()
        published.rename(consumed / final.name)
    monkeypatch.setattr(sync, 'atomic_json', write)
    resumed = sync.Sync(config)
    assert resumed.round(force=True)
    assert (resumed.remote.labels / destination / final.name).exists()
    if destination:
        assert not (resumed.remote.labels / final.name).exists()
    assert resumed.sent == {final.name}


def test_pull_numeric_snapshot_atomic_and_keep_last_three(config, monkeypatch):
    loop = sync.Sync(config)
    loop.remote.snapshots.mkdir(parents=True)
    for n in (115, 120, 125):
        (config.teacher / f'm{n}.pt').write_bytes(str(n).encode())
    for name in ('m999.pt', 'm1000.pt', 'm9999.pt.part', 'latest.pt'):
        (loop.remote.snapshots / name).write_bytes(name.encode())
    replace = sync.os.replace

    def inspect(source, destination):
        if Path(destination).parent == config.teacher:
            assert Path(source).name == 'm1000.pt.part'
            assert Path(source).read_bytes() == b'm1000.pt'
            assert not Path(destination).exists()
        replace(source, destination)

    monkeypatch.setattr(sync.os, 'replace', inspect)
    loop.pull()
    assert sorted(p.name for p in config.teacher.glob('*.pt')) == ['m1000.pt', 'm120.pt', 'm125.pt']
    loop.pull()
    assert not list(config.teacher.glob('*.part'))
    assert sync.newest_snapshot(['m099.pt', 'm100.pt', 'm100.pt.part']) == 'm100.pt'


def test_failed_teacher_fetch_never_publishes_or_prunes(config, monkeypatch):
    loop = sync.Sync(config)
    (config.teacher / 'm115.pt').write_bytes(b'old')
    monkeypatch.setattr(loop.remote, 'newest', lambda: 'm120.pt')

    def fail(name, destination):
        destination.write_bytes(b'partial')
        raise OSError('interrupted fetch')

    monkeypatch.setattr(loop.remote, 'fetch', fail)
    assert not loop.round(force=True)
    assert not (config.teacher / 'm120.pt').exists()
    assert (config.teacher / 'm115.pt').exists()
    assert loop.failures['teacher'] == 1


def test_encoded_powershell_and_retry_guards():
    command = sync.powershell(sync.publish_script("C:/user's path/labels", ['labels-r-000000.npz']))
    script = base64.b64decode(command.split()[-1]).decode('utf-16le')
    assert '$ProgressPreference="SilentlyContinue"' in script
    assert "C:/user''s path/labels" in script
    assert "'used'" in script and "'heldout'" in script
    assert 'Move-Item -LiteralPath $part -Destination $final -ErrorAction Stop' in script


def test_batch_limit_and_fallback_teacher_status(config):
    config.batch = 2
    loop = sync.Sync(config)
    (config.teacher / 'initial.pt').write_bytes(b'initial teacher')
    for index in range(3):
        (config.labels / f'labels-run-{index:06d}.npz').write_bytes(b'labels')
    assert loop.round(force=True)
    assert len(loop.sent) == 2 and len(loop.backlog()) == 1
    status = json.loads((config.state / 'status.json').read_text())
    assert status['teacher'] == 'initial.pt'
    assert loop.round(force=True)
    assert len(loop.sent) == 3 and not loop.backlog()


def test_pruning_includes_old_unnumbered_seed(config):
    loop = sync.Sync(config)
    (config.teacher / 'initial.pt').write_bytes(b'initial teacher')
    for n in (115, 120, 125):
        (config.teacher / f'm{n}.pt').write_bytes(str(n).encode())
    loop.pull()
    assert len(list(config.teacher.glob('*.pt'))) == 3
    assert not (config.teacher / 'initial.pt').exists()
