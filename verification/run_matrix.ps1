# 전체 검증 매트릭스: 언어 × 백엔드
# (SQLite / PostgreSQL / MariaDB / CockroachDB / libSQL — SQL Server는 T-SQL
#  방언이라 별도 스모크: verification/mssql/run.ps1)
# 사용법: pwsh verification/run_matrix.ps1 [-Backends sqlite,postgres,mysql,cockroach,libsql] [-Langs c,cpp,...]
# 서버 DB 접속 정보는 아래 기본값 또는 환경변수로 재정의.
param(
    [string[]]$Backends = @("sqlite", "postgres", "mysql", "cockroach", "libsql"),
    [string[]]$Langs = @("c", "cpp", "csharp", "python", "node", "java", "kotlin")
)

$ErrorActionPreference = "Continue"
$here = $PSScriptRoot

# `pwsh -File`은 "a,b"를 원소 하나로 넘기므로 콤마를 직접 분리한다.
$Backends = @($Backends | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$Langs = @($Langs | ForEach-Object { $_ -split ',' } | Where-Object { $_ })

$urls = @{
    sqlite    = "sqlite::memory:"
    postgres  = $(if ($env:POWDER_PG_URL) { $env:POWDER_PG_URL } else { "postgres://postgres:postgres@127.0.0.1:5432/powder_test" })
    mysql     = $(if ($env:POWDER_MY_URL) { $env:POWDER_MY_URL } else { "mysql://root:powder@127.0.0.1:3306/powder_test" })
    cockroach = $(if ($env:POWDER_CRDB_URL) { $env:POWDER_CRDB_URL } else { "postgres://root@127.0.0.1:26257/powder_test" })
    libsql    = $(if ($env:POWDER_LIBSQL_URL) { $env:POWDER_LIBSQL_URL } else { "libsql://127.0.0.1:8880?tls=false" })
}

foreach ($be in $Backends) {
    if (-not $urls.ContainsKey($be)) { Write-Error "알 수 없는 백엔드: $be"; exit 2 }
}

$results = @()
foreach ($lang in $Langs) {
    foreach ($be in $Backends) {
        $url = $urls[$be]
        Write-Host "`n########## $lang / $be ##########" -ForegroundColor Cyan
        switch ($lang) {
            "java"   { & "$here\java\run.ps1" -Url $url }
            "kotlin" { & "$here\kotlin\run.ps1" -Url $url }
            default  { & "$here\$lang\run.ps1" $url }
        }
        $ok = ($LASTEXITCODE -eq 0)
        $results += [pscustomobject]@{ Language = $lang; Backend = $be; Result = $(if ($ok) { "PASS" } else { "FAIL" }) }
    }
}

Write-Host "`n=================== 결과 ==================="
$results | Format-Table -AutoSize
if ($results | Where-Object { $_.Result -eq "FAIL" }) { exit 1 } else { exit 0 }
