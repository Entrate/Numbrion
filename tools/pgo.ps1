param(
    # Training battles; must not be the benchmark corpus.
    [Parameter(Mandatory)][string]$Corpus,
    [string]$Out = 'scratch/pgo',
    [int]$TrainSeconds = 20,
    [int]$Jobs = 4,
    # Also build a PGO pyengine wheel into <Out>/wheels (needs maturin; never installs it).
    [string]$Maturin = '',
    # Python >= 3.9 that runs the instrumented wheel's workload (with -Maturin; numpy not needed).
    [string]$Python = 'python',
    # Team pool for the pyengine workload (e.g. data/teams/pool-s1-200k.txt.gz).
    [string]$Pool = 'crates/pyengine/testdata/pool-400.txt.gz',
    # Build with the opt-in `search` cargo feature.
    [switch]$Search
)
# Profile-guided build of the difftest binaries (bench, difftest, search_bench) and,
# optionally, the pyengine wheel. Needs the MSVC toolchain: the windows-gnu sysroot
# ships no profiler_builtins (docs/design/PGO.md). Profiles go stale when engine code
# changes; rerun this script after every engine change.

$ErrorActionPreference = 'Stop'
$toolchain = 'stable-x86_64-pc-windows-msvc'
$target = 'x86_64-pc-windows-msvc'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path

function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Exe $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

$sysroot = (& rustup run $toolchain rustc --print sysroot).Trim()
$profdata = Join-Path $sysroot "lib\rustlib\$target\bin\llvm-profdata.exe"
if (-not (Test-Path -LiteralPath $profdata)) {
    throw "llvm-profdata not found; run: rustup component add llvm-tools --toolchain $toolchain"
}
$Corpus = (Resolve-Path -LiteralPath $Corpus).Path
$Pool = (Resolve-Path -LiteralPath $Pool).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path -LiteralPath $Out).Path
$raw = Join-Path $Out 'profraw'
$merged = Join-Path $Out 'merged.profdata'
$manifest = 'crates/difftest/Cargo.toml'
$build = @("+$toolchain", 'build', '--manifest-path', $manifest, '--release', '--locked', '-j', "$Jobs", '--bins', '--target', $target)
$wheelBuild = @('build', '--release', '--locked', '-m', 'crates/pyengine/Cargo.toml', '--target', $target)
if ($Search) {
    $build += @('--features', 'engine/search')
    $wheelBuild += @('--features', 'python,search')
}

# 1. Instrumented build. Rust flags with --target leave build scripts uninstrumented. They go through
#    CARGO_ENCODED_RUSTFLAGS so a profile path with spaces (C:\Users\First Last) stays one argument.
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $raw
$env:CARGO_ENCODED_RUSTFLAGS = "-Cprofile-generate=$raw"
Invoke-Checked cargo ($build + @('--target-dir', (Join-Path $Out 'target-gen')))
$gen = Join-Path $Out "target-gen\$target\release"
# The wheel is profiled through itself: a profile only applies to code built exactly like the
# profiled build, and maturin's cdylib build differs from any plain cargo build (crates/pyengine/src/pgo.rs).
if ($Maturin) {
    $env:RUSTUP_TOOLCHAIN = $toolchain
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $Out 'wheels-gen')
    Invoke-Checked $Maturin ($wheelBuild + @('--target-dir', (Join-Path $Out 'target-gen'), '-o', (Join-Path $Out 'wheels-gen')))
    Remove-Item Env:RUSTUP_TOOLCHAIN
}

# 2. Training workload: NoLog self-play replays (training configuration), a short
#    TextLog pass, the search primitives (restore, clone, reset, leaf turns), and in the
#    instrumented wheel the pyengine workload (BatchEnv self-play with and without TextLog,
#    GridExecutor, clone/restore/replace_hidden_set).
Invoke-Checked (Join-Path $gen 'bench.exe') @($Corpus, '--threads', '1', '--seconds', "$TrainSeconds")
Invoke-Checked (Join-Path $gen 'bench.exe') @($Corpus, '--threads', '1', '--seconds', '3', '--textlog')
foreach ($op in 'restore', 'clone', 'reset', 'leaf') {
    Invoke-Checked (Join-Path $gen 'search_bench.exe') @($Corpus, '--op', $op, '--seconds', '2')
}
if ($Maturin) {
    Invoke-Checked $Python @('tools/pgo_wheel_workload.py', (Join-Path $Out 'wheels-gen'), (Join-Path $Out 'wheel-gen'), '10', $Pool)
}

# 3. Merge.
Invoke-Checked $profdata @('merge', '-o', $merged, $raw)

# 4. Optimized build. Profile mismatch warnings for code the workload never runs are harmless.
$env:CARGO_ENCODED_RUSTFLAGS = "-Cprofile-use=$merged"
Invoke-Checked cargo ($build + @('--target-dir', (Join-Path $Out 'target-use')))
"PGO binaries: $(Join-Path $Out "target-use\$target\release")"

# 5. Optional wheel, never installed anywhere.
if ($Maturin) {
    $env:RUSTUP_TOOLCHAIN = $toolchain
    Invoke-Checked $Maturin ($wheelBuild + @('--target-dir', (Join-Path $Out 'target-use'), '-o', (Join-Path $Out 'wheels')))
    "PGO wheel: $(Join-Path $Out 'wheels')"
    Remove-Item Env:RUSTUP_TOOLCHAIN
}
Remove-Item Env:CARGO_ENCODED_RUSTFLAGS
