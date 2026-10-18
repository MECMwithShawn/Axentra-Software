param([string]$TargetDir = "$PSScriptRoot\target")
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    cargo test --locked --target-dir $TargetDir
    if ($LASTEXITCODE) { throw 'Tests failed' }
    cargo build --release --locked --target-dir $TargetDir
    if ($LASTEXITCODE) { throw 'Release build failed' }
    $version = (Select-String -LiteralPath "$PSScriptRoot\Cargo.toml" -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
    $binary = Join-Path $TargetDir 'release\axentra-usb.exe'
    $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (!$iscc) { throw 'Install Inno Setup 6 to produce the Windows installer.' }
    & $iscc /Q "/DAppVersion=$version" "/DAppBinary=$binary" "$PSScriptRoot\installer\AxentraUSB.iss"
    if ($LASTEXITCODE) { throw 'Installer build failed' }
    python "$PSScriptRoot\packaging\package.py" --windows $binary --output "$PSScriptRoot\dist"
    if ($LASTEXITCODE) { throw 'Portable/source packaging failed' }
    Get-Content -LiteralPath "$PSScriptRoot\dist\SHA256SUMS.txt"
} finally { Pop-Location }
