# Profile-guided optimization

Two scripts with the same stages: `tools/pgo.ps1` (Windows, MSVC toolchain) and `tools/pgo.sh` (macOS and
Linux). Each builds instrumented binaries, runs the training workload, merges the profile with
`llvm-profdata`, and rebuilds `bench`, `difftest` and `search_bench` into `<out>/target-use`. With a maturin
it also builds a PGO pyengine wheel into `<out>/wheels`. Neither script installs anything. Profiles go stale
when engine code changes, so rerun the script after every engine change. A stale profile is still correct,
just slower.

The training corpus must not be the benchmark corpus. Generate one with the oracle, or reuse fixtures you do
not benchmark on:

```sh
node tools/oracle/gen-fixtures.mjs <showdown> --count 2000 --seed 4242 --threads 2 --verify-replay 0 --out scratch/corpus/pgo-train.jsonl.gz
# or, without Showdown (gzip members concatenate):
mkdir -p scratch/corpus && cat data/fixtures/slice0-200.jsonl.gz data/fixtures/slice1-200.jsonl.gz > scratch/corpus/pgo-train.jsonl.gz
```

## Windows

PGO needs the **MSVC** Rust toolchain. The default `stable-x86_64-pc-windows-gnu` sysroot ships no
`profiler_builtins`, so `-Cprofile-generate` fails there with `E0463`. The MSVC toolchain
(`stable-x86_64-pc-windows-msvc`, same rustc 1.99.0, with Visual Studio 2022 Build Tools and the
`llvm-tools` component) has the runtime and `llvm-profdata`.

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc
rustup component add llvm-tools --toolchain stable-x86_64-pc-windows-msvc
powershell -File tools/pgo.ps1 -Corpus scratch/corpus/pgo-train.jsonl.gz [-Maturin <maturin.exe>] [-Python <python.exe>] [-Pool <pool.txt.gz>] [-Search]
```

The PowerShell script supports output paths with spaces. It uses `-Python` for both wheel builds and
the instrumented wheel workload, and `-Jobs` limits both cargo and maturin builds. `-Pool` is only
needed with `-Maturin`. The working directory and build environment are restored even if a stage fails.
An inherited `LLVM_PROFILE_FILE` is temporarily overridden so every profile lands under `<out>/profraw`.

## macOS and Linux

```sh
rustup component add llvm-tools-preview
tools/pgo.sh --corpus scratch/corpus/pgo-train.jsonl.gz [--maturin <maturin>] [--python <python>] [--pool <pool.txt.gz>] [--no-wheel] [--native] [--search]
```

`pgo.sh` finds `llvm-profdata` under `$(rustc --print sysroot)/lib/rustlib/*/bin`. It uses the maturin on
`PATH`, else `<repo>/.venv/bin/maturin`, and the Python next to maturin, else `python3`. Builds are portable;
`--native` adds `-Ctarget-cpu=native` to both builds. `--no-wheel` skips the wheel and the pyengine workload.
The script ends by printing the PGO binaries directory and the wheel path.

`--search` / `-Search` builds everything with the opt-in `search` cargo feature (`engine/search` for
difftest). The default build, and the one to profile for training, has it off.

## The workload

- difftest `bench` replays the corpus with `NoLog` (training configuration) for `--train-seconds` (default
  20), then 3 s with `TextLog`.
- difftest `search_bench` runs restore, clone, reset and leaf turns for 2 s each.
- The pyengine workload (`crates/pyengine/src/pgo.rs`, 10 s) runs inside the instrumented wheel. It plays
  a TextLog `BatchEnv` the way the trainers drive it (`random_actions`, `mask_slot1`, `step`, `observe` for
  every env and side), then a NoLog `BatchEnv` (`run_random`, `step`, `clone_game`). A further 15% of the
  time budget runs TextLog smart self-play: every step encodes both sides (`encode_row`, partner matching,
  and damage/token features) and calls `SmartAgent.first`/`second` with conditional legal masks. This
  profiles the observation and scripted-player fast paths inside the wheel too. The dex is loaded from
  the build's source checkout (`data/training/dex.json`), which must remain available for this workload.
  Next come
  `GridExecutor` calls on mid-battle NoLog and TextLog roots (both stop modes, reseeded cells, requests on,
  multi-world calls on determinized worlds), and finally per-cell clone, `restore_from` and
  `replace_hidden_set`. Teams come from `--pool` / `-Pool` (the real training pools are
  `data/teams/pool-s*-200k.txt.gz`) or the bundled `crates/pyengine/testdata/pool-400.txt.gz`.

**The wheel has to profile itself.** A profile applies only to functions whose mangled names match the
profiled build, and the wheel's names never match a plain cargo build. Maturin compiles `_numbrion` as a
`cdylib` LTO root (`cargo rustc --crate-type cdylib`), which gives `engine` and `_numbrion` different
`-C metadata` hashes than any `cargo build`. Most engine code is `Battle<L>`, generic over the log sink, and
is instantiated inside `_numbrion`, so its symbols carry `_numbrion`'s hash. I checked this on the M1 Pro.
A profile from difftest plus a cargo-built `pgo_workload` example matched 0 of the wheel's 411 remaining
engine symbols. The PGO wheel was no faster than the plain wheel (table below). That is also what the
Windows wheel got before this change, when `pgo.ps1` profiled only the difftest binaries.

So the scripts build the wheel twice. The first build is instrumented and gets loaded by
`tools/pgo_wheel_workload.py`, which unpacks it under `<out>/wheel-gen` and calls the hidden
`_numbrion._pgo_workload` from any Python >= 3.9, with no numpy and no install. The second build uses the
merged profile. With that, 665 of 678 engine symbols and 126 of 131 `_numbrion` symbols in the PGO wheel
have profile data. `cargo run --release -p pyengine --example pgo_workload -- [--seconds 10] [--pool ...]`
runs the same workload without Python, for timing and smoke tests. Its profile does not fit the wheel.

## Results

### 2026-10-11, Apple M1 Pro (macOS 26, rustc 1.94.1, aarch64-apple-darwin, portable build)

The machine was busy (load average 4 to 9) throughout, so take ratios, not absolute numbers.
`tools/pgo.sh` took about 5 minutes end to end on `slice0-200` + `slice1-200` and the bundled pool. The PGO
wheel passes `pytest crates/pyengine/pytests` (27 tests) in a throwaway venv. The PGO `difftest` replays
`slice2-200` and `sample-50` with every battle passing.

| One thread, alternating runs | No PGO | PGO (wheel-profiled) | PGO profile from cargo builds |
| --- | --- | --- | --- |
| difftest `bench`, `slice2-200` (held out), 3 x 8 s, battles/s | 1,995 / 2,065 / 2,066 | 2,668 / 2,932 / 2,944 (**+39%**) | n/a |
| `bench.py` rust level (`run_random`), 3 x 6 s, battles/s | 2,169 / 2,250 / 2,242 | 2,755 / 2,848 / 2,853 (**+27%**) | 2,267 / 2,222 / 2,240 (+1%) |
| `bench.py` step level, 3 x 6 s, battles/s | 1,800 / 1,858 / 1,904 | 2,221 / 2,279 / 2,345 (**+23%**) | 1,855 / 1,823 / 1,818 (-1%) |
| `search_bench.py` `executor/1`, 4 x (4 x 1 s), cells/s | 33,886 / 30,188 / 34,536 / 36,491 | 44,252 / 46,109 / 45,814 / 46,571 (**+35%**) | 32,479 / 29,458 / 35,798 / 35,343 (-1%) |

The `bench.py` and `search_bench.py` runs use the bundled pool, which the pyengine workload also trained on
(with different seeds and roots). The cargo-profile column is the wheel from an earlier `pgo.sh` revision
that ran the `pgo_workload` example from an instrumented `cargo build`. Re-measure on an idle machine before
quoting absolute numbers.

### 2026-10-11, Windows (MSVC rustc 1.99.0, `main` at `4dd4517`)

A correctness run, not a speed measurement. `pgo.ps1` ran end to end twice, default and `-Search`, with the
wheel (`-Maturin`, `-Python`, the bundled pool) and `-TrainSeconds 1`. Both output paths had spaces
(`scratch/windows pgo default`, `scratch/windows pgo search`). The caller had set `CARGO_ENCODED_RUSTFLAGS`,
`RUSTUP_TOOLCHAIN` (GNU) and `LLVM_PROFILE_FILE` beforehand. In both runs the script restored them and the
working directory, and no profile was written to the inherited `LLVM_PROFILE_FILE` path.

- Both PGO `difftest.exe` builds replay `sample-50`, `slice2-200` and `directed-smoke` (634 battles, not in the
  training corpus) with every battle passing, and they pass the 124 harness selftests.
- In a throwaway venv, `pytest crates/pyengine/pytests` passes with both PGO wheels: default 27 passed and 4
  skipped (the search tests), search 31 passed. Each wheel reports the expected `numbrion.SEARCH`.
- `tools/check.ps1` passes: engine unit tests with `search` but without `reveal-audit`, engine integration
  tests with all features, pyengine with and without `search`, difftest, the fixture replays and the selftests.

### 2026-10-10, Windows (branch `perf3`)

- Correctness at `a5fee3d`: the PGO `difftest.exe` (profile retrained on that head) replays all 13,034 gate
  battles bit-exact and passes the 124 harness selftests. Engine (351) and pyengine (18) tests passed on the
  MSVC toolchain at `158b4f2`; the GNU gate at `a5fee3d` has 359 and 22.
- Speed at `a5fee3d`, 1 thread, fuzz-2000, three interleaved rounds while a training run shared the CPU
  (ratios meaningful, absolute values low): GNU 1,149, MSVC without PGO 1,166, **MSVC with PGO 1,559
  battles/s** (+34% over the same code built with MSVC without PGO). An earlier busy-CPU run at `158b4f2`
  measured 743 / 757 / 910. Re-measure on an idle machine before quoting.
- That wheel was built from the difftest-only profile and, per the analysis above, most likely got no PGO
  benefit. The wheel-profiled `pgo.ps1` now runs on Windows (2026-10-11 above). Its wheel speedup has not
  been measured there yet.
