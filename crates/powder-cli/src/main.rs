//! `powder` — the Powder ORM command-line tool.
//!
//! All logic lives in [`powder_cli::cli::run`] (unit-tested); this shim only
//! collects argv, prints the result, and maps errors to the exit code.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    match powder_cli::cli::run(&args, &cwd) {
        Ok(out) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
