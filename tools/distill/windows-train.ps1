param(
    [ValidateSet('start', 'stop', 'status', 'supervise')][string]$Action = 'status',
    [string]$Resume = '',
    [string]$RepoPath = '',
    [string]$TrainerDirectory = '',
    [string]$LabelDirectory = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = 'C:\Users\aminu\.t3\projects\numbrion'
if ($env:DISTILL_WINDOWS_REPO) { $Repo = $env:DISTILL_WINDOWS_REPO }
if ($RepoPath) { $Repo = $RepoPath }
$Output = Join-Path $Repo 'scratch\distill-loop\train'
$Labels = Join-Path $Repo 'scratch\distill-loop\labels'
if ($env:DISTILL_WINDOWS_TRAIN) { $Output = $env:DISTILL_WINDOWS_TRAIN }
if ($env:DISTILL_WINDOWS_LABELS) { $Labels = $env:DISTILL_WINDOWS_LABELS }
if ($TrainerDirectory) { $Output = $TrainerDirectory }
if ($LabelDirectory) { $Labels = $LabelDirectory }
$Python = Join-Path $Repo '.venv\Scripts\python.exe'
$Trainer = Join-Path $Repo 'crates\pyengine\train_selfplay.py'
$Flags = @(
    '--width', '192', '--layers', '4', '--minutes', 'inf', '--schedule-minutes', '120',
    '--lr', '3e-4', '--lr-final', '0.33', '--entropy-target', '2.5', '--entropy-target-final', '1.2',
    '--teacher-coef', '0.2', '--teacher-end-minutes', '36.5', '--ko-bonus-end', '0.15',
    '--pool-minutes', '3', '--seed', '42', '--pool', 'data/teams/train-s42-2000.txt',
    '--distill-stream', $Labels, '--distill-coef', '0.5', '--distill-end-minutes', 'inf',
    '--distill-batch', '128', '--distill-max-batches', '4', '--output', $Output
)
$SupervisorPid = Join-Path $Output 'supervisor.pid'
$TrainerPid = Join-Path $Output 'trainer.pid'
$StopFile = Join-Path $Output 'STOP'
$Log = Join-Path $Output 'trainer.log'
$LockPath = Join-Path $Output 'supervisor.lock'
$LaunchLockPath = Join-Path $Output 'launch.lock'
$Latest = Join-Path $Output 'latest.pt'
$Hash = [System.Security.Cryptography.SHA256]::Create()
$TaskId = ([BitConverter]::ToString($Hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($Output)))).Replace('-', '').Substring(0, 12)
$Hash.Dispose()
$TaskName = 'NumbrionDistill-' + $TaskId
New-Item -ItemType Directory -Force -Path $Output, $Labels | Out-Null

function Quote-Argument([string]$Value) {
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Read-Process([string]$Path) {
    if (Test-Path -LiteralPath $Path) {
        $ProcessId = 0
        if ([int]::TryParse((Get-Content -LiteralPath $Path -Raw).Trim(), [ref]$ProcessId)) {
            return Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
        }
    }
    return $null
}

function Open-Lock([string]$Path) {
    return [IO.File]::Open($Path, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
}

if ($Action -eq 'start') {
    $LaunchLock = $null
    try { $LaunchLock = Open-Lock $LaunchLockPath }
    catch { throw 'Another start command is running.' }
    try {
        $Supervisor = Read-Process $SupervisorPid
        $Training = Read-Process $TrainerPid
        $Task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
        if ($Supervisor -or ($Task -and (($Task.State -eq 'Running') -or ($Task.State -eq 'Queued')))) {
            throw 'Supervisor already running. Use -Action status.'
        }
        if ($Training) { throw 'Trainer still running. Use -Action stop and wait for it to exit.' }
        $CheckLock = $null
        try { $CheckLock = Open-Lock $LockPath }
        catch { throw 'Supervisor lock is held. Use -Action status.' }
        finally { if ($CheckLock) { $CheckLock.Dispose() } }
        if (!(Test-Path -LiteralPath $Latest) -and (!$Resume -or !(Test-Path -LiteralPath $Resume))) {
            throw 'Supply -Resume with the first checkpoint, or provide output\latest.pt.'
        }
        if (!(Test-Path -LiteralPath $Python)) { throw "Python missing: $Python" }
        Remove-Item -LiteralPath $StopFile -ErrorAction SilentlyContinue
        $Arguments = '-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File ' + (Quote-Argument $PSCommandPath) + ' -Action supervise'
        $Arguments += ' -RepoPath ' + (Quote-Argument $Repo) + ' -TrainerDirectory ' + (Quote-Argument $Output) + ' -LabelDirectory ' + (Quote-Argument $Labels)
        if ($Resume) { $Arguments += ' -Resume ' + (Quote-Argument $Resume) }
        $TaskAction = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $Arguments -WorkingDirectory $Repo
        $User = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        $Principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Limited
        $Settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        Register-ScheduledTask -TaskName $TaskName -Action $TaskAction -Principal $Principal -Settings $Settings -Force | Out-Null
        Start-ScheduledTask -TaskName $TaskName
        $Deadline = (Get-Date).AddSeconds(15)
        while (!(Read-Process $SupervisorPid) -and ((Get-Date) -lt $Deadline)) { Start-Sleep -Milliseconds 500 }
        if (!(Read-Process $SupervisorPid)) { throw "Task did not start. Keep the Windows user logged in; inspect task $TaskName and $Log." }
        Write-Output "Started task $TaskName, supervisor pid=$((Read-Process $SupervisorPid).Id)"
    } finally { $LaunchLock.Dispose() }
    exit 0
}

if ($Action -eq 'stop') {
    New-Item -ItemType File -Force -Path $StopFile | Out-Null
    Write-Output 'STOP created; trainer saves at the next update boundary and supervisor exits.'
    exit 0
}

if ($Action -eq 'status') {
    $Supervisor = Read-Process $SupervisorPid
    $Training = Read-Process $TrainerPid
    $Task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    Write-Output "supervisor pid=$($Supervisor.Id) trainer pid=$($Training.Id) task=$TaskName state=$($Task.State)"
    $Metrics = Join-Path $Output 'metrics.jsonl'
    if (Test-Path -LiteralPath $Metrics) {
        $Line = Get-Content -LiteralPath $Metrics -Tail 1
        Write-Output $Line
        try {
            $Metric = $Line | ConvertFrom-Json
            Write-Output "update=$($Metric.update) minutes=$($Metric.minutes) distill_fresh=$($Metric.distill_fresh) distill_used=$($Metric.distill_used) distill_teacher_lag=$($Metric.distill_teacher_lag)"
        } catch { Write-Output 'Latest metrics line is incomplete.' }
    }
    foreach ($Dir in @($Labels, (Join-Path $Labels 'used'), (Join-Path $Labels 'heldout'))) {
        $Count = @(Get-ChildItem -LiteralPath $Dir -Filter 'labels-*.npz' -File -ErrorAction SilentlyContinue).Count
        Write-Output "$Dir shards=$Count"
    }
    $PartCount = @(Get-ChildItem -LiteralPath $Labels -Filter '*.part' -File -ErrorAction SilentlyContinue).Count
    Write-Output "arriving parts=$PartCount"
    exit 0
}

$Lock = $null
try { $Lock = Open-Lock $LockPath }
catch { Write-Error 'Another supervisor holds the lock.'; exit 1 }
try {
    Set-Location -LiteralPath $Repo
    Set-Content -LiteralPath $SupervisorPid -Value $PID
    while (!(Test-Path -LiteralPath $StopFile)) {
        $Checkpoint = $Resume
        if (Test-Path -LiteralPath $Latest) { $Checkpoint = $Latest }
        if (!$Checkpoint -or !(Test-Path -LiteralPath $Checkpoint)) { throw 'Resume checkpoint missing.' }
        $RunFlags = @('-u', $Trainer) + $Flags + @('--resume', $Checkpoint)
        $Info = New-Object System.Diagnostics.ProcessStartInfo
        $Info.FileName = $Python
        $Info.Arguments = ($RunFlags | ForEach-Object { Quote-Argument $_ }) -join ' '
        $Info.WorkingDirectory = $Repo
        $Info.UseShellExecute = $false
        $Info.CreateNoWindow = $true
        $Info.RedirectStandardOutput = $true
        $Info.RedirectStandardError = $true
        $Process = New-Object System.Diagnostics.Process
        $Process.StartInfo = $Info
        # File streams keep both pipes draining even while the trainer emits nothing.
        $Stdout = [IO.File]::Open($Log, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::Read)
        $Stderr = [IO.File]::Open(($Log + '.err'), [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::Read)
        try {
            [void]$Process.Start()
            Set-Content -LiteralPath $TrainerPid -Value $Process.Id
            $OutCopy = $Process.StandardOutput.BaseStream.CopyToAsync($Stdout)
            $ErrCopy = $Process.StandardError.BaseStream.CopyToAsync($Stderr)
            $Process.WaitForExit()
            $OutCopy.GetAwaiter().GetResult()
            $ErrCopy.GetAwaiter().GetResult()
        } finally {
            $Stdout.Dispose()
            $Stderr.Dispose()
            Remove-Item -LiteralPath $TrainerPid -ErrorAction SilentlyContinue
            $Process.Dispose()
        }
        if (Test-Path -LiteralPath $StopFile) { break }
        for ($Wait = 0; $Wait -lt 30; $Wait++) {
            if (Test-Path -LiteralPath $StopFile) { break }
            Start-Sleep -Seconds 1
        }
    }
} finally {
    Remove-Item -LiteralPath $SupervisorPid -ErrorAction SilentlyContinue
    $Lock.Dispose()
}
