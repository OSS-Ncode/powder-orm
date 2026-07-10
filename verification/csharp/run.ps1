# Run the C# real-world query verification.
#   .\run.ps1 [connection-url]
# POWDER_LIB may be pre-set; defaults to the local release build of powder_ffi.dll.

$ErrorActionPreference = "Stop"

if (-not $env:POWDER_LIB) {
    $env:POWDER_LIB = "C:\Users\User\AppData\Local\Temp\ncode-target\release\powder_ffi.dll"
}

Push-Location $PSScriptRoot
try {
    dotnet run --project . -- @args
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
