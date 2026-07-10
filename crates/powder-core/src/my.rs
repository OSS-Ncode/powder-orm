//! MySQL / MariaDB runtime backend (feature `mysql`).
//!
//! Mirrors the SQLite/Postgres paths: run a query, stream rows into the
//! shared [`ColumnBuilder`]s, hand back a [`RecordBatch`] the codec encodes
//! to PCB unchanged. MySQL uses `?` placeholders natively, so no SQL
//! rewriting is needed. The `mysql` crate is synchronous; calls are
//! dispatched to Tokio's blocking pool by the [`crate::Client`] wrapper.

use std::sync::Mutex;

use mysql::consts::ColumnType as MyType;
use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder, Row, Value as MyValue};

use crate::array::ColumnBuilder;
use crate::batch::RecordBatch;
use crate::error::{Error, Result};
use crate::query::Value;
use crate::schema::DataType;

pub struct MyBackend {
    conn: Mutex<Conn>,
}

impl MyBackend {
    pub fn connect(url: &str) -> Result<Self> {
        let opts = Opts::from_url(url)
            .map_err(|e| Error::InvalidUrl(format!("mysql url: {e}")))?;
        // Multi-statement batches (DDL scripts, seeds) come through execute().
        let opts = OptsBuilder::from_opts(opts)
            .additional_capabilities(mysql::consts::CapabilityFlags::CLIENT_MULTI_STATEMENTS);
        let conn = Conn::new(opts).map_err(|e| Error::Database(format!("mysql connect: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<usize> {
        // Every binding's transaction helper speaks SQLite: `BEGIN IMMEDIATE`
        // (no IMMEDIATE mode in MySQL) and `RELEASE <sp>` (MySQL requires the
        // SAVEPOINT keyword). Normalize both.
        let trimmed = sql.trim();
        let rewritten: Option<String> = if trimmed.eq_ignore_ascii_case("BEGIN IMMEDIATE") {
            Some("BEGIN".into())
        } else if trimmed.len() > 8
            && trimmed[..8].eq_ignore_ascii_case("RELEASE ")
            && !trimmed[8..].trim_start().get(..9).is_some_and(|w| w.eq_ignore_ascii_case("SAVEPOINT"))
        {
            Some(format!("RELEASE SAVEPOINT {}", trimmed[8..].trim_start()))
        } else {
            None
        };
        let sql = rewritten.as_deref().unwrap_or(sql);
        let mut conn = self.lock()?;
        if params.is_empty() {
            // query_drop drains multi-statement results too.
            conn.query_drop(sql)
                .map_err(|e| Error::Database(e.to_string()))?;
        } else {
            conn.exec_drop(sql, bind(params))
                .map_err(|e| Error::Database(e.to_string()))?;
        }
        Ok(conn.affected_rows() as usize)
    }

    pub fn query(&self, sql: &str, params: &[Value]) -> Result<RecordBatch> {
        let mut conn = self.lock()?;
        let rows: Vec<Row> = if params.is_empty() {
            conn.query(sql).map_err(|e| Error::Database(e.to_string()))?
        } else {
            conn.exec(sql, bind(params))
                .map_err(|e| Error::Database(e.to_string()))?
        };
        drop(conn);
        rows_to_batch(&rows)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Conn>> {
        self.conn
            .lock()
            .map_err(|_| Error::Database("mysql connection mutex poisoned".into()))
    }
}

fn bind(params: &[Value]) -> Vec<MyValue> {
    params
        .iter()
        .map(|v| match v {
            Value::Null => MyValue::NULL,
            Value::Int(i) => MyValue::Int(*i),
            Value::Float(f) => MyValue::Double(*f),
            Value::Text(s) => MyValue::Bytes(s.clone().into_bytes()),
            Value::Bool(b) => MyValue::Int(*b as i64),
        })
        .collect()
}

/// Map a MySQL column type onto one of the four PCB types.
fn pcb_type(ty: MyType, name: &str) -> Result<DataType> {
    use MyType::*;
    Ok(match ty {
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_INT24
        | MYSQL_TYPE_LONGLONG | MYSQL_TYPE_YEAR | MYSQL_TYPE_BIT => DataType::Int64,
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => DataType::Float64,
        MYSQL_TYPE_VARCHAR | MYSQL_TYPE_VAR_STRING | MYSQL_TYPE_STRING | MYSQL_TYPE_BLOB
        | MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB
        | MYSQL_TYPE_JSON | MYSQL_TYPE_ENUM | MYSQL_TYPE_SET => DataType::Utf8,
        other => {
            return Err(Error::Unsupported(format!(
                "mysql column `{name}` has type {other:?}; cast it in SQL (e.g. CAST(col AS CHAR))"
            )))
        }
    })
}

fn rows_to_batch(rows: &[Row]) -> Result<RecordBatch> {
    if rows.is_empty() {
        return RecordBatch::try_new(vec![]);
    }
    let cols = rows[0].columns_ref().to_vec();
    let mut out = Vec::with_capacity(cols.len());
    for (ci, col) in cols.iter().enumerate() {
        let name = col.name_str().to_string();
        let dt = pcb_type(col.column_type(), &name)?;
        let is_bit = col.column_type() == MyType::MYSQL_TYPE_BIT;
        let mut b = ColumnBuilder::new(dt);
        for r in rows {
            push_cell(&mut b, r, ci, dt, &name, is_bit)?;
        }
        out.push(b.finish(name));
    }
    RecordBatch::try_new(out)
}

fn push_cell(
    b: &mut ColumnBuilder,
    row: &Row,
    ci: usize,
    dt: DataType,
    name: &str,
    is_bit: bool,
) -> Result<()> {
    let val = row
        .as_ref(ci)
        .ok_or_else(|| Error::Database(format!("column `{name}`: missing cell")))?;
    match (dt, val) {
        (_, MyValue::NULL) => b.push_null(),
        (DataType::Int64, MyValue::Int(i)) => b.push_i64(*i)?,
        (DataType::Int64, MyValue::UInt(u)) => b.push_i64(*u as i64)?,
        (DataType::Int64, MyValue::Bytes(bs)) => {
            if is_bit {
                // BIT columns arrive as big-endian byte strings.
                let mut acc: i64 = 0;
                for byte in bs {
                    acc = (acc << 8) | *byte as i64;
                }
                b.push_i64(acc)?;
            } else {
                // Text-protocol rows (parameterless queries) carry every
                // value as its decimal string.
                let s = std::str::from_utf8(bs).map_err(|_| {
                    Error::Database(format!("column `{name}`: non-UTF-8 integer payload"))
                })?;
                let v: i64 = s.trim().parse().map_err(|_| {
                    Error::Database(format!("column `{name}`: cannot parse `{s}` as integer"))
                })?;
                b.push_i64(v)?;
            }
        }
        (DataType::Float64, MyValue::Float(f)) => b.push_f64(*f as f64)?,
        (DataType::Float64, MyValue::Double(f)) => b.push_f64(*f)?,
        (DataType::Float64, MyValue::Int(i)) => b.push_f64(*i as f64)?,
        (DataType::Float64, MyValue::Bytes(bs)) => {
            // Text-protocol float (see the Int64 branch above).
            let s = std::str::from_utf8(bs).map_err(|_| {
                Error::Database(format!("column `{name}`: non-UTF-8 float payload"))
            })?;
            let v: f64 = s.trim().parse().map_err(|_| {
                Error::Database(format!("column `{name}`: cannot parse `{s}` as float"))
            })?;
            b.push_f64(v)?;
        }
        (DataType::Utf8, MyValue::Bytes(bs)) => {
            let s = std::str::from_utf8(bs).map_err(|_| {
                Error::Unsupported(format!(
                    "column `{name}`: non-UTF-8 binary payload (BLOB) has no PCB mapping"
                ))
            })?;
            b.push_str(s)?;
        }
        (_, other) => {
            return Err(Error::Database(format!(
                "column `{name}`: unexpected value {other:?} for {dt:?}"
            )))
        }
    }
    Ok(())
}
