//! SQL dialect layer: everything DDL-shaped goes through here so adding a
//! second backend means implementing one trait, not editing call sites.

use crate::schema::{Column, Table};

pub trait SqlDialect {
    /// `CREATE TABLE IF NOT EXISTS ...` for the whole table, including
    /// composite primary keys and foreign-key constraints.
    fn create_table(&self, table: &Table) -> String;

    /// `ALTER TABLE ... ADD COLUMN ...`, or an error when the dialect cannot
    /// add this column in place (e.g. a primary-key member).
    fn add_column(&self, table: &Table, col: &Column) -> Result<String, String>;
}

pub struct Sqlite;

impl Sqlite {
    fn column_def(&self, table: &Table, col: &Column, inline_pk: bool) -> String {
        let mut s = format!("{} {}", col.name, col.def.column_type.sql_type());
        if inline_pk && col.def.primary_key {
            s.push_str(" PRIMARY KEY");
        } else if !col.def.nullable {
            s.push_str(" NOT NULL");
        }
        let _ = table;
        s
    }
}

impl SqlDialect for Sqlite {
    fn create_table(&self, table: &Table) -> String {
        let pk_cols: Vec<&Column> = table.columns.iter().filter(|c| c.def.primary_key).collect();
        // A single PK stays inline (`INTEGER PRIMARY KEY` keeps the rowid
        // alias); a composite PK becomes a table-level constraint.
        let inline_pk = pk_cols.len() == 1;

        let mut parts: Vec<String> = table
            .columns
            .iter()
            .map(|c| self.column_def(table, c, inline_pk))
            .collect();

        if pk_cols.len() > 1 {
            parts.push(format!(
                "PRIMARY KEY ({})",
                pk_cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        for col in &table.columns {
            if let Some(r) = &col.def.references {
                parts.push(format!(
                    "FOREIGN KEY ({}) REFERENCES {}({})",
                    col.name, r.table, r.column
                ));
            }
        }

        format!(
            "CREATE TABLE IF NOT EXISTS {} ({})",
            table.name,
            parts.join(", ")
        )
    }

    fn add_column(&self, table: &Table, col: &Column) -> Result<String, String> {
        if col.def.primary_key {
            return Err(format!(
                "table `{}`: cannot add primary key column `{}` to an existing table (use --rebuild)",
                table.name, col.name
            ));
        }
        let mut ddl = format!(
            "ALTER TABLE {} ADD COLUMN {} {}",
            table.name,
            col.name,
            col.def.column_type.sql_type()
        );
        // SQLite requires a default when adding NOT NULL columns to a
        // populated table.
        if !col.def.nullable {
            ddl.push_str(if col.def.column_type.sql_type() == "TEXT" {
                " NOT NULL DEFAULT ''"
            } else {
                " NOT NULL DEFAULT 0"
            });
        }
        if let Some(r) = &col.def.references {
            ddl.push_str(&format!(" REFERENCES {}({})", r.table, r.column));
        }
        Ok(ddl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schema;

    #[test]
    fn composite_pk_becomes_table_constraint() {
        let schema = Schema::parse(
            r#"{"tables":{"m":{"columns":{
                "a":{"type":"int","primaryKey":true},
                "b":{"type":"text","primaryKey":true},
                "v":{"type":"float","nullable":true}
            }}}}"#,
        )
        .unwrap();
        let ddl = Sqlite.create_table(&schema.tables[0]);
        assert_eq!(
            ddl,
            "CREATE TABLE IF NOT EXISTS m (a INTEGER NOT NULL, b TEXT NOT NULL, v REAL, PRIMARY KEY (a, b))"
        );
    }

    #[test]
    fn foreign_keys_render_as_constraints() {
        let schema = Schema::parse(
            r#"{"tables":{
                "users":{"columns":{"id":{"type":"int","primaryKey":true}}},
                "posts":{"columns":{
                    "id":{"type":"int","primaryKey":true},
                    "user_id":{"type":"int","references":{"table":"users","column":"id"}}
                }}
            }}"#,
        )
        .unwrap();
        let posts = schema.tables.iter().find(|t| t.name == "posts").unwrap();
        let ddl = Sqlite.create_table(posts);
        assert_eq!(
            ddl,
            "CREATE TABLE IF NOT EXISTS posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, FOREIGN KEY (user_id) REFERENCES users(id))"
        );
    }
}
