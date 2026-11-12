<#
.SYNOPSIS
  Measures the release desktop app on this machine: time until the main window exists and
  idle memory (private bytes and working set) of the app and the processes it started
  (WebView2, spare Windows bridge), excluding models.

.DESCRIPTION
  Build first: npm --prefix apps/desktop run tauri -- build --no-bundle
  The script refuses to run if AiWin11 is already open, never touches another application,
  and closes only the instance it started. The started instance uses the normal data folder
  (one more session in the journal).
  The permission-decision latency is measured separately:
  cargo test --locked -p aiwin11-core --release measure_ -- --ignored --nocapture

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts/measure.ps1 -Runs 3
#>
param(
    [int]$Runs = 3,
    [int]$IdleSeconds = 15
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\release\aiwin11-desktop.exe'
if (-not (Test-Path $exe)) { throw "Missing $exe : build the release app first." }
if (Get-Process aiwin11-desktop -ErrorAction SilentlyContinue) {
    throw 'AiWin11 is already running: close it yourself before measuring.'
}

function Get-Tree([int]$rootId) {
    $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
    $ids = [System.Collections.Generic.List[int]]::new()
    $ids.Add($rootId)
    $index = 0
    while ($index -lt $ids.Count) {
        $parent = $ids[$index]
        foreach ($child in $all | Where-Object { $_.ParentProcessId -eq $parent }) { $ids.Add([int]$child.ProcessId) }
        $index++
    }
    return $ids
}

$rows = @()
for ($run = 1; $run -le $Runs; $run++) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $exe -PassThru
    $windowMs = $null
    while ($watch.Elapsed.TotalSeconds -lt 30) {
        $process.Refresh()
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) { $windowMs = $watch.ElapsedMilliseconds; break }
        Start-Sleep -Milliseconds 20
    }
    Start-Sleep -Seconds $IdleSeconds
    $tree = Get-Tree $process.Id
    $processes = @(Get-Process -Id $tree -ErrorAction SilentlyContinue)
    $private = ($processes | Measure-Object -Property PrivateMemorySize64 -Sum).Sum / 1MB
    $working = ($processes | Measure-Object -Property WorkingSet64 -Sum).Sum / 1MB
    $rows += [pscustomobject]@{
        Run = $run
        WindowMs = $windowMs
        Processes = $processes.Count
        PrivateMB = [math]::Round($private, 1)
        WorkingSetMB = [math]::Round($working, 1)
    }
    # Close only what this script started: the app first, then its leftover children.
    $null = $process.CloseMainWindow()
    if (-not $process.WaitForExit(5000)) { Stop-Process -Id $process.Id -Force }
    foreach ($id in $tree) {
        if ($id -ne $process.Id) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
    }
    Start-Sleep -Seconds 2
}
$rows | Format-Table -AutoSize
