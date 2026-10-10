param([string]$Python = 'python', [int]$Teams = 2000)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$savedPath = $env:Path
$savedVenv = $env:VIRTUAL_ENV

function Invoke-Checked {
    param([string]$Executable, [string[]]$CommandArguments)
    & $Executable @CommandArguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Executable $($CommandArguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

if ($Teams -lt 1) { throw '-Teams must be positive.' }
Push-Location $repoRoot
try {
    $env:Path = (Join-Path $env:USERPROFILE '.cargo\bin') + ';' + $env:Path
    $localTools = Join-Path $env:LOCALAPPDATA 'Programs\numbrion-tools'
    if (Test-Path -LiteralPath $localTools) {
        $node = Get-ChildItem -LiteralPath $localTools -Directory -Filter 'node-v24.*-win-x64' |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($node) { $env:Path = $node.FullName + ';' + $env:Path }
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'Install stable Rust and a matching Windows linker first; see docs/training/WINDOWS-GPU.md.'
    }
    if (-not (Test-Path -LiteralPath .venv\Scripts\python.exe)) {
        Invoke-Checked $Python @('-m', 'venv', '.venv')
    }
    $pythonExe = Join-Path $repoRoot '.venv\Scripts\python.exe'
    $pythonVersion = [version]((& $pythonExe --version) -replace '^Python ', '')
    if ($pythonVersion -lt [version]'3.9' -or $pythonVersion -ge [version]'3.13') {
        throw 'torch-directml requires a Python 3.9-3.12 environment. Use Python 3.12 for this setup.'
    }
    $env:VIRTUAL_ENV = Join-Path $repoRoot '.venv'
    $env:Path = (Join-Path $env:VIRTUAL_ENV 'Scripts') + ';' + $env:Path
    Invoke-Checked $pythonExe @('-m', 'pip', 'install', '-r', 'crates/pyengine/requirements-directml.txt')
    Invoke-Checked maturin @('develop', '--release', '--manifest-path', 'crates/pyengine/Cargo.toml', '--locked')
    Invoke-Checked maturin @('build', '--release', '--manifest-path', 'crates/pyengine/Cargo.toml', '--locked')
    Invoke-Checked $pythonExe @('-m', 'pytest', 'crates/pyengine/pytests', 'crates/pyengine/training_tests', '-q')
    Invoke-Checked $pythonExe @('crates/pyengine/gpu_check.py', '--device', 'directml', '--output', 'scratch/training/gpu-check.json')

    $pool = Join-Path $repoRoot "data\teams\train-s42-$Teams.txt"
    if (-not (Test-Path -LiteralPath $pool)) {
        $oracle = Join-Path $repoRoot 'scratch\pokemon-showdown'
        if (-not (Test-Path -LiteralPath (Join-Path $oracle 'dist\sim\index.js'))) {
            throw 'Build the pinned Showdown checkout first (README.md); it generates fresh training teams.'
        }
        New-Item -ItemType Directory -Path data\teams -Force | Out-Null
        Invoke-Checked node @('tools/oracle/gen-teams.mjs', $oracle, '--count', "$Teams", '--seed', '42', '--out', $pool)
    }
    Write-Output "Ready: .venv\Scripts\python.exe crates\pyengine\train.py --device directml --pool $pool"
}
finally {
    $env:Path = $savedPath
    $env:VIRTUAL_ENV = $savedVenv
    Pop-Location
}
