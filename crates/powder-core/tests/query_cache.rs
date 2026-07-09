//! Query-result cache correctness: hits must be provably fresh, and anything
//! the cache cannot prove safe must bypass it.

use std::sync::Arc;

use powder_core::{Client, RecordBatch, Value};

async fn seeded() -> Client {
    let client = Client::connect(":memory:").await.unwrap();
    client
        .execute("CREATE TABLE t (id INTEGER, name TEXT)", vec![])
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO t VALUES (1, 'a'), (2, 'b')",
            vec![],
        )
        .await
        .unwrap();
    client
}

#[tokio::test]
async fn repeated_query_returns_the_same_shared_buffer() {
    let client = seeded().await;
    let a = client
        .query_bytes_shared("SELECT id, name FROM t", vec![])
        .await
        .unwrap();
    let b = client
        .query_bytes_shared("SELECT id, name FROM t", vec![])
        .await
        .unwrap();
    // Same Arc — the second call never re-ran the scan or the encoder.
    assert!(Arc::ptr_eq(&a, &b));

    // The synchronous probe sees it too.
    let probed = client.probe_cache("SELECT id, name FROM t", vec![]).unwrap();
    assert!(Arc::ptr_eq(&a, &probed));
}

#[tokio::test]
async fn different_params_are_different_entries() {
    let client = seeded().await;
    let a = client
        .query_bytes_shared("SELECT name FROM t WHERE id = ?", vec![Value::Int(1)])
        .await
        .unwrap();
    let b = client
        .query_bytes_shared("SELECT name FROM t WHERE id = ?", vec![Value::Int(2)])
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert_eq!(
        RecordBatch::decode(&a).unwrap().column("name").unwrap().str(0),
        Some("a")
    );
    assert_eq!(
        RecordBatch::decode(&b).unwrap().column("name").unwrap().str(0),
        Some("b")
    );
}

#[tokio::test]
async fn writes_invalidate_the_cache() {
    let client = seeded().await;
    let before = client
        .query_bytes_shared("SELECT COUNT(*) AS n FROM t", vec![])
        .await
        .unwrap();
    client
        .execute("INSERT INTO t VALUES (3, 'c')", vec![])
        .await
        .unwrap();
    assert!(client.probe_cache("SELECT COUNT(*) AS n FROM t", vec![]).is_none());
    let after = client
        .query_bytes_shared("SELECT COUNT(*) AS n FROM t", vec![])
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&before, &after));
    let batch = RecordBatch::decode(&after).unwrap();
    assert_eq!(batch.column("n").unwrap().i64(0), Some(3));
}

#[tokio::test]
async fn nondeterministic_sql_is_never_cached() {
    let client = seeded().await;
    let sql = "SELECT random() AS r";
    let a = client.query_bytes_shared(sql, vec![]).await.unwrap();
    let b = client.query_bytes_shared(sql, vec![]).await.unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(client.probe_cache(sql, vec![]).is_none());
}

#[tokio::test]
async fn dml_returning_is_never_cached_and_always_executes() {
    let client = seeded().await;
    let sql = "INSERT INTO t (id, name) VALUES (9, 'x') RETURNING id";
    let a = client.query_bytes_shared(sql, vec![]).await.unwrap();
    let b = client.query_bytes_shared(sql, vec![]).await.unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    let n = client
        .query("SELECT COUNT(*) AS n FROM t WHERE id = 9", vec![])
        .await
        .unwrap();
    assert_eq!(n.column("n").unwrap().i64(0), Some(2)); // both inserts ran
}

#[tokio::test]
async fn file_db_sees_writes_from_other_connections() {
    let dir = std::env::temp_dir().join(format!("powder-cache-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("shared.db");
    let _ = std::fs::remove_file(&path);
    let url = path.to_string_lossy().to_string();

    let a = Client::connect(&url).await.unwrap();
    a.execute("CREATE TABLE t (id INTEGER)", vec![]).await.unwrap();
    a.execute("INSERT INTO t VALUES (1)", vec![]).await.unwrap();

    let count = |bytes: &[u8]| {
        RecordBatch::decode(bytes)
            .unwrap()
            .column("n")
            .unwrap()
            .i64(0)
            .unwrap()
    };

    let first = a
        .query_bytes_shared("SELECT COUNT(*) AS n FROM t", vec![])
        .await
        .unwrap();
    assert_eq!(count(&first), 1);

    // A *different* connection writes; client `a`'s cache must not serve stale rows.
    let b = Client::connect(&url).await.unwrap();
    b.execute("INSERT INTO t VALUES (2)", vec![]).await.unwrap();

    let second = a
        .query_bytes_shared("SELECT COUNT(*) AS n FROM t", vec![])
        .await
        .unwrap();
    assert_eq!(count(&second), 2);

    // File DBs never answer from the lock-free probe.
    assert!(a.probe_cache("SELECT COUNT(*) AS n FROM t", vec![]).is_none());

    drop((a, b));
    let _ = std::fs::remove_file(&path);
}
