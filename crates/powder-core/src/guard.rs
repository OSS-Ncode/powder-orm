//! SQL injection guards shared by every backend.
//!
//! Powder's first line of defense is architectural: values only cross as
//! bound parameters, and every identifier the ORM interpolates is validated
//! against the parsed schema (`[A-Za-z_][A-Za-z0-9_]*`). What that cannot
//! stop is an *application* concatenating untrusted text into the SQL string
//! itself. The classic escalation of such a bug is statement stacking —
//! `'; DROP TABLE users; --` — so [`Client`](crate::Client) rejects a second
//! statement in every call that is not an explicit no-parameter batch.
//!
//! The scanner walks raw bytes and skips everything a `;` can legally hide
//! in: single/double-quoted literals (with `''` doubling), backtick and
//! `[...]` quoted identifiers (MySQL / T-SQL), `-- ...` line comments, and
//! `/* ... */` block comments. A trailing `;` (followed by only whitespace
//! or comments) is fine.

/// Whether `sql` contains a second statement after the first.
pub fn has_stacked_statements(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0usize;
    let mut seen_terminator = false;

    while i < bytes.len() {
        let c = bytes[i];

        // Inside comments, nothing counts — including after a terminator.
        if c == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i < bytes.len() {
                if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Any other non-whitespace token after a `;` is a second statement.
        if seen_terminator {
            return true;
        }

        match c {
            b';' => {
                seen_terminator = true;
                i += 1;
            }
            quote @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        // Doubled quote is an escaped quote, not a close.
                        if i + 1 < bytes.len() && bytes[i + 1] == quote {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'[' => {
                // T-SQL bracketed identifier; `]]` is an escaped bracket.
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b']' {
                        if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// Guard for parameterized calls: exactly one statement, or an error that
/// names the escape hatch.
pub fn reject_stacked(sql: &str) -> crate::error::Result<()> {
    if has_stacked_statements(sql) {
        return Err(crate::error::Error::Database(
            "multiple SQL statements in one call are rejected (SQL-injection guard); \
             run them one per call, or use execute() without parameters for trusted DDL/seed batches"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_statements_pass() {
        assert!(!has_stacked_statements("SELECT * FROM t WHERE id = ?"));
        assert!(!has_stacked_statements("SELECT 1;")); // trailing ; is fine
        assert!(!has_stacked_statements("SELECT 1 ; \n  -- done\n"));
        assert!(!has_stacked_statements("SELECT 1; /* trailing comment */"));
        assert!(!has_stacked_statements(""));
    }

    #[test]
    fn literals_and_comments_hide_semicolons() {
        assert!(!has_stacked_statements("SELECT ';' FROM t"));
        assert!(!has_stacked_statements("SELECT 'it''s; fine' FROM t"));
        assert!(!has_stacked_statements("SELECT \"a;b\" FROM t"));
        assert!(!has_stacked_statements("SELECT `a;b` FROM t"));
        assert!(!has_stacked_statements("SELECT [a;b] FROM t"));
        assert!(!has_stacked_statements("SELECT [a]];b] FROM t"));
        assert!(!has_stacked_statements("SELECT 1 -- ; DROP TABLE t\n FROM x"));
        assert!(!has_stacked_statements("SELECT /* ; nope */ 1"));
    }

    #[test]
    fn stacked_statements_are_caught() {
        assert!(has_stacked_statements("SELECT 1; DROP TABLE users"));
        assert!(has_stacked_statements("SELECT 1;DROP TABLE users;--"));
        assert!(has_stacked_statements(
            "SELECT * FROM t WHERE name = 'x'; DELETE FROM t; --'"
        ));
        assert!(has_stacked_statements("SELECT 1; /* c */ SELECT 2"));
        assert!(has_stacked_statements("SELECT 1;\n\nSELECT 2"));
        // The classic: injected text closes the literal, then stacks.
        let user_input = "x'; DROP TABLE users; --";
        let sql = format!("SELECT * FROM t WHERE name = '{user_input}'");
        assert!(has_stacked_statements(&sql));
    }

    #[test]
    fn reject_names_the_guard() {
        let err = reject_stacked("SELECT 1; SELECT 2").unwrap_err();
        assert!(err.to_string().contains("SQL-injection guard"));
        assert!(reject_stacked("SELECT 1").is_ok());
    }
}
