# Starts the portable Ollama in strict local mode (no cloud features). Keep this window open.
$root = Split-Path -Parent $PSScriptRoot
$env:OLLAMA_HOST = '127.0.0.1:11434'
$env:OLLAMA_NO_CLOUD = '1'
$env:OLLAMA_MODELS = Join-Path $root '.tools\ollama-models'
& (Join-Path $root '.tools\ollama\ollama.exe') serve
