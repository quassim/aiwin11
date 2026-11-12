<#
.SYNOPSIS
  Measures the release desktop app on this machine: time until the main window exists, first paint and
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
  powershell -ExecutionPolicy Bypass -File scripts/measure.ps1 -Runs 3 -NavigationSamples 20
#>
param(
    [ValidateRange(1, 30)][int]$Runs = 3,
    [ValidateRange(0, 120)][int]$IdleSeconds = 15,
    [ValidateRange(0, 100)][int]$NavigationSamples = 0
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\release\aiwin11-desktop.exe'
if (-not (Test-Path $exe)) { throw "Missing $exe : build the release app first." }
if (Get-Process -Name aiwin11-desktop, AiWin11 -ErrorAction SilentlyContinue) {
    throw 'AiWin11 is already running: close it yourself before measuring.'
}

$store = Join-Path $env:LOCALAPPDATA 'app.aiwin11.desktop\aiwin11.sqlite3'
$python = (Get-Command python.exe -CommandType Application | Select-Object -First 1).Source
if ($NavigationSamples -gt 0) {
    Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
}

function Read-Metrics([long]$after) {
    $query = @'
import json, sqlite3, sys
from pathlib import Path
try:
    with sqlite3.connect(Path(sys.argv[1]).resolve().as_uri() + '?mode=ro', uri=True) as db:
        last = db.execute('SELECT COALESCE(MAX(sequence), 0) FROM events').fetchone()[0]
        session = db.execute('SELECT session_id FROM events WHERE kind = ? AND sequence > ? ORDER BY sequence DESC LIMIT 1',
                             ('session.started', int(sys.argv[2]))).fetchone()
        rows = db.execute('SELECT kind, code, timestamp FROM events WHERE session_id = ? AND kind IN (?, ?) ORDER BY sequence',
                          (session[0], 'ui.ready', 'ui.navigation')).fetchall() if session else []
except sqlite3.Error:
    last, rows = 0, []
ready = next(((int(code[3:]), stamp) for kind, code, stamp in rows
              if kind == 'ui.ready' and code and code[3:].isdigit()), (None, None))
print(json.dumps({'last': last, 'ready': ready[0], 'readyAt': ready[1],
                  'navigation': [int(code[3:]) for kind, code, _ in rows
                                 if kind == 'ui.navigation' and code and code[3:].isdigit()]}))
'@
    $result = & $python -B -c $query $store $after
    if ($LASTEXITCODE -ne 0) { throw 'Could not read the performance journal.' }
    return $result | ConvertFrom-Json
}

function Invoke-Navigation([IntPtr]$window, [string]$name) {
    $rootElement = [System.Windows.Automation.AutomationElement]::FromHandle($window)
    $nameCondition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $name)
    $buttonCondition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $target = $rootElement.FindFirst(
        [System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.AndCondition]::new($nameCondition, $buttonCondition))
    if ($null -eq $target) { throw "Navigation button unavailable: $name" }
    $target.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
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
$navigation = @()
for ($run = 1; $run -le $Runs; $run++) {
    $baseline = (Read-Metrics 0).last
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $exe -PassThru
    try {
        $windowMs = $null
        while ($watch.Elapsed.TotalSeconds -lt 30) {
            $process.Refresh()
            if ($process.HasExited) { throw "AiWin11 exited before opening its window (code $($process.ExitCode))." }
            if ($process.MainWindowHandle -ne [IntPtr]::Zero) { $windowMs = $watch.ElapsedMilliseconds; break }
            Start-Sleep -Milliseconds 20
        }
        do {
            $metrics = Read-Metrics $baseline
            if ($null -ne $metrics.ready -or $process.HasExited) { break }
            Start-Sleep -Milliseconds 100
        } while ($watch.Elapsed.TotalSeconds -lt 30)
        if ($null -eq $metrics.ready) { throw "The first render was not recorded on run $run." }
        $readyAt = [DateTimeOffset]::Parse($metrics.readyAt, [Globalization.CultureInfo]::InvariantCulture)
        $readyFromStart = ($readyAt.UtcDateTime - $process.StartTime.ToUniversalTime()).TotalMilliseconds
        Start-Sleep -Seconds $IdleSeconds
        $tree = Get-Tree $process.Id
        $processes = @(Get-Process -Id $tree -ErrorAction SilentlyContinue)
        $private = ($processes | Measure-Object -Property PrivateMemorySize64 -Sum).Sum / 1MB
        $working = ($processes | Measure-Object -Property WorkingSet64 -Sum).Sum / 1MB
        $rows += [pscustomobject]@{
            Run = $run
            WindowMs = $windowMs
            ReadyMs = $metrics.ready
            ReadyFromStartMs = [math]::Round($readyFromStart)
            Processes = $processes.Count
            PrivateMB = [math]::Round($private, 1)
            WorkingSetMB = [math]::Round($working, 1)
        }
        for ($sample = 0; $sample -lt $NavigationSamples; $sample++) {
            $count = $metrics.navigation.Count
            $button = if ($sample % 2 -eq 0) { "Mod$([char]0x00e8)les" } else { 'Accueil' }
            Invoke-Navigation $process.MainWindowHandle $button
            $wait = [Diagnostics.Stopwatch]::StartNew()
            do {
                Start-Sleep -Milliseconds 100
                $metrics = Read-Metrics $baseline
            } while ($metrics.navigation.Count -le $count -and $wait.Elapsed.TotalSeconds -lt 5)
            if ($metrics.navigation.Count -le $count) { throw "Navigation to $button did not produce a render measurement." }
            $navigation += $metrics.navigation[$count]
        }
    } finally {
        if (!$process.HasExited) {
            $null = $process.CloseMainWindow()
            if (-not $process.WaitForExit(5000)) { Stop-Process -InputObject $process -Force }
        }
    }
    Start-Sleep -Seconds 2
}
$rows | Format-Table -AutoSize
if ($navigation.Count -gt 0) {
    $ordered = @($navigation | Sort-Object)
    [pscustomobject]@{
        NavigationSamples = $ordered.Count
        NavigationP95Ms = $ordered[[int][math]::Ceiling($ordered.Count * 0.95) - 1]
    } | Format-Table -AutoSize
}
