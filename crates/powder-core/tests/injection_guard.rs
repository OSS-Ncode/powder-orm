//! End-to-end checks of the SQL-injection guard on the live client: stacked
//! statements are rejected on query() and parameterized execute(), while
//! trusted parameterless batches keep working.

use powder_core::Client;

#[tokio::test]
async fn stacked_query_is_rejected() {
    let db = Client::connect("sqlite::memory:").await.unwrap();
    db.execute("CREATE TABLE users (id INTEGER, name TEXT)", vec![])
        .await
        .unwrap();

    // The classic: user input closes the literal and stacks a statement.
    let user_input = "x'; DROP TABLE users; --";
    let sql = format!("SELECT * FROM users WHERE name = '{user_input}'");
    let err = db.query(&sql, vec![]).await.unwrap_err();
    assert!(err.to_string().contains("SQL-injection guard"), "{err}");

    // The table must still exist.
    let batch = db
        .query("SELECT COUNT(*) AS c FROM users", vec![])
        .await
        .unwrap();
    assert_eq!(batch.num_rows, 1);
}

#[tokio::test]
async fn stacked_parameterized_execute_is_rejected() {
    let db = Client::connect("sqlite::memory:").await.unwrap();
    db.execute("CREATE TABLE t (id INTEGER)", vec![]).await.unwrap();

    let err = db
        .execute(
            "INSERT INTO t VALUES (?); DROP TABLE t",
            vec![powder_core::Value::Int(1)],
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("SQL-injection guard"), "{err}");
}

#[tokio::test]
async fn parameterless_batches_still_work() {
    let db = Client::connect("sqlite::memory:").await.unwrap();
    // Trusted DDL/seed scripts stay a supported feature.
    db.execute(
        "CREATE TABLE a (id INTEGER); CREATE TABLE b (id INTEGER); \
         INSERT INTO a VALUES (1); INSERT INTO b VALUES (2);",
        vec![],
    )
    .await
    .unwrap();
    let batch = db.query("SELECT id FROM b", vec![]).await.unwrap();
    assert_eq!(batch.num_rows, 1);
}

#[tokio::test]
async fn semicolons_inside_literals_are_fine() {
    let db = Client::connect("sqlite::memory:").await.unwrap();
    db.execute("CREATE TABLE t (id INTEGER, note TEXT)", vec![])
        .await
        .unwrap();
    db.execute(
        "INSERT INTO t VALUES (?, ?)",
        vec![
            powder_core::Value::Int(1),
            powder_core::Value::Text("a; b'; c".into()),
        ],
    )
    .await
    .unwrap();
    let batch = db
        .query("SELECT note FROM t WHERE note LIKE '%;%'; -- trailing ok", vec![])
        .await
        .unwrap();
    assert_eq!(batch.num_rows, 1);
}
