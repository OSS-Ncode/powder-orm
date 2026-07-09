# Powder

**Rust 코어** 기반의 고성능 데이터베이스 엔진. 쿼리 결과를 **zero-copy, Apache Arrow 스타일 컬럼 바이너리 포맷**으로 반환하며, **TypeScript**(napi-rs)와 **Python**(PyO3)에 관용적이고 완전한 `async` API로 노출된다.

```
          ┌────────────────────────────────────────────┐
          │            powder-core  (Rust)               │
          │  Client · 쿼리 빌더 · RecordBatch             │
          │  PCB 컬럼 코덱 (zero-copy)                    │
          │  async 엔진 (Tokio) → rusqlite 백엔드         │
          └───────────────┬───────────────┬─────────────┘
                          │               │
              napi-rs     │               │   PyO3 + pyo3-async-runtimes
         ┌────────────────▼──┐         ┌──▼──────────────────┐
         │   @powder/node     │         │      powder (py)      │
         │  Promise · TS      │         │  asyncio · typing    │
         │  typed-array 리더  │         │  memoryview 리더     │
         └────────────────────┘         └──────────────────────┘
```

## 왜 만들었나

관계형 결과 집합을 언어 경계 너머로 옮기려면 보통 JSON으로 직렬화하거나 호스트 언어 객체를 수백만 개 만들어야 한다. Powder는 대신 **하나의 연속된 컬럼 버퍼**(*PCB* 포맷)를 옮기고, 호스트 언어가 그 바이트 위에 typed-array / `memoryview` 뷰를 바로 얹게 한다 — Node의 `Float64Array`나 Python의 `memoryview.cast('d')`가 엔진 출력을 **값 단위 복사 없이** 읽는다.

## 구성

| 크레이트 / 패키지       | 역할                                                        |
| ---------------------- | ----------------------------------------------------------- |
| `crates/powder-core`    | Rust 코어: async 클라이언트, 쿼리 빌더, PCB 코덱             |
| `crates/powder-node`    | napi-rs 바인딩 + TypeScript 래퍼 (`@powder/node`)             |
| `crates/powder-python`  | PyO3 바인딩 + 순수 Python 래퍼 (`powder`)                     |

와이어 포맷 명세는 [`docs/FORMAT.md`](docs/FORMAT.md) 참고.

## Rust

```rust
use powder_core::{Client, query::Query, query::Order};

# async fn demo() -> powder_core::Result<()> {
let db = Client::connect("sqlite::memory:").await?;
db.execute("CREATE TABLE users (id INTEGER, name TEXT, score REAL)", vec![]).await?;
db.execute(
    "INSERT INTO users VALUES (?, ?, ?)",
    vec![1.into(), "alice".into(), 9.5.into()],
).await?;

let (sql, params) = Query::table("users")
    .select(["id", "name", "score"])
    .filter("score > ?", [5.0])
    .order_by("id", Order::Asc)
    .build();

let batch = db.query(&sql, params).await?;
println!("{} rows", batch.num_rows);
println!("first name = {:?}", batch.column("name").unwrap().str(0));
# Ok(())
# }
```

```bash
cargo test -p powder-core        # 코어 단위 + 통합 테스트 실행
```

## Node.js / TypeScript

```ts
import { Client, Query } from "@powder/node";

const db = await Client.connect("sqlite::memory:");
await db.execute("CREATE TABLE users (id INTEGER, name TEXT, score REAL)");
await db.execute("INSERT INTO users VALUES (?, ?, ?)", [1, "alice", 9.5]);

const batch = await db.run(
  Query.table("users").select("id", "name", "score").filter("score > ?", 5),
);

// 엔진 출력 버퍼 위의 zero-copy typed-array 뷰:
const score = batch.column("score")!;        // Float64Array 기반
console.log(score.get(0));                    // 9.5
console.log(batch.toRows());                  // [{ id: 1n, name: "alice", score: 9.5 }]
```

네이티브 애드온 + 타입 빌드:

```bash
cd crates/powder-node
npm install
npm run build        # napi build --release && tsc
```

## Python

```python
import asyncio, powder

async def main():
    db = await powder.connect("sqlite::memory:")
    await db.execute("CREATE TABLE users (id INTEGER, name TEXT, score REAL)")
    await db.execute("INSERT INTO users VALUES (?, ?, ?)", [1, "alice", 9.5])

    batch = await db.run(
        powder.Query.table("users").select("id", "name", "score").filter("score > ?", 5)
    )
    # 엔진 출력 버퍼 위의 zero-copy memoryview:
    print(batch.column("score").get(0))   # 9.5
    print(batch.to_rows())                 # [{'id': 1, 'name': 'alice', 'score': 9.5}]

asyncio.run(main())
```

확장 빌드 & 설치:

```bash
cd crates/powder-python
python -m venv .venv && source .venv/bin/activate
pip install maturin
maturin develop          # Rust 확장을 빌드하고 `powder`를 설치
```

## 지원 타입

`Int64`, `Float64`, `Bool`, `Utf8` — 모두 validity 비트맵으로 nullable.

## 라이선스

MIT
