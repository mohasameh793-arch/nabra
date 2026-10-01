# One-time setup: Python env for the engine + local AI assets.
#   powershell -ExecutionPolicy Bypass -File scripts\setup.ps1 [-From "C:\path\to\folder\with\llama-and-models"]
#
# Assets layout (git-ignored):
#   .assets\llama\llama-server.exe (+ CUDA DLLs)     llama.cpp Windows CUDA 13 build
#   .assets\models\Qwen3-8B-Q4_K_M.gguf               local LLM (Apache-2.0), 5 GB
# Whisper large-v3 is fetched by faster-whisper into the Hugging Face cache on first run (~3 GB).
param([string]$From = "$env:USERPROFILE\Desktop\ai voice flow\.local")

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

if (-not (Test-Path .venv)) { python -m venv .venv }
& .venv\Scripts\python -m pip install --quiet --upgrade pip
& .venv\Scripts\pip install --quiet -r requirements.txt
Write-Host "Python env ready"

# Hard links: no extra disk space, and they keep working if the source folder is deleted.
# They require the same drive; otherwise we fall back to copying.
function Link-Tree($src, $dst) {
    New-Item -ItemType Directory -Force $dst | Out-Null
    Get-ChildItem $src -File | ForEach-Object {
        $target = Join-Path $dst $_.Name
        if (Test-Path $target) { return }
        try { New-Item -ItemType HardLink -Path $target -Target $_.FullName | Out-Null }
        catch { Copy-Item $_.FullName $target }
    }
}

if (Test-Path "$From\llama\llama-server.exe") {
    Link-Tree "$From\llama" "$root\.assets\llama"
    Link-Tree "$From\models" "$root\.assets\models"
    Write-Host "Linked AI assets from $From"
} else {
    Write-Host "No local assets at $From. Download into .assets\ manually:"
    Write-Host "  llama.cpp: https://github.com/ggml-org/llama.cpp/releases (llama-*-bin-win-cuda-13*-x64.zip + cudart-*.zip)"
    Write-Host "  model:     https://huggingface.co/Qwen/Qwen3-8B-GGUF (Qwen3-8B-Q4_K_M.gguf)"
    Write-Host "Without them Nabra still dictates; AI cleanup and call summaries are off."
}
