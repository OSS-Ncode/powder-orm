# C++ 실무 쿼리 검증 — 빌드 + 실행
# 사용법: pwsh verification/cpp/run.ps1 [connection-url]   (기본 sqlite::memory:)
param([string]$Url = "")

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\..\.."
$targetDir = "C:\Users\User\AppData\Local\Temp\ncode-target\release"
$vcvars = "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat"

if (-not (Test-Path "$targetDir\powder_ffi.dll.lib")) {
    Write-Host "powder_ffi 빌드 중..."
    cargo build -p powder-ffi --release
    if ($LASTEXITCODE -ne 0) { exit 1 }
}

$out = "$PSScriptRoot\build"
New-Item -ItemType Directory -Force $out | Out-Null
$src = "$PSScriptRoot\realworld_queries.cpp"
$exe = "$out\realworld_queries.exe"
if (-not (Test-Path $exe) -or (Get-Item $src).LastWriteTime -gt (Get-Item $exe).LastWriteTime) {
    cmd /c "`"$vcvars`" >nul 2>&1 && cl /nologo /std:c++17 /EHsc /W3 /utf-8 /I`"$root\bindings\cpp\include`" /Fo`"$out\\`" /Fe`"$exe`" `"$src`" /link `"$targetDir\powder_ffi.dll.lib`""
    if ($LASTEXITCODE -ne 0) { exit 1 }
}

Copy-Item "$targetDir\powder_ffi.dll" "$out\powder_ffi.dll" -Force
if ($Url) { & $exe $Url } else { & $exe }
exit $LASTEXITCODE
