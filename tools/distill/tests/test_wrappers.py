"""Local wrapper admission checks. No SSH, trainer or actual label maker is launched."""
from __future__ import annotations

import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path

import pytest
import torch

TOOLS = Path(__file__).resolve().parents[1]
WINDOWS = (TOOLS / 'windows-train.ps1').read_text()
CHECK = re.search(r'\$Check = "([^"\n]+)"', WINDOWS)[1]
REQUIRED = {'model', 'optimizer', 'pool', 'elapsed', 'update', 'battles', 'samples', 'numpy_rng'}


def check_checkpoint(path):
    return subprocess.run([sys.executable, '-c', CHECK, str(path)], capture_output=True, text=True)


@pytest.mark.parametrize('missing', sorted(REQUIRED))
def test_windows_rejects_incomplete_trainer_checkpoint(tmp_path, missing):
    path = tmp_path / 'checkpoint with spaces.pt'
    torch.save({key: {} for key in REQUIRED - {missing}}, path)
    result = check_checkpoint(path)
    assert result.returncode != 0
    assert f'missing {missing}' in result.stderr
    assert 'Use latest.pt' in result.stderr


def test_windows_accepts_full_checkpoint_and_rejects_teacher_snapshot(tmp_path):
    full, snapshot = tmp_path / 'latest.pt', tmp_path / 'm115.pt'
    torch.save({key: {} for key in REQUIRED}, full)
    torch.save({'model': {}, 'width': 192, 'layers': 4, 'minutes': 115}, snapshot)
    assert check_checkpoint(full).returncode == 0
    result = check_checkpoint(snapshot)
    assert result.returncode != 0
    assert all(key in result.stderr for key in ('optimizer', 'pool', 'elapsed'))
    assert 'teacher snapshots cannot be resumed' in result.stderr


@pytest.mark.parametrize('kind', ['corrupt', 'non_dict'])
def test_windows_rejects_unreadable_or_non_dict_checkpoint(tmp_path, kind):
    path = tmp_path / 'latest.pt'
    if kind == 'corrupt':
        path.write_bytes(b'partial checkpoint')
    else:
        torch.save([], path)
    assert check_checkpoint(path).returncode != 0


def test_windows_checks_explicit_resume_and_latest_before_task_creation():
    start = WINDOWS[WINDOWS.index("if ($Action -eq 'start')"):WINDOWS.index("if ($Action -eq 'stop')")]
    create = start.index('New-ScheduledTaskAction')
    assert start.index('if ($Resume) { Confirm-ResumeCheckpoint $Resume }') < create
    assert start.index('Confirm-ResumeCheckpoint $Latest') < create
    assert '& $Python -c $Check $Path' in WINDOWS
    assert '$LASTEXITCODE -ne 0' in WINDOWS


@pytest.fixture
def mac_start(tmp_path):
    base = tmp_path / 'loop'
    base.mkdir()
    python = tmp_path / 'fake python'
    python.write_text(f'#!{sys.executable}\n' + """import os, pathlib, signal, sys, time
base = pathlib.Path(os.environ['DISTILL_BASE'])
if sys.argv[1] == '-':
    if os.environ.get('FAIL_VALIDATION'):
        sys.exit('No finished teacher')
    (base / 'checking').touch()
    deadline = time.monotonic() + 10
    while not (base / 'continue').exists():
        if time.monotonic() > deadline:
            sys.exit('validation wait timed out')
        time.sleep(0.01)
else:
    name = 'labels' if sys.argv[2].endswith('search_labels.py') else 'sync'
    (base / (name + '.started')).touch()
    signal.signal(signal.SIGTERM, lambda *a: sys.exit(0))
    while True:
        time.sleep(0.01)
""")
    python.chmod(0o755)
    env = {**os.environ, 'DISTILL_BASE': str(base), 'DISTILL_LABELS': str(base / 'labels'),
           'DISTILL_TEACHER': str(base / 'teacher'), 'DISTILL_SYNC_STATE': str(base / 'sync'),
           'PY': str(python)}
    yield base, env
    for name in ('labels', 'sync'):
        pid_file = base / (name + '.pid')
        if pid_file.exists():
            try:
                os.kill(int(pid_file.read_text()), signal.SIGTERM)
            except ProcessLookupError:
                pass


def test_mac_concurrent_start_is_refused_before_pid_checks(mac_start):
    base, env = mac_start
    first = subprocess.Popen(['bash', str(TOOLS / 'mac-labels.sh'), 'start'], env=env,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.monotonic() + 5
        while not (base / 'checking').exists() and time.monotonic() < deadline:
            assert first.poll() is None
            time.sleep(0.01)
        assert (base / 'checking').exists() and (base / 'start.lock').is_dir()
        second = subprocess.run(['bash', str(TOOLS / 'mac-labels.sh'), 'start'], env=env,
                                capture_output=True, text=True, timeout=5)
        assert second.returncode != 0 and 'Another start command holds' in second.stderr
        assert not (base / 'labels.pid').exists() and not (base / 'sync.pid').exists()
        (base / 'continue').touch()
        stdout, stderr = first.communicate(timeout=5)
        assert first.returncode == 0, stderr
        assert 'Started label maker' in stdout
        assert (base / 'labels.started').exists() and (base / 'sync.started').exists()
        assert not (base / 'start.lock').exists()
        repeated = subprocess.run(['bash', str(TOOLS / 'mac-labels.sh'), 'start'], env=env,
                                  capture_output=True, text=True, timeout=5)
        assert repeated.returncode != 0 and 'Already running' in repeated.stderr
        assert not (base / 'start.lock').exists()
    finally:
        (base / 'continue').touch()
        if first.poll() is None:
            first.terminate()
        first.communicate(timeout=5)


def test_mac_start_releases_lock_when_teacher_check_fails(mac_start):
    base, env = mac_start
    result = subprocess.run(['bash', str(TOOLS / 'mac-labels.sh'), 'start'],
                            env={**env, 'FAIL_VALIDATION': '1'}, capture_output=True, text=True, timeout=5)
    assert result.returncode != 0 and 'No finished teacher' in result.stderr
    assert not (base / 'start.lock').exists()
    assert not (base / 'labels.pid').exists() and not (base / 'sync.pid').exists()
