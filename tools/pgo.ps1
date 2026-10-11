param(
    # Training battles; must not be the benchmark corpus.
    [Parameter(Mandatory)][string]$Corpus,
    [string]$Out = 'scratch/pgo',
    [ValidateRange(1, 2147483647)][int]$TrainSeconds = 20,
    [ValidateRange(1, 2147483647)][int]$Jobs = 4,
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
$savedEnvironment = @{}
foreach ($name in 'Path', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTUP_TOOLCHAIN', 'LLVM_PROFILE_FILE') {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Exe $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

function Reset-GeneratedDirectory([string]$Directory) {
    $fullPath = [IO.Path]::GetFullPath($Directory)
    $outPrefix = $Out.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if (-not $fullPath.StartsWith($outPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Generated directory must be inside the output directory: $fullPath"
    }
    if (Test-Path -LiteralPath $fullPath) {
        Remove-Item -LiteralPath $fullPath -Recurse -Force
    }
}

Push-Location $repoRoot
try {
    $env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
    $env:RUSTUP_TOOLCHAIN = $toolchain
    $sysroot = (Invoke-Checked rustup @('run', $toolchain, 'rustc', '--print', 'sysroot')).Trim()
    $profdata = Join-Path $sysroot "lib\rustlib\$target\bin\llvm-profdata.exe"
    if (-not (Test-Path -LiteralPath $profdata -PathType Leaf)) {
        throw "llvm-profdata not found; run: rustup component add llvm-tools --toolchain $toolchain"
    }
    $Corpus = (Resolve-Path -LiteralPath $Corpus).Path
    if (-not (Test-Path -LiteralPath $Corpus -PathType Leaf)) { throw "Training corpus must be a file: $Corpus" }
    if ($Maturin) {
        $Maturin = (Get-Command $Maturin -ErrorAction Stop).Source
        $Python = (Get-Command $Python -ErrorAction Stop).Source
        Invoke-Checked $Python @('-c', 'import sys; sys.exit(sys.version_info < (3, 9))')
        $Pool = (Resolve-Path -LiteralPath $Pool).Path
        if (-not (Test-Path -LiteralPath $Pool -PathType Leaf)) { throw "Team pool must be a file: $Pool" }
    }
    New-Item -ItemType Directory -Force -Path $Out | Out-Null
    $Out = (Resolve-Path -LiteralPath $Out).Path
    $raw = Join-Path $Out 'profraw'
    $merged = Join-Path $Out 'merged.profdata'
    $manifest = 'crates/difftest/Cargo.toml'
    $build = @("+$toolchain", 'build', '--manifest-path', $manifest, '--release', '--locked', '-j', "$Jobs", '--bins', '--target', $target)
    $wheelBuild = @('build', '--release', '--locked', '-m', 'crates/pyengine/Cargo.toml', '--target', $target, '-j', "$Jobs", '--interpreter', $Python)
    if ($Search) {
        $build += @('--features', 'engine/search')
        $wheelBuild += @('--features', 'python,search')
    }

    # 1. Instrumented build. Encoded flags keep a profile path with spaces in one argument.
    #    --target leaves build scripts uninstrumented. Override any inherited profile destination.
    Reset-GeneratedDirectory $raw
    New-Item -ItemType Directory -Path $raw | Out-Null
    $env:LLVM_PROFILE_FILE = Join-Path $raw 'default_%m_%p.profraw'
    $env:CARGO_ENCODED_RUSTFLAGS = "-Cprofile-generate=$raw"
    Invoke-Checked cargo ($build + @('--target-dir', (Join-Path $Out 'target-gen')))
    $gen = Join-Path $Out "target-gen\$target\release"
    # The wheel profiles itself: maturin's cdylib symbols differ from plain cargo builds (src/pgo.rs).
    if ($Maturin) {
        Reset-GeneratedDirectory (Join-Path $Out 'wheels-gen')
        Invoke-Checked $Maturin ($wheelBuild + @('--target-dir', (Join-Path $Out 'target-gen'), '-o', (Join-Path $Out 'wheels-gen')))
    }

    # 2. Train NoLog and TextLog replays, search primitives and the instrumented wheel's
    #    BatchEnv, GridExecutor, clone/restore/replace_hidden_set workload.
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

    # 4. Optimized build. Warnings for code the workload never runs are harmless.
    $env:CARGO_ENCODED_RUSTFLAGS = "-Cprofile-use=$merged"
    Invoke-Checked cargo ($build + @('--target-dir', (Join-Path $Out 'target-use')))
    "PGO binaries: $(Join-Path $Out "target-use\$target\release")"

    # 5. Optional wheel, never installed anywhere.
    if ($Maturin) {
        Invoke-Checked $Maturin ($wheelBuild + @('--target-dir', (Join-Path $Out 'target-use'), '-o', (Join-Path $Out 'wheels')))
        "PGO wheel: $(Join-Path $Out 'wheels')"
    }
}
finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
    Pop-Location
}
