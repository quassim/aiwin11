# Builds what is needed and starts AiWin11 in development mode.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:Path = "$env:ProgramFiles\nodejs;$env:Path"

if (-not (Test-Path 'apps\desktop\node_modules')) { npm --prefix apps/desktop ci }
foreach ($project in 'services/windows/AiWin11.Windows.csproj', 'services/admin/AiWin11.Admin.csproj') {
    dotnet publish $project --configuration Debug --runtime win-x64 --self-contained false --output target/debug -nologo -v q
    if ($LASTEXITCODE -ne 0) { throw "Build failed: $project" }
}
Copy-Item -LiteralPath services/agent/agent.py -Destination target/debug/aiwin11_agent.py -Force
npm --prefix apps/desktop run tauri -- dev
