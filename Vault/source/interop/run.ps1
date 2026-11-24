# KeePass <-> Axentra Vault interop: KeePassLib writes a fixture with every KeePass feature, the
# Rust core opens and re-saves it, then KeePassLib verifies the result (and passkey files both ways).
param([string]$KeePassExe = "C:\Program Files\KeePass Password Safe 2\KeePass.exe")
$ErrorActionPreference = "Stop"

$root = Split-Path $PSScriptRoot
$out = Join-Path $PSScriptRoot "out"
$fixtures = Join-Path $out "fixtures"
Remove-Item -Recurse -Force $fixtures -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out, $fixtures | Out-Null

$csc = "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
Copy-Item $KeePassExe $out -Force
$src = @("$PSScriptRoot\Interop.cs", "$root\keepass-plugin\src\Envelope.cs", "$root\keepass-plugin\src\WebAuthn.cs")
& $csc /nologo /target:exe /platform:anycpu "/r:$KeePassExe" /r:System.Drawing.dll "/out:$out\Interop.exe" @src
if ($LASTEXITCODE -ne 0) { throw "Interop build failed" }

& "$out\Interop.exe" make $fixtures
if ($LASTEXITCODE -ne 0) { throw "Fixture generation failed" }

$env:AXV_INTEROP_DIR = $fixtures
Push-Location $root
try {
    cargo test -p vault-core --test interop -- --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw "Rust interop tests failed" }
} finally { Pop-Location; Remove-Item Env:\AXV_INTEROP_DIR }

& "$out\Interop.exe" verify $fixtures
if ($LASTEXITCODE -ne 0) { throw "KeePassLib verification failed" }
