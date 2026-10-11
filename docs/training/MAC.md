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
caffeinate -i .venv/bin/python crates/pyengine/train_selfplay.py --minutes 60 --compile --output scratch/training/first-run
.venv/bin/python crates/pyengine/evaluate.py --run scratch/training/first-run
```

`caffeinate -i` keeps macOS from idle-sleeping while the run lasts (a sleeping Mac pauses training, and the
time budget keeps counting). Closing a laptop's lid still sleeps it unless it is on power with an external
display. `--compile` compiles the minibatch loss with `torch.compile` (below); leave it out to run eagerly.
Options and outputs are as in [SELFPLAY.md](SELFPLAY.md). Checkpoints store CPU tensors, so runs move between the
Mac and the Windows machine with `--resume`.

## Performance

M1 Pro, default settings (128 envs, 4 actor processes, width 128, 3 layers, minibatch 1024), real pool, 4-minute
runs: **3,510 trained samples/s and 127 battles/s with `--compile`, 3,120 and 114 without** (Windows RX 5500:
1,700 and 59; this Mac before the changes below: 1,440 and 43). The learner is still the bottleneck: 1.8 s of PPO
per update against 1.1 s of rollout. The first update takes ~2.5 s (~7 s with `--compile`) while Metal compiles
its kernels.

`learner_bench.py` times the PPO update alone on a saved rollout, with a per-phase breakdown:

```sh
.venv/bin/python crates/pyengine/learner_bench.py --collect scratch/bench/rollout.npz
.venv/bin/python crates/pyengine/learner_bench.py --device mps --breakdown
.venv/bin/python crates/pyengine/learner_bench.py --device mps --compile
.venv/bin/python crates/pyengine/learner_bench.py --device cpu --threads 4
```

One update (6,834 samples), cumulative:

| Learner | s/update |
|---|---|
| MPS, before | 4.75 (backward 3.6) |
| + move vectors as a matmul | 2.33 |
| + rollout uploaded once per update, fused Adam and clipping | 2.09 |
| + `--compile` | 1.71 |
| CPU, 1 thread / 4 threads (for comparison) | 5.5 / 2.8 |

All MPS-only (`device.type == "mps"`); the CPU and DirectML paths are unchanged, and the CPU update is
bit-identical to before. Eager MPS training is deterministic (two runs give identical weights).

* **Move vectors.** MPS is slow at reducing over broadcast axes in the backward pass. Building the 40 move-code
  vectors as `actor + moves + targets + tera` broadcast to `[B, 4, 5, 2, W]` cost 45 ms per minibatch there
  (4 ms on the CPU); `Model.action_vectors` sums them with a constant 0/1 matrix (`move_combination`) instead,
  3 ms. Outputs are identical, gradients agree to float rounding (3e-7 relative), the state dict is unchanged.
* **Rollout upload.** `ppo_update` copies the whole rollout to the device once per update and gathers the
  minibatches there, instead of gathering each with NumPy and copying it: 0.14 -> 0.03 s per update (~5%),
  bit-identical results.
* **Adam and clipping.** `torch._fused_adam_` on the same `m`/`v` state (1.4 ms per step against 4.8 ms for the
  per-tensor loop) and one norm over the concatenated gradients (0.9 ms against 2.3): ~2% of an update. Same
  update up to rounding; the optimizer state format is unchanged, so checkpoints resume on any device.
* **`torch.compile`** of `ppo_loss`: 18% per update on an idle machine; end to end +12% over 4 minutes (while
  another training run started) and +22% over 2 minutes (3,835 against 3,151 samples/s). Needs `max_fusion_unique_io_buffers = 24`: unbounded fusion generated Metal kernels with
  more than the 31 buffer arguments Metal allows. Rounding differs from eager (weights 8e-4 apart after 28 Adam
  steps, the same size as the run-to-run noise of a non-deterministic kernel), so it is a flag, not the default.
* **Tried and dropped.** `F.embedding` for the single-id lookups: 2-3x faster in isolation, but a whole update got
  ~1.5% slower and its atomic backward made MPS runs non-reproducible. `torch._foreach_*` Adam: no faster than the
  loop on MPS. A 2048 minibatch: 12% faster but halves the gradient steps per update. Attention layouts (chunked
  q/k/v, contiguous copies, manual `bmm`): within 10% of `scaled_dot_product_attention`.
* The MPS learner beats the CPU learner and leaves the CPU cores to the actors, so `--device auto` keeps MPS.

The metrics report memory on macOS too: `rss_mb` (resident) and `private_mb` (physical footprint, which includes
Metal allocations) for the learner and the largest actor, plus `mps_mb` (Metal driver allocation) for the learner.
A 12-minute run (321 updates, 77k battles) held steady: from minute 3 on, the learner stayed at ~3.1 GB private
(2.15 GB of it Metal) and each actor grew from 412 to 440 MB, slowing down. `--compile` adds ~250 MB.
