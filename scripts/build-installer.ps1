# Builds dist\Nabra-Setup.exe, the Windows installer you attach to a GitHub Release.
#   powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
#
# Needs: scripts\setup.ps1 done once, Rust, and Node.js (for the Tauri CLI via npx).
# The installer contains the app + the speech engine (no models): first launch downloads the models.

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

# 1. Freeze the Python engine into build\engine\nabra-engine\nabra-engine.exe.
#    CUDA libraries are left out on purpose (≈1.3 GB): Setup downloads them only on PCs with an NVIDIA GPU.
& .venv\Scripts\pip install --quiet "pyinstaller>=6.10"
& .venv\Scripts\pyinstaller --noconfirm --clean --onedir --console --name nabra-engine `
    --distpath build\engine --workpath build\pyinstaller --specpath build `
    --paths engine `
    --add-data "$root\engine\lexicon_builtin.tsv;." `
    --collect-all faster_whisper --collect-all ctranslate2 --collect-binaries onnxruntime --collect-binaries av `
    --hidden-import mcp_notes --hidden-import service `
    --exclude-module nvidia --exclude-module tkinter --exclude-module matplotlib `
    engine\__main__.py
if ($LASTEXITCODE) { throw "PyInstaller failed" }

# 2. Smoke test the frozen engine: MCP mode answers without loading any model.
$reply = '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | & build\engine\nabra-engine\nabra-engine.exe --mcp --notes $env:TEMP
if ($reply -notmatch "list_notes") { throw "Frozen engine smoke test failed: $reply" }

# 3. Build the app + NSIS installer with the engine bundled as a resource.
Push-Location desktop
npx --yes @tauri-apps/cli@2 build --config tauri.release.conf.json
if ($LASTEXITCODE) { Pop-Location; throw "Tauri build failed" }
Pop-Location

# 4. Copy to dist\ with a stable name for the release.
New-Item -ItemType Directory -Force dist | Out-Null
$setup = Get-ChildItem desktop\target\release\bundle\nsis\*-setup.exe | Sort-Object LastWriteTime | Select-Object -Last 1
Copy-Item $setup.FullName dist\Nabra-Setup.exe -Force
# Updater signature (only when TAURI_SIGNING_PRIVATE_KEY was set: see scripts\release.ps1).
Remove-Item dist\Nabra-Setup.exe.sig -ErrorAction SilentlyContinue
if (Test-Path "$($setup.FullName).sig") { Copy-Item "$($setup.FullName).sig" dist\Nabra-Setup.exe.sig -Force }
$hash = (Get-FileHash dist\Nabra-Setup.exe -Algorithm SHA256).Hash
"{0}  Nabra-Setup.exe" -f $hash.ToLower() | Set-Content dist\Nabra-Setup.exe.sha256
Write-Host ("Built dist\Nabra-Setup.exe ({0:N0} MB)  SHA-256 {1}" -f ((Get-Item dist\Nabra-Setup.exe).Length / 1MB), $hash)
