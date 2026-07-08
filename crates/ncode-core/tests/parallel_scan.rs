//! The parallel range-partitioned scan must be indistinguishable from the
//! serial scan, and every unsupported shape must fall back safely.

use ncode_core::{Client, Value};

const N: usize = 150_000; // above the parallel threshold (64k rowid span)

async fn seed_big(client: &Client) {
    client
        .execute(
            "CREATE TABLE big (id INTEGER, name TEXT, score REAL)",
            vec![],
        )
        .await
        .unwrap();
    let mut i = 0usize;
    while i < N {
        let end = (i + 500).min(N);
        let mut sql = String::from("INSERT INTO big (id, name, score) VALUES ");
        for r in i..end {
            if r > i {
                sql.push(',');
            }
            sql.push_str(&format!("({}, 'user_{}', {}.5)", r + 1, r, r % 100));
        }
        client.execute(&sql, vec![]).await.unwrap();
        i = end;
    }
}

#[tokio::test]
async fn parallel_scan_matches_expected_data_and_order() {
    let client = Client::connect(":memory:").await.unwrap();
    seed_big(&client).await;

    let batch = client
        .query("SELECT id, name, score FROM big ORDER BY id ASC", vec![])
        .await
        .unwrap();
    assert_eq!(batch.num_rows, N);

    let ids = batch.column("id").unwrap();
    let names = batch.column("name").unwrap();
    let scores = batch.column("score").unwrap();
    let mut id_sum = 0i64;
    for r in 0..N {
        assert_eq!(ids.i64(r), Some(r as i64 + 1), "row {r}");
        id_sum += ids.i64(r).unwrap();
    }
    assert_eq!(id_sum, (N as i64) * (N as i64 + 1) / 2);
    assert_eq!(names.str(0), Some("user_0"));
    assert_eq!(names.str(N - 1), Some(&format!("user_{}", N - 1)[..]));
    assert_eq!(scores.f64(7), Some(7.5));
}

#[tokio::test]
async fn parallel_scan_desc_permutes_all_columns_in_lockstep() {
    let client = Client::connect(":memory:").await.unwrap();
    seed_big(&client).await;

    let batch = client
        .query("SELECT id, name FROM big ORDER BY id DESC", vec![])
        .await
        .unwrap();
    assert_eq!(batch.num_rows, N);
    let ids = batch.column("id").unwrap();
    let names = batch.column("name").unwrap();
    assert_eq!(ids.i64(0), Some(N as i64));
    assert_eq!(names.str(0), Some(&format!("user_{}", N - 1)[..]));
    assert_eq!(ids.i64(N - 1), Some(1));
    assert_eq!(names.str(N - 1), Some("user_0"));
}

#[tokio::test]
async fn mixed_type_column_across_ranges_merges_like_serial_inference() {
    let client = Client::connect(":memory:").await.unwrap();
    client.execute("CREATE TABLE m (v)", vec![]).await.unwrap();
    // First half integers, second half text — different ranges will infer
    // different storage classes and the merge must unify them to Utf8.
    let mut i = 0usize;
    while i < N {
        let end = (i + 500).min(N);
        let mut sql = String::from("INSERT INTO m (v) VALUES ");
        for r in i..end {
            if r > i {
                sql.push(',');
            }
            if r < N / 2 {
                sql.push_str(&format!("({})", r));
            } else {
                sql.push_str(&format!("('t{}')", r));
            }
        }
        client.execute(&sql, vec![]).await.unwrap();
        i = end;
    }

    let batch = client.query("SELECT v FROM m", vec![]).await.unwrap();
    assert_eq!(batch.num_rows, N);
    let v = batch.column("v").unwrap();
    assert_eq!(v.str(0), Some("0"));
    assert_eq!(v.str(N / 2 - 1), Some(&format!("{}", N / 2 - 1)[..]));
    assert_eq!(v.str(N / 2), Some(&format!("t{}", N / 2)[..]));
    assert_eq!(v.str(N - 1), Some(&format!("t{}", N - 1)[..]));

    // Sorting on the mixed column must keep SQLite's storage-class order
    // (all numerics before all text), i.e. the engine sort bails out.
    let sorted = client
        .query("SELECT v FROM m ORDER BY v ASC", vec![])
        .await
        .unwrap();
    let sv = sorted.column("v").unwrap();
    assert_eq!(sv.str(0), Some("0"));
    // First text after all the integers; BINARY collation puts "t100000"
    // ahead of "t75000".
    assert_eq!(sv.str(N / 2), Some("t100000"));
}

#[tokio::test]
async fn memory_clients_stay_isolated() {
    let a = Client::connect(":memory:").await.unwrap();
    let b = Client::connect(":memory:").await.unwrap();
    a.execute("CREATE TABLE only_in_a (x INTEGER)", vec![])
        .await
        .unwrap();
    // `b` must not see `a`'s tables despite the shared-cache implementation.
    assert!(b.query("SELECT * FROM only_in_a", vec![]).await.is_err());
}

#[tokio::test]
async fn user_column_named_rowid_falls_back_serially() {
    let client = Client::connect(":memory:").await.unwrap();
    client
        .execute("CREATE TABLE r (rowid INTEGER, v TEXT)", vec![])
        .await
        .unwrap();
    let mut i = 0usize;
    while i < 100_000 {
        let end = i + 500;
        let mut sql = String::from("INSERT INTO r (rowid, v) VALUES ");
        for k in i..end {
            if k > i {
                sql.push(',');
            }
            sql.push_str(&format!("({}, 'v{}')", k % 7, k)); // non-unique, small span
        }
        client.execute(&sql, vec![]).await.unwrap();
        i = end;
    }
    let batch = client.query("SELECT rowid, v FROM r", vec![]).await.unwrap();
    assert_eq!(batch.num_rows, 100_000); // no rows lost to a bogus partition
}

#[tokio::test]
async fn without_rowid_table_falls_back_serially() {
    let client = Client::connect(":memory:").await.unwrap();
    client
        .execute(
            "CREATE TABLE w (id INTEGER PRIMARY KEY, v TEXT) WITHOUT ROWID",
            vec![],
        )
        .await
        .unwrap();
    for chunk in 0..200 {
        let mut sql = String::from("INSERT INTO w (id, v) VALUES ");
        for k in 0..500 {
            let id = chunk * 500 + k;
            if k > 0 {
                sql.push(',');
            }
            sql.push_str(&format!("({id}, 'v{id}')"));
        }
        client.execute(&sql, vec![]).await.unwrap();
    }
    let batch = client
        .query("SELECT id, v FROM w ORDER BY id ASC", vec![])
        .await
        .unwrap();
    assert_eq!(batch.num_rows, 100_000);
    assert_eq!(batch.column("id").unwrap().i64(99_999), Some(99_999));
}

#[tokio::test]
async fn chunked_encode_is_byte_identical_to_merged_encode() {
    let client = Client::connect(":memory:").await.unwrap();
    seed_big(&client).await;

    // query() merges chunks into a RecordBatch and encodes; query_bytes takes
    // the chunk-direct encoder. The wire bytes must be identical.
    for sql in [
        "SELECT id, name, score FROM big",
        "SELECT id, name, score FROM big ORDER BY id ASC",
        "SELECT id, name, score FROM big ORDER BY id DESC",
    ] {
        let via_batch = client.query(sql, vec![]).await.unwrap().encode();
        let direct = client.query_bytes(sql, vec![]).await.unwrap();
        assert_eq!(via_batch, direct, "mismatch for `{sql}`");
    }
}

#[tokio::test]
async fn where_clause_stays_serial_and_correct() {
    let client = Client::connect(":memory:").await.unwrap();
    seed_big(&client).await;
    let batch = client
        .query(
            "SELECT id FROM big WHERE id <= ? ORDER BY id ASC",
            vec![Value::Int(10)],
        )
        .await
        .unwrap();
    assert_eq!(batch.num_rows, 10);
    assert_eq!(batch.column("id").unwrap().i64(9), Some(10));
}
