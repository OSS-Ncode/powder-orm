//! Live MySQL migration/validation test — runs only when `POWDER_MYSQL_URL`
//! is set. Without it, passes as a skip.

use powder_cli::db;
use powder_cli::schema::Schema;

#[test]
fn mysql_migrate_seed_validate_roundtrip() {
    let Ok(url) = std::env::var("POWDER_MYSQL_URL") else {
        eprintln!("POWDER_MYSQL_URL not set; skipping live mysql migration test");
        return;
    };

    let schema = Schema::parse(
        r#"{"tables":{
            "my_mig_users":{"columns":{
                "id":{"type":"int","primaryKey":true},
                "name":{"type":"text"},
                "score":{"type":"float","nullable":true},
                "active":{"type":"bool"}
            }}
        }}"#,
    )
    .unwrap();

    let mut conn = db::open(&url).expect("connect");
    conn.execute_batch("DROP TABLE IF EXISTS my_mig_users").unwrap();

    let applied = db::migrate(&mut conn, &schema).expect("migrate");
    assert_eq!(applied.len(), 1, "{applied:?}");
    assert!(applied[0].contains("BIGINT"), "mysql types: {applied:?}");

    assert!(db::migrate(&mut conn, &schema).unwrap().is_empty());
    let problems = db::validate(&mut conn, &schema).expect("validate");
    assert!(problems.is_empty(), "{problems:?}");

    let n = db::seed(
        &mut conn,
        "seed.json",
        r#"{"my_mig_users": [{"id": 1, "name": "alice", "score": 9.5, "active": true}]}"#,
    )
    .expect("seed");
    assert_eq!(n, 1);

    assert!(db::migrate_rebuild(&mut conn, &schema).unwrap_err().contains("SQLite-only"));
    conn.execute_batch("DROP TABLE my_mig_users").unwrap();
}
