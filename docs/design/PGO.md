# PGO on Windows GNU

PGO is blocked on the installed `stable-x86_64-pc-windows-gnu` toolchain.
On 2026-10-10, a minimal executable built with `-Cprofile-generate` failed
with `E0463` because the target sysroot has no `profiler_builtins` crate.
The same executable without instrumentation compiled and ran successfully.
The PGO task stopped at its required runtime check; no working PGO pipeline,
optimized binaries, wheel, or performance result is claimed.

## Verified environment and cause

- Rust: `1.99.0`, commit `b940084d7eb6a299eb4bfeb8e34901bc051e7ac4`
  (2026-09-28), host and target `x86_64-pc-windows-gnu`.
- LLVM in rustc: `23.1.1`. Installed `llvm-profdata.exe`:
  `23.1.1-rust-1.99.0-stable`.
- Sysroot: `C:\Users\aminu\.rustup\toolchains\stable-x86_64-pc-windows-gnu`.
- Profdata: `<sysroot>\lib\rustlib\x86_64-pc-windows-gnu\bin\llvm-profdata.exe`.
- `<sysroot>\lib\rustlib\x86_64-pc-windows-gnu\lib` has no files matching
  `*profiler*`; all installed component manifests have zero entries matching
  `profiler_builtins` or `libprofiler`. `compiler_builtins` is present and is a
  separate crate.

The failure occurs during Rust runtime crate resolution, before the linker:

```text
error[E0463]: can't find crate for `profiler_builtins`
  |
  = note: the compiler may have been built without the profiler runtime
```

Rust's [PGO implementation documentation](https://rustc-dev-guide.rust-lang.org/profile-guided-optimization.html)
explains that `profiler_builtins` packages LLVM compiler-rt's profiling runtime
and requires `profiler = true` when building Rust. Installing `llvm-tools`
supplies the merge tool, but does not supply this missing runtime crate.
The current upstream [distribution configuration](https://github.com/rust-lang/rust/blob/main/src/ci/github-actions/jobs.yml)
has no `--enable-profiler` in the `dist-x86_64-mingw` job, consistent with the
observed omission. That upstream configuration is corroborating evidence;
the installed compiler's exact bootstrap configuration was not available.
This check establishes the limitation of this installation, not a universal
inability to build a GNU profiling runtime.

## Reproduce the check

Run from the worktree root in PowerShell. These commands use the compiler
directly, with an explicit target and an absolute profile directory. Check
`$LASTEXITCODE` after each native command; Windows PowerShell can render native
stderr as error records without stopping execution.

```powershell
$rustSysroot = 'C:\Users\aminu\.rustup\toolchains\stable-x86_64-pc-windows-gnu'
$rustcExe = Join-Path $rustSysroot 'bin\rustc.exe'
$probeDir = Join-Path (Get-Location).Path 'scratch\pgo\probe'
$profileDir = Join-Path $probeDir 'profiles'
New-Item -ItemType Directory -Force -Path $profileDir | Out-Null
@'
fn main() {
    let count = std::env::args().count();
    println!("PGO runtime probe: {count} arguments");
}
'@ | Set-Content -LiteralPath (Join-Path $probeDir 'main.rs') -Encoding ASCII

& $rustcExe --edition=2024 --target x86_64-pc-windows-gnu -O `
    (Join-Path $probeDir 'main.rs') -o (Join-Path $probeDir 'plain.exe')
# Compile exit 0.
& (Join-Path $probeDir 'plain.exe')
# Run exit 0; prints: PGO runtime probe: 1 arguments

& $rustcExe --edition=2024 --target x86_64-pc-windows-gnu -O `
    "-Cprofile-generate=$profileDir" (Join-Path $probeDir 'main.rs') `
    -o (Join-Path $probeDir 'instrumented.exe')
# Compile exit 1; E0463, missing profiler_builtins; zero .profraw files.

$rustlibDir = Join-Path $rustSysroot 'lib\rustlib'
& $rustcExe -vV
& (Join-Path $rustlibDir 'x86_64-pc-windows-gnu\bin\llvm-profdata.exe') --version
Get-ChildItem -LiteralPath `
    (Join-Path $rustlibDir 'x86_64-pc-windows-gnu\lib') -Filter '*profiler*'
$manifestPaths = Get-ChildItem -LiteralPath $rustlibDir -Filter 'manifest-*' -File |
    Select-Object -ExpandProperty FullName
rg -n -i 'profiler_builtins|libprofiler' $manifestPaths
# Both runtime searches return no matches; rg exits 1.
```

Local evidence is saved under `scratch/pgo/probe/`: `main.rs`,
`plain-build.log`, `plain-run.log`, `instrumented-build.log`, and `result.json`.
These scratch artifacts are ignored by Git.

## Resuming the build experiment

First obtain a compatible GNU Rust sysroot built with the profiler runtime
enabled, then repeat the check through executable execution and actual
`.profraw` generation. Building Rust with `profiler = true` is the documented
runtime prerequisite; its compatibility with this specific GNU setup remains
untested. No toolchain was installed, switched, or modified for this task.

Once the check passes, implement `tools/pgo.ps1` for `bench` and `pyengine`:

1. Use explicit `--target x86_64-pc-windows-gnu`, `--release`, `--locked`,
   `-j 4`, separate instrumented/optimized target directories, and absolute
   generation/use paths under `scratch/pgo/`. Keep other build flags identical.
2. Train the executable on committed fixtures and optionally fresh seed-4242
   battles. Keep `fuzz-2000` out of training. Train the extension through its
   own Python driver, including restored positions if practical. Respect
   `crates/pyengine/pyproject.toml`'s `python` feature.
3. Merge newly generated raw profiles with the matching `llvm-profdata merge`;
   build with `-Cprofile-use=<absolute merged.profdata path>`. Regenerate
   profiles when engine code, dependencies, toolchain, features, or flags change.
   Keep mismatch diagnostics enabled initially. Add
   `-Cllvm-args=-pgo-warn-mismatch=false` only after investigating a demonstrated
   need; suppressing diagnostics does not repair a stale profile.
4. Validate the optimized difftest on all 13,034 required battles, the selftest,
   and the engine/pyengine Rust tests. Compare same-commit non-PGO/PGO benches
   in at least four interleaved rounds on `fuzz-2000`, one thread, 10 seconds.
5. Build the optimized wheel into `scratch/pgo/wheels/` and verify in a
   disposable scratch venv; never install into the user's training venv.

The [Rust PGO guide](https://doc.rust-lang.org/rustc/profile-guided-optimization.html)
documents the generation, training, merge, and use stages and the reasons for
an explicit target and absolute profile paths.

Gain is **unmeasured**. The supplied review's 0–8% range (3–5% planning estimate)
is a hypothesis, not a result. PGO is not ready for adoption on this installation;
revisit it after resolving the runtime prerequisite and measuring both consumers.
