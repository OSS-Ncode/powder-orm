# verification — 실무 쿼리 검증 스위트

문서(`docs-site/content/docs/`)에 적힌 기능이 실무 수준 쿼리에서 실제로
동작하는지 8개 언어 × 6개 DB 백엔드로 검증한다. 시나리오 정의와 기대값은
[SCENARIOS.md](SCENARIOS.md), 공용 스키마는 [powder.schema.json](powder.schema.json).

## 한 번에 전부

```powershell
pwsh verification/run_matrix.ps1                       # 7개 언어 × 5개 백엔드
pwsh verification/run_matrix.ps1 -Langs cpp,python     # 언어 골라서
pwsh verification/run_matrix.ps1 -Backends sqlite      # SQLite만
pwsh verification/mssql/run.ps1                        # SQL Server (T-SQL 방언 스모크)
```

기본 서버 접속 정보 (환경변수로 재정의 가능):

| 백엔드 | URL | 재정의 |
|---|---|---|
| SQLite | `sqlite::memory:` | — |
| PostgreSQL 17 | `postgres://postgres:postgres@127.0.0.1:5432/powder_test` | `POWDER_PG_URL` |
| MariaDB 12.3 | `mysql://root:powder@127.0.0.1:3306/powder_test` | `POWDER_MY_URL` |
| CockroachDB v25.2 | `postgres://root@127.0.0.1:26257/powder_test` | `POWDER_CRDB_URL` |
| libSQL (sqld v0.24) | `libsql://127.0.0.1:8880?tls=false` | `POWDER_LIBSQL_URL` |
| SQL Server 2008 Express | `mssql://127.0.0.1:1433/powder_test` (통합 인증) | run.ps1 인자 |

## 언어별 실행

모든 스크립트는 연결 URL 하나를 선택 인자로 받는다 (기본 `sqlite::memory:`).

```powershell
pwsh verification/c/run.ps1       [url]   # MSVC cl — powder_ffi C ABI 직접 호출
pwsh verification/cpp/run.ps1     [url]   # MSVC cl — RAII 래퍼 + ORM
pwsh verification/csharp/run.ps1  [url]   # dotnet run — P/Invoke + ORM
pwsh verification/python/run.ps1  [url]   # powder generate --py 코드젠 → asyncio
pwsh verification/node/run.ps1    [url]   # napi 애드온 — JS(mjs) + TS 각각 실행
pwsh verification/java/run.ps1    -Url <url>   # JNI powder_java.dll
pwsh verification/kotlin/run.ps1  -Url <url>   # IntelliJ 번들 kotlinc 사용
```

## 사전 조건

- `cargo build -p powder-ffi -p powder-java --release`
  (전역 cargo 설정이 target-dir을 `C:\Users\User\AppData\Local\Temp\ncode-target`로
  리다이렉트한다 — 산출물은 저장소 `target/`이 아니라 거기에 생긴다.)
- Python: `maturin build --release` 후 휠 설치 (run.ps1이 인터프리터 경로를 알고 있음)
- Node: `crates/powder-node`에서 `npm run build` (napi가 target-dir 리다이렉트를 못 보므로
  `CARGO_TARGET_DIR` 환경변수를 명시해야 함)
- 서버 DB: PostgreSQL 17 서비스(`postgresql-x64-17`), MariaDB는
  `"C:\Program Files\MariaDB 12.3\bin\mysqld.exe" --datadir="C:\ProgramData\MariaDB\data" --console`
  로 기동 (root 비밀번호 `powder`), 각각 `powder_test` DB 필요.

## 시나리오 요약 (언어당 35~39 체크)

전자상거래 데이터셋(고객·상품·주문·주문항목, FK 2단)으로:
대시보드 GROUP BY 집계 · JOIN+HAVING 리포트 · NOT IN 서브쿼리 ·
중첩 AND/OR/NOT + like/in 파인더 · 페이지네이션 · include/join 관계 로드 ·
groupBy+having 별칭 · aggregate(빈 집합 null) · 중첩 트랜잭션/세이브포인트 ·
FK 위반 처리 · NULL 컬럼 · 파라미터화 LTV 쿼리 · 한글/이모지 왕복.

## 이 스위트가 찾아서 고친 엔진 버그 (powder-core / powder-cli)

1. **PG: NULL 파라미터 직렬화 실패** — `Option::<i64>::None`은 i64 슬롯만 수락.
   → 슬롯 타입에 적응하는 `PgVal` ToSql로 교체 (`pg.rs`).
2. **PG: 정수 표기 숫자 vs float 슬롯** — JSON 경유 바인딩이 `100000.0`을
   `100000`으로 떨어뜨려 i64로 바인딩 → float8 슬롯 거부.
   → `PgVal`이 슬롯 타입으로 변환 + ORM이 스키마 타입으로 강제변환 (`orm.rs`).
3. **PG: 오류 메시지 소실** — `postgres::Error::to_string()`은 "db error"뿐.
   → `as_db_error()` 메시지 + SQLSTATE 노출 (`pg.rs`, `powder-cli/db.rs`).
4. **MySQL: 텍스트 프로토콜 디코드** — 파라미터 없는 쿼리는 모든 값이
   `Bytes`로 와서 숫자 컬럼 디코드 실패, `COUNT(*)`가 ASCII 바이트값(8→56)
   반환. → BIT 컬럼만 비트 누적, 나머지는 문자열 파싱 (`my.rs`).
5. **서버 DB: `BEGIN IMMEDIATE` / `RELEASE <sp>`** — 모든 바인딩의 트랜잭션
   헬퍼가 SQLite 문법을 하드코딩. → PG/MySQL 백엔드에서 `BEGIN` /
   `RELEASE SAVEPOINT`로 정규화 (`pg.rs`, `my.rs`).
6. **CLI: PG introspection SQL 오류** — JOIN과 콤마 조인 혼용으로 `con` 참조
   무효 → PG에서 migrate/validate가 아예 불가였음. `CROSS JOIN LATERAL`로
   수정 (`powder-cli/db.rs`).

## 추가 백엔드 (2026-07-11 추가 구현/검증)

- **CockroachDB**: PG wire 호환 — 기존 `postgres` 드라이버로 전 언어
  전 시나리오 통과. 별도 코드 불필요, URL만 `postgres://root@host:26257/db`.
- **libSQL** (`libsql` 기능, `crates/powder-core/src/ls.rs`): 원격
  sqld/Turso를 `libsql://host[:port][?tls=false][&authToken=…]`로 연결.
  SQLite 방언 그대로라 전 시나리오 통과. 연결 시 `PRAGMA foreign_keys=ON`.
- **SQL Server** (`mssql` 기능, `crates/powder-core/src/ms.rs`): tiberius
  기반, `mssql://[user:pass@]host[:port][/db][?encrypt=true]` (사용자 없으면
  Windows 통합 인증). `?`→`@PN` 변환, `BEGIN/SAVEPOINT/ROLLBACK TO/RELEASE`
  → T-SQL 정규화, ORM `limit`→`TOP (n)` / `offset`→`OFFSET..FETCH`(2012+,
  orderBy 필수). SQL Server 2008 Express에서 19체크 스모크 통과
  (`verification/mssql/`). 공용 매트릭스에서 빠진 이유: 언어 테스트의
  DDL(BOOLEAN/TEXT)이 T-SQL과 달라서 — 드라이버가 아니라 테스트 DDL 문제.
- CLI(`powder migrate/validate/seed/ddl`)도 6종 전부 지원: `--dialect mssql`
  포함, libSQL은 SQLite 방언으로 처리. (MSSQL/libSQL 실서버에서
  migrate→validate "in sync" + 한글 seed 왕복 확인 완료.)

### 서버 기동 (로컬)

```powershell
# CockroachDB (C:\Users\User\bin\cockroach.exe — PATH에 있음)
cockroach start-single-node --insecure --listen-addr=127.0.0.1:26257 --http-addr=127.0.0.1:8081 --store="C:\ProgramData\cockroach"
# libSQL — WSL Ubuntu (~/sqld/sqld)
wsl -d Ubuntu -- bash -c "nohup ~/sqld/sqld --http-listen-addr 0.0.0.0:8880 --db-path ~/powder_libsql.db >/dev/null 2>&1 &"
# SQL Server — SQLEXPRESS 서비스 (TCP 1433은 활성화 완료)
```
