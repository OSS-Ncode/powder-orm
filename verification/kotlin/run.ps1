# Compile and run the Kotlin real-world query verification.
#
#   pwsh -File run.ps1 [-Url "sqlite::memory:"] [-Lib <powder_java.dll>] [-SkipCompile]
#
# Requirements: JDK (javac/java) on PATH and a kotlinc installation
# ($KotlincHome below — defaults to the IntelliJ IDEA bundled compiler).

param(
    [string]$KotlincHome = "C:\Program Files\JetBrains\IntelliJ IDEA 2026.1.3\plugins\Kotlin\kotlinc",
    [string]$Lib = "$env:LOCALAPPDATA\Temp\ncode-target\release\powder_java.dll",
    [string]$Url = $(if ($env:POWDER_URL) { $env:POWDER_URL } else { "sqlite::memory:" }),
    [switch]$SkipCompile
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$here     = $PSScriptRoot
$repo     = (Resolve-Path (Join-Path $here "..\..")).Path
$javaOut  = Join-Path $repo "crates\powder-java\out"
$kotlinc  = Join-Path $KotlincHome "bin\kotlinc.bat"
$stdlib   = Join-Path $KotlincHome "lib\kotlin-stdlib.jar"
$schema   = Join-Path $repo "verification\powder.schema.json"
$outDir   = Join-Path $here "out"

if (-not (Test-Path $kotlinc)) { throw "kotlinc not found: $kotlinc" }
if (-not (Test-Path $Lib))     { throw "powder_java native library not found: $Lib" }

# Java binding classes (reused if already built).
if (-not (Test-Path (Join-Path $javaOut "com\powder\Client.class"))) {
    Write-Host "compiling Java binding classes..."
    & javac -encoding UTF-8 -d $javaOut (Get-ChildItem (Join-Path $repo "crates\powder-java\java\com\powder\*.java")).FullName
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if (-not $SkipCompile) {
    Write-Host "compiling Kotlin sources (binding + RealworldQueries)..."
    & $kotlinc -cp $javaOut `
        (Join-Path $repo "bindings\kotlin\src\dev\powder\Powder.kt") `
        (Join-Path $here "RealworldQueries.kt") `
        -d $outDir
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

Write-Host "running against $Url"
& java "-Dfile.encoding=UTF-8" "-Dstdout.encoding=UTF-8" "-Dstderr.encoding=UTF-8" `
    --enable-native-access=ALL-UNNAMED `
    -cp "$outDir;$javaOut;$stdlib" RealworldQueriesKt $Lib $schema $Url
exit $LASTEXITCODE
