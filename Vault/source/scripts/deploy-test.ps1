# Installs the current test build to a stable location and keeps a desktop shortcut pointing at
# it, so the app can be launched normally while development rebuilds happen in target\.
#   .\scripts\deploy-test.ps1            # copy target\debug build
#   .\scripts\deploy-test.ps1 -Release   # copy target\release build (may need code signing to run)
param([switch]$Release)
$ErrorActionPreference = "Stop"

$root = Split-Path $PSScriptRoot
$profile = if ($Release) { "release" } else { "debug" }
$src = Join-Path $root "target\$profile\axentra-vault.exe"
$dir = Join-Path $env:LOCALAPPDATA "Programs\Axentra Vault (Test)"
$dst = Join-Path $dir "axentra-vault.exe"

if (-not (Test-Path $src)) { throw "No $profile build at $src; run cargo build first." }

$running = Get-Process axentra-vault -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $dst }
if ($running) { throw "Axentra Vault (Test) is open. Close it, then run this again." }

New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item $src $dst -Force

# Desktop shortcut (the exe carries the Circuit Halo icon resource).
$desktop = [Environment]::GetFolderPath("Desktop")
$lnk = Join-Path $desktop "Axentra Vault (Test).lnk"
$shell = New-Object -ComObject WScript.Shell
$s = $shell.CreateShortcut($lnk)
$s.TargetPath = $dst
$s.WorkingDirectory = $dir
$s.IconLocation = "$dst,0"
$s.Description = "Axentra Vault test build ($profile, $(Get-Date -Format 'yyyy-MM-dd HH:mm'))"
$s.Save()

$built = (Get-Item $src).LastWriteTime.ToString("yyyy-MM-dd HH:mm")
"Deployed $profile build from $built"
"  app:      $dst"
"  shortcut: $lnk"
