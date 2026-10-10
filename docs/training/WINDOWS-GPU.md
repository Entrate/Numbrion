# GPU training on Windows

Use the `main` branch. The RX 5500 on this Windows 11 machine works with
PyTorch DirectML. CUDA is for NVIDIA cards; this setup uses the existing AMD
driver and DirectX 12, with no CUDA or ROCm installation.

Microsoft documents [native Windows training with DirectML](https://learn.microsoft.com/en-us/windows/ai/directml/pytorch-windows).
The [published Windows wheels](https://pypi.org/project/torch-directml/) support
Python up to 3.12. Our isolated `.venv` uses Python 3.12, `torch-directml`
0.2.5.dev240914, PyTorch 2.4.1, and NumPy 1.26.4. The underlying Torch version
reports `2.4.1+cpu`; the DirectML plugin supplies GPU execution separately.
The GPU device reports `privateuseone:0`, not `cuda:0`.

## Setup

Prerequisites: Python 3.12, stable Rust, a matching linker, and Node 24 for
generating fresh teams from the pinned Showdown source. This machine uses the
already-installed MinGW GCC and Rust's `x86_64-pc-windows-gnu` toolchain; the
Python extension, its tests, and its abi3 wheel have been built and run natively
on Windows. MSVC plus Visual Studio C++ Build Tools is another supported build
configuration. No change to the global Python installation is needed.

Build the pinned Showdown checkout following the root README if it is not already
present. Then, from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\setup-training.ps1
```

This installs the pinned training dependencies into `.venv`, builds and installs
the Python engine API, creates a wheel under `target/wheels`, runs the Python
tests and GPU gradient check, and creates a fresh 2,000-team pool if absent.
The process-local execution-policy option leaves Windows' saved policy unchanged.

To rerun only the GPU check:

```powershell
.venv\Scripts\python.exe crates\pyengine\gpu_check.py --device directml
```

The check verifies GPU-resident model parameters and gradients, optimizer updates,
and falling loss. Its Adam optimizer currently warns that `lerp` falls back to
CPU. The self-play trainer uses RMSprop to avoid that unsupported operation.
DirectML does not cover every PyTorch operator; check backend warnings when
changing the model. The trainer never silently chooses CPU when DirectML fails.

## Start or resume self-play

```powershell
.venv\Scripts\python.exe crates\pyengine\train.py --device directml --updates 100
```

Default settings are 32 environments, four Rust worker threads, 64 boundaries per
rollout, four PPO epochs, batch size 256, and a 128-wide MLP. Both sides use the
current policy. Rewards are terminal +1/-1/0, with no HP shaping. Slot B's policy
is conditioned on slot A's action and masked by `BatchEnv.mask_slot1`.
Forward/backward and learning run on the GPU; the Rust simulator, protocol encoder,
and action sampling run on the CPU.

The run writes `scratch/training/directml/latest.pt` after each update and appends
metrics to `metrics.jsonl`. Resume model weights, optimizer state, update counters,
and RNG state with:

```powershell
.venv\Scripts\python.exe crates\pyengine\train.py --device directml --updates 100 --resume scratch\training\directml\latest.pt
```

`--updates` means additional updates. Resume starts fresh battles; it does not
restore a partially played environment. `--width` must match the checkpoint.
CPU copies of weights and optimizer state make checkpoints usable with `--device cpu`
or CUDA on a different machine. Only load checkpoints you trust.

Use `--pool <file>` for a larger fresh packed-team pool, `--envs` and `--batch-size`
to tune memory use, and `--adapter` for another DirectML GPU. `Ctrl+C` stops a run;
the previous complete update remains saved. The generated pools, logs, checkpoints,
virtualenv, and wheel build output are git-ignored.

## What this baseline establishes

The small trainer establishes the complete GPU training path; it is not a claim
of a strong battle agent. It reads only each side's own request and player-view
logs, tracks revealed opponents across boundaries, and clears that tracker on
auto-reset. It never reads an omniscient log or clones hidden battle state for
policy input. GAE stops at terminal boundaries instead of bootstrapping from the
auto-reset battle. Tests cover these boundaries and masked sampling.

The encoder is deliberately small: numerical own-team features, publicly revealed
opponent HP/status, and hashed species/move/item/ability/field features. It omits
many timing, boost, damage-calculation and belief features from `TIPS.md`.
The authoritative action masks also have the hidden-information caveat described
in `TRAINING-API.md`; they are not identical to request-derived ladder legality.
Add a richer encoder, opponent checkpoint pool, and held-out evaluation before
judging strength or committing to a long run. A separate seed-43 evaluation team
pool has been generated locally for that next step.

## Verified on this machine

- Python training API: 19 tests passed; the Rust API test suite also passed.
- Training helpers: five tests passed (GAE boundaries, masks and public observation tracking).
- GPU: RX 5500 selected as `privateuseone:0`; 50 gradient updates reduced synthetic
  loss from 0.264 to 0.000189 and changed model weights.
- Real-engine GPU self-play: five PPO updates, 94 completed battles, finite losses,
  updated weights, checkpoint saving and resume. RMSprop emitted no fallback warning.
- Default-size GPU run: 100 PPO updates and 9,793 completed battles with 32
  environments and four Rust worker threads. The saved checkpoint is
  `scratch/training/directml/latest.pt`; the run resumed successfully from update 88.
- Full Windows check: 373 Rust tests passed, with 11 pre-existing engine tests
  ignored; all 434 committed fixture battles passed through the real engine, and
  all 124 harness fault-injection checks passed. The Unix-only profiler is gated
  off on Windows, and log-replay tests accept both LF and CRLF fixture files.
- Two-second environment benchmark: about 852 battles/s with one Rust worker and
  4,410 battles/s with six workers, without logging; these are environment-only
  rates, not model-training throughput.

Useful commands:

```powershell
.venv\Scripts\python.exe -m pytest crates\pyengine\pytests crates\pyengine\training_tests -q
.venv\Scripts\python.exe crates\pyengine\bench.py --threads 1 6 --seconds 6 --log
```
