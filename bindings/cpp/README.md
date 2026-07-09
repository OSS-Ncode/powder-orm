# powder-cpp — C++ 바인딩

헤더 하나(`include/powder.hpp`, C++17)로 끝나는 RAII 래퍼 + PCB 디코더.

```cpp
#include "powder.hpp"

powder::Client db("sqlite::memory:");
db.execute("CREATE TABLE t (id INTEGER, name TEXT, score REAL)");
db.execute("INSERT INTO t VALUES (?, ?, ?)", {int64_t{1}, "alice", 9.5});

powder::Batch b = db.query("SELECT * FROM t WHERE score >= ?", {5.0});
for (size_t r = 0; r < b.num_rows(); ++r)
    std::cout << b["id"].i64(r) << " " << b["name"].str(r) << "\n";

// 트랜잭션: 반환 시 COMMIT, 예외 시 ROLLBACK. 중첩은 세이브포인트.
db.transaction([](powder::Client& tx) {
    tx.execute("INSERT INTO t VALUES (2, 'bob', 1.0)");
});
```

- `Client`/`Batch` 모두 move-only RAII — 소멸 시 연결/네이티브 버퍼가 해제된다.
- 컬럼 읽기는 PCB 버퍼 위의 뷰: `i64/f64/boolean/str(row)`, `is_valid(row)`
  (`str`은 `string_view` — 배치 수명 내에서만 유효).
- 오류는 전부 `powder::Error`(엔진 메시지 포함)로 던져진다.

## 빌드 & 테스트

```bat
cargo build -p powder-ffi --release
cl /std:c++17 /EHsc /W3 /utf-8 test\test_powder.cpp /link <target>\release\powder_ffi.dll.lib
```
