<#
.SYNOPSIS
  Runs the AiWin11 checks locally, with optional interactive tests and a release build.

.PARAMETER Live
  Also runs the live tests (Ollama, the published bridge and agent, an interactive desktop).
  Tests that open Notepad are skipped when a Notepad window is already open.

.PARAMETER Tauri
  Also builds the release desktop executable (slower).

.PARAMETER Ipc
  Runs the Windows gRPC named-pipe transport probe.

.PARAMETER CloudVendor
  Runs T-003 with the selected provider and the saved Windows credential.
  Sends one synthetic prompt and may use paid tokens. Requires CloudModel.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts/verify.ps1
  powershell -ExecutionPolicy Bypass -File scripts/verify.ps1 -Live -Tauri
#>
param(
    [switch]$Live,
    [switch]$Tauri,
    [switch]$Ipc,
    [ValidateSet('openai', 'deepseek', 'gemini', 'anthropic')][string]$CloudVendor,
    [string]$CloudModel,
    [string]$Model = 'qwen3:8b',
    [string]$VisionModel = 'gemma3:4b'
)

$ErrorActionPreference = 'Stop'
if ([bool]$CloudVendor -ne [bool]$CloudModel) { throw 'Pass CloudVendor and CloudModel together for T-003.' }
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:Path = "$env:ProgramFiles\nodejs;$env:Path"
$results = [System.Collections.Generic.List[object]]::new()

function Step([string]$name, [scriptblock]$command) {
    Write-Host "`n=== $name ===" -ForegroundColor Cyan
    $watch = [Diagnostics.Stopwatch]::StartNew()
    & $command
    $ok = $LASTEXITCODE -eq 0
    $results.Add([pscustomobject]@{ Step = $name; Result = $(if ($ok) { 'OK' } else { 'ECHEC' }); Seconds = [int]$watch.Elapsed.TotalSeconds })
    if (-not $ok) { Write-Host "$name : ECHEC (code $LASTEXITCODE)" -ForegroundColor Red }
}

Step 'Agent Python' { python -B services/agent/test_agent.py }
$venvPython = Join-Path $root '.venv\Scripts\python.exe'
if (Test-Path $venvPython) {
    Step 'Ruff (lint + format)' {
        & $venvPython -m ruff check services/agent
        if ($LASTEXITCODE -eq 0) { & $venvPython -m ruff format --check services/agent }
    }
    Step 'Mypy (strict)' { & $venvPython -m mypy --config-file services/agent/pyproject.toml services/agent }
} else {
    Write-Host 'Ruff/mypy sautés : créez .venv (voir services/agent/requirements-dev.txt).' -ForegroundColor Yellow
    $results.Add([pscustomobject]@{ Step = 'Ruff + mypy'; Result = 'SAUTÉ'; Seconds = 0 })
}
Step 'ESLint + Prettier' { npm --prefix apps/desktop run lint }
Step 'Interface (tsc + vite)' { npm --prefix apps/desktop run build }
Step 'Tests de composants (Vitest)' { npm --prefix apps/desktop run test }
Step 'cargo fmt --check' { cargo fmt --all --check }
Step 'Pont .NET' { dotnet build services/windows/AiWin11.Windows.csproj --configuration Release -nologo -v q }
Step 'Assistant administrateur .NET' { dotnet build services/admin/AiWin11.Admin.csproj --configuration Release -nologo -v q }
Step 'Tests Rust' { cargo test --locked -p aiwin11-core }
Step 'Clippy' { cargo clippy --locked --workspace --all-targets -- -D warnings }

if ($Ipc) {
    Step 'Transport gRPC Windows' { dotnet run --project services/ipc-probe/AiWin11.IpcProbe.csproj --configuration Release }
    Step 'Services gRPC Rust' { cargo test --locked -p aiwin11-model-ipc }
}

if ($CloudVendor) {
    $env:AIWIN11_TEST_CLOUD_VENDOR = $CloudVendor
    $env:AIWIN11_TEST_CLOUD_MODEL = $CloudModel
    try {
        Step 'T-003 fournisseur cloud réel' {
            cargo test --locked -p aiwin11-model-ipc grpc_catalog_contract_in_another_process
        }
    } finally {
        Remove-Item Env:AIWIN11_TEST_CLOUD_VENDOR, Env:AIWIN11_TEST_CLOUD_MODEL
    }
}

if ($Live) {
    $env:AIWIN11_TEST_MODEL = $Model
    $env:AIWIN11_TEST_VISION_MODEL = $VisionModel
    $env:AIWIN11_PYTHON = (Get-Command python.exe -CommandType Application | Select-Object -First 1).Source
    Step 'Publication du pont pour les tests' {
        dotnet publish services/windows/AiWin11.Windows.csproj --configuration Debug --runtime win-x64 `
            --self-contained false --output target/debug/deps -nologo -v q
    }
    Copy-Item -LiteralPath services/agent/agent.py -Destination target/debug/deps/aiwin11_agent.py -Force
    $skip = @()
    if (@(Get-Process notepad -ErrorAction SilentlyContinue).Count -gt 0) {
        Write-Host 'Un Bloc-notes est ouvert : les tests qui ouvrent le Bloc-notes sont sautés.' -ForegroundColor Yellow
        $skip = @('--skip', 'opens_notepad')
    }
    Step 'Tests réels' { cargo test --locked -p aiwin11-core -- --ignored --test-threads=1 @skip }
}

if ($Tauri) {
    Step 'Build Tauri (release)' { npm --prefix apps/desktop run tauri -- build --no-bundle }
}

Write-Host "`n=== Résumé ===" -ForegroundColor Cyan
$results | Format-Table -AutoSize
if ($results.Result -contains 'ECHEC') { exit 1 }
exit 0
