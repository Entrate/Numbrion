"""Bounded stream reuse, use-once compatibility, checkpoint/archive ordering, live delivery, validation and infinite schedules."""
import json
import math
import shutil
import zlib
from pathlib import Path
from types import SimpleNamespace
from zipfile import ZipFile

import numpy as np
import pytest
import torch

from test_distill import OBS_KEYS, POOL, model_for_test, synthetic_labels
import train_selfplay as trainer
from training.actors import Actors
from training.distill import DistillData, DistillStream, validate
from training.ppo import Adam, EntropyController

DEVICE = torch.device("cpu")


@pytest.fixture(scope="module")
def labels(tmp_path_factory):
    torch.set_num_threads(1)
    directory = tmp_path_factory.mktemp("stream-labels")
    synthetic_labels(directory, shards=1)
    with np.load(directory / "labels-0000.npz") as data:
        return {k: data[k] for k in data.files}


def write_shard(directory, labels, seq, rows=32, holdout=False, teacher_minutes=20.0, created=None):
    directory.mkdir(parents=True, exist_ok=True)
    take = np.arange(rows) % len(labels["x"])
    raw = {k: v[take].copy() for k, v in labels.items()}
    raw["field"][:, 0] = seq * 1000 + np.arange(rows)
    raw.update(teacher=np.array("m020.pt"), teacher_minutes=np.array(teacher_minutes, dtype=np.float64),
               created=np.array(float(seq if created is None else created)), holdout=np.array(holdout))
    path = directory / f"labels-20261012T180000Z-{seq:06d}.npz"
    with path.open("wb") as stream:
        np.savez_compressed(stream, **raw)
    return path


def row_ids(batches):
    return [int(row) for batch in batches.values() for row in batch["field"][:, 0].tolist()]


@pytest.mark.parametrize("max_uses", [1, 2, 4])
@pytest.mark.parametrize("archive_before_resume", [False, True])
def test_reuse_limits_and_gap_survive_every_update_resume(tmp_path, labels, max_uses, archive_before_resume):
    paths = [write_shard(tmp_path, labels, seq, rows=rows) for seq, rows in enumerate((6, 9))]
    stream = DistillStream(tmp_path, DEVICE, max_resident_rows=8, max_uses=max_uses, reuse_gap=3)
    rng, uses, last = np.random.default_rng(7), {}, {}
    for update in range(1, 101):
        ids = row_ids(stream.begin_update(rng, 1, 2, 5, elapsed_minutes=30))
        assert len(ids) == len(set(ids)) and len(ids) <= 2
        fresh = sum(row not in uses for row in ids)
        for row in ids:
            assert row not in last or update - last[row] >= 3
            uses[row], last[row] = uses.get(row, 0) + 1, update
            assert uses[row] <= max_uses
        assert stream.metrics["distill_used_fresh"] == fresh
        assert stream.metrics["distill_used_total"] == sum(uses.values())
        assert stream.size <= 8
        assert stream.rows_spent_total + stream.size + stream.backlog_rows == 15
        trainer.save(tmp_path / "latest.pt", {"distill_stream": stream.state_dict(), "numpy_rng": rng.bit_generator.state})
        if archive_before_resume:
            stream.archive_used()
        saved = torch.load(tmp_path / "latest.pt", weights_only=False)
        stream = DistillStream(tmp_path, DEVICE, max_resident_rows=8, max_uses=max_uses, reuse_gap=3,
                                state=saved["distill_stream"])
        rng.bit_generator.state = saved["numpy_rng"]
    assert len(uses) == 15 and set(uses.values()) == {max_uses}
    assert stream.rows_spent_total == 15 and stream.rows_retired_total == stream.retired_unused_total == 0
    assert stream.metrics == {} and not stream.row_state and not stream.shards
    assert not stream.begin_update(rng, 1, 2, 5, elapsed_minutes=30)
    assert stream.metrics["distill_mean_uses_completed"] == max_uses
    stream.archive_used()
    shutil.copyfile(tmp_path / "used" / paths[0].name, paths[0])
    assert not stream.begin_update(rng, 1, 2, 5, elapsed_minutes=30)
    assert stream.used_total == 15 * max_uses
    trainer.save(tmp_path / "latest.pt", {"distill_stream": stream.state_dict()})
    stream.archive_used()
    assert not paths[0].exists()


@pytest.mark.parametrize("gap", [0, 1, 3, 50])
def test_exact_gap_boundary_and_never_twice_in_one_update(tmp_path, labels, gap):
    path = write_shard(tmp_path, labels, 0, rows=8)
    stream = DistillStream(tmp_path, DEVICE, max_uses=2, reuse_gap=gap)
    rng = np.random.default_rng(8)
    assert set(row_ids(stream.begin_update(rng, 4, 8, 10, update=100))) == set(range(8))
    state = stream.state_dict()["row_state"][path.name]
    assert (state["uses"] == 1).all() and (state["last_used"] == 100).all()
    assert not stream.begin_update(rng, 4, 8, 10, update=100)
    for update in range(101, 100 + max(gap, 1)):
        assert not stream.begin_update(rng, 4, 8, 10, update=update)
        assert stream.metrics["distill_eligible"] == 0
    ids = row_ids(stream.begin_update(rng, 4, 8, 10, update=100 + max(gap, 1)))
    assert len(ids) == len(set(ids)) == 8
    assert stream.metrics["distill_used_fresh"] == 0 and stream.rows_spent_total == 8
    assert not stream.begin_update(rng, 4, 8, 10, update=100 + max(gap, 1))


def test_least_used_first_across_shards_and_use_count_tiers(tmp_path, labels):
    paths = [write_shard(tmp_path, labels, seq, rows=8) for seq in range(2)]
    stream, rng = DistillStream(tmp_path, DEVICE, max_uses=4, reuse_gap=0), np.random.default_rng(3)
    first = row_ids(stream.begin_update(rng, 4, 1, 5))
    second = row_ids(stream.begin_update(rng, 4, 1, 5))
    assert set(first).isdisjoint(second)
    batches = stream.begin_update(rng, 4, 4, 5)
    ids = row_ids(batches)
    fresh = set(range(8)) | set(range(1000, 1008))
    fresh -= set(first + second)
    assert set(ids[:8]) == fresh and len(ids) == len(set(ids)) == 16
    assert stream.metrics["distill_used_fresh"] == 8
    assert stream.metrics["distill_eligible"] == 16
    previous = {seq * 1000 + row: int(stream.row_state[p.name]["uses"][row])
                for seq, p in enumerate(paths) for row in range(8)}
    batches = stream.begin_update(rng, 4, 3, 5)
    ids = row_ids(batches)
    assert [previous[row] for row in ids[:8]] == [1] * 8
    assert [previous[row] for row in ids[8:]] == [2] * 4
    assert stream.metrics["distill_used_fresh"] == 0


def test_use_once_selection_matches_original_rng_choice(tmp_path, labels):
    write_shard(tmp_path, labels, 0, rows=31)
    stream = DistillStream(tmp_path, DEVICE, max_uses=1)
    actual, expected, remaining = np.random.default_rng(6), np.random.default_rng(6), np.arange(31)
    for _ in range(7):
        count = min(len(remaining) // 4, 2, 3)
        picked = expected.choice(len(remaining), size=count * 4, replace=False)
        assert row_ids(stream.begin_update(actual, 4, 2, 3)) == remaining[picked].tolist()
        remaining = np.delete(remaining, picked)
        assert actual.bit_generator.state == expected.bit_generator.state


@pytest.mark.parametrize("teacher_minutes", [20.0, np.nan])
def test_age_retirement_boundary_nan_admission_and_persisted_metrics(tmp_path, labels, teacher_minutes):
    path = write_shard(tmp_path, labels, 0, rows=10, teacher_minutes=teacher_minutes)
    stream = DistillStream(tmp_path, DEVICE, max_uses=4, reuse_gap=0, max_age_minutes=5)
    rng = np.random.default_rng(4)
    assert len(row_ids(stream.begin_update(rng, 2, 1, 3, elapsed_minutes=20))) == 2
    state = stream.state_dict()
    assert state["row_state"][path.name]["admitted_minutes"] == 20
    # Saved state must not share mutable use counters with later, discarded updates.
    assert len(row_ids(stream.begin_update(rng, 2, 1, 3, elapsed_minutes=21))) == 2
    assert state["row_state"][path.name]["uses"].sum() == 2
    trainer.save(tmp_path / "latest.pt", {"distill_stream": state})
    loaded = torch.load(tmp_path / "latest.pt", weights_only=False)["distill_stream"]
    stream = DistillStream(tmp_path, DEVICE, state=loaded, max_uses=4, reuse_gap=0, max_age_minutes=5)
    assert not stream.begin_update(rng, 2, 0, 3, elapsed_minutes=25)
    assert stream.metrics["distill_eligible"] == 10 and stream.rows_retired_total == 0
    assert not stream.begin_update(rng, 2, 4, 5, elapsed_minutes=25.001)
    assert stream.metrics["distill_eligible"] == stream.metrics["distill_used"] == 0
    assert stream.metrics["distill_used_total"] == 2
    assert stream.metrics["distill_rows_spent_total"] == 0
    assert stream.metrics["distill_rows_retired_total"] == 10
    assert stream.metrics["distill_retired_unused_total"] == 8
    assert stream.metrics["distill_mean_uses_completed"] == 0.2
    assert stream.size == stream.backlog_rows == 0 and not stream.row_state
    assert path.name in stream.completed and path.exists() and not (tmp_path / "used").exists()
    state = stream.state_dict()
    trainer.save(tmp_path / "latest.pt", {"distill_stream": state})
    stream.archive_used()
    shutil.copyfile(tmp_path / "used" / path.name, path)
    resumed = DistillStream(tmp_path, DEVICE, state=state, max_age_minutes=5)
    assert not resumed.begin_update(rng, 2, 4, 5, elapsed_minutes=30)
    assert resumed.metrics["distill_mean_uses_completed"] == 0.2 and resumed.rows_retired_total == 10


def test_spent_and_retired_rows_both_contribute_to_completed_mean(tmp_path, labels):
    path = write_shard(tmp_path, labels, 0, rows=8, teacher_minutes=0)
    stream, rng = DistillStream(tmp_path, DEVICE, max_uses=2, reuse_gap=0, max_age_minutes=5), np.random.default_rng(4)
    for _ in range(3):
        stream.begin_update(rng, 4, 1, 3, elapsed_minutes=0)
    assert stream.rows_spent_total == 4 and stream.metrics["distill_mean_uses_completed"] == 2
    assert not stream.begin_update(rng, 4, 1, 3, elapsed_minutes=6)
    assert stream.rows_retired_total == 4 and stream.retired_unused_total == 0
    assert stream.metrics["distill_mean_uses_completed"] == 1.5
    assert stream.used_total == 12 and path.name in stream.completed


def test_stale_backlog_retires_on_admission_and_frees_full_cache(tmp_path, labels, monkeypatch):
    old = [write_shard(tmp_path, labels, seq, rows=20, teacher_minutes=0) for seq in range(3)]
    stream = DistillStream(tmp_path, DEVICE, max_resident_rows=4, reuse_gap=50, max_age_minutes=5)
    rng = np.random.default_rng(4)
    stream.begin_update(rng, 2, 1, 5, elapsed_minutes=0)
    assert stream.size == 4 and stream.backlog_rows == 56
    fresh = write_shard(tmp_path, labels, 3, rows=8, teacher_minutes=6)
    reads, original = [], stream._read

    def read(path, rows, info):
        assert len(rows) + stream.size <= 4
        reads.append(path.name)
        return original(path, rows, info)

    monkeypatch.setattr(stream, "_read", read)
    stream.begin_update(rng, 2, 1, 5, elapsed_minutes=6)
    # Stale disk shards retire unread in the same scan, despite the full cache.
    assert reads == [fresh.name] and stream.completed == {p.name for p in old}
    assert stream.rows_retired_total == 60 and stream.retired_unused_total == 58
    assert stream.size == 4 and stream.backlog_rows == 4
    assert not stream.begin_update(rng, 2, 1, 5, elapsed_minutes=12)
    assert stream.completed == {p.name for p in [*old, fresh]}
    assert stream.rows_retired_total == 68 and stream.retired_unused_total == 64
    assert stream.backlog_rows == stream.size == 0


def test_nan_backlog_age_starts_at_each_shards_first_admission(tmp_path, labels):
    old = write_shard(tmp_path, labels, 0, rows=10, teacher_minutes=np.nan)
    new = write_shard(tmp_path, labels, 1, rows=10, teacher_minutes=np.nan)
    stream, rng = DistillStream(tmp_path, DEVICE, max_resident_rows=4, max_age_minutes=5), np.random.default_rng(2)
    stream.begin_update(rng, 2, 0, 5, elapsed_minutes=100)
    assert stream.row_state[new.name]["admitted_minutes"] == 100 and old.name not in stream.row_state
    stream.begin_update(rng, 2, 0, 5, elapsed_minutes=106)
    assert stream.completed == {new.name} and stream.rows_retired_total == 10
    assert stream.row_state[old.name]["admitted_minutes"] == 106
    assert stream.size == 4 and stream.backlog_rows == 6
    stream.begin_update(rng, 2, 0, 5, elapsed_minutes=112)
    assert stream.rows_retired_total == stream.retired_unused_total == 20


def test_legacy_checkpoint_keeps_consumed_rows_done_with_reuse(tmp_path, labels):
    path = write_shard(tmp_path, labels, 0, rows=8, teacher_minutes=np.nan)
    state = dict(used_rows={path.name: np.array([True] * 4 + [False] * 4)}, completed=[], used_total=4)
    stream = DistillStream(tmp_path, DEVICE, state=state, reuse_gap=0, update=100, elapsed_minutes=30)
    rng = np.random.default_rng(7)
    for _ in range(8):
        ids = row_ids(stream.begin_update(rng, 4, 1, 3, elapsed_minutes=30))
        assert set(ids) <= set(range(4, 8))
    assert stream.used_total == 20 and stream.rows_spent_total == 8
    assert stream.metrics["distill_mean_uses_completed"] == 2.5


def test_use_once_many_updates_and_live_delivery(tmp_path, labels):
    paths = [write_shard(tmp_path, labels, seq, rows=37) for seq in range(3)]
    stream, rng, seen = DistillStream(tmp_path, DEVICE, max_uses=1), np.random.default_rng(7), set()
    for update in range(40):
        if update == 10:
            paths.append(write_shard(tmp_path, labels, 3, rows=25))
        batches = stream.begin_update(rng, 8, 2, 7, elapsed_minutes=30)
        ids = row_ids(batches)
        assert len(ids) == len(set(ids)) and seen.isdisjoint(ids)
        seen.update(ids)
        assert stream.metrics["distill_used"] == len(ids)
        assert stream.metrics["distill_used_total"] == len(seen)
        assert stream.metrics["distill_teacher_lag_min"] == (10.0 if ids else None)
        assert stream.size == stream.metrics["distill_eligible"] - len(ids)
        for shard in stream.shards.values():
            assert len(shard["rows"]) == len(shard["data"]["weight"])
            assert all(v.device == DEVICE for k, v in shard["data"].items() if k != "_reference")
        if len(batches) == 2:
            assert list(batches) == [0, 6]
        assert len(batches) <= 2 and all(len(b["weight"]) == 8 for b in batches.values())
    assert len(seen) == 136 and stream.size == 0 and not stream.shards
    assert all(p.exists() for p in paths) and not (tmp_path / "used").exists()
    trainer.save(tmp_path / "latest.pt", {"distill_stream": stream.state_dict()})
    stream.archive_used()
    assert all((tmp_path / "used" / p.name).exists() and not p.exists() for p in paths)


@pytest.mark.parametrize("archive_before_resume", [False, True])
def test_use_once_checkpoint_resume_partial_and_complete_shards(tmp_path, labels, archive_before_resume):
    first = write_shard(tmp_path, labels, 0, rows=8)
    stream, rng = DistillStream(tmp_path, DEVICE, max_uses=1), np.random.default_rng(9)
    seen = set(row_ids(stream.begin_update(rng, 4, 2, 2)))
    partial = write_shard(tmp_path, labels, 1, rows=31)
    seen.update(row_ids(stream.begin_update(rng, 4, 2, 3)))
    state = stream.state_dict()
    assert first.name in state["completed"] and state["row_state"][partial.name]["uses"].sum() == 8
    trainer.save(tmp_path / "latest.pt", {"distill_stream": state, "numpy_rng": rng.bit_generator.state})
    if archive_before_resume:
        stream.archive_used()
    loaded = torch.load(tmp_path / "latest.pt", weights_only=False)
    resumed = DistillStream(tmp_path, DEVICE, max_uses=1, state=loaded["distill_stream"])
    rng.bit_generator.state = loaded["numpy_rng"]
    for _ in range(20):
        ids = row_ids(resumed.begin_update(rng, 1, 4, 5))
        assert len(ids) == len(set(ids)) and seen.isdisjoint(ids)
        seen.update(ids)
    assert len(seen) == 39 and resumed.used_total == 39
    trainer.save(tmp_path / "latest.pt", {"distill_stream": resumed.state_dict()})
    resumed.archive_used()
    # Sync can resend an already consumed filename. The checkpoint's completed set still rejects it.
    shutil.copyfile(tmp_path / "used" / first.name, first)
    assert not resumed.begin_update(rng, 1, 4, 5)
    assert resumed.used_total == 39
    resumed.archive_used()
    assert not first.exists()


def test_unsaved_updates_can_be_replayed_without_losing_rows(tmp_path, labels):
    write_shard(tmp_path, labels, 0, rows=32)
    stream, rng = DistillStream(tmp_path, DEVICE, max_uses=1), np.random.default_rng(2)
    committed = set(row_ids(stream.begin_update(rng, 4, 1, 3)))
    state = stream.state_dict()
    row_ids(stream.begin_update(rng, 4, 1, 3))
    assert sum(s["uses"].sum() for s in state["row_state"].values()) == 4
    restored = DistillStream(tmp_path, DEVICE, max_uses=1, state=state)
    ids = row_ids(restored.begin_update(rng, 4, 4, 5))
    assert committed.isdisjoint(ids) and restored.size == 12


def test_newest_holdout_rows_never_trained_and_reload_newest_first(tmp_path, labels):
    write_shard(tmp_path, labels, 0, rows=16)
    old = write_shard(tmp_path, labels, 19, rows=12, holdout=True)
    new = write_shard(tmp_path, labels, 39, rows=12, holdout=True)
    stream = DistillStream(tmp_path, DEVICE, max_uses=1, holdout_rows=18)
    batches = stream.begin_update(np.random.default_rng(3), 4, 4, 5)
    assert set(row_ids(batches)) == set(range(16))
    expected = set(range(39000, 39012)) | set(range(19006, 19012))
    assert set(stream.held["field"][:, 0].tolist()) == expected and stream.held_size == 18
    assert not old.exists() and not new.exists()
    assert (tmp_path / "heldout" / old.name).exists()
    model = model_for_test()
    metrics = validate(model, stream, chunk=7)
    assert np.isfinite(metrics["distill_val_loss"]) and 0 <= metrics["distill_val_agree"] <= 1
    reads = []
    original = DistillStream._read
    with pytest.MonkeyPatch.context() as patch:
        patch.setattr(DistillStream, "_read",
                      staticmethod(lambda p, *args: (reads.append(p.name), original(p, *args))[1]))
        resumed = DistillStream(tmp_path, DEVICE, max_uses=1, holdout_rows=18, state=stream.state_dict())
    assert reads == [new.name, old.name]
    assert set(resumed.held["field"][:, 0].tolist()) == expected
    newest = write_shard(tmp_path, labels, 59, rows=25, holdout=True)
    assert not resumed.begin_update(np.random.default_rng(4), 4, 4, 5)
    assert set(resumed.held["field"][:, 0].tolist()) == set(range(59007, 59025))
    assert (tmp_path / "heldout" / newest.name).exists()


def test_ignores_temporary_and_partial_shards_and_quarantines_invalid_final_name(tmp_path, labels):
    directory = tmp_path / "arriving"
    stream = DistillStream(directory, DEVICE, max_uses=1)
    assert not stream.begin_update(np.random.default_rng(0), 4, 4, 5)
    final = write_shard(directory, labels, 0, rows=8)
    contents = final.read_bytes()
    for suffix in (".tmp", ".part"):
        (directory / (final.name + suffix)).write_bytes(contents)
    (directory / "unrelated.npz").write_bytes(contents)
    (directory / "labels-directory.npz").mkdir()
    final.write_bytes(contents[:100])
    rng = np.random.default_rng(1)
    assert not stream.begin_update(rng, 4, 4, 5) and stream.size == 0
    assert not final.exists() and (directory / "rejected" / final.name).exists()
    final.write_bytes(contents)
    assert set(row_ids(stream.begin_update(rng, 4, 4, 5))) == set(range(8))


def rewrite_shard(path, key, value):
    with np.load(path, allow_pickle=False) as data:
        raw = {k: data[k] for k in data.files}
    raw[key] = value(raw[key]) if callable(value) else value
    with path.open("wb") as output:
        np.savez_compressed(output, **raw)


def test_corrupt_zlib_shard_is_quarantined_and_does_not_crash_or_retry(tmp_path, labels, capsys):
    bad = write_shard(tmp_path, labels, 1, rows=8)
    good = write_shard(tmp_path, labels, 0, rows=8)
    contents = bytearray(bad.read_bytes())
    with ZipFile(bad) as archive:
        member = archive.getinfo("ids.npy")
    offset = member.header_offset
    start = offset + 30 + int.from_bytes(contents[offset + 26:offset + 28], "little") \
        + int.from_bytes(contents[offset + 28:offset + 30], "little")
    contents[start] = (contents[start] & ~6) | 6  # reserved DEFLATE block type
    bad.write_bytes(contents)
    with pytest.raises(zlib.error):
        with np.load(bad, allow_pickle=False) as data:
            data["ids"]
    stream = DistillStream(tmp_path, DEVICE, max_uses=1)
    assert set(row_ids(stream.begin_update(np.random.default_rng(4), 1, 8, 8))) == set(range(8))
    assert good.exists() and not bad.exists() and (tmp_path / "rejected" / bad.name).exists()
    lines = capsys.readouterr().out.splitlines()
    assert len(lines) == 1 and bad.name in lines[0] and "decompress" in lines[0]
    resumed = DistillStream(tmp_path, DEVICE, max_uses=1, state=stream.state_dict())
    assert not resumed.begin_update(np.random.default_rng(4), 1, 8, 8)
    assert capsys.readouterr().out == ""


@pytest.mark.parametrize("key,change", [
    ("x", lambda x: np.array(1, dtype=np.float32)),
    ("x", lambda x: x[:, 0]),
    ("x", lambda x: x[:, :-1]),
    ("ids", lambda x: x[:, :, :-1]),
    ("field", lambda x: x[:-1]),
    ("mask1", lambda x: x[:, :, :-1]),
    ("pairs", lambda x: x[:, :, 0]),
    ("x", lambda x: x.astype(np.int32)),
    ("ids", lambda x: x.astype(np.float32)),
    ("mask0", lambda x: x.astype(np.int8)),
    ("prior", lambda x: x.astype(object)),
    ("holdout", lambda x: np.array([False, True])),
    ("created", lambda x: np.array([1.0])),
    ("teacher_minutes", lambda x: np.array([20.0])),
    ("teacher", lambda x: np.array(["m020.pt"])),
    ("holdout", lambda x: np.array(0)),
    ("teacher_minutes", lambda x: np.array(20.0, dtype=np.float32)),
    ("x", lambda x: np.zeros_like(x)),
    ("x", lambda x: np.where(x > 0, -x, x)),
    ("x", lambda x: np.full_like(x, np.nan)),
    ("x", lambda x: np.full_like(x, np.inf)),
    ("ids", lambda x: np.full_like(x, 1000000)),
    ("mask0", lambda x: np.zeros_like(x)),
])
def test_invalid_shards_are_rejected_before_training(tmp_path, labels, capsys, key, change):
    bad = write_shard(tmp_path, labels, 1, rows=8)
    rewrite_shard(bad, key, change)
    write_shard(tmp_path, labels, 0, rows=8)
    stream = DistillStream(tmp_path, DEVICE, max_uses=1)
    assert set(row_ids(stream.begin_update(np.random.default_rng(3), 1, 8, 8))) == set(range(8))
    assert not bad.exists() and (tmp_path / "rejected" / bad.name).exists()
    lines = capsys.readouterr().out.splitlines()
    assert len(lines) == 1 and bad.name in lines[0]
    assert not stream.begin_update(np.random.default_rng(3), 1, 8, 8)
    assert capsys.readouterr().out == ""


def test_resident_cap_admits_newest_first_and_leaves_backlog_on_disk(tmp_path, labels, monkeypatch):
    paths = [write_shard(tmp_path, labels, seq, rows=8) for seq in range(6)]
    stream = DistillStream(tmp_path, DEVICE, max_uses=1, max_resident_rows=10)
    reads = []
    original = DistillStream._read

    def read(path, rows, info):
        assert len(rows) + stream.size <= stream.max_resident_rows
        reads.append((path.name, len(rows)))
        return original(path, rows, info)

    monkeypatch.setattr(stream, "_read", read)
    stream.begin_update(np.random.default_rng(4), 2, 0, 5)
    assert stream.size == 10 and stream.metrics["distill_backlog_rows"] == 38
    assert reads == [(paths[5].name, 8), (paths[4].name, 2)]
    assert set(stream.shards) == {paths[5].name, paths[4].name}
    assert all(path.exists() for path in paths) and not (tmp_path / "used").exists()
    reads.clear()
    stream.scan()
    assert not reads and stream.size == 10 and stream.backlog_rows == 38
    seen = set()
    for _ in range(8):
        ids = row_ids(stream.begin_update(np.random.default_rng(4), 1, 10, 10))
        assert seen.isdisjoint(ids)
        seen.update(ids)
        assert stream.size <= 10
        assert len(seen) + stream.size + stream.metrics["distill_backlog_rows"] == 48
    assert seen == {seq * 1000 + row for seq in range(6) for row in range(8)}
    assert stream.backlog_rows == stream.size == 0


@pytest.mark.parametrize("max_uses", [1, 4])
def test_non_mps_cache_stays_on_cpu_and_only_sampled_rows_move_to_device(tmp_path, labels, max_uses):
    write_shard(tmp_path, labels, 0, rows=12)
    stream = DistillStream(tmp_path, torch.device("meta"), max_uses=max_uses, max_resident_rows=10)
    stream.scan()
    assert stream.size == 10 and stream.backlog_rows == 2
    batches = stream.begin_update(np.random.default_rng(6), 2, 1, 3)
    assert all(v.device.type == "meta" and len(v) == 2 for batch in batches.values() for v in batch.values())
    assert stream.size == (8 if max_uses == 1 else 10)
    assert all(v.device.type == "cpu" for shard in stream.shards.values()
               for k, v in shard["data"].items() if k != "_reference")


@pytest.mark.parametrize("archive_before_resume", [False, True])
def test_use_once_resume_with_backlog_and_shards_larger_than_cap(tmp_path, labels, archive_before_resume):
    paths = [write_shard(tmp_path, labels, seq, rows=rows) for seq, rows in enumerate((13, 9, 10))]
    stream, rng, seen = DistillStream(tmp_path, DEVICE, max_uses=1, max_resident_rows=7), np.random.default_rng(9), set()
    seen.update(row_ids(stream.begin_update(rng, 1, 3, 5)))
    assert stream.size == 4 and stream.backlog_rows == 25
    assert all(path.exists() for path in paths)
    for update in range(15):
        trainer.save(tmp_path / "latest.pt", {"distill_stream": stream.state_dict()})
        if archive_before_resume:
            stream.archive_used()
        state = torch.load(tmp_path / "latest.pt", weights_only=False)["distill_stream"]
        # A new cap must preserve the original row masks, including the unadmitted portion of a shard.
        stream = DistillStream(tmp_path, DEVICE, max_uses=1, max_resident_rows=5, state=state)
        ids = row_ids(stream.begin_update(rng, 1, 3, 5))
        assert seen.isdisjoint(ids) and len(ids) == len(set(ids))
        seen.update(ids)
        assert stream.size <= 5 and stream.used_total == len(seen)
        assert len(seen) + stream.size + stream.backlog_rows == 32
    assert seen == {seq * 1000 + row for seq, rows in enumerate((13, 9, 10)) for row in range(rows)}
    assert stream.size == stream.backlog_rows == 0
    trainer.save(tmp_path / "latest.pt", {"distill_stream": stream.state_dict()})
    stream.archive_used()
    assert all((tmp_path / "used" / path.name).exists() and not path.exists() for path in paths)
    shutil.copyfile(tmp_path / "used" / paths[2].name, paths[2])
    resumed = DistillStream(tmp_path, DEVICE, max_uses=1, max_resident_rows=5, state=stream.state_dict())
    assert not resumed.begin_update(rng, 1, 3, 5) and resumed.used_total == 32


def test_partial_reads_support_fortran_order_and_noncontiguous_unused_rows(tmp_path, labels):
    path = write_shard(tmp_path, labels, 0, rows=12)
    for key in OBS_KEYS:
        rewrite_shard(path, key, np.asfortranarray)
    stream, rng, seen = DistillStream(tmp_path, DEVICE, max_uses=1, max_resident_rows=5), np.random.default_rng(2), set()
    for _ in range(6):
        ids = row_ids(stream.begin_update(rng, 1, 2, 3))
        assert seen.isdisjoint(ids)
        seen.update(ids)
        stream = DistillStream(tmp_path, DEVICE, max_uses=1, max_resident_rows=5, state=stream.state_dict())
    assert seen == set(range(12))


def test_round_one_and_metadata_shards_still_load_offline(tmp_path, labels):
    for seq in range(2):
        write_shard(tmp_path, labels, seq, rows=8, holdout=bool(seq))
    offline = DistillData([tmp_path], DEVICE, holdout_every=2)
    assert offline.size == offline.held_size == 8
    (tmp_path / "old").mkdir()
    synthetic_labels(tmp_path / "old", shards=1)
    assert DistillData([tmp_path / "old"], DEVICE).size >= 24


def test_batches_limited_by_ppo_minibatches_and_unknown_teacher_lag(tmp_path, labels):
    write_shard(tmp_path, labels, 0, rows=100, teacher_minutes=np.nan)
    stream = DistillStream(tmp_path, DEVICE, max_uses=1)
    batches = stream.begin_update(np.random.default_rng(2), 8, 4, 2, elapsed_minutes=30)
    assert len(batches) == 2 and stream.metrics["distill_used"] == 16
    assert stream.metrics["distill_teacher_lag_min"] is None
    before = stream.used_total
    assert not stream.begin_update(np.random.default_rng(3), 8, 4, 0)
    assert stream.used_total == before


@pytest.fixture(scope="module")
def rollout():
    model = model_for_test()
    with Actors(4, 16, [POOL], seed=3, workers=1, model=model) as actors:
        actors.set_weights("learner", trainer.cpu_state(model))
        actors.start(0, np.zeros(4, dtype=np.int8), ko_bonus=0)
        collected = actors.wait()
        arrays = {k: v.copy() for k, v in actors.buffers[0].arrays.items()}
    return arrays, collected


def update_args():
    return SimpleNamespace(gamma=0.995, lam=0.95, clip=0.2, value_coef=0.5, aux_coef=0.03,
                           tera_entropy_weight=0.0, max_grad_norm=0.5, epochs=2, minibatch=32,
                           distill_batch=4, distill_max_batches=2)


def test_ppo_stream_reuse_tracks_trainer_updates_across_resume(tmp_path, labels):
    steps, rows = 2, 8
    arrays = {k: v[:steps * rows].reshape(steps, rows, *v.shape[1:]).copy() for k, v in labels.items() if k in OBS_KEYS}
    arrays.update(a0=labels["pairs"][:steps * rows, 0, 0].reshape(steps, rows).astype(np.int64),
                  a1=labels["pairs"][:steps * rows, 0, 1].reshape(steps, rows).astype(np.int64),
                  mask0=labels["mask0"][:steps * rows].reshape(steps, rows, -1).copy(),
                  mask1=labels["mask1"][:steps * rows, 0].reshape(steps, rows, -1).copy(),
                  foe_match=np.full((steps, rows, 6), -1, np.int64),
                  reward=np.random.default_rng(0).normal(size=(steps, rows)).astype(np.float32),
                  value=np.zeros((steps, rows), np.float32), logp=np.zeros((steps, rows), np.float32),
                  done=np.ones((steps, rows), bool), train=np.ones((steps, rows), bool), bootstrap=np.zeros(rows, np.float32))
    write_shard(tmp_path, labels, 0, rows=8)
    model, args, rng = model_for_test(), update_args(), np.random.default_rng(5)
    args.minibatch = 8
    optimizer = Adam(model.parameters(), 3e-4)
    stream = DistillStream(tmp_path, DEVICE, max_uses=2, reuse_gap=3)
    stats, samples = trainer.ppo_update(model, optimizer, arrays, args, rng, 0.01, DEVICE,
                                       distill=stream, distill_coef=0.5, elapsed=30 * 60, update=100)
    assert samples == 16 and stats["distill_used"] == stats["distill_used_fresh"] == 8
    assert stats["distill_batches"] == 2 and np.isfinite(stats["distill_loss"])
    trainer.save(tmp_path / "latest.pt", {"distill_stream": stream.state_dict()})
    loaded = torch.load(tmp_path / "latest.pt", weights_only=False)["distill_stream"]
    stream = DistillStream(tmp_path, DEVICE, state=loaded, max_uses=2, reuse_gap=3)
    for update, expected in ((101, 0), (102, 0), (103, 8), (103, 0), (200, 0)):
        stats, _ = trainer.ppo_update(model, optimizer, arrays, args, rng, 0.01, DEVICE,
                                     distill=stream, distill_coef=0.5, elapsed=30 * 60, update=update)
        assert stats["distill_used"] == expected and stats["distill_used_fresh"] == 0
        assert (stats["distill_loss"] is None) == (expected == 0)
    assert stream.used_total == 16 and stream.rows_spent_total == 8 and stream.size == 0


def test_starvation_is_exactly_plain_ppo_then_live_shard_is_consumed_once(tmp_path, labels, rollout, monkeypatch):
    arrays, _ = rollout
    plain, streamed = model_for_test(), model_for_test()
    args = update_args()
    stream = DistillStream(tmp_path, DEVICE, max_uses=1)
    scan_calls = []
    original_scan = stream.scan
    monkeypatch.setattr(stream, "scan", lambda: (scan_calls.append(1), original_scan())[1])
    plain_stats, plain_samples = trainer.ppo_update(plain, Adam(plain.parameters(), 3e-4), arrays, args,
                                                    np.random.default_rng(5), 0.01, DEVICE)
    optimizer = Adam(streamed.parameters(), 3e-4)
    stats, samples = trainer.ppo_update(streamed, optimizer, arrays, args, np.random.default_rng(5),
                                       0.01, DEVICE, distill=stream, distill_coef=0.5)
    assert len(scan_calls) == 1 and samples == plain_samples
    assert all(stats[k] == v for k, v in plain_stats.items())
    assert stats["distill_used"] == stats["distill_batches"] == 0
    assert stats["distill_loss"] is stats["distill_agree"] is None
    for key in plain.state_dict():
        torch.testing.assert_close(plain.state_dict()[key], streamed.state_dict()[key], rtol=0, atol=0)
    write_shard(tmp_path, labels, 0, rows=24)
    used, positions, step = [], [], [0]
    original_loss, original_ppo = trainer.distill_loss, trainer.ppo_loss

    def distill_loss(model, batch):
        used.extend(int(i) for i in batch["field"][:, 0].tolist())
        positions.append(step[0] - 1)
        return original_loss(model, batch)

    def ppo_loss(*args, **kwargs):
        step[0] += 1
        return original_ppo(*args, **kwargs)

    monkeypatch.setattr(trainer, "distill_loss", distill_loss)
    monkeypatch.setattr(trainer, "ppo_loss", ppo_loss)
    for _ in range(6):
        stats, _ = trainer.ppo_update(streamed, optimizer, arrays, args, np.random.default_rng(6),
                                      0.01, DEVICE, distill=stream, distill_coef=0.5, elapsed=30 * 60)
    n_minibatches = args.epochs * sum(1 for _ in trainer.minibatches(np.arange(plain_samples), args.minibatch))
    assert positions[:2] == [0, n_minibatches - 1]
    assert len(scan_calls) == 7 and len(used) == len(set(used)) == 24
    assert stream.used_total == 24 and stats["distill_used"] == 0


@pytest.mark.parametrize("minutes,schedule", [(60.0, None), (math.inf, 120.0), (math.inf, None), (10.0, 120.0)])
def test_schedule_clamps_and_inf_budget(minutes, schedule):
    argv = ["--minutes", str(minutes)]
    if schedule is not None:
        argv += ["--schedule-minutes", str(schedule)]
    args = trainer.parse_args(argv)
    duration = minutes if schedule is None else schedule
    assert args.schedule_minutes == duration
    assert trainer.schedule_progress(args, 0) == 0
    if math.isinf(duration):
        assert trainer.schedule_progress(args, 1e12) == 0
    else:
        assert trainer.schedule_progress(args, duration * 30) == 0.5
        assert trainer.schedule_progress(args, duration * 60) == 1
        assert trainer.schedule_progress(args, duration * 600) == 1
    args = trainer.parse_args(["--minutes", "inf", "--schedule-minutes", "120", "--distill-stream", "/tmp/labels",
                               "--distill-coef", "0.5", "--distill-end-minutes", "inf"])
    progress = trainer.schedule_progress(args, 125 * 60)
    assert progress == 1
    assert args.lr * (1 - (1 - args.lr_final) * progress) == pytest.approx(args.lr * args.lr_final)
    assert EntropyController(0.01, 2.5, 1.2).target(progress) == pytest.approx(1.2)
    assert args.ko_bonus * max(0, 1 - progress / args.ko_bonus_end) == 0
    assert trainer.distill_coefficient(args, 1e12) == 0.5


@pytest.mark.parametrize("argv", [["--minutes", "nan"], ["--minutes", "0"], ["--schedule-minutes", "0"],
                                  ["--schedule-minutes", "nan"], ["--stop-minutes", "-1"],
                                  ["--distill", "a", "--distill-stream", "b"], ["--distill-batch", "0"],
                                  ["--distill-max-uses", "0"], ["--distill-max-uses", "-1"],
                                  ["--distill-reuse-gap", "-1"], ["--distill-max-age-minutes", "-1"],
                                  ["--distill-max-age-minutes", "nan"],
                                  ["--distill-max-resident-rows", "0"], ["--distill-max-resident-rows", "-1"],
                                  ["--distill-stream", "a", "--distill-max-resident-rows", "127"]])
def test_invalid_schedule_and_stream_arguments(argv):
    with pytest.raises(SystemExit):
        trainer.parse_args(argv)


def test_stream_flag_defaults_and_overrides():
    defaults = trainer.parse_args([])
    assert defaults.distill_max_resident_rows == 60000 and defaults.distill_max_batches == 4
    assert defaults.distill_max_uses == 4 and defaults.distill_reuse_gap == 50 and defaults.distill_max_age_minutes == 60
    args = trainer.parse_args(["--distill-stream", "a", "--distill-max-resident-rows", "256", "--distill-max-uses", "1",
                               "--distill-reuse-gap", "0", "--distill-max-age-minutes", "inf"])
    assert args.distill_max_resident_rows == 256
    assert args.distill_max_uses == 1 and args.distill_reuse_gap == 0 and args.distill_max_age_minutes == math.inf


def test_main_stop_checkpoint_archive_and_resume_new_output(tmp_path, labels, rollout, monkeypatch):
    arrays, collected = rollout
    directory = tmp_path / "stream"
    shard = write_shard(directory, labels, 0, rows=8)
    used, validations = [], []
    original_update, original_archive = trainer.ppo_update, DistillStream.archive_used
    first, second = tmp_path / "first", tmp_path / "second"

    class FakeActors:
        def __init__(self, *args, **kwargs):
            self.buffers = [SimpleNamespace(arrays=arrays), SimpleNamespace(arrays=arrays)]

        def __enter__(self):
            return self

        def __exit__(self, *args):
            pass

        def set_weights(self, *args):
            pass

        def start(self, *args, **kwargs):
            pass

        def wait(self):
            return collected

    def update(*positional, **kwargs):
        result = original_update(*positional, **kwargs)
        args = positional[3]
        (args.output / "STOP").touch()
        return result

    def archive(stream):
        output = second if second.exists() else first
        state = torch.load(output / "latest.pt", weights_only=False)
        assert state["distill_stream"]["used_total"] == stream.used_total
        original_archive(stream)

    original_loss = trainer.distill_loss

    def record(model, batch):
        used.extend(int(i) for i in batch["field"][:, 0].tolist())
        return original_loss(model, batch)

    monkeypatch.setattr(trainer, "Actors", FakeActors)
    monkeypatch.setattr(trainer, "ppo_update", update)
    monkeypatch.setattr(trainer, "distill_loss", record)
    monkeypatch.setattr(trainer, "validate", lambda *args: (validations.append(1), {})[1])
    monkeypatch.setattr(DistillStream, "archive_used", archive)
    argv = ["--minutes", "inf", "--schedule-minutes", "120", "--width", "32", "--layers", "1", "--envs", "4",
            "--workers", "1", "--steps", "16", "--epochs", "1", "--minibatch", "32", "--device", "cpu",
            "--distill-stream", str(directory), "--distill-coef", "0.5", "--distill-end-minutes", "inf",
            "--distill-batch", "4", "--distill-max-batches", "1", "--distill-validate-every", "1",
            "--distill-max-uses", "1", "--distill-max-age-minutes", "inf"]
    monkeypatch.setattr("sys.argv", ["train_selfplay.py", *argv, "--output", str(first)])
    trainer.main()
    checkpoint = torch.load(first / "latest.pt", weights_only=False)
    assert checkpoint["update"] == 1 and checkpoint["distill_stream"]["used_total"] == 4
    assert shard.exists() and not (directory / "used").exists()
    config = json.loads((first / "config.json").read_text())
    assert config["minutes"] == "inf" and config["schedule_minutes"] == "120.0"
    # Reproduce a resumed run already beyond its schedule, without changing the first run's checkpoint.
    checkpoint["elapsed"] = 125 * 60
    trainer.save(tmp_path / "resume.pt", checkpoint)
    monkeypatch.setattr("sys.argv", ["train_selfplay.py", *argv, "--output", str(second),
                                     "--resume", str(tmp_path / "resume.pt")])
    trainer.main()
    resumed = torch.load(second / "latest.pt", weights_only=False)
    metrics = json.loads((second / "metrics.jsonl").read_text())
    assert resumed["update"] == 2 and resumed["distill_stream"]["used_total"] == 8
    assert metrics["progress"] == 1 and metrics["lr"] == pytest.approx(9e-5)
    assert metrics["entropy_target"] == 1.5 and metrics["ko_bonus"] == 0
    assert metrics["distill_used"] == 4 and len(used) == len(set(used)) == 8
    assert len(validations) == 2
    assert not shard.exists() and (directory / "used" / shard.name).exists()
    assert torch.load(first / "latest.pt", weights_only=False)["update"] == 1


def test_normal_end_saves_latest_with_inf_snapshot_and_pool(tmp_path, rollout, monkeypatch):
    class FakeActors:
        def __init__(self, *args, **kwargs):
            pass

        def __enter__(self):
            return self

        def __exit__(self, *args):
            pass

        def set_weights(self, *args):
            pass

        def start(self, *args, **kwargs):
            pass

        def wait(self):
            return rollout[1]

    monkeypatch.setattr(trainer, "Actors", FakeActors)
    monkeypatch.setattr("sys.argv", ["train_selfplay.py", "--minutes", "inf", "--stop-minutes", "0",
                                     "--snapshot-minutes", "inf", "--pool-minutes", "inf", "--width", "32",
                                     "--layers", "1", "--device", "cpu", "--output", str(tmp_path)])
    trainer.main()
    assert torch.load(tmp_path / "latest.pt", weights_only=False)["update"] == 0


def test_lower_max_uses_on_resume_never_exceeds_the_new_limit(tmp_path, labels):
    write_shard(tmp_path, labels, 0, rows=8, teacher_minutes=0)
    stream = DistillStream(tmp_path, DEVICE, max_uses=4, reuse_gap=1, max_age_minutes=1e9)
    rng = np.random.default_rng(0)
    for update in range(2):
        stream.begin_update(rng, 8, 1, 1, update=update)
    resumed = DistillStream(tmp_path, DEVICE, max_uses=2, reuse_gap=1, max_age_minutes=1e9, state=stream.state_dict())
    assert not resumed.begin_update(rng, 8, 1, 1, update=5)
