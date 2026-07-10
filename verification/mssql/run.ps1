# SQL Server 백엔드 스모크 — T-SQL 방언 (기본: 로컬 SQLEXPRESS, 통합 인증).
# 사용법: pwsh verification/mssql/run.ps1 [mssql-url]
#   기본 URL: mssql://127.0.0.1:1433/powder_test
#   SQL 인증: mssql://user:pass@host:port/db, 암호화: ?encrypt=true
param([string]$Url = "")

$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\..\.."
$targetDir = "C:\Users\User\AppData\Local\Temp\ncode-target\release"
$vcvars = "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat"

$out = "$PSScriptRoot\build"
New-Item -ItemType Directory -Force $out | Out-Null
$src = "$PSScriptRoot\mssql_smoke.cpp"
$exe = "$out\mssql_smoke.exe"
if (-not (Test-Path $exe) -or (Get-Item $src).LastWriteTime -gt (Get-Item $exe).LastWriteTime) {
    cmd /c "`"$vcvars`" >nul 2>&1 && cl /nologo /std:c++17 /EHsc /W3 /utf-8 /I`"$root\bindings\cpp\include`" /Fo`"$out\\`" /Fe`"$exe`" `"$src`" /link `"$targetDir\powder_ffi.dll.lib`""
    if ($LASTEXITCODE -ne 0) { exit 1 }
}

Copy-Item "$targetDir\powder_ffi.dll" "$out\powder_ffi.dll" -Force
if ($Url) { & $exe $Url } else { & $exe }
exit $LASTEXITCODE
