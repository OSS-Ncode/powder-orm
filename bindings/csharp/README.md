# powder-csharp — C# / .NET 바인딩

P/Invoke로 `powder_ffi` 네이티브 라이브러리를 호출하고, PCB 컬럼 버퍼를
관리 코드에서 디코드한다. .NET 8+.

```csharp
using Powder;

using var db = Client.Connect("sqlite::memory:");
db.Execute("CREATE TABLE t (id INTEGER, name TEXT, score REAL)");
db.Execute("INSERT INTO t VALUES (?,?,?)", 1L, "alice", 9.5);

Batch batch = db.Query("SELECT * FROM t WHERE score >= ?", 5.0);
for (int r = 0; r < batch.NumRows; r++)
    Console.WriteLine($"{batch["id"].GetInt64(r)} {batch["name"].GetString(r)}");

// 트랜잭션: 반환 시 COMMIT, 예외 시 ROLLBACK. 중첩은 세이브포인트.
db.Transaction(tx => tx.Execute("INSERT INTO t VALUES (2, 'bob', 1.0)"));
```

## 네이티브 라이브러리 위치

1. 환경변수 `POWDER_LIB`에 전체 경로 지정, 또는
2. `powder_ffi.dll` / `libpowder_ffi.so`를 앱 옆이나 로더 검색 경로에 배치.

## 빌드 & 테스트

```bash
cargo build -p powder-ffi --release
cd bindings/csharp/Powder.Tests
POWDER_LIB=<target>/release/powder_ffi.dll dotnet run
# -> csharp binding OK (17 checks)
```

- 파라미터는 `long`/`double`/`bool`/`string`/`null` (내부적으로 JSON 배열로 전달).
- `Column.Get(row)`은 박싱된 값 또는 SQL NULL이면 `null`;
  `GetInt64/GetDouble/GetBoolean/GetString`은 무박싱 경로.
- 오류는 전부 `PowderException`(엔진 메시지 포함).
