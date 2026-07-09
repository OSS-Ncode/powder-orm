//! The JSON Schema for `powder.schema.json` itself.
//!
//! `powder new` / `powder init` write this next to the config as
//! `powder.schema.schema.json` and point the config's `$schema` at it, which
//! is what gives editors (VS Code & friends) completion, validation, and
//! hover docs while editing the Powder schema — table/column/type names are
//! offered as you type.

/// JSON Schema (draft-07 — the dialect VS Code supports best) describing
/// `powder.schema.json`.
pub const SCHEMA_OF_SCHEMA: &str = r##"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Powder schema",
  "description": "Declarative database schema consumed by `powder generate` / `powder migrate`.",
  "type": "object",
  "additionalProperties": false,
  "required": ["tables"],
  "properties": {
    "$schema": { "type": "string" },
    "database": {
      "type": "string",
      "description": "Informational database label (e.g. a default connection URL)."
    },
    "tables": {
      "type": "object",
      "description": "Table name -> table definition.",
      "propertyNames": { "pattern": "^[A-Za-z_][A-Za-z0-9_]*$" },
      "additionalProperties": { "$ref": "#/definitions/table" }
    },
    "queries": {
      "type": "object",
      "description": "Named queries, AOT-compiled by `powder generate` and exposed as db.$queries.<name>() / db.queries.<name>().",
      "propertyNames": { "pattern": "^[A-Za-z_][A-Za-z0-9_]*$" },
      "additionalProperties": { "$ref": "#/definitions/query" }
    }
  },
  "definitions": {
    "columnType": {
      "type": "string",
      "enum": ["int", "float", "text", "bool"],
      "description": "Column type. int -> INTEGER/BIGINT, float -> REAL/DOUBLE, text -> TEXT, bool -> 0/1 boolean."
    },
    "table": {
      "type": "object",
      "additionalProperties": false,
      "required": ["columns"],
      "properties": {
        "columns": {
          "type": "object",
          "description": "Column name -> column definition.",
          "propertyNames": { "pattern": "^[A-Za-z_][A-Za-z0-9_]*$" },
          "additionalProperties": { "$ref": "#/definitions/column" },
          "minProperties": 1
        },
        "foreignKeys": {
          "type": "array",
          "description": "Composite (multi-column) foreign keys. Single-column FKs are simpler as `references` on the column.",
          "items": { "$ref": "#/definitions/foreignKey" }
        }
      }
    },
    "column": {
      "type": "object",
      "additionalProperties": false,
      "required": ["type"],
      "properties": {
        "type": { "$ref": "#/definitions/columnType" },
        "nullable": {
          "type": "boolean",
          "default": false,
          "description": "Whether NULL is allowed. Non-nullable columns render as NOT NULL."
        },
        "primaryKey": {
          "type": "boolean",
          "default": false,
          "description": "Primary-key member. Multiple columns form a composite key."
        },
        "references": {
          "type": "object",
          "additionalProperties": false,
          "required": ["table", "column"],
          "description": "Single-column foreign key: this column references table(column). Also derives the ORM relation (belongsTo / hasMany).",
          "properties": {
            "table": { "type": "string" },
            "column": { "type": "string" }
          }
        }
      }
    },
    "foreignKey": {
      "type": "object",
      "additionalProperties": false,
      "required": ["columns", "references"],
      "properties": {
        "columns": {
          "type": "array",
          "items": { "type": "string" },
          "minItems": 1,
          "description": "Local columns, in order."
        },
        "references": {
          "type": "object",
          "additionalProperties": false,
          "required": ["table", "columns"],
          "properties": {
            "table": { "type": "string" },
            "columns": {
              "type": "array",
              "items": { "type": "string" },
              "minItems": 1,
              "description": "Referenced columns, aligned with `columns`."
            }
          }
        }
      }
    },
    "query": {
      "type": "object",
      "additionalProperties": false,
      "required": ["sql"],
      "properties": {
        "sql": {
          "type": "string",
          "description": "SQL with :name placeholders. Compiled to positional binds by `powder generate`; every placeholder must be declared in `params` and vice versa."
        },
        "params": {
          "type": "object",
          "description": "Parameter name -> type.",
          "propertyNames": { "pattern": "^[A-Za-z_][A-Za-z0-9_]*$" },
          "additionalProperties": { "$ref": "#/definitions/columnType" }
        },
        "returns": {
          "type": "string",
          "description": "Optional table whose row shape the query returns — gives the generated method a typed result."
        }
      }
    }
  }
}
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Schema, SAMPLE_SCHEMA};

    #[test]
    fn schema_of_schema_is_valid_json() {
        let v: serde_json::Value = serde_json::from_str(SCHEMA_OF_SCHEMA).unwrap();
        assert_eq!(v["title"], "Powder schema");
    }

    #[test]
    fn sample_schema_with_dollar_schema_key_still_parses() {
        let mut v: serde_json::Value = serde_json::from_str(SAMPLE_SCHEMA).unwrap();
        v["$schema"] = serde_json::Value::String("./powder.schema.schema.json".into());
        Schema::parse(&v.to_string()).unwrap();
    }
}
