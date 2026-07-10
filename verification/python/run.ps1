# Powder Python real-world query verification: codegen -> run.
# Usage: .\run.ps1 [connection-url]   (default sqlite::memory:, or $env:POWDER_URL)
param([string]$Url)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

$powderExe = "C:\Users\User\AppData\Local\Temp\ncode-target\release\powder.exe"
$python = "C:\Users\User\AppData\Local\Programs\Python\Python310\python.exe"
if (-not (Get-Command $python -ErrorAction SilentlyContinue)) { $python = "python" }

# 1) Codegen: schema -> powder_models.py
& $powderExe generate --schema (Join-Path $here "..\powder.schema.json") --py (Join-Path $here "powder_models.py")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 2) Run the verification suite; pass through the exit code.
if ($Url) {
    & $python (Join-Path $here "realworld_queries.py") $Url
} else {
    & $python (Join-Path $here "realworld_queries.py")
}
exit $LASTEXITCODE
