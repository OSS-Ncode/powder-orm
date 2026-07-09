//! `powder` — the Powder ORM command-line tool.
//!
//! ```text
//! powder init                                  # write a starter powder.schema.json
//! powder generate [--schema F] [--ts F] [--py F] [--ts-import M]
//! powder migrate  --db URL [--schema F]        # create tables / add columns
//! powder validate --db URL [--schema F]        # schema<->db gate; exits 1 on drift
//! powder seed     --db URL --file F            # apply .json or .sql seed data
//! ```

use std::process::ExitCode;

use powder_cli::{codegen, db, dialect, scaffold, schema::Schema, schema::SAMPLE_SCHEMA};

const USAGE: &str = "\
powder — Powder ORM CLI

USAGE:
  powder new <dir>                              # scaffold a new Powder project
  powder init
  powder generate [--schema powder.schema.json] [--ts <out.ts>] [--py <out.py>] [--ts-import <module>]
  powder ddl      [--schema powder.schema.json] [--dialect sqlite|postgres]
  powder migrate  --db <url> [--schema powder.schema.json] [--rebuild]
  powder validate --db <url> [--schema powder.schema.json]
  powder seed     --db <url> --file <seed.json|seed.sql>

`migrate` is additive (CREATE TABLE / ADD COLUMN). With --rebuild, tables
whose live shape drifted destructively (dropped columns, type or key changes)
are rebuilt in place, preserving data in surviving columns.
`ddl` prints CREATE TABLE statements for the chosen dialect (default sqlite).

Database URLs accept the same forms as the Powder client:
  sqlite::memory: | sqlite://path/to.db | path/to.db
";

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn load_schema(args: &[String]) -> Result<Schema, String> {
    let path = flag(args, "--schema").unwrap_or_else(|| "powder.schema.json".into());
    let json = std::fs::read_to_string(&path).map_err(|e| format!("cannot read `{path}`: {e}"))?;
    Schema::parse(&json)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("new") => {
            let dir = args
                .get(1)
                .filter(|a| !a.starts_with("--"))
                .ok_or("new: a project directory is required")?;
            let written = scaffold::scaffold(dir)?;
            for f in &written {
                println!("wrote {f}");
            }
            println!("\nnext steps:\n  cd {dir}\n  npm install\n  npm run migrate && npm run seed && npm run demo");
            Ok(())
        }
        Some("init") => {
            let path = "powder.schema.json";
            if std::path::Path::new(path).exists() {
                return Err(format!("`{path}` already exists"));
            }
            std::fs::write(path, SAMPLE_SCHEMA).map_err(|e| e.to_string())?;
            println!("wrote {path}");
            Ok(())
        }
        Some("generate") => {
            let schema = load_schema(&args)?;
            let mut wrote = false;
            if let Some(ts_out) = flag(&args, "--ts") {
                let import = flag(&args, "--ts-import").unwrap_or_else(|| "@powder/node".into());
                std::fs::write(&ts_out, codegen::typescript(&schema, &import))
                    .map_err(|e| e.to_string())?;
                println!("wrote {ts_out}");
                wrote = true;
            }
            if let Some(py_out) = flag(&args, "--py") {
                std::fs::write(&py_out, codegen::python(&schema)).map_err(|e| e.to_string())?;
                println!("wrote {py_out}");
                wrote = true;
            }
            if !wrote {
                return Err("generate: pass --ts <out.ts> and/or --py <out.py>".into());
            }
            Ok(())
        }
        Some("ddl") => {
            let schema = load_schema(&args)?;
            let name = flag(&args, "--dialect").unwrap_or_else(|| "sqlite".into());
            let d = dialect::by_name(&name)?;
            for table in schema.tables_in_dependency_order() {
                println!("{};", d.create_table(table));
            }
            Ok(())
        }
        Some("migrate") => {
            let url = flag(&args, "--db").ok_or("migrate: --db <url> is required")?;
            let schema = load_schema(&args)?;
            let conn = db::open(&url)?;
            let rebuild = args.iter().any(|a| a == "--rebuild");
            let applied = if rebuild {
                db::migrate_rebuild(&conn, &schema)?
            } else {
                db::migrate(&conn, &schema)?
            };
            if applied.is_empty() {
                println!("database already up to date");
            } else {
                for ddl in &applied {
                    println!("applied: {ddl}");
                }
            }
            Ok(())
        }
        Some("validate") => {
            let url = flag(&args, "--db").ok_or("validate: --db <url> is required")?;
            let schema = load_schema(&args)?;
            let conn = db::open(&url)?;
            let problems = db::validate(&conn, &schema)?;
            if problems.is_empty() {
                println!("schema and database are in sync");
                Ok(())
            } else {
                for p in &problems {
                    eprintln!("mismatch: {p}");
                }
                Err(format!("{} schema mismatch(es) found", problems.len()))
            }
        }
        Some("seed") => {
            let url = flag(&args, "--db").ok_or("seed: --db <url> is required")?;
            let file = flag(&args, "--file").ok_or("seed: --file <seed.json|seed.sql> is required")?;
            let contents =
                std::fs::read_to_string(&file).map_err(|e| format!("cannot read `{file}`: {e}"))?;
            let conn = db::open(&url)?;
            let n = db::seed(&conn, &file, &contents)?;
            println!("seeded {n} row(s) from {file}");
            Ok(())
        }
        Some("--help" | "-h" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
