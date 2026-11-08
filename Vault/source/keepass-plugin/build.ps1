# Builds AxentraVault.dll (KeePass 2.x plugin) and runs the offline tests.
# Uses the .NET Framework compiler that ships with Windows; no SDK needed.
param(
    [string]$KeePassExe = "C:\Program Files\KeePass Password Safe 2\KeePass.exe",
    [switch]$SkipTests
)
$ErrorActionPreference = "Stop"

$root = $PSScriptRoot
$csc = "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$out = Join-Path $root "out"
$testOut = Join-Path $out "tests"
New-Item -ItemType Directory -Force $out, $testOut | Out-Null

if (-not (Test-Path $KeePassExe)) { throw "KeePass.exe not found at $KeePassExe" }
$src = Get-ChildItem (Join-Path $root "src") -Filter *.cs | ForEach-Object FullName
$refs = @("/r:$KeePassExe", "/r:System.Windows.Forms.dll", "/r:System.Core.dll")

& $csc /nologo /target:library /platform:anycpu /optimize+ /warnaserror+ @refs "/out:$out\AxentraVault.dll" @src
if ($LASTEXITCODE -ne 0) { throw "Plugin build failed" }
Write-Host "Built $out\AxentraVault.dll"

if ($SkipTests) { return }

# Tests reference KeePassLib inside KeePass.exe, so it is copied next to the test binary.
Copy-Item $KeePassExe $testOut -Force
& $csc /nologo /target:exe /platform:anycpu /warnaserror+ @refs "/out:$testOut\Tests.exe" @src (Join-Path $root "tests\Tests.cs")
if ($LASTEXITCODE -ne 0) { throw "Test build failed" }
& "$testOut\Tests.exe"
if ($LASTEXITCODE -ne 0) { throw "Tests failed" }
