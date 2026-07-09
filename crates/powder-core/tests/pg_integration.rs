//! Live PostgreSQL integration test — runs only when `POWDER_PG_URL` points
//! at a reachable server (e.g. `postgres://user:pass@localhost/postgres`).
//! Without the env var the test passes as a skip, so CI without Postgres
//! stays green while machines with a server get real end-to-end coverage.

#![cfg(feature = "postgres")]

use powder_core::{Client, Value};

#[tokio::test]
async fn postgres_roundtrip_when_server_available() {
    let Ok(url) = std::env::var("POWDER_PG_URL") else {
        eprintln!("POWDER_PG_URL not set; skipping live postgres test");
        return;
    };

    let client = Client::connect(&url).await.expect("connect");
    client
        .execute("DROP TABLE IF EXISTS powder_it; CREATE TABLE powder_it (id BIGINT PRIMARY KEY, name TEXT, score DOUBLE PRECISION, active BOOLEAN)", vec![])
        .await
        .expect("ddl");
    client
        .execute(
            "INSERT INTO powder_it VALUES (?, ?, ?, ?), (?, ?, ?, ?)",
            vec![
                Value::Int(1),
                Value::Text("alice".into()),
                Value::Float(9.5),
                Value::Bool(true),
                Value::Int(2),
                Value::Text("bob".into()),
                Value::Null,
                Value::Bool(false),
            ],
        )
        .await
        .expect("insert");

    let batch = client
        .query("SELECT id, name, score, active FROM powder_it ORDER BY id", vec![])
        .await
        .expect("query");
    assert_eq!(batch.num_rows, 2);
    assert_eq!(batch.columns[0].i64(0), Some(1));
    assert_eq!(batch.columns[1].str(1), Some("bob"));
    assert_eq!(batch.columns[2].f64(0), Some(9.5));
    assert_eq!(batch.columns[2].f64(1), None); // NULL score
    assert_eq!(batch.columns[3].bool(1), Some(false));

    // The PCB encoding path works over Postgres-sourced batches too.
    let bytes = client
        .query_bytes("SELECT id, name FROM powder_it ORDER BY id", vec![])
        .await
        .expect("query_bytes");
    let decoded = powder_core::RecordBatch::decode(&bytes).expect("decode");
    assert_eq!(decoded.num_rows, 2);

    client
        .execute("DROP TABLE powder_it", vec![])
        .await
        .expect("cleanup");
}
