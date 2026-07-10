# Powder Node(JS/TS) real-world query verification.
# Usage: .\run.ps1 [connection-url]   (default sqlite::memory:; or set POWDER_URL)
param([string]$Url = "")

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not (Test-Path node_modules)) {
    # --install-links: the P: drive rejects symlinks, so copy the file: dep instead.
    npm install --install-links
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# Compile the TS test + generated models (dist/ is shared by both tests).
npx tsc -p tsconfig.json
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "`n=== JS (realworld_queries.mjs) ===" -ForegroundColor Cyan
node realworld_queries.mjs $Url
$js = $LASTEXITCODE

Write-Host "`n=== TS (dist/realworld_queries.js) ===" -ForegroundColor Cyan
node dist/realworld_queries.js $Url
$ts = $LASTEXITCODE

if ($js -ne 0) { exit $js }
exit $ts
