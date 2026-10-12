"""Use-once stream consumption, checkpoint/archive ordering, live delivery, validation and infinite schedules."""
import json
import math
import shutil
from pathlib import Path
from types import SimpleNamespace

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


def test_use_once_many_updates_and_live_delivery(tmp_path, labels):
    paths = [write_shard(tmp_path, labels, seq, rows=37) for seq in range(3)]
    stream, rng, seen = DistillStream(tmp_path, DEVICE), np.random.default_rng(7), set()
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
        assert stream.size == stream.metrics["distill_fresh"] - len(ids)
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
    stream, rng = DistillStream(tmp_path, DEVICE), np.random.default_rng(9)
    seen = set(row_ids(stream.begin_update(rng, 4, 2, 2)))
    partial = write_shard(tmp_path, labels, 1, rows=31)
    seen.update(row_ids(stream.begin_update(rng, 4, 2, 3)))
    state = stream.state_dict()
    assert first.name in state["completed"] and state["used_rows"][partial.name].sum() == 8
    trainer.save(tmp_path / "latest.pt", {"distill_stream": state, "numpy_rng": rng.bit_generator.state})
    if archive_before_resume:
        stream.archive_used()
    loaded = torch.load(tmp_path / "latest.pt", weights_only=False)
    resumed = DistillStream(tmp_path, DEVICE, state=loaded["distill_stream"])
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
    stream, rng = DistillStream(tmp_path, DEVICE), np.random.default_rng(2)
    committed = set(row_ids(stream.begin_update(rng, 4, 1, 3)))
    state = stream.state_dict()
    row_ids(stream.begin_update(rng, 4, 1, 3))
    assert sum(mask.sum() for mask in state["used_rows"].values()) == 4
    restored = DistillStream(tmp_path, DEVICE, state=state)
    ids = row_ids(restored.begin_update(rng, 4, 4, 5))
    assert committed.isdisjoint(ids) and restored.size == 12


def test_newest_holdout_rows_never_trained_and_reload_newest_first(tmp_path, labels):
    write_shard(tmp_path, labels, 0, rows=16)
    old = write_shard(tmp_path, labels, 19, rows=12, holdout=True)
    new = write_shard(tmp_path, labels, 39, rows=12, holdout=True)
    stream = DistillStream(tmp_path, DEVICE, holdout_rows=18)
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
        patch.setattr(DistillStream, "_read", staticmethod(lambda p: (reads.append(p.name), original(p))[1]))
        resumed = DistillStream(tmp_path, DEVICE, holdout_rows=18, state=stream.state_dict())
    assert reads == [new.name, old.name]
    assert set(resumed.held["field"][:, 0].tolist()) == expected
    newest = write_shard(tmp_path, labels, 59, rows=25, holdout=True)
    assert not resumed.begin_update(np.random.default_rng(4), 4, 4, 5)
    assert set(resumed.held["field"][:, 0].tolist()) == set(range(59007, 59025))
    assert (tmp_path / "heldout" / newest.name).exists()


def test_ignores_temporary_and_partial_shards_and_retries_final_name(tmp_path, labels):
    directory = tmp_path / "arriving"
    stream = DistillStream(directory, DEVICE)
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
    final.write_bytes(contents)
    assert set(row_ids(stream.begin_update(rng, 4, 4, 5))) == set(range(8))


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
    stream = DistillStream(tmp_path, DEVICE)
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


def test_starvation_is_exactly_plain_ppo_then_live_shard_is_consumed_once(tmp_path, labels, rollout, monkeypatch):
    arrays, _ = rollout
    plain, streamed = model_for_test(), model_for_test()
    args = update_args()
    stream = DistillStream(tmp_path, DEVICE)
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
                                  ["--distill", "a", "--distill-stream", "b"], ["--distill-batch", "0"]])
def test_invalid_schedule_and_stream_arguments(argv):
    with pytest.raises(SystemExit):
        trainer.parse_args(argv)


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
            "--distill-batch", "4", "--distill-max-batches", "1", "--distill-validate-every", "1"]
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
