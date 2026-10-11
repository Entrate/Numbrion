# GPU training on Apple silicon

The trainer runs on the Apple GPU through PyTorch's MPS (Metal) backend. `--device auto` picks it on a Mac
with Apple silicon; `--device mps` asks for it explicitly and fails instead of falling back to the CPU. The
device reports `mps:0`. Verified on an M1 Pro (8 cores, 16 GB) with macOS 26, Python 3.14, PyTorch 2.14.1
and NumPy 2.5.4. The DirectML workarounds in `training/ppo.py` and `training/model.py` (plain Adam, scalar
gradient clipping, softplus BCE) are plain PyTorch, so they run unchanged on MPS.

## Setup

Prerequisites: Python 3.10+ (Homebrew's `python3` is fine), stable Rust via rustup. From the repository root:

```sh
python3 -m venv .venv
.venv/bin/pip install -r crates/pyengine/requirements-mac.txt
(cd crates/pyengine && VIRTUAL_ENV=$PWD/../../.venv ../../.venv/bin/maturin develop --release)
.venv/bin/python crates/pyengine/gpu_check.py
.venv/bin/python -m pytest crates/pyengine/pytests crates/pyengine/training_tests -q
```

`maturin develop` builds the engine (portable, no `target-cpu=native`) and installs it into `.venv` as an
editable package; rerun it after changing the Rust code.

The team pools are git-ignored. The trainer, `evaluate.py` and the training tests read
`data/teams/train-s42-2000.txt` and `data/teams/eval-s43-200.txt`. Both are deterministic, so copy them
from the Windows machine or regenerate them from the pinned Showdown checkout (root README):

```sh
node tools/oracle/gen-teams.mjs <path-to-pokemon-showdown> --count 2000 --seed 42 --out data/teams/train-s42-2000.txt
node tools/oracle/gen-teams.mjs <path-to-pokemon-showdown> --count 200 --seed 43 --out data/teams/eval-s43-200.txt
```

## Run

```sh
.venv/bin/python crates/pyengine/train_selfplay.py --minutes 60 --output scratch/training/first-run
.venv/bin/python crates/pyengine/evaluate.py --run scratch/training/first-run
```

Options and outputs are as in [SELFPLAY.md](SELFPLAY.md). Checkpoints store CPU tensors, so runs move between
the Mac and the Windows machine with `--resume`.

## First measurement

A 3-minute smoke run with the default settings (128 envs, 4 actor processes, width 128, 3 layers) on the
400-team test pool (`crates/pyengine/testdata/pool-400.txt.gz`) gave 1,440 trained samples/s and 43
battles/s after warm-up: 4.7 s of PPO per update against 1.1 s of rollout, so the learner is the bottleneck,
as on Windows (1,700 samples/s and 59 battles/s on the RX 5500). The first update takes ~11 s while Metal
compiles its kernels. `memory_mb()` reports only on Windows, so the metrics have no memory columns here yet.
