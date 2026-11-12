# Builds the AiWin11 Windows installer (NSIS, per-user, no administrator rights needed).
# The installer embeds the Windows bridge, the elevated helper and the Python agent next to
# AiWin11.exe. Python 3.13, the .NET 10 runtime and Ollama must still be installed separately.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:Path = "$env:ProgramFiles\nodejs;$env:Path"
$companions = Join-Path $root 'target\companions'
if (Test-Path $companions) { Remove-Item -Recurse -Force $companions }
New-Item -ItemType Directory -Force $companions | Out-Null

dotnet publish services/windows/AiWin11.Windows.csproj --configuration Release --runtime win-x64 --self-contained false --output $companions -nologo -v q
if ($LASTEXITCODE -ne 0) { throw 'Build failed: bridge' }
# The elevated helper is one file, so its pinned SHA-256 covers all of its code.
dotnet publish services/admin/AiWin11.Admin.csproj --configuration Release --runtime win-x64 --self-contained false `
    -p:PublishSingleFile=true --output $companions -nologo -v q
if ($LASTEXITCODE -ne 0) { throw 'Build failed: admin helper' }
Copy-Item -LiteralPath services/agent/agent.py -Destination (Join-Path $companions 'aiwin11_agent.py')
# Pin the elevated helper: AiWin11 refuses to start a helper whose SHA-256 differs.
$env:AIWIN11_ADMIN_SHA256 = (Get-FileHash -Algorithm SHA256 (Join-Path $companions 'AiWin11.Admin.exe')).Hash.ToLowerInvariant()
# Debug symbols are not shipped.
Get-ChildItem $companions -Filter *.pdb | Remove-Item -Force

if (-not (Test-Path 'apps\desktop\node_modules')) { npm --prefix apps/desktop ci }
npm --prefix apps/desktop run tauri -- build --config src-tauri/tauri.bundle.json
if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
Get-ChildItem (Join-Path $root 'target\release\bundle\nsis') -Filter *.exe | ForEach-Object { Write-Host "Installateur : $($_.FullName)" }
