# Publish a new version: every installed Nabra (v2.1.0+) updates itself to it.
#   powershell -ExecutionPolicy Bypass -File scripts\release.ps1 -Version 2.1.1 [-Notes "What changed"]
#
# Needs: a clean, committed tree on `main`; `gh` logged in; the updater private key at
#   %USERPROFILE%\.nabra-signing\updater.key   (back it up! Without it you can never ship an update again)

param(
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
    [string]$Notes = "",
    [string]$Repo = "mohasameh793-arch/nabra"
)
# Not "Stop": Windows PowerShell 5.1 turns any stderr line from git/gh/cargo (progress, "release not found")
# into a fatal error. Every native command below is checked through $LASTEXITCODE instead.
$ErrorActionPreference = "Continue"
function Check($what) { if ($LASTEXITCODE) { throw "$what failed (exit $LASTEXITCODE)" } }
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$key = Join-Path $env:USERPROFILE ".nabra-signing\updater.key"
if (-not (Test-Path $key)) { throw "Updater signing key not found at $key" }
if (git status --porcelain) { throw "Commit or stash your changes first." }
if ((git rev-parse --abbrev-ref HEAD) -ne "main") { throw "Release from the main branch." }
gh release view "v$Version" --repo $Repo *> $null
if ($LASTEXITCODE -eq 0) { throw "v$Version is already released." }

# 1. Version everywhere.
$cargo = Get-Content desktop\Cargo.toml -Raw
$cargo = [regex]::Replace($cargo, '(?m)^version = "\d+\.\d+\.\d+"', "version = `"$Version`"", 1)
[IO.File]::WriteAllText("$root\desktop\Cargo.toml", $cargo)  # no BOM: Cargo, Tauri and JSON readers reject it
$conf = Get-Content desktop\tauri.conf.json -Raw
$conf = [regex]::Replace($conf, '"version": "\d+\.\d+\.\d+"', "`"version`": `"$Version`"", 1)
[IO.File]::WriteAllText("$root\desktop\tauri.conf.json", $conf)
$mcp = Get-Content engine\mcp_notes.py -Raw
$mcp = [regex]::Replace($mcp, '("name": "nabra-notes", "version": ")\d+\.\d+\.\d+', "`${1}$Version")
[IO.File]::WriteAllText("$root\engine\mcp_notes.py", $mcp)

# 2. Tests must pass before anything is published.
& .venv\Scripts\python engine\selftest.py
if ($LASTEXITCODE) { throw "Engine selftest failed" }
Push-Location desktop
cargo test --quiet
if ($LASTEXITCODE) { Pop-Location; throw "Rust tests failed" }
Pop-Location

git add -A
git commit -q -m "Release v$Version"; Check "git commit"
git push -q; Check "git push"

# 3. Build + sign (the key file path is passed through the environment, never written anywhere).
$env:TAURI_SIGNING_PRIVATE_KEY = $key
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
if ($LASTEXITCODE) { throw "Installer build failed" }
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
if (-not (Test-Path dist\Nabra-Setup.exe.sig)) { throw "No updater signature was produced" }

# 4. latest.json: what every installed Nabra checks.
$manifest = [ordered]@{
    version  = $Version
    notes    = $(if ($Notes) { $Notes } else { "Nabra $Version" })
    pub_date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = (Get-Content dist\Nabra-Setup.exe.sig -Raw).Trim()
            url       = "https://github.com/$Repo/releases/download/v$Version/Nabra-Setup.exe"
        }
    }
}
[IO.File]::WriteAllText("$root\dist\latest.json", ($manifest | ConvertTo-Json -Depth 5))

# 5. Publish. The release becomes "latest", so installed apps find it.
$sha = (Get-Content dist\Nabra-Setup.exe.sha256).Split(" ")[0]
$body = @"
$(if ($Notes) { $Notes } else { "Nabra $Version" })

**Already have Nabra 2.1.0 or newer?** It updates itself, so there's nothing to do.

**New install:** download ``Nabra-Setup.exe`` below and run it. If Windows shows *"Windows protected your PC"*, click **More info → Run anyway**.

SHA-256 of ``Nabra-Setup.exe``: ``$sha``
"@
gh release create "v$Version" dist\Nabra-Setup.exe dist\Nabra-Setup.exe.sha256 dist\latest.json `
    --repo $Repo --target main --title "Nabra $Version" --notes $body --latest
Check "gh release create"
Write-Host "Released v$Version. Installed copies will update within ~6 hours (or via Settings → About → Check for updates)."
