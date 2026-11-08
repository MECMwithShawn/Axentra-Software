# Baseline evidence tests intentionally assert vulnerabilities. Always archive the audited
# revision so this runner stays meaningful on audit/fixes and never reverts product files.
$ErrorActionPreference = 'Stop'
$auditRepo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$auditEvidence = $PSScriptRoot
$auditRun = Join-Path $auditRepo ('target\audit\baseline-' + [guid]::NewGuid().ToString('N'))
$auditHarness = Join-Path $auditRun 'source'
$auditArchive = Join-Path $auditRun 'baseline.zip'
New-Item -ItemType Directory -Path $auditRun | Out-Null
git -C $auditRepo archive --format=zip --output=$auditArchive 722c40ae0f9a0403ec40d703a9749275bf032cbf Cargo.toml Cargo.lock app crates
if ($LASTEXITCODE -ne 0) { throw 'Could not archive the audited baseline' }
Expand-Archive -LiteralPath $auditArchive -DestinationPath $auditHarness
foreach ($auditModule in @('session','siteicons','app')) {
    $auditTestPath = (Join-Path $auditEvidence ($auditModule + '_tests.rs')).Replace('\','/')
    Add-Content -LiteralPath (Join-Path $auditHarness ('app\src\' + $auditModule + '.rs')) -Encoding utf8 -Value ("`n#[cfg(test)]`n#[path = `"" + $auditTestPath + "`"]`nmod audit_tests;")
}
$env:PATH = (Join-Path $env:USERPROFILE '.cargo\bin') + ';' + $env:PATH
$env:TEMP = Join-Path $auditRepo 'target\audit-temp'
$env:TMP = $env:TEMP
$env:APPDATA = Join-Path $auditRun 'appdata'
New-Item -ItemType Directory -Force -Path $env:TEMP,$env:APPDATA | Out-Null
$env:AUDIT_GZIP = Join-Path $auditRun 'eight-mib.gz'
$env:CARGO_TARGET_DIR = Join-Path $auditRepo 'target'
$auditStream = [System.IO.File]::Create($env:AUDIT_GZIP)
try {
    $auditGzip = [System.IO.Compression.GZipStream]::new($auditStream, [System.IO.Compression.CompressionMode]::Compress)
    try {
        $auditBytes = [System.Text.Encoding]::ASCII.GetBytes(('A' * (8 * 1024 * 1024)))
        $auditGzip.Write($auditBytes, 0, $auditBytes.Length)
    } finally { $auditGzip.Dispose() }
} finally { $auditStream.Dispose() }
# Cargo can reuse same-named local workspace artifacts when an archive carries old mtimes.
# Remove only the three local packages' generated build artifacts; keep dependency caches.
if ((Resolve-Path $env:CARGO_TARGET_DIR).Path -ne (Join-Path $auditRepo 'target')) { throw 'Unexpected build target' }
cargo clean --manifest-path (Join-Path $auditHarness 'Cargo.toml') -p axentra-vault -p vault-core -p passkey-win
if ($LASTEXITCODE -ne 0) { throw 'Could not invalidate local package build artifacts' }
cargo test --offline --locked --manifest-path (Join-Path $auditHarness 'Cargo.toml') -p axentra-vault audit_ -- --nocapture --test-threads=1 2>&1 | Tee-Object -FilePath (Join-Path $auditEvidence 'reproduction-results.txt')
$auditExit = $LASTEXITCODE
$auditLog = Join-Path $auditEvidence 'reproduction-results.txt'
$auditText = [System.IO.File]::ReadAllText($auditLog).Replace("`r`n", "`n")
[System.IO.File]::WriteAllText($auditLog, $auditText, [System.Text.UTF8Encoding]::new($false))
exit $auditExit
