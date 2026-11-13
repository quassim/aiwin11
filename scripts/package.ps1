# Builds the AiWin11 Windows installer (NSIS, per-user, no administrator rights needed).
# The installer embeds the Windows bridge, the elevated helper (self-contained .NET 10 by default)
# and the Python agent next to AiWin11.exe. Pass -PythonRuntimeZip or -BundlePython to bundle
# the Python 3.13 x64 embeddable runtime.
param(
    [string]$PythonRuntimeZip,
    [switch]$BundlePython,
    [switch]$FrameworkDependentDotnet,
    [string]$UpdatePublicKeyFile
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$pythonVersion = '3.13.3'
$pythonArchive = if ($PythonRuntimeZip) {
    (Resolve-Path -LiteralPath $PythonRuntimeZip).ProviderPath
} elseif ($BundlePython) {
    $toolsDir = Join-Path $root '.tools'
    New-Item -ItemType Directory -Force $toolsDir | Out-Null
    $cachedZip = Join-Path $toolsDir "python-${pythonVersion}-embed-amd64.zip"
    $baseUrl = "https://www.python.org/ftp/python/${pythonVersion}/python-${pythonVersion}-embed-amd64.zip"
    $spdx = Invoke-RestMethod -Uri "${baseUrl}.spdx.json" -UseBasicParsing
    $expectedSha256 = ($spdx.packages | Where-Object {
        $_.packageFileName -eq "python-${pythonVersion}-embed-amd64.zip" -and $_.downloadLocation -eq $baseUrl
    } | Select-Object -First 1).checksums |
        Where-Object { $_.algorithm -eq 'SHA256' } | Select-Object -ExpandProperty checksumValue -First 1
    if ($expectedSha256 -notmatch '^[0-9a-fA-F]{64}$') { throw "Missing SHA256 checksum in Python SPDX manifest." }
    if (-not (Test-Path -LiteralPath $cachedZip -PathType Leaf)) {
        Invoke-WebRequest -Uri $baseUrl -OutFile $cachedZip -UseBasicParsing
    }
    $actualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $cachedZip).Hash.ToLowerInvariant()
    if ($actualSha256 -ne $expectedSha256.ToLowerInvariant()) {
        Remove-Item -LiteralPath $cachedZip -Force -ErrorAction SilentlyContinue
        throw "Python runtime checksum mismatch: expected $expectedSha256, got $actualSha256"
    }
    $cachedZip
}
$updatePublicKey = if ($UpdatePublicKeyFile) {
    (Get-Content -Raw -LiteralPath $UpdatePublicKeyFile).Trim()
}
if ($UpdatePublicKeyFile -and (-not $updatePublicKey -or -not $env:TAURI_SIGNING_PRIVATE_KEY)) {
    throw 'A public key and TAURI_SIGNING_PRIVATE_KEY are required for updater artifacts.'
}
Set-Location $root
$env:Path = "$env:ProgramFiles\nodejs;$env:Path"
$companions = Join-Path $root 'target\companions'
foreach ($directory in @((Join-Path $root 'target'), $companions)) {
    if (Test-Path -LiteralPath $directory) {
        $item = Get-Item -LiteralPath $directory -Force
        if ($item.FullName -ne $directory -or -not $item.PSIsContainer -or
            ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Unexpected packaging directory: $directory"
        }
    }
}
if (Test-Path -LiteralPath $companions) { Remove-Item -LiteralPath $companions -Recurse -Force }
New-Item -ItemType Directory -Force $companions | Out-Null

$selfContained = if ($FrameworkDependentDotnet) { 'false' } else { 'true' }
$singleFileArgs = @('-p:PublishSingleFile=true')
if (-not $FrameworkDependentDotnet) {
    $singleFileArgs += '-p:IncludeNativeLibrariesForSelfExtract=true'
}

dotnet publish services/windows/AiWin11.Windows.csproj --configuration Release --runtime win-x64 --self-contained $selfContained `
    @singleFileArgs --output $companions -nologo -v q
if ($LASTEXITCODE -ne 0) { throw 'Build failed: bridge' }
# The elevated helper is one file, so its pinned SHA-256 covers all of its code.
dotnet publish services/admin/AiWin11.Admin.csproj --configuration Release --runtime win-x64 --self-contained $selfContained `
    @singleFileArgs --output $companions -nologo -v q
if ($LASTEXITCODE -ne 0) { throw 'Build failed: admin helper' }
Copy-Item -LiteralPath services/agent/agent.py -Destination (Join-Path $companions 'aiwin11_agent.py')
if ($pythonArchive) {
    $pythonDirectory = Join-Path $companions 'python'
    Expand-Archive -LiteralPath $pythonArchive -DestinationPath $pythonDirectory
    foreach ($file in @('python.exe', 'python313.dll', 'python313.zip', 'python313._pth')) {
        if (-not (Test-Path -LiteralPath (Join-Path $pythonDirectory $file) -PathType Leaf)) {
            throw "Python 3.13 embeddable ZIP is missing $file"
        }
    }
}
# Optional code signing: set AIWIN11_SIGN_THUMBPRINT to the SHA-1 thumbprint of a code-signing
# certificate in the current user's store (signtool.exe from the Windows SDK must be in PATH).
$thumbprint = $env:AIWIN11_SIGN_THUMBPRINT
if ($thumbprint) {
    signtool sign /sha1 $thumbprint /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 `
        (Join-Path $companions 'AiWin11.Admin.exe') (Join-Path $companions 'AiWin11.Windows.exe')
    if ($LASTEXITCODE -ne 0) { throw 'Signing failed' }
}
# Pin the elevated helper: AiWin11 refuses to start a helper whose SHA-256 differs.
$env:AIWIN11_ADMIN_SHA256 = (Get-FileHash -Algorithm SHA256 (Join-Path $companions 'AiWin11.Admin.exe')).Hash.ToLowerInvariant()
# Debug symbols are not shipped.
Get-ChildItem $companions -Filter *.pdb | Remove-Item -Force

if (-not (Test-Path 'apps\desktop\node_modules')) { npm --prefix apps/desktop ci }
$configs = @('--config', 'src-tauri/tauri.bundle.json')
if ($updatePublicKey) {
    $updaterConfig = Join-Path $root 'target\tauri.updater.json'
    @{
        bundle = @{ createUpdaterArtifacts = $true }
        plugins = @{ updater = @{ pubkey = $updatePublicKey } }
    } | ConvertTo-Json -Depth 4 | Set-Content -Encoding ascii -LiteralPath $updaterConfig
    $configs += @('--config', $updaterConfig)
}
if ($pythonArchive) {
    $pythonConfig = Join-Path $root 'target\tauri.python.json'
    '{"bundle":{"resources":{"../../../target/companions/python":"python/"}}}' |
        Set-Content -Encoding ascii -LiteralPath $pythonConfig
    $configs += @('--config', $pythonConfig)
}
if ($thumbprint) {
    $signing = Join-Path $root 'target\tauri.signing.json'
    "{ ""bundle"": { ""windows"": { ""certificateThumbprint"": ""$thumbprint"", ""digestAlgorithm"": ""sha256"", ""timestampUrl"": ""http://timestamp.digicert.com"" } } }" |
        Set-Content -Encoding ascii $signing
    $configs += @('--config', $signing)
}
$priorUpdatePublicKey = $env:AIWIN11_UPDATE_PUBLIC_KEY
try {
    $env:AIWIN11_UPDATE_PUBLIC_KEY = $updatePublicKey
    npm --prefix apps/desktop run tauri -- build @configs
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
} finally {
    $env:AIWIN11_UPDATE_PUBLIC_KEY = $priorUpdatePublicKey
}
if ($updatePublicKey) {
    $version = (Get-Content -Raw -LiteralPath 'apps/desktop/src-tauri/tauri.conf.json' | ConvertFrom-Json).version
    $bundleDirectory = Join-Path $root 'target\release\bundle\nsis'
    $installer = "AiWin11_${version}_x64-setup.exe"
    $signature = (Get-Content -Raw -LiteralPath (Join-Path $bundleDirectory "$installer.sig")).Trim()
    if (-not $signature) { throw 'Empty installer signature' }
    @{
        version = $version
        platforms = @{
            'windows-x86_64' = @{
                signature = $signature
                url = "https://github.com/quassim/aiwin11/releases/download/v$version/$installer"
            }
        }
    } | ConvertTo-Json -Depth 4 | Set-Content -Encoding ascii -LiteralPath (Join-Path $bundleDirectory 'latest.json')
}
Get-ChildItem (Join-Path $root 'target\release\bundle\nsis') -Filter *.exe | ForEach-Object { Write-Host "Installateur : $($_.FullName)" }
$dotnetSummary = if ($FrameworkDependentDotnet) { 'externe (.NET 10 requis)' } else { 'embarqué (autonome)' }
$pythonSummary = if ($pythonArchive) { 'embarqué (3.13)' } else { 'détecté via PATH / AIWIN11_PYTHON' }
$updaterSummary = if ($updatePublicKey) { 'activé (latest.json généré)' } else { 'désactivé' }
$signSummary = if ($thumbprint) { "signé ($thumbprint)" } else { 'non signé' }
Write-Host "Distribution : .NET $dotnetSummary · Python $pythonSummary · Mises à jour $updaterSummary · $signSummary"
