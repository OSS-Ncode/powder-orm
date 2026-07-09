//! Live-database operations: migrate, validate, seed.
//!
//! `validate` is the build gate the plan calls "No Compile": wire it before
//! the application build (`powder validate && tsc`) and a schema/database
//! mismatch stops the pipeline before it can become a runtime error.

use rusqlite::Connection;

use crate::dialect::{SqlDialect, Sqlite};
use crate::schema::{Schema, Table};

/// Open a connection using the same URL forms the Powder client accepts.
pub fn open(url: &str) -> Result<Connection, String> {
    let conn = if url == ":memory:" || url == "sqlite::memory:" {
        Connection::open_in_memory()
    } else if let Some(path) = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
    {
        Connection::open(path)
    } else {
        Connection::open(url)
    };
    conn.map_err(|e| format!("cannot open database `{url}`: {e}"))
}

/// One column as reported by `PRAGMA table_info`.
#[derive(Debug, PartialEq)]
struct DbColumn {
    name: String,
    sql_type: String,
    notnull: bool,
    /// 0 = not part of the primary key; otherwise the 1-based position
    /// within a (possibly composite) primary key.
    pk: i64,
}

fn introspect(conn: &Connection, table: &str) -> Result<Vec<DbColumn>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let cols = stmt
        .query_map([], |row| {
            Ok(DbColumn {
                name: row.get::<_, String>(1)?,
                sql_type: row.get::<_, String>(2)?.to_ascii_uppercase(),
                notnull: row.get::<_, i64>(3)? != 0,
                pk: row.get::<_, i64>(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(cols)
}

/// One foreign key as reported by `PRAGMA foreign_key_list`, with composite
/// keys grouped: SQLite emits one row per column sharing an `id`, ordered by
/// `seq`.
struct DbForeignKey {
    from: Vec<String>,
    table: String,
    to: Vec<String>,
}

fn introspect_fks(conn: &Connection, table: &str) -> Result<Vec<DbForeignKey>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA foreign_key_list({table})"))
        .map_err(|e| e.to_string())?;
    // Columns: id, seq, table, from, to, on_update, on_delete, match.
    struct Row {
        id: i64,
        seq: i64,
        table: String,
        from: String,
        to: Option<String>,
    }
    let mut rows = stmt
        .query_map([], |row| {
            Ok(Row {
                id: row.get(0)?,
                seq: row.get(1)?,
                table: row.get(2)?,
                from: row.get(3)?,
                to: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    // Group by id, order each group by seq.
    rows.sort_by(|a, b| a.id.cmp(&b.id).then(a.seq.cmp(&b.seq)));
    let mut out: Vec<DbForeignKey> = Vec::new();
    let mut cur_id: Option<i64> = None;
    for r in rows {
        if cur_id != Some(r.id) {
            cur_id = Some(r.id);
            out.push(DbForeignKey {
                from: Vec::new(),
                table: r.table.clone(),
                to: Vec::new(),
            });
        }
        let fk = out.last_mut().unwrap();
        fk.from.push(r.from);
        // `to` is NULL when the FK references the implicit primary key.
        fk.to.push(r.to.unwrap_or_default());
    }
    Ok(out)
}

/// Apply the schema to the database: create missing tables (in dependency
/// order), add missing columns. Additive only — nothing is dropped or
/// retyped; use [`migrate_rebuild`] for destructive drift.
pub fn migrate(conn: &Connection, schema: &Schema) -> Result<Vec<String>, String> {
    let mut applied = Vec::new();
    for table in schema.tables_in_dependency_order() {
        let existing = introspect(conn, &table.name)?;
        if existing.is_empty() {
            let ddl = Sqlite.create_table(table);
            conn.execute_batch(&ddl).map_err(|e| e.to_string())?;
            applied.push(ddl);
            continue;
        }
        for col in &table.columns {
            if existing.iter().any(|c| c.name == col.name) {
                continue;
            }
            let ddl = Sqlite.add_column(table, col)?;
            conn.execute_batch(&ddl).map_err(|e| e.to_string())?;
            applied.push(ddl);
        }
    }
    Ok(applied)
}

/// Destructive migration: any table whose live shape mismatches the schema is
/// rebuilt in place (SQLite's documented rebuild pattern — create the new
/// shape, copy the intersection of columns, drop, rename). Data in surviving
/// columns is preserved; dropped columns are lost by definition.
pub fn migrate_rebuild(conn: &Connection, schema: &Schema) -> Result<Vec<String>, String> {
    // Additive pass first so simple gaps don't force a rebuild.
    let mut applied = migrate(conn, schema)?;

    conn.execute_batch("PRAGMA foreign_keys = OFF")
        .map_err(|e| e.to_string())?;
    let result = (|| {
        for table in schema.tables_in_dependency_order() {
            let existing = introspect(conn, &table.name)?;
            let mut problems = Vec::new();
            check_table(table, &existing, &mut problems);
            check_foreign_keys(conn, table, &mut problems)?;
            if problems.is_empty() {
                continue;
            }

            let tmp = format!("__powder_rebuild_{}", table.name);
            let tmp_table = Table {
                name: tmp.clone(),
                columns: table.columns.clone(),
                foreign_keys: table.foreign_keys.clone(),
            };
            // Common columns copy straight over; NOT NULL columns absent from
            // the old shape (or nullable there) are backfilled with a
            // type-appropriate default.
            let select_list: Vec<String> = table
                .columns
                .iter()
                .map(|c| {
                    let default = match c.def.column_type.sql_type() {
                        "TEXT" => "''".to_string(),
                        _ => "0".to_string(),
                    };
                    match existing.iter().any(|e| e.name == c.name) {
                        true if c.def.nullable => c.name.clone(),
                        true => format!("COALESCE({}, {})", c.name, default),
                        false => default,
                    }
                })
                .collect();
            let cols: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();

            conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
            let steps = (|| {
                let mut steps = Vec::new();
                let create = Sqlite.create_table(&tmp_table);
                conn.execute_batch(&create).map_err(|e| e.to_string())?;
                steps.push(create);
                let copy = format!(
                    "INSERT INTO {tmp} ({}) SELECT {} FROM {}",
                    cols.join(", "),
                    select_list.join(", "),
                    table.name
                );
                conn.execute_batch(&copy).map_err(|e| e.to_string())?;
                steps.push(copy);
                let drop = format!("DROP TABLE {}", table.name);
                conn.execute_batch(&drop).map_err(|e| e.to_string())?;
                steps.push(drop);
                let rename = format!("ALTER TABLE {tmp} RENAME TO {}", table.name);
                conn.execute_batch(&rename).map_err(|e| e.to_string())?;
                steps.push(rename);
                Ok::<_, String>(steps)
            })();
            match steps {
                Ok(mut s) => {
                    conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                    applied.append(&mut s);
                }
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(format!("rebuild of `{}` failed: {e}", table.name));
                }
            }
        }
        Ok::<_, String>(())
    })();
    let _ = conn.execute_batch("PRAGMA foreign_keys = ON");
    result?;
    Ok(applied)
}

/// Compare the live database against the schema (columns, types, primary
/// keys — composite included — and foreign keys). Returns the list of
/// mismatches; empty means the two are in sync.
pub fn validate(conn: &Connection, schema: &Schema) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    for table in &schema.tables {
        let existing = introspect(conn, &table.name)?;
        if existing.is_empty() {
            problems.push(format!("table `{}`: missing from database", table.name));
            continue;
        }
        check_table(table, &existing, &mut problems);
        check_foreign_keys(conn, table, &mut problems)?;
    }
    Ok(problems)
}

fn check_table(table: &Table, existing: &[DbColumn], problems: &mut Vec<String>) {
    let pk = table.primary_key();
    for col in &table.columns {
        let Some(db) = existing.iter().find(|c| c.name == col.name) else {
            problems.push(format!(
                "table `{}`: column `{}` missing from database",
                table.name, col.name
            ));
            continue;
        };
        let want_type = col.def.column_type.sql_type();
        if db.sql_type != want_type {
            problems.push(format!(
                "table `{}`, column `{}`: type is `{}` in database, schema wants `{}`",
                table.name, col.name, db.sql_type, want_type
            ));
        }
        // Expected 1-based position within the (possibly composite) key.
        let want_pk = pk
            .iter()
            .position(|c| c.name == col.name)
            .map(|i| i as i64 + 1)
            .unwrap_or(0);
        if db.pk != want_pk {
            problems.push(format!(
                "table `{}`, column `{}`: primary-key position mismatch (db: {}, schema: {})",
                table.name, col.name, db.pk, want_pk
            ));
        }
        // An INTEGER PRIMARY KEY is implicitly NOT NULL in SQLite even when
        // table_info reports notnull=0, so nullability only applies elsewhere.
        if !col.def.primary_key {
            let want_notnull = !col.def.nullable;
            if db.notnull != want_notnull {
                problems.push(format!(
                    "table `{}`, column `{}`: nullability mismatch (db notnull: {}, schema nullable: {})",
                    table.name, col.name, db.notnull, col.def.nullable
                ));
            }
        }
    }
    for db in existing {
        if !table.columns.iter().any(|c| c.name == db.name) {
            problems.push(format!(
                "table `{}`: database has extra column `{}` not in schema",
                table.name, db.name
            ));
        }
    }
}

fn check_foreign_keys(
    conn: &Connection,
    table: &Table,
    problems: &mut Vec<String>,
) -> Result<(), String> {
    let db_fks = introspect_fks(conn, &table.name)?;
    // A DB foreign key matches a schema one when the local columns, target
    // table, and referenced columns all agree (in order). `to` is empty when
    // SQLite defaulted to the referenced table's primary key.
    let matches = |schema: &crate::schema::ForeignKey, db: &DbForeignKey| {
        schema.columns == db.from
            && schema.ref_table == db.table
            && (db.to.iter().all(|s| s.is_empty()) || schema.ref_columns == db.to)
    };
    for fk in &table.foreign_keys {
        if !db_fks.iter().any(|db| matches(fk, db)) {
            problems.push(format!(
                "table `{}`: foreign key ({}) -> `{}`({}) missing from database",
                table.name,
                fk.columns.join(", "),
                fk.ref_table,
                fk.ref_columns.join(", ")
            ));
        }
    }
    for db in &db_fks {
        if !table.foreign_keys.iter().any(|fk| matches(fk, db)) {
            problems.push(format!(
                "table `{}`: database has extra foreign key ({}) -> `{}`({}) not in schema",
                table.name,
                db.from.join(", "),
                db.table,
                db.to.join(", ")
            ));
        }
    }
    Ok(())
}

/// Seed the database. `.sql` files run as a batch script; `.json` files hold
/// `{ "<table>": [ { "<col>": value, ... }, ... ] }`.
pub fn seed(conn: &Connection, path: &str, contents: &str) -> Result<usize, String> {
    if path.ends_with(".sql") {
        conn.execute_batch(contents).map_err(|e| e.to_string())?;
        return Ok(0);
    }
    let doc: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(contents).map_err(|e| format!("invalid seed JSON: {e}"))?;
    let mut inserted = 0usize;
    for (table, rows) in doc {
        let rows = rows
            .as_array()
            .ok_or_else(|| format!("seed `{table}`: expected an array of row objects"))?;
        for row in rows {
            let obj = row
                .as_object()
                .ok_or_else(|| format!("seed `{table}`: rows must be objects"))?;
            let cols: Vec<&str> = obj.keys().map(String::as_str).collect();
            let sql = format!(
                "INSERT INTO {table} ({}) VALUES ({})",
                cols.join(", "),
                vec!["?"; cols.len()].join(", ")
            );
            let params: Vec<rusqlite::types::Value> = obj
                .values()
                .map(|v| match v {
                    serde_json::Value::Null => Ok(rusqlite::types::Value::Null),
                    serde_json::Value::Bool(b) => Ok(rusqlite::types::Value::Integer(*b as i64)),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Ok(rusqlite::types::Value::Integer(i))
                        } else {
                            Ok(rusqlite::types::Value::Real(n.as_f64().unwrap()))
                        }
                    }
                    serde_json::Value::String(s) => Ok(rusqlite::types::Value::Text(s.clone())),
                    other => Err(format!("seed `{table}`: unsupported value {other}")),
                })
                .collect::<Result<_, _>>()?;
            conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
                .map_err(|e| format!("seed `{table}`: {e}"))?;
            inserted += 1;
        }
    }
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SAMPLE_SCHEMA;

    fn sample() -> Schema {
        Schema::parse(SAMPLE_SCHEMA).unwrap()
    }

    #[test]
    fn migrate_then_validate_is_clean() {
        let conn = Connection::open_in_memory().unwrap();
        let schema = sample();
        let applied = migrate(&conn, &schema).unwrap();
        assert_eq!(applied.len(), 2); // users + posts (with FK)
        assert!(validate(&conn, &schema).unwrap().is_empty());
        // Idempotent.
        assert!(migrate(&conn, &schema).unwrap().is_empty());
    }

    #[test]
    fn migrate_adds_missing_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .unwrap();
        let applied = migrate(&conn, &sample()).unwrap();
        assert_eq!(applied.len(), 3); // +score, +active, CREATE posts
        assert!(validate(&conn, &sample()).unwrap().is_empty());
    }

    #[test]
    fn composite_pk_migrates_and_validates() {
        let schema = Schema::parse(
            r#"{"tables":{"m":{"columns":{
                "a":{"type":"int","primaryKey":true},
                "b":{"type":"text","primaryKey":true},
                "v":{"type":"float","nullable":true}
            }}}}"#,
        )
        .unwrap();
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn, &schema).unwrap();
        assert!(validate(&conn, &schema).unwrap().is_empty());

        // The composite key is enforced by the database.
        conn.execute_batch("INSERT INTO m VALUES (1, 'x', 0.5)").unwrap();
        assert!(conn.execute_batch("INSERT INTO m VALUES (1, 'x', 9.9)").is_err());
        conn.execute_batch("INSERT INTO m VALUES (1, 'y', 1.5)").unwrap();

        // Key order matters: a DB with (b, a) must be flagged.
        let conn2 = Connection::open_in_memory().unwrap();
        conn2
            .execute_batch(
                "CREATE TABLE m (a INTEGER NOT NULL, b TEXT NOT NULL, v REAL, PRIMARY KEY (b, a))",
            )
            .unwrap();
        let problems = validate(&conn2, &schema).unwrap();
        assert!(problems.iter().any(|p| p.contains("primary-key position")), "{problems:?}");
    }

    #[test]
    fn foreign_keys_validate_and_enforce() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        migrate(&conn, &sample()).unwrap();
        assert!(validate(&conn, &sample()).unwrap().is_empty());

        conn.execute_batch("INSERT INTO users VALUES (1, 'alice', NULL, 1)").unwrap();
        conn.execute_batch("INSERT INTO posts VALUES (1, 1, 'hello')").unwrap();
        // FK is real: inserting a post for a missing user fails.
        assert!(conn.execute_batch("INSERT INTO posts VALUES (2, 99, 'nope')").is_err());

        // A DB missing the FK is reported as drift.
        let bare = Connection::open_in_memory().unwrap();
        bare.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL, active INTEGER NOT NULL);
             CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, title TEXT NOT NULL)",
        )
        .unwrap();
        let problems = validate(&bare, &sample()).unwrap();
        assert!(
            problems.iter().any(|p| p.contains("foreign key") && p.contains("missing from database")),
            "{problems:?}"
        );
    }

    #[test]
    fn composite_foreign_key_migrates_validates_and_enforces() {
        let schema = Schema::parse(
            r#"{"tables":{
                "orders":{"columns":{
                    "id":{"type":"int","primaryKey":true},
                    "year":{"type":"int","primaryKey":true}
                }},
                "line_items":{
                    "columns":{
                        "id":{"type":"int","primaryKey":true},
                        "order_id":{"type":"int"},
                        "order_year":{"type":"int"}
                    },
                    "foreignKeys":[
                        {"columns":["order_id","order_year"],"references":{"table":"orders","columns":["id","year"]}}
                    ]
                }
            }}"#,
        )
        .unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        let applied = migrate(&conn, &schema).unwrap();
        // The composite FK is a table-level constraint in the DDL.
        assert!(
            applied.iter().any(|s| s.contains("FOREIGN KEY (order_id, order_year) REFERENCES orders(id, year)")),
            "{applied:?}"
        );
        assert!(validate(&conn, &schema).unwrap().is_empty());

        // The two-column FK is enforced by the database.
        conn.execute_batch("INSERT INTO orders VALUES (1, 2026)").unwrap();
        conn.execute_batch("INSERT INTO line_items VALUES (1, 1, 2026)").unwrap();
        // Right id, wrong year -> no matching parent row -> rejected.
        assert!(conn.execute_batch("INSERT INTO line_items VALUES (2, 1, 2025)").is_err());

        // A DB missing the composite FK is drift.
        let bare = Connection::open_in_memory().unwrap();
        bare.execute_batch(
            "CREATE TABLE orders (id INTEGER NOT NULL, year INTEGER NOT NULL, PRIMARY KEY (id, year));
             CREATE TABLE line_items (id INTEGER PRIMARY KEY, order_id INTEGER NOT NULL, order_year INTEGER NOT NULL)",
        )
        .unwrap();
        assert!(!validate(&bare, &schema).unwrap().is_empty());
    }

    #[test]
    fn rebuild_fixes_destructive_drift_and_keeps_data() {
        let schema = Schema::parse(
            r#"{"tables":{"t":{"columns":{
                "id":{"type":"int","primaryKey":true},
                "name":{"type":"text"},
                "score":{"type":"float","nullable":true}
            }}}}"#,
        )
        .unwrap();
        let conn = Connection::open_in_memory().unwrap();
        // Old shape: `name` has the wrong type, `ghost` must be dropped,
        // `score` is missing.
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name REAL, ghost TEXT);
             INSERT INTO t VALUES (1, 'alice', 'x'), (2, 'bob', 'y')",
        )
        .unwrap();
        assert!(!validate(&conn, &schema).unwrap().is_empty());

        let applied = migrate_rebuild(&conn, &schema).unwrap();
        assert!(applied.iter().any(|s| s.contains("__powder_rebuild_t")), "{applied:?}");
        assert!(validate(&conn, &schema).unwrap().is_empty());

        // Surviving data intact, dropped column gone.
        let name: String = conn
            .query_row("SELECT name FROM t WHERE id = 2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "bob");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        assert!(conn.query_row("SELECT ghost FROM t", [], |r| r.get::<_, String>(0)).is_err());

        // Rebuild on an in-sync database is a no-op.
        assert!(migrate_rebuild(&conn, &schema).unwrap().is_empty());
    }

    #[test]
    fn validate_reports_mismatches() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name REAL, active INTEGER NOT NULL, ghost TEXT)",
        )
        .unwrap();
        let problems = validate(&conn, &sample()).unwrap();
        let text = problems.join("\n");
        assert!(text.contains("column `name`: type is `REAL`"), "{text}");
        assert!(text.contains("column `score` missing"), "{text}");
        assert!(text.contains("extra column `ghost`"), "{text}");
        assert!(text.contains("column `name`: nullability mismatch"), "{text}");
    }

    #[test]
    fn seed_inserts_json_rows() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn, &sample()).unwrap();
        let n = seed(
            &conn,
            "seed.json",
            r#"{"users": [
                {"id": 1, "name": "alice", "score": 9.5, "active": true},
                {"id": 2, "name": "bob", "score": null, "active": false}
            ]}"#,
        )
        .unwrap();
        assert_eq!(n, 2);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }
}
