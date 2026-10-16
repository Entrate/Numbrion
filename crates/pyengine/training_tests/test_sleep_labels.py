"""Sleep labels: training.sleep's effective-target rules on directed battles, sleep-forced decisions in the shallow
search (rate, targets, partner slot, labels), the actors' flags and precedence over --explore, the unchanged default,
label sets in distill_train.py (parsing, weighted sampling, per-set value mix, --init, caps), and a sleep-probe smoke
test."""
import json
import sys
from dataclasses import asdict
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import numpy as np
import pytest
import torch

import distill_train as trainer
import shallow_labels
from test_distill import synthetic_labels
from test_protocol import FILLER, directed_battle, mon
from test_value_labels import add_value_fields
from training.dex import load_dex
from training.distill import DistillStream, label_losses, value_target
from training.encoder import PF, Encoder
from training.guards import MoveGuards
from training.model import Model
from training.search_opponent import OpponentSearch, SearchOpponent, load_bench_sets
from training.sleep import SleepCounter, SleepMoves

REPO = Path(__file__).resolve().parents[3]
POOL = REPO / "data" / "teams" / "train-s42-2000.txt"
OBS_KEYS = ("ids", "floats", "field", "action_features", "token_features")

BRELOOM = mon("Breloom", ["spore", "machpunch", "protect", "bulletseed"], "Technician")
HYPNO = mon("Hypno", ["hypnosis", "psychic", "protect", "yawn"], "Insomnia")
SNORLAX = mon("Snorlax", ["bodyslam", "protect", "curse", "rest"], "Thick Fat")
AMOONGUSS = mon("Amoonguss", ["spore", "protect", "gigadrain", "clearsmog"], "Regenerator", "F")
PINCURCHIN = mon("Pincurchin", ["protect", "spikes", "thunderbolt", "suckerpunch"], "Electric Surge")
SKARMORY = mon("Skarmory", ["bravebird", "protect", "roost", "spikes"], "Sturdy")
FORRETRESS = mon("Forretress", ["protect", "spikes", "bodypress", "substitute"], "Overcoat")
INSOMNIAC = mon("Hypno", ["hypnosis", "psychic", "protect", "yawn"], "Insomnia", "F")


def encode(battle):
    encoder = Encoder(2)
    b = encoder.encode_batch([dict(battle_id=1, request=battle.request_json(s), log=battle.drain_player_log(s))
                              for s in (0, 1)])
    return encoder, b


def options(battle, side=0):
    _, b = encode(battle)
    return SleepMoves(load_dex()).options(b.arrays(), side, battle.legal_mask(side))


# Codes: move slot * 10 + target * 2; target 3 / 4 = foe slot a / b. Breloom (slot a): Spore is move 0, so 6 / 8.
# Hypno (slot b): Hypnosis move 0 (6 / 8), Yawn move 3 (36 / 38).
@pytest.mark.parametrize("case, foes, turn, expected", [
    ("open", [SNORLAX, SKARMORY], None, [[6, 8], [6, 8, 36, 38]]),
    ("grass vs spore", [SNORLAX, AMOONGUSS], None, [[6], [6, 8, 36, 38]]),
    ("overcoat vs spore", [SNORLAX, FORRETRESS], None, [[6], [6, 8, 36, 38]]),
    ("insomnia", [INSOMNIAC, SKARMORY], None, [[8], [8, 38]]),
    ("electric terrain: only the Flying target", [PINCURCHIN, SKARMORY], None, [[8], [8, 38]]),
    # Spore puts Snorlax to sleep: it is statused, and Sleep Clause protects Skarmory.
    ("asleep + sleep clause", [SNORLAX, SKARMORY], ("move 1 +1, move 3", "move 3, move 3"), [[], []]),
    # Yawn makes Snorlax drowsy: no second Yawn on it, but it can still be put to sleep now.
    ("drowsy", [SNORLAX, SKARMORY], ("move 3, move 4 +1", "move 3, move 3"), [[6, 8], [6, 8, 38]]),
    # Forretress behind a Substitute.
    ("substitute", [SNORLAX, FORRETRESS], ("move 3, move 3", "move 3, move 4"), [[6], [6, 36]]),
])
def test_eligible_sleep_codes_skip_ineffective_targets(case, foes, turn, expected):
    battle = directed_battle([BRELOOM, HYPNO] + FILLER, foes + FILLER)
    if turn is not None:
        battle.choose(0, turn[0])
        battle.choose(1, turn[1])
    assert options(battle) == expected, case


def test_sleep_counter_counts_onsets_once_across_reorders_and_separates_rest():
    counter = SleepCounter(1)
    ids = np.zeros((1, 12, 30), dtype=np.int64)
    floats = np.zeros((1, 12, len(PF)), dtype=np.float32)
    ids[0, :6, 0] = [11, 12, 13, 14, 15, 16]
    floats[0, :6, PF["hp"]] = 0.5
    none = np.zeros(1, dtype=bool)
    assert [int(x[0]) for x in counter.update(floats, ids, none)] == [0, 0]
    floats[0, 1, PF["slp"]] = 1  # species 12 is put to sleep
    assert [int(x[0]) for x in counter.update(floats, ids, none)] == [1, 0]
    ids[0, :2, 0] = [12, 11]  # a switch reorders the request: the sleeper moves token, no new onset
    floats[0, :2, PF["slp"]] = [1, 0]
    assert [int(x[0]) for x in counter.update(floats, ids, none)] == [0, 0]
    floats[0, 3, PF["slp"]], floats[0, 3, PF["hp"]] = 1, 1.0  # species 14 rests: asleep at full HP from 0.5
    assert [int(x[0]) for x in counter.update(floats, ids, none)] == [0, 1]
    floats[0, 4, PF["slp"]] = 1  # a new battle's sleeper is not counted on the reset boundary
    assert [int(x[0]) for x in counter.update(floats, ids, np.ones(1, dtype=bool))] == [0, 0]


class Capture:
    def __init__(self):
        self.rows, self.written = [], 0

    def add(self, row, minutes, teacher="selfplay-search"):
        self.rows.append(row)

    def flush(self):
        self.written = len(self.rows)


def search_once(battle, searcher, model):
    encoder, b = encode(battle)
    with torch.no_grad():
        state = model.trunk(*(torch.from_numpy(getattr(b, k)) for k in OBS_KEYS))
    picks = searcher.act(model, SimpleNamespace(clone_battle=lambda e: battle.clone()), encoder, state,
                         {**b.arrays(), "guard": b.guard}, np.array([0]),
                         np.array([[battle.needs_action(0), battle.needs_action(1)]]), 1.0, side=0)
    return picks[0], searcher.pending.pop(0)[0][0], b


def test_sleep_forced_decisions_rate_targets_partner_and_labels():
    torch.manual_seed(5)
    dex = load_dex()
    model = Model(dex, 32, 1).eval()
    battle = directed_battle([BRELOOM, HYPNO] + FILLER, [SNORLAX, AMOONGUSS] + FILLER)
    eligible = options(battle)
    mask0, joint = battle.legal_mask(0)
    bench = load_bench_sets([POOL])
    make = lambda p: SearchOpponent(str(dex.path), MoveGuards(), OpponentSearch(iterations=20), seed=3,
                                    labels=Capture(), bench_sets=bench, sleep_explore=p)
    forcing, plain = make(0.5), make(0.0)
    forced = 0
    for _ in range(160):
        pair, row, _ = search_once(battle, forcing, model)
        reference_pair, reference, _ = search_once(battle, plain, model)
        assert "sleep_forced" in row and "sleep_forced" not in reference
        for key in ("pairs", "x", "q", "prior", "value", "mask0", "mask1", "ids", "floats"):
            np.testing.assert_array_equal(row[key], reference[key])  # the label is the search's, unchanged
        if not row["sleep_forced"]:
            assert pair == reference_pair
            continue
        forced += 1
        slot = 0 if pair[0] in eligible[0] else 1
        assert pair[slot] in eligible[slot]  # an effective target, never Spore on the Grass-type Amoonguss
        assert joint[pair[0], pair[1]]
        # The partner: its part of the most weighted searched pair that is jointly legal with the sleep code.
        pairs, x = row["pairs"], row["x"]
        compatible = [k for k in np.argsort(-x, kind="stable") if pairs[k][0] >= 0 and
                      (joint[pair[0], pairs[k][1]] if slot == 0 else joint[pairs[k][0], pair[1]])]
        if compatible:
            assert pair[1 - slot] == pairs[compatible[0]][1 - slot]
    assert forcing.stats["sleep_eligible"] == 160 and forcing.stats["sleep_forced"] == forced
    assert 0.35 < forced / 160 < 0.65
    assert plain.stats["sleep_forced"] == 0


def test_no_eligible_target_means_no_forcing():
    torch.manual_seed(5)
    dex = load_dex()
    battle = directed_battle([BRELOOM, HYPNO] + FILLER, [SNORLAX, SKARMORY] + FILLER)
    battle.choose(0, "move 1 +1, move 3")  # Spore on Snorlax: Sleep Clause from now on
    battle.choose(1, "move 3, move 3")
    searcher = SearchOpponent(str(dex.path), MoveGuards(), OpponentSearch(iterations=20), seed=3, labels=Capture(),
                              bench_sets=load_bench_sets([POOL]), sleep_explore=1.0)
    for _ in range(10):
        _, row, _ = search_once(battle, searcher, Model(dex, 32, 1).eval())
        assert not row["sleep_forced"]
    assert searcher.stats["sleep_eligible"] == searcher.stats["sleep_forced"] == 0


def run_actors(out, explore=0.0, sleep=0.0, sleep_moves=None, envs=8, steps=40, label_only=True, seed=3):
    from training.actors import Actors
    from training.league import SEARCH

    torch.manual_seed(8)
    model = Model(load_dex(), 32, 1).eval()
    with Actors(envs, steps, [POOL], seed, 1, model, search=asdict(OpponentSearch(iterations=20)), search_labels=out,
                search_both=True, search_explore=explore, search_fair_labels=True, label_only=label_only,
                search_sleep_explore=sleep, search_sleep_moves=sleep_moves) as actors:
        actors.set_weights("learner", model.state_dict())
        actors.set_weights("opponent", model.state_dict())
        actors.start(0, np.full(envs, SEARCH), 0.0)
        stats = actors.wait()
        arrays = None if label_only else {k: v.copy() for k, v in actors.buffers[0].arrays.items()}
    shards = [DistillStream._read(p) for p in sorted(out.glob("labels-*.npz"))]
    return shards, stats, arrays


def test_sleep_explore_zero_or_without_matching_moves_changes_nothing(tmp_path):
    """P = 0 writes no new fields; forcing machinery that never fires leaves every row and stat unchanged."""
    base, base_stats, _ = run_actors(tmp_path / "zero", steps=16)
    idle, idle_stats, _ = run_actors(tmp_path / "idle", sleep=1.0, sleep_moves=["notamove"], steps=16)
    assert "sleep_forced" not in base[0] and "sleep_explore" not in base[0]
    assert all(float(s["sleep_explore"]) == 1.0 for s in idle) and not any(s["sleep_forced"].any() for s in idle)
    rows = lambda shards, key: np.concatenate([s[key] for s in shards])
    for key in base[0]:
        if base[0][key].ndim:
            np.testing.assert_array_equal(rows(base, key), rows(idle, key))
    for key in ("search_decisions", "search_explored", "search_rejected_labels", "completed"):
        assert base_stats[key] == idle_stats[key]
    assert idle_stats["search_sleep_eligible"] == idle_stats["search_sleep_forced"] == 0
    assert shallow_labels.parse_args(["--checkpoint", "x.pt", "--out", "o"]).sleep_explore == 0


def test_actors_force_sleep_over_exploration_and_flag_the_rows(tmp_path):
    """With --explore 1 and --sleep-explore 1, every eligible decision plays a sleep code (not the learner's
    sample), its row says sleep_forced, and the shards carry sleep_explore."""
    shards, stats, arrays = run_actors(tmp_path, explore=1.0, sleep=1.0, label_only=False, envs=12, steps=48, seed=4)
    assert stats["search_sleep_forced"] == stats["search_sleep_eligible"] > 0
    assert all(float(s["sleep_explore"]) == 1.0 for s in shards)
    flags = np.concatenate([s["sleep_forced"] for s in shards])
    assert flags.dtype == bool and 0 < flags.sum() <= stats["search_sleep_forced"]
    sleep = SleepMoves(load_dex())
    ids = np.concatenate([s["ids"] for s in shards])
    floats = np.concatenate([s["floats"] for s in shards])
    checked = 0
    for k in np.flatnonzero(flags):
        match = np.argwhere((arrays["ids"] == ids[k]).all((2, 3)) & (arrays["floats"] == floats[k]).all((2, 3)))
        assert len(match) == 1
        t, r = match[0]
        a0, a1 = int(arrays["a0"][t, r]), int(arrays["a1"][t, r])
        # One slot plays a sleep move (code < 40: move slot code // 10 of that slot's own active token).
        played = [slot for slot, code in ((0, a0), (1, a1))
                  if code < 40 and sleep.kind[ids[k, slot, 6 + code // 10]] >= 0 and (code % 10) // 2 in (2, 3, 4)]
        assert played
        assert abs(float(arrays["logp"][t, r])) < 1e-6  # the forced pair is one-hot, not a policy sample
        checked += 1
    assert checked == flags.sum()
    # Every acting search row explores (explore = 1) except the sleep-forced ones.
    assert stats["search_explored"] == stats["search_decisions"] + stats["search_forced"] - stats["search_sleep_forced"]


# ---- distill_train label sets -------------------------------------------------------------------------------------
def test_label_spec_parsing(tmp_path):
    existing = tmp_path / "odd:1"
    existing.mkdir()
    assert trainer.parse_label_spec(str(existing)) == (existing, [])
    assert trainer.parse_label_spec("a/b") == (Path("a/b"), [])
    assert trainer.parse_label_spec("a/b:2") == (Path("a/b"), ["2"])
    assert trainer.parse_label_spec("a/b::0.8") == (Path("a/b"), ["", "0.8"])
    assert trainer.parse_label_spec("a/b:1:0.8:4M") == (Path("a/b"), ["1", "0.8", "4M"])
    assert trainer.parse_label_spec("C:\\labels:3") == (Path("C:\\labels"), ["3"])
    sets, legacy = trainer.label_sets(["x", "y"], 0.5)
    assert legacy and len(sets) == 1 and sets[0].paths == [Path("x"), Path("y")]
    sets, legacy = trainer.label_sets(["sleep:1:0.8:500k", "gen1/:2"], 0.5)
    assert not legacy
    assert [(s.name, s.weight, s.value_mix, s.max_rows) for s in sets] == [("sleep", 1.0, 0.8, 500_000),
                                                                          ("gen1", 2.0, 0.5, None)]
    with pytest.raises(SystemExit):
        trainer.parse_args(["train", "--labels", "a:1:1.5", "--output", "o"])


def value_dirs(tmp_path, names=("a", "b"), shards=(3, 3)):
    paths = []
    for name, count, seed in zip(names, shards, (5, 6)):
        directory = tmp_path / name
        directory.mkdir()
        synthetic_labels(directory, shards=count, seed=seed)
        for i, path in enumerate(sorted(directory.glob("*.npz"))):
            raw = add_value_fields(DistillStream._read(path))
            raw["holdout"] = np.asarray(i == count - 1)  # the writer's metadata: the last shard is held out
            raw.update(teacher=np.asarray("t"), teacher_minutes=np.asarray(0.0), created=np.asarray(1.0))
            np.savez_compressed(path, **raw)
        paths.append(directory)
    return paths


def test_mixed_sets_sample_by_weight_with_their_value_mix(tmp_path):
    a, b = value_dirs(tmp_path)
    sets, legacy = trainer.label_sets([f"{a}:1:0.8", f"{b}:3:0.2"], 0.5)
    for s in sets:
        s.train, s.held = trainer.split_shards(s.paths)
    stream = trainer.MixedEpochs(sets, 4, 10_000, 16)
    rng = np.random.default_rng(0)
    mixes = []
    for _ in range(6):
        batches = list(stream.batches(rng, 32, torch.device("cpu")))
        assert sum(len(x["weight"]) for x in batches) == stream.rows
        mixes.append(torch.cat([x["value_mix"] for x in batches]).numpy())
    mixes = np.concatenate(mixes)
    np.testing.assert_array_equal(np.unique(mixes), np.float32([0.2, 0.8]))
    assert abs((mixes == np.float32(0.8)).mean() - 0.25) < 0.05
    assert abs(stream.drawn[0] / stream.drawn.sum() - 0.25) < 0.05
    # Rows of set a keep a's value mix in the value target.
    batch = next(iter(stream.batches(rng, 64, torch.device("cpu"))))
    expected = batch["value_mix"] * batch["outcome"] * .995 ** batch["remaining_steps"] + \
        (1 - batch["value_mix"]) * batch["search_value"]
    torch.testing.assert_close(value_target(batch, 0.5), expected)
    model = Model(load_dex(), 32, 1)
    policy, value, aux, _ = label_losses(model, batch, 0.5)
    assert torch.isfinite(policy + value + aux)


def test_cap_rows_keeps_a_seeded_subset_of_whole_shards(tmp_path):
    (a,) = value_dirs(tmp_path, names=("a",), shards=(6,))
    train, held = trainer.split_shards([a])
    size = train[0][1]["size"]
    kept, kept_held = trainer.cap_rows(train, held, size + 1, seed=1)
    assert len(kept) == 2 and len(kept_held) == 1 and set(p for p, _ in kept) <= set(p for p, _ in train)
    assert trainer.cap_rows(train, held, None, 1) == (train, held)
    assert trainer.cap_rows(train, held, size + 1, seed=1) == (kept, kept_held)


def test_train_label_sets_from_init_with_per_set_holdouts_and_resume(tmp_path):
    a, b = value_dirs(tmp_path)
    dex = load_dex()
    torch.manual_seed(1)
    init = Model(dex, 32, 1)
    path = tmp_path / "init.pt"
    torch.save(dict(model=init.state_dict(), width=32, layers=1), path)
    output = tmp_path / "student"
    args = trainer.parse_args(["train", "--labels", f"{a}:1:0.8", f"{b}:2", "--init", str(path), "--output",
                               str(output), "--device", "cpu", "--epochs", "1", "--batch", "16", "--lr", "1e-9"])
    result = trainer.train(args)
    assert set(result["heldout"]) == {"a", "b"} and all(m["value_rows"] > 0 for m in result["heldout"].values())
    state = torch.load(output / "latest.pt", weights_only=False)
    assert state["width"] == 32 and len(state["distill_training"]["sets"]) == 2
    rows = sum(i["size"] for s in state["distill_training"]["sets"] for i in
               [DistillStream._inspect(Path(p)) for p in s["train"]])
    assert state["optimizer"]["step"] == -(-rows // 16)  # a fresh optimizer: this run's updates only
    for key, value in init.state_dict().items():  # fine-tuned from the init weights (lr ~ 0)
        torch.testing.assert_close(state["model"][key], value, atol=1e-5, rtol=0)
    config = json.loads((output / "config.json").read_text())
    assert [s["value_mix"] for s in config["sets"]] == [0.8, 0.5]
    resume = trainer.parse_args(["train", "--resume", str(output / "latest.pt"), "--output", str(output),
                                 "--device", "cpu", "--epochs", "2", "--batch", "16", "--lr", "1e-9"])
    again = trainer.train(resume)
    assert again["epoch"] == 2 and again["rows_seen"] == 2 * result["rows_seen"] and set(again["heldout"]) == {"a", "b"}


def test_shards_with_sleep_fields_load_everywhere(tmp_path):
    (a,) = value_dirs(tmp_path, names=("a",), shards=(1,))
    path = next(a.glob("*.npz"))
    raw = DistillStream._read(path)
    raw["sleep_forced"] = np.arange(len(raw["x"])) % 2 == 0
    raw["sleep_explore"] = np.asarray(0.5, np.float64)
    np.savez_compressed(path, **raw)
    info = DistillStream._inspect(path)
    assert float(info["metadata"]["sleep_explore"]) == 0.5
    assert DistillStream._read(path)["sleep_forced"].sum() == (len(raw["x"]) + 1) // 2
    assert len(trainer.compact(path, info)["weight"]) == len(raw["x"])
    raw["sleep_forced"] = raw["sleep_forced"].astype(np.float32)
    np.savez_compressed(path, **raw)
    with pytest.raises(ValueError, match="sleep_forced"):
        DistillStream._inspect(path)


# ---- probe ------------------------------------------------------------------------------------------------------
def test_sleep_probe_smoke(tmp_path):
    sys.path.insert(0, str(REPO / "tools"))
    import sleep_probe

    torch.manual_seed(2)
    path = tmp_path / "tiny.pt"
    torch.save(dict(model=Model(load_dex(), 32, 1).state_dict(), width=32, layers=1), path)
    summary = sleep_probe.main(["--checkpoint", str(path), "--heads", str(path), "--positions", "2", "--playouts",
                                "2", "--max-games", "60", "--max-turns", "60", "--out", str(tmp_path / "probe"),
                                "--threads", "1", "--pool", str(REPO / "data/teams/eval-s43-200.txt")])
    rows = [json.loads(line) for line in (tmp_path / "probe" / "positions.jsonl").read_text().splitlines()]
    assert summary["positions"] == len(rows) == 2
    for row in rows:
        assert set(row["head_cost"]) == {"tiny", str(path)}
        assert row["empirical_cost"] == row["result_awake"] - row["result_asleep"]
        assert -2 <= row["empirical_cost_discounted"] <= 2
    assert np.isfinite(summary["head_cost[tiny]"][0]) and np.isfinite(summary["empirical_cost"][0])
