# Profile-guided optimization (Windows)

PGO needs the **MSVC** Rust toolchain. The default `stable-x86_64-pc-windows-gnu` sysroot ships no
`profiler_builtins`, so `-Cprofile-generate` fails there with `E0463`. The MSVC toolchain
(`stable-x86_64-pc-windows-msvc`, same rustc 1.99.0, with Visual Studio 2022 Build Tools and the
`llvm-tools` component) has the runtime and `llvm-profdata`.

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc
rustup component add llvm-tools --toolchain stable-x86_64-pc-windows-msvc
# Training corpus: never the benchmark corpus.
node tools/oracle/gen-fixtures.mjs <showdown> --count 2000 --seed 4242 --threads 2 --verify-replay 0 --out scratch/corpus/pgo-train.jsonl.gz
powershell -File tools/pgo.ps1 -Corpus scratch/corpus/pgo-train.jsonl.gz [-Maturin <path to maturin.exe>]
```

`tools/pgo.ps1` builds instrumented difftest binaries, runs the training workload (NoLog replays,
a short TextLog pass, and the search primitives restore/clone/reset/leaf), merges the profile,
and rebuilds `bench`, `difftest` and `search_bench` into `scratch/pgo/target-use`. With
`-Maturin` it also builds a PGO pyengine wheel into `scratch/pgo/wheels` (it installs nothing).
Profiles go stale when engine code changes: rerun the script after every engine change.

## Results (2026-10-10, branch `perf3`)

- Correctness at `a5fee3d`: the PGO `difftest.exe` (profile retrained on that head) replays all 13,034 gate
  battles bit-exact and passes the 124 harness selftests. Engine (351) and pyengine (18) tests passed on the
  MSVC toolchain at `158b4f2`; the GNU gate at `a5fee3d` has 359 and 22.
- Speed at `a5fee3d`, 1 thread, fuzz-2000, three interleaved rounds while a training run shared the CPU
  (ratios meaningful, absolute values low): GNU 1,149, MSVC without PGO 1,166, **MSVC with PGO 1,559
  battles/s** (+34% over the same code built with MSVC without PGO). An earlier busy-CPU run at `158b4f2`
  measured 743 / 757 / 910. Re-measure on an idle machine before quoting.
- The PGO wheel is untested in Python so far.
