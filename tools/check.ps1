param([string]$ShowdownPath = $env:NUMBRION_SHOWDOWN)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$savedPath = $env:Path
$savedOracle = $env:NUMBRION_SHOWDOWN

function Invoke-Checked {
    param([string]$Executable, [string[]]$CommandArguments)
    & $Executable @CommandArguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Executable $($CommandArguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Push-Location $repoRoot
try {
    # Rustup and the local Node 24 installation may be newer than this shell's PATH.
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (Test-Path -LiteralPath (Join-Path $cargoBin 'cargo.exe')) {
        $env:Path = $cargoBin + ';' + $env:Path
    }
    $localTools = Join-Path $env:LOCALAPPDATA 'Programs\numbrion-tools'
    if (Test-Path -LiteralPath $localTools) {
        $localNode = Get-ChildItem -LiteralPath $localTools -Directory -Filter 'node-v24.*-win-x64' |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($localNode) {
            $env:Path = $localNode.FullName + ';' + $env:Path
        }
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'Install Rust using rustup before running checks.'
    }
    if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
        throw 'Install Node 24 before running checks.'
    }
    $nodeVersion = [version]((& node --version).TrimStart('v'))
    if ($nodeVersion -lt [version]'22.18.0') {
        throw "Node $nodeVersion is too old for the pinned Showdown source; install Node 24."
    }
    if (-not $ShowdownPath) {
        $ShowdownPath = Join-Path $repoRoot 'scratch\pokemon-showdown'
    }
    $env:NUMBRION_SHOWDOWN = (Resolve-Path -LiteralPath $ShowdownPath).Path
    if (-not (Test-Path -LiteralPath (Join-Path $env:NUMBRION_SHOWDOWN 'dist\sim\index.js'))) {
        throw 'Build the pinned Showdown checkout with npm ci and npm run build first.'
    }

    Invoke-Checked cargo @('build', '-p', 'engine', '--all-features', '--locked', '--release')
    Invoke-Checked cargo @('build', '--manifest-path', 'crates/difftest/Cargo.toml', '--all-features', '--locked', '--release')
    Invoke-Checked cargo @('test', '-p', 'engine', '--all-features', '--locked')
    Invoke-Checked cargo @('test', '-p', 'pyengine', '--release', '--locked')
    Invoke-Checked cargo @('test', '--manifest-path', 'crates/difftest/Cargo.toml', '--all-features', '--locked')
    $difftest = Join-Path $repoRoot 'crates\difftest\target\release\difftest.exe'
    foreach ($fixture in @('sample-50', 'directed-smoke')) {
        Invoke-Checked $difftest @('replay', "data/fixtures/$fixture.jsonl.gz", '--sim', 'engine', '--quiet')
    }
    Invoke-Checked $difftest @('selftest', 'data/fixtures/sample-50.jsonl.gz')
}
finally {
    $env:Path = $savedPath
    $env:NUMBRION_SHOWDOWN = $savedOracle
    Pop-Location
}
