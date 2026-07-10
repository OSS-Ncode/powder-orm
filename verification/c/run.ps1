# C 실무 쿼리 검증 — 빌드 + 실행
# 사용법: pwsh verification/c/run.ps1 [connection-url]
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
cmd /c "`"$vcvars`" >nul 2>&1 && cl /nologo /W3 /utf-8 /I`"$root\bindings\c\include`" /Fo`"$out\\`" /Fe`"$out\realworld_queries.exe`" `"$PSScriptRoot\realworld_queries.c`" /link `"$targetDir\powder_ffi.dll.lib`""
if ($LASTEXITCODE -ne 0) { exit 1 }

Copy-Item "$targetDir\powder_ffi.dll" "$out\powder_ffi.dll" -Force
if ($args.Count -gt 0) {
    & "$out\realworld_queries.exe" $args[0]
} else {
    & "$out\realworld_queries.exe"
}
exit $LASTEXITCODE
