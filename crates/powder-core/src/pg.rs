//! PostgreSQL runtime backend (feature `postgres`).
//!
//! Mirrors the SQLite path: run a query, stream rows into the shared
//! [`ColumnBuilder`]s, hand back a [`RecordBatch`] the codec can encode to
//! PCB unchanged. The `postgres` crate is synchronous, so calls are
//! dispatched to Tokio's blocking pool by the [`crate::Client`] wrapper,
//! same as SQLite.
//!
//! SQL arrives with SQLite-style `?` placeholders (that is what the query
//! builder and every binding emit); they are rewritten to `$1..$n` here,
//! skipping string literals, quoted identifiers, and comments.

use std::sync::Mutex;

use postgres::types::{ToSql, Type};
use postgres::{Client as PgConn, NoTls, Row};

use crate::array::ColumnBuilder;
use crate::batch::RecordBatch;
use crate::error::{Error, Result};
use crate::query::Value;
use crate::schema::DataType;

pub struct PgBackend {
    conn: Mutex<PgConn>,
}

impl PgBackend {
    pub fn connect(url: &str) -> Result<Self> {
        let conn = PgConn::connect(url, NoTls)
            .map_err(|e| Error::Database(format!("postgres connect: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<usize> {
        let sql = translate_placeholders(sql);
        let bound = bind(params);
        let refs: Vec<&(dyn ToSql + Sync)> =
            bound.iter().map(|b| b.as_ref() as &(dyn ToSql + Sync)).collect();
        let mut conn = self.lock()?;
        // `execute` rejects statements that return rows; batch DDL (multiple
        // statements) needs `batch_execute`. Route by shape.
        if params.is_empty() && sql.contains(';') {
            conn.batch_execute(&sql)
                .map_err(|e| Error::Database(e.to_string()))?;
            return Ok(0);
        }
        let n = conn
            .execute(sql.as_str(), &refs)
            .map_err(|e| Error::Database(e.to_string()))?;
        Ok(n as usize)
    }

    pub fn query(&self, sql: &str, params: &[Value]) -> Result<RecordBatch> {
        let sql = translate_placeholders(sql);
        let bound = bind(params);
        let refs: Vec<&(dyn ToSql + Sync)> =
            bound.iter().map(|b| b.as_ref() as &(dyn ToSql + Sync)).collect();
        let mut conn = self.lock()?;
        let rows = conn
            .query(sql.as_str(), &refs)
            .map_err(|e| Error::Database(e.to_string()))?;
        drop(conn);
        rows_to_batch(&rows)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, PgConn>> {
        self.conn
            .lock()
            .map_err(|_| Error::Database("postgres connection mutex poisoned".into()))
    }
}

/// `?` → `$1..$n`, leaving `'…'` / `"…"` / `-- …` / `/* … */` untouched.
///
/// Works on raw bytes (only ASCII is ever inserted, so the output stays
/// valid UTF-8 for any valid UTF-8 input).
pub fn translate_placeholders(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(sql.len() + 8);
    let mut n = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'\'' | b'"') => {
                out.push(quote);
                i += 1;
                while i < bytes.len() {
                    out.push(bytes[i]);
                    // Doubled quote ('') is an escaped quote, not a close.
                    if bytes[i] == quote {
                        if i + 1 < bytes.len() && bytes[i + 1] == quote {
                            out.push(quote);
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                out.extend_from_slice(b"/*");
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                        out.extend_from_slice(b"*/");
                        i += 2;
                        break;
                    }
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'?' => {
                n += 1;
                out.push(b'$');
                out.extend_from_slice(n.to_string().as_bytes());
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).expect("only ASCII inserted into valid UTF-8")
}

fn bind(params: &[Value]) -> Vec<Box<dyn ToSql + Sync + Send>> {
    params
        .iter()
        .map(|v| -> Box<dyn ToSql + Sync + Send> {
            match v {
                Value::Null => Box::new(Option::<i64>::None),
                Value::Int(i) => Box::new(*i),
                Value::Float(f) => Box::new(*f),
                Value::Text(s) => Box::new(s.clone()),
                Value::Bool(b) => Box::new(*b),
            }
        })
        .collect()
}

/// Map a Postgres column type onto one of the four PCB types.
fn pcb_type(ty: &Type) -> Result<DataType> {
    Ok(match *ty {
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => DataType::Int64,
        Type::FLOAT4 | Type::FLOAT8 => DataType::Float64,
        Type::BOOL => DataType::Bool,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => DataType::Utf8,
        _ => {
            return Err(Error::Unsupported(format!(
                "postgres type `{ty}` has no PCB mapping; cast it in SQL (e.g. `col::text`)"
            )))
        }
    })
}

/// Decode a Postgres result set into the columnar model.
fn rows_to_batch(rows: &[Row]) -> Result<RecordBatch> {
    if rows.is_empty() {
        return RecordBatch::try_new(vec![]);
    }
    let cols = rows[0].columns();
    let mut out = Vec::with_capacity(cols.len());
    for (ci, col) in cols.iter().enumerate() {
        let name = col.name().to_string();
        let ty = col.type_().clone();
        let dt = pcb_type(&ty).map_err(|e| match e {
            Error::Unsupported(m) => Error::Unsupported(format!("column `{name}`: {m}")),
            other => other,
        })?;
        let mut b = ColumnBuilder::new(dt);
        for r in rows {
            push_cell(&mut b, r, ci, &ty)?;
        }
        out.push(b.finish(name));
    }
    RecordBatch::try_new(out)
}

fn push_cell(b: &mut ColumnBuilder, row: &Row, ci: usize, ty: &Type) -> Result<()> {
    match *ty {
        Type::INT2 => match row.try_get::<_, Option<i16>>(ci).map_err(pg_err)? {
            Some(v) => b.push_i64(v as i64)?,
            None => b.push_null(),
        },
        Type::INT4 => match row.try_get::<_, Option<i32>>(ci).map_err(pg_err)? {
            Some(v) => b.push_i64(v as i64)?,
            None => b.push_null(),
        },
        Type::INT8 => match row.try_get::<_, Option<i64>>(ci).map_err(pg_err)? {
            Some(v) => b.push_i64(v)?,
            None => b.push_null(),
        },
        Type::OID => match row.try_get::<_, Option<u32>>(ci).map_err(pg_err)? {
            Some(v) => b.push_i64(v as i64)?,
            None => b.push_null(),
        },
        Type::FLOAT4 => match row.try_get::<_, Option<f32>>(ci).map_err(pg_err)? {
            Some(v) => b.push_f64(v as f64)?,
            None => b.push_null(),
        },
        Type::FLOAT8 => match row.try_get::<_, Option<f64>>(ci).map_err(pg_err)? {
            Some(v) => b.push_f64(v)?,
            None => b.push_null(),
        },
        Type::BOOL => match row.try_get::<_, Option<bool>>(ci).map_err(pg_err)? {
            Some(v) => b.push_bool(v)?,
            None => b.push_null(),
        },
        _ => match row.try_get::<_, Option<String>>(ci).map_err(pg_err)? {
            Some(v) => b.push_str(&v)?,
            None => b.push_null(),
        },
    }
    Ok(())
}

fn pg_err(e: postgres::Error) -> Error {
    Error::Database(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_become_dollar_n() {
        assert_eq!(
            translate_placeholders("SELECT * FROM t WHERE a = ? AND b IN (?, ?)"),
            "SELECT * FROM t WHERE a = $1 AND b IN ($2, $3)"
        );
    }

    #[test]
    fn placeholders_inside_literals_survive() {
        assert_eq!(
            translate_placeholders("SELECT '?' , \"col?\" , x FROM t WHERE y = ? -- t?\n AND z = ?"),
            "SELECT '?' , \"col?\" , x FROM t WHERE y = $1 -- t?\n AND z = $2"
        );
        assert_eq!(
            translate_placeholders("SELECT 'it''s ?' /* ? */ WHERE a = ?"),
            "SELECT 'it''s ?' /* ? */ WHERE a = $1"
        );
    }

    #[test]
    fn multibyte_text_is_copied_verbatim() {
        assert_eq!(
            translate_placeholders("SELECT * FROM t WHERE name = ? -- 한글 주석"),
            "SELECT * FROM t WHERE name = $1 -- 한글 주석"
        );
    }
}
