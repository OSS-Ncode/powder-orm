//! `powder.schema.json` — the single source of truth for Powder ORM.
//!
//! The same file drives migration DDL, live-database validation, and the AOT
//! code generators for TypeScript and Python, which is what keeps every
//! language binding's model layer in lockstep with the database.

use serde::Deserialize;
use serde_json::Map;

/// Logical column types. Deliberately the NCB type set: what the wire format
/// can carry zero-copy is exactly what a model may declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    Int,
    Float,
    Text,
    Bool,
}

impl ColumnType {
    /// SQLite storage type used in DDL and expected by `validate`.
    pub fn sql_type(self) -> &'static str {
        match self {
            ColumnType::Int | ColumnType::Bool => "INTEGER",
            ColumnType::Float => "REAL",
            ColumnType::Text => "TEXT",
        }
    }

    pub fn ts_type(self) -> &'static str {
        match self {
            ColumnType::Int | ColumnType::Float => "number",
            ColumnType::Text => "string",
            ColumnType::Bool => "boolean",
        }
    }

    pub fn py_type(self) -> &'static str {
        match self {
            ColumnType::Int => "int",
            ColumnType::Float => "float",
            ColumnType::Text => "str",
            ColumnType::Bool => "bool",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ColumnType::Int => "int",
            ColumnType::Float => "float",
            ColumnType::Text => "text",
            ColumnType::Bool => "bool",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub table: String,
    pub column: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnDef {
    #[serde(rename = "type")]
    pub column_type: ColumnType,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default, rename = "primaryKey")]
    pub primary_key: bool,
    /// Foreign key: this column references `table.column`.
    #[serde(default)]
    pub references: Option<Reference>,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub def: ColumnDef,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub tables: Vec<Table>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSchema {
    #[serde(default)]
    #[allow(dead_code)]
    database: Option<String>,
    tables: Map<String, serde_json::Value>,
}

/// A simple identifier: what Powder allows for table/column names. Everything
/// generated (SQL, TS, Python) interpolates these bare, so the gate is strict.
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Schema {
    /// Parse and structurally validate a `powder.schema.json` document.
    pub fn parse(json: &str) -> Result<Schema, String> {
        let raw: RawSchema =
            serde_json::from_str(json).map_err(|e| format!("invalid schema JSON: {e}"))?;
        let mut tables = Vec::with_capacity(raw.tables.len());
        for (tname, tval) in raw.tables {
            if !is_ident(&tname) {
                return Err(format!("table `{tname}`: not a valid identifier"));
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct RawTable {
                columns: Map<String, serde_json::Value>,
            }
            let rt: RawTable = serde_json::from_value(tval)
                .map_err(|e| format!("table `{tname}`: {e}"))?;
            if rt.columns.is_empty() {
                return Err(format!("table `{tname}`: has no columns"));
            }
            let mut columns = Vec::with_capacity(rt.columns.len());
            for (cname, cval) in rt.columns {
                if !is_ident(&cname) {
                    return Err(format!("table `{tname}`: column `{cname}` is not a valid identifier"));
                }
                let def: ColumnDef = serde_json::from_value(cval)
                    .map_err(|e| format!("table `{tname}`, column `{cname}`: {e}"))?;
                if def.primary_key && def.nullable {
                    return Err(format!(
                        "table `{tname}`, column `{cname}`: a primary key cannot be nullable"
                    ));
                }
                if let Some(r) = &def.references {
                    if !is_ident(&r.table) || !is_ident(&r.column) {
                        return Err(format!(
                            "table `{tname}`, column `{cname}`: reference target is not a valid identifier"
                        ));
                    }
                }
                columns.push(Column { name: cname, def });
            }
            tables.push(Table { name: tname, columns });
        }
        if tables.is_empty() {
            return Err("schema has no tables".into());
        }
        let schema = Schema { tables };
        schema.check_references()?;
        Ok(schema)
    }

    /// Every `references` must point at an existing table + column, and the
    /// referenced column must be that table's (single) primary key or at
    /// least exist. Composite keys may be referenced column-by-column.
    fn check_references(&self) -> Result<(), String> {
        for table in &self.tables {
            for col in &table.columns {
                let Some(r) = &col.def.references else { continue };
                let Some(target) = self.tables.iter().find(|t| t.name == r.table) else {
                    return Err(format!(
                        "table `{}`, column `{}`: references unknown table `{}`",
                        table.name, col.name, r.table
                    ));
                };
                let Some(tcol) = target.columns.iter().find(|c| c.name == r.column) else {
                    return Err(format!(
                        "table `{}`, column `{}`: references unknown column `{}.{}`",
                        table.name, col.name, r.table, r.column
                    ));
                };
                if tcol.def.column_type != col.def.column_type {
                    return Err(format!(
                        "table `{}`, column `{}`: type `{}` does not match referenced `{}.{}` type `{}`",
                        table.name,
                        col.name,
                        col.def.column_type.name(),
                        r.table,
                        r.column,
                        tcol.def.column_type.name()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Tables ordered so referenced tables come before referencing ones
    /// (best-effort: cycles fall back to declaration order).
    pub fn tables_in_dependency_order(&self) -> Vec<&Table> {
        let mut placed: Vec<&Table> = Vec::with_capacity(self.tables.len());
        let mut remaining: Vec<&Table> = self.tables.iter().collect();
        while !remaining.is_empty() {
            let before = placed.len();
            remaining.retain(|t| {
                let deps_ok = t.columns.iter().all(|c| match &c.def.references {
                    Some(r) => {
                        r.table == t.name || placed.iter().any(|p| p.name == r.table)
                    }
                    None => true,
                });
                if deps_ok {
                    placed.push(t);
                    false
                } else {
                    true
                }
            });
            if placed.len() == before {
                // Cycle: append the rest in declaration order.
                placed.extend(remaining.drain(..));
            }
        }
        placed
    }
}

impl Table {
    /// `CREATE TABLE IF NOT EXISTS ...` DDL for this table (SQLite dialect).
    pub fn create_ddl(&self) -> String {
        use crate::dialect::{Sqlite, SqlDialect};
        Sqlite.create_table(self)
    }

    /// The columns forming the primary key, in declaration order.
    pub fn primary_key(&self) -> Vec<&Column> {
        self.columns.iter().filter(|c| c.def.primary_key).collect()
    }
}

/// The default schema written by `powder init` / `powder new`.
pub const SAMPLE_SCHEMA: &str = r#"{
  "database": "sqlite",
  "tables": {
    "users": {
      "columns": {
        "id": { "type": "int", "primaryKey": true },
        "name": { "type": "text" },
        "score": { "type": "float", "nullable": true },
        "active": { "type": "bool" }
      }
    },
    "posts": {
      "columns": {
        "id": { "type": "int", "primaryKey": true },
        "user_id": { "type": "int", "references": { "table": "users", "column": "id" } },
        "title": { "type": "text" }
      }
    }
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sample_and_generates_ddl() {
        let schema = Schema::parse(SAMPLE_SCHEMA).unwrap();
        assert_eq!(schema.tables.len(), 2);
        let users = &schema.tables[0];
        assert_eq!(users.name, "users");
        assert_eq!(
            users.create_ddl(),
            "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL, active INTEGER NOT NULL)"
        );
        let posts = &schema.tables[1];
        assert!(posts.create_ddl().contains("FOREIGN KEY (user_id) REFERENCES users(id)"));
    }

    #[test]
    fn rejects_bad_identifiers_and_shapes() {
        assert!(Schema::parse(r#"{"tables":{"bad name":{"columns":{"a":{"type":"int"}}}}}"#).is_err());
        assert!(Schema::parse(r#"{"tables":{"t":{"columns":{}}}}"#).is_err());
        assert!(Schema::parse(r#"{"tables":{"t":{"columns":{"a":{"type":"wat"}}}}}"#).is_err());
        assert!(Schema::parse(
            r#"{"tables":{"t":{"columns":{"a":{"type":"int","primaryKey":true,"nullable":true}}}}}"#
        )
        .is_err());
        assert!(Schema::parse(r#"{"tables":{}}"#).is_err());
    }

    #[test]
    fn composite_primary_keys_are_supported() {
        let schema = Schema::parse(
            r#"{"tables":{"m":{"columns":{
                "a":{"type":"int","primaryKey":true},
                "b":{"type":"text","primaryKey":true}
            }}}}"#,
        )
        .unwrap();
        assert_eq!(schema.tables[0].primary_key().len(), 2);
    }

    #[test]
    fn references_are_validated() {
        // Unknown table.
        assert!(Schema::parse(
            r#"{"tables":{"t":{"columns":{"x":{"type":"int","references":{"table":"nope","column":"id"}}}}}}"#
        )
        .is_err());
        // Type mismatch with the referenced column.
        assert!(Schema::parse(
            r#"{"tables":{
                "u":{"columns":{"id":{"type":"int","primaryKey":true}}},
                "t":{"columns":{"x":{"type":"text","references":{"table":"u","column":"id"}}}}
            }}"#
        )
        .is_err());
    }

    #[test]
    fn dependency_order_puts_referenced_tables_first() {
        let schema = Schema::parse(
            r#"{"tables":{
                "posts":{"columns":{"user_id":{"type":"int","references":{"table":"users","column":"id"}}}},
                "users":{"columns":{"id":{"type":"int","primaryKey":true}}}
            }}"#,
        )
        .unwrap();
        let order: Vec<&str> = schema
            .tables_in_dependency_order()
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(order, ["users", "posts"]);
    }
}
