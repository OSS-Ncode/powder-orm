//! libSQL runtime backend (feature `libsql`) — remote sqld / Turso.
//!
//! Mirrors the other server paths: run a query, stream rows into the shared
//! [`ColumnBuilder`]s, hand back a [`RecordBatch`]. The `libsql` crate is
//! async-only, so the backend owns a small current-thread runtime and
//! exposes the same *synchronous* surface as the other server backends —
//! [`crate::Client`] already dispatches these calls to Tokio's blocking pool.
//!
//! libSQL speaks the SQLite dialect (`?` placeholders, `BEGIN IMMEDIATE`,
//! savepoints), so no SQL rewriting is needed. Values are dynamically typed
//! like SQLite; column PCB types are inferred from the first non-NULL value.
//!
//! URL forms:
//! - `libsql://host[:port][?authToken=…]` — remote over HTTPS (Turso, sqld)
//! - `libsql://host[:port]?tls=false[&authToken=…]` — remote over plain HTTP
//!   (local sqld)

use std::sync::Mutex;

use libsql::Value as LsValue;

use crate::array::ColumnBuilder;
use crate::batch::RecordBatch;
use crate::error::{Error, Result};
use crate::query::Value;
use crate::schema::DataType;

pub struct LsBackend {
    rt: tokio::runtime::Runtime,
    conn: Mutex<libsql::Connection>,
    /// Keeps the database (and its background tasks) alive.
    _db: libsql::Database,
}

impl LsBackend {
    pub fn connect(url: &str) -> Result<Self> {
        let rest = url
            .strip_prefix("libsql://")
            .ok_or_else(|| Error::InvalidUrl(format!("not a libsql url: {url}")))?;

        let (base, query) = match rest.split_once('?') {
            Some((b, q)) => (b, q),
            None => (rest, ""),
        };
        let mut token = String::new();
        let mut tls = true;
        for kv in query.split('&').filter(|s| !s.is_empty()) {
            match kv.split_once('=') {
                Some(("authToken", v)) => token = v.to_string(),
                Some(("tls", v)) if v == "false" || v == "0" => tls = false,
                _ => {}
            }
        }
        let remote = format!("{}://{}", if tls { "https" } else { "http" }, base);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::Database(format!("libsql runtime: {e}")))?;
        let (db, conn) = rt
            .block_on(async {
                let db = libsql::Builder::new_remote(remote, token).build().await?;
                let conn = db.connect()?;
                // Match the bundled SQLite backend: enforce foreign keys.
                conn.execute("PRAGMA foreign_keys = ON", ()).await?;
                Ok::<_, libsql::Error>((db, conn))
            })
            .map_err(|e| Error::Database(format!("libsql connect: {e}")))?;
        Ok(Self {
            rt,
            conn: Mutex::new(conn),
            _db: db,
        })
    }

    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<usize> {
        let conn = self.lock()?;
        if params.is_empty() && sql.contains(';') {
            // Multi-statement batches (DDL scripts, seeds).
            self.rt
                .block_on(conn.execute_batch(sql))
                .map_err(ls_err)?;
            return Ok(0);
        }
        let n = self
            .rt
            .block_on(conn.execute(sql, bind(params)))
            .map_err(ls_err)?;
        Ok(n as usize)
    }

    pub fn query(&self, sql: &str, params: &[Value]) -> Result<RecordBatch> {
        let conn = self.lock()?;
        let (names, rows) = self
            .rt
            .block_on(async {
                let mut rows = conn.query(sql, bind(params)).await?;
                let n = rows.column_count();
                let names: Vec<String> = (0..n)
                    .map(|i| rows.column_name(i).unwrap_or("").to_string())
                    .collect();
                let mut out: Vec<Vec<LsValue>> = Vec::new();
                while let Some(row) = rows.next().await? {
                    let mut cells = Vec::with_capacity(n as usize);
                    for i in 0..n {
                        cells.push(row.get_value(i)?);
                    }
                    out.push(cells);
                }
                Ok::<_, libsql::Error>((names, out))
            })
            .map_err(ls_err)?;
        drop(conn);
        rows_to_batch(&names, &rows)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, libsql::Connection>> {
        self.conn
            .lock()
            .map_err(|_| Error::Database("libsql connection mutex poisoned".into()))
    }
}

fn bind(params: &[Value]) -> Vec<LsValue> {
    params
        .iter()
        .map(|v| match v {
            Value::Null => LsValue::Null,
            Value::Int(i) => LsValue::Integer(*i),
            Value::Float(f) => LsValue::Real(*f),
            Value::Text(s) => LsValue::Text(s.clone()),
            Value::Bool(b) => LsValue::Integer(*b as i64),
        })
        .collect()
}

/// Column types are dynamic (SQLite model): infer each column's PCB type
/// from its first non-NULL value; an all-NULL column decodes as Int64 with
/// every slot invalid — the same shape the SQLite backend produces.
fn rows_to_batch(names: &[String], rows: &[Vec<LsValue>]) -> Result<RecordBatch> {
    if rows.is_empty() {
        return RecordBatch::try_new(vec![]);
    }
    let mut out = Vec::with_capacity(names.len());
    for (ci, name) in names.iter().enumerate() {
        let dt = rows
            .iter()
            .find_map(|r| match &r[ci] {
                LsValue::Null => None,
                LsValue::Integer(_) => Some(Ok(DataType::Int64)),
                LsValue::Real(_) => Some(Ok(DataType::Float64)),
                LsValue::Text(_) => Some(Ok(DataType::Utf8)),
                LsValue::Blob(_) => Some(Err(Error::Unsupported(format!(
                    "column `{name}`: BLOB has no PCB mapping"
                )))),
            })
            .unwrap_or(Ok(DataType::Int64))?;
        let mut b = ColumnBuilder::new(dt);
        for r in rows {
            match (&r[ci], dt) {
                (LsValue::Null, _) => b.push_null(),
                (LsValue::Integer(i), DataType::Int64) => b.push_i64(*i)?,
                (LsValue::Integer(i), DataType::Float64) => b.push_f64(*i as f64)?,
                (LsValue::Real(f), DataType::Float64) => b.push_f64(*f)?,
                (LsValue::Text(s), DataType::Utf8) => b.push_str(s)?,
                (other, _) => {
                    return Err(Error::Database(format!(
                        "column `{name}`: mixed value {other:?} in a {dt:?} column"
                    )))
                }
            }
        }
        out.push(b.finish(name.clone()));
    }
    RecordBatch::try_new(out)
}

fn ls_err(e: libsql::Error) -> Error {
    Error::Database(format!("libsql: {e}"))
}
