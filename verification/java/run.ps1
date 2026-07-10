# Compile and run the Java real-world query verification.
#
#   .\run.ps1 [-Dll <path-to-powder_java.dll>] [-Url <connection-url>]
#
# Defaults: dll from $env:POWDER_JAVA_DLL or the local cargo target dir;
# url from $env:POWDER_URL or "sqlite::memory:".
param(
    [string]$Dll = $(if ($env:POWDER_JAVA_DLL) { $env:POWDER_JAVA_DLL }
                     else { "C:\Users\User\AppData\Local\Temp\ncode-target\release\powder_java.dll" }),
    [string]$Url = $(if ($env:POWDER_URL) { $env:POWDER_URL } else { "sqlite::memory:" })
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent (Split-Path -Parent $here)   # verification/java -> repo root
$binding = Join-Path $repo "crates\powder-java\java"
$out = Join-Path $here "out"

if (-not (Test-Path $Dll)) {
    Write-Error "native library not found: $Dll"
    exit 2
}

javac -encoding UTF-8 -d $out `
    (Join-Path $binding "com\powder\*.java") `
    (Join-Path $here "RealworldQueries.java")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Push-Location $here   # so ../powder.schema.json resolves
try {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    java "-Dfile.encoding=UTF-8" "-Dstdout.encoding=UTF-8" "-Dstderr.encoding=UTF-8" `
        --enable-native-access=ALL-UNNAMED -cp $out RealworldQueries $Dll $Url
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
