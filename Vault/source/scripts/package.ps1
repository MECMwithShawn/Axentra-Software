# Builds the release exe, the Setup.exe (Inno Setup) and the .msi (WiX v7), plus a source zip (GPL), into dist\.
# Writes SHA-256 checksums to dist\SHA256SUMS.txt. Nothing is uploaded or published.
param(
    [string]$Iscc = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    [string]$Wix = "C:\Program Files\WiX Toolset v7.0\bin\wix.exe"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot
Set-Location $root
$env:Path = "$HOME\.cargo\bin;$env:Path"

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Axentra Vault $version"

cargo build --release -p axentra-vault
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
$exe = Join-Path $root "target\release\axentra-vault.exe"

$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force $dist | Out-Null
Get-ChildItem $dist -Filter "AxentraVault-$version*" | Remove-Item

& $Iscc /Q "/DAppVersion=$version" "/DSourceExe=$exe" "packaging\windows\AxentraVault.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }

$msi = Join-Path $dist "AxentraVault-$version.msi"
& $Wix build -acceptEula wix7 -arch x64 "packaging\windows\AxentraVault.wxs" -o $msi `
    -d "Version=$version" -d "SourceExe=$exe" -d "LicenseFile=$root\LICENSE" `
    -d "IconFile=$root\app\assets\app\vault.ico"
if ($LASTEXITCODE -ne 0) { throw "WiX build failed" }
Remove-Item (Join-Path $dist "*.wixpdb") -ErrorAction SilentlyContinue

# Source for exactly this commit (GPL-3.0: binaries ship with their source).
$commit = (git rev-parse --short HEAD).Trim()
git archive --format=zip --prefix="axentra-vault-$version/" -o (Join-Path $dist "AxentraVault-$version-source.zip") HEAD
if ($LASTEXITCODE -ne 0) { throw "git archive failed" }

$sums = Get-ChildItem $dist -Filter "AxentraVault-$version*" | Sort-Object Name | ForEach-Object {
    "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
}
$sums | Set-Content (Join-Path $dist "SHA256SUMS.txt")
Write-Host "Built from commit $commit"
Get-ChildItem $dist -Filter "AxentraVault-$version*" | ForEach-Object { "{0,-40} {1,8:N1} MB" -f $_.Name, ($_.Length / 1MB) }
$sums
