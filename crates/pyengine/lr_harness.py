"""Continue one checkpoint for ``--minutes`` under several training configurations from the same start, then score
each against the start with compare.py (docs/training/SELFPLAY.md, "Noise scale and automatic learning rate").

Run from crates/pyengine:

    ../../.venv/bin/python lr_harness.py --checkpoint RUN/latest.pt --minutes 60 --out DIR \\
        --common "--schedule-minutes 600 --lr-final 0.3" --lr 1e-4 --ema 200

* Start: a ``latest.pt`` (with optimizer state) is resumed as it is: every run gets ``--minutes`` on top of its
  elapsed time, and its schedules continue (give the original run's ``--schedule-minutes``, ``--lr-final`` and the
  rest in ``--common``, or progress is measured against the new ``--minutes``). A snapshot (weights only, e.g.
  ``m535.pt``) becomes ``DIR/start.pt``: those weights with a fresh Adam, an empty opponent pool and the entropy
  coefficient at ``--entropy``, at elapsed time 0. ``--width``/``--layers``/``--joint-features`` come from the
  checkpoint.
* Configurations: ``--config NAME=FLAGS`` (repeatable), flags for train_selfplay.py where ``{lr}``, ``{lr*X}`` and
  ``{ema}`` take ``--lr`` (times X) and ``--ema``. Default: A ``--lr {lr}`` (the current schedule), B ``--lr
  {lr*0.25}`` + EMA, C ``--lr {lr*0.1}`` + EMA, D ``--lr {lr} --auto-lr`` + EMA. Every run uses the same seed, so the
  first rollout is the same. Runs are sequential (they would share the machine) and logged to ``DIR/NAME/train.log``.
* Scoring: each run's final weights (``ema/latest.pt`` when the run averages, else ``latest.pt``) against the start
  with compare.py (``--compare-args``, default 40 h2h and 20 panel rounds; ``--ref`` defaults to the start), into
  ``DIR/NAME/compare``; the panel cache is ``DIR/panel-cache.json``.
* Rerunning resumes: finished runs (``DIR/NAME/DONE``) are kept, an unfinished run directory is deleted and run
  again, and compare.py reuses its cache. ``DIR/summary.json`` and ``summary.md`` collect the scores and each run's
  last learning rate, ``lr_factor`` and ``noise_b``.

An EMA starts from the start weights: with ``--ema`` near the run's update count, half of the average is still the
start. Keep ``--ema`` at a fraction of the updates a run makes (``update`` in its metrics.jsonl).
"""
from __future__ import annotations

import argparse
import json
import re
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
DEFAULT_CONFIGS = ("A=--lr {lr}", "B=--lr {lr*0.25} --ema-updates {ema}", "C=--lr {lr*0.1} --ema-updates {ema}",
                   "D=--lr {lr} --auto-lr --ema-updates {ema}")


def expand(flags: str, lr: float, ema: float) -> list[str]:
    """``{lr}``, ``{lr*X}`` and ``{ema}`` in ``flags``, split like a shell."""
    flags = re.sub(r"\{lr(?:\*([0-9.eE+-]+))?\}", lambda m: f"{lr * float(m.group(1) or 1):.6g}", flags)
    return shlex.split(flags.replace("{ema}", f"{ema:g}"))


def parse_config(text: str) -> tuple[str, str]:
    name, sep, flags = text.partition("=")
    if not sep or not re.fullmatch(r"[A-Za-z0-9_.-]+", name):
        raise argparse.ArgumentTypeError(f"--config wants NAME=FLAGS with a plain NAME, got {text!r}")
    return name, flags


def start_file(checkpoint: Path, out: Path, seed: int) -> tuple[Path, dict]:
    """The file the runs resume and its facts (elapsed minutes, model shape)."""
    state = torch.load(checkpoint, map_location="cpu", weights_only=False)
    facts = dict(width=int(state.get("width", 128)), layers=int(state.get("layers", 3)),
                 joint_features="act_joint.weight" in state["model"])
    if "optimizer" in state:
        return checkpoint, dict(facts, elapsed_minutes=float(state["elapsed"]) / 60, fresh_optimizer=False)
    from training.dex import load_dex
    from training.league import SnapshotPool
    from training.model import Model
    from training.ppo import Adam

    path = out / "start.pt"
    if not path.exists():
        model = Model.from_state(load_dex(), state["model"], facts["width"], facts["layers"])
        payload = {"model": {k: v.detach().clone() for k, v in model.state_dict().items()},
                   "optimizer": Adam(model.parameters(), lr=1.0).state_dict(), "pool": SnapshotPool().state_dict(),
                   "entropy_controller": None, "elapsed": 0.0, "update": 0, "battles": 0, "samples": 0,
                   "numpy_rng": np.random.default_rng(seed).bit_generator.state, "width": facts["width"],
                   "layers": facts["layers"], "started_from": str(checkpoint)}
        out.mkdir(parents=True, exist_ok=True)
        torch.save(payload, path.with_suffix(".tmp"))
        path.with_suffix(".tmp").replace(path)
    return path, dict(facts, elapsed_minutes=0.0, fresh_optimizer=True)


def last_metrics(run: Path) -> dict:
    lines = (run / "metrics.jsonl").read_text().splitlines() if (run / "metrics.jsonl").exists() else []
    return json.loads(lines[-1]) if lines else {}


def train(name: str, flags: list[str], args, start: Path, facts: dict) -> Path:
    run = args.out / name
    if (run / "DONE").exists():
        print(f"{name}: already trained", flush=True)
        return run
    if run.exists():
        shutil.rmtree(run)  # an interrupted run: start it again from the same point
    run.mkdir(parents=True)
    command = [args.python, str(HERE / "train_selfplay.py"), "--resume", str(start), "--output", str(run),
               "--minutes", f"{facts['elapsed_minutes'] + args.minutes:g}", "--width", str(facts["width"]),
               "--layers", str(facts["layers"]), "--seed", str(args.seed),
               *(["--joint-features"] if facts["joint_features"] else []), *expand(args.common, args.lr, args.ema),
               *flags]
    (run / "command.json").write_text(json.dumps(command, indent=1))
    print(f"{name}: training {args.minutes:g} min: {shlex.join(command[2:])}", flush=True)
    started = time.time()
    with (run / "train.log").open("w") as log:
        code = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, cwd=HERE).returncode
    if code:
        raise SystemExit(f"{name}: train_selfplay.py exited with {code}, see {run / 'train.log'}")
    (run / "DONE").write_text(f"{time.time() - started:.0f} s\n")
    return run


def score(name: str, run: Path, args) -> dict:
    candidate = run / "ema" / "latest.pt" if (run / "ema" / "latest.pt").exists() else run / "latest.pt"
    command = [args.python, str(HERE / "compare.py"), "--candidate", str(candidate), "--best", str(args.checkpoint),
               "--out", str(run / "compare"), "--label", name, "--ref", str(args.ref or args.checkpoint),
               "--cache", str(args.out / "panel-cache.json"), "--run-flag", "", *shlex.split(args.compare_args)]
    print(f"{name}: scoring {candidate.relative_to(args.out)} against the start", flush=True)
    with (run / "compare.log").open("w") as log:
        code = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, cwd=HERE).returncode
    if code:
        raise SystemExit(f"{name}: compare.py exited with {code}, see {run / 'compare.log'}")
    result = json.loads((run / "compare" / "compare.json").read_text())
    rows = {}
    for m in result["matchups"]:
        if m["a"] == "candidate":
            rows.setdefault(f"T={m['temperature']:g}", {})["h2h" if m["b"] == "best" else m["b"]] = dict(
                score=m["score"], interval=m["pair_95pct_interval"], games=m["games"])
    metrics = last_metrics(run)
    return dict(candidate=str(candidate), verdict=(result.get("decision") or {}).get("verdict"), scores=rows,
                updates=metrics.get("update"), lr=metrics.get("lr"), lr_factor=metrics.get("lr_factor"),
                noise_b=metrics.get("noise_b"), accumulate=metrics.get("accumulate"))


def markdown(summary: dict) -> str:
    lines = [f"# LR harness: {summary['checkpoint']}", "",
             f"{summary['minutes']:g} minutes per run from the same start; scores are the candidate's (95% pair "
             "intervals) against the start (h2h) and the panel.", ""]
    temperatures = sorted({t for r in summary["runs"].values() for t in r["scores"]})
    header = "| run | flags | updates | last lr | lr_factor | noise_b | verdict |" + "".join(
        f" {t} h2h | {t} smart | {t} heuristic |" for t in temperatures)
    lines += [header, "|" + "---|" * (7 + 3 * len(temperatures))]

    def cell(entry):
        return f"{entry['score']:.3f} [{entry['interval'][0]:.3f}, {entry['interval'][1]:.3f}]" if entry else ""

    for name, r in summary["runs"].items():
        lr = "" if r["lr"] is None else f"{r['lr']:.3g}"
        row = f"| {name} | `{' '.join(r['flags'])}` | {r['updates']} | {lr} | {r['lr_factor']} | {r['noise_b']} | {r['verdict']} |"
        for t in temperatures:
            s = r["scores"].get(t, {})
            row += f" {cell(s.get('h2h'))} | {cell(s.get('smart'))} | {cell(s.get('heuristic'))} |"
        lines.append(row)
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--checkpoint", type=Path, required=True, help="latest.pt or a snapshot to continue")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--minutes", type=float, required=True, help="training minutes per configuration")
    parser.add_argument("--lr", type=float, default=3e-4, help="value of {lr} in the configurations")
    parser.add_argument("--ema", type=float, default=200, help="value of {ema} (updates) in the configurations")
    parser.add_argument("--common", default="", help="train_selfplay.py flags for every run")
    parser.add_argument("--config", type=parse_config, action="append",
                        help=f"NAME=FLAGS, repeatable (default: {'; '.join(DEFAULT_CONFIGS)})")
    parser.add_argument("--compare-args", default="", help="extra compare.py flags (rounds, envs, temperatures)")
    parser.add_argument("--ref", type=Path, help="compare.py's panel reference (default: the start checkpoint)")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--skip-compare", action="store_true", help="only train")
    args = parser.parse_args()
    configs = dict(args.config or [parse_config(c) for c in DEFAULT_CONFIGS])
    if not args.minutes > 0:
        parser.error("--minutes must be positive")
    args.out.mkdir(parents=True, exist_ok=True)
    start, facts = start_file(args.checkpoint, args.out, args.seed)
    print(json.dumps({"start": str(start), **facts}), flush=True)
    runs = {}
    for name, flags in configs.items():
        runs[name] = (train(name, expand(flags, args.lr, args.ema), args, start, facts), expand(flags, args.lr, args.ema))
    summary = dict(checkpoint=str(args.checkpoint), start=str(start), minutes=args.minutes, common=args.common,
                   lr=args.lr, ema=args.ema, facts=facts, runs={})
    if not args.skip_compare:
        for name, (run, flags) in runs.items():
            summary["runs"][name] = dict(flags=flags, **score(name, run, args))
            (args.out / "summary.json").write_text(json.dumps(summary, indent=1))
        (args.out / "summary.md").write_text(markdown(summary))
        print(markdown(summary), flush=True)


if __name__ == "__main__":
    main()
