# Setup for running Nabra from source: creates the Python environment the engine runs in.
#   powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
#
# The speech and AI models are NOT downloaded here: on first launch the app's Setup screen downloads what
# your PC needs (GPU or CPU) into %LOCALAPPDATA%\Nabra\assets, verified against their published checksums.

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) { throw "Python 3.11+ is required: https://www.python.org/downloads/" }
if (-not (Test-Path .venv)) { python -m venv .venv }
& .venv\Scripts\python -m pip install --quiet --upgrade pip
& .venv\Scripts\pip install --quiet -r requirements.txt
& .venv\Scripts\python engine\selftest.py

Write-Host ""
Write-Host "Python environment ready. Next:"
Write-Host "  cd desktop"
Write-Host "  cargo run          # first launch opens Setup, which downloads the models"
