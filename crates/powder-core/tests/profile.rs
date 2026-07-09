//! Component-level profiling for the bench-site query shape:
//! 200k rows of (id INTEGER, name TEXT, score REAL), `SELECT ... ORDER BY id`.
//!
//! Ignored by default; run with:
//! `cargo test -p powder-core --release --test profile -- --ignored --nocapture`

use std::time::Instant;

use powder_core::Client;
use rusqlite::Connection;

// Match the allocator the language bindings ship with.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const ROWS: usize = 200_000;
const CHUNK: usize = 500;

fn insert_chunks(mut exec: impl FnMut(&str)) {
    let mut i = 0usize;
    while i < ROWS {
        let end = (i + CHUNK).min(ROWS);
        let mut sql = String::from("INSERT INTO bench_users (id, name, score) VALUES ");
        for r in i..end {
            if r > i {
                sql.push(',');
            }
            let name = format!("user_{}_{}", r, (r * 2654435761) % 1000);
            let score = (((r * 37) % 10000) as f64 / 7.0).round() / 10.0;
            sql.push_str(&format!("({}, '{}', {})", r + 1, name, score));
        }
        exec(&sql);
        i = end;
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn time_n<F: FnMut()>(n: usize, mut f: F) -> f64 {
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        f();
        samples.push(t.elapsed().as_secs_f64() * 1e3);
    }
    median(samples)
}

#[test]
#[ignore = "profiling harness, not a correctness test"]
fn profile_query_pipeline() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE bench_users (id INTEGER, name TEXT, score REAL)")
        .unwrap();
    insert_chunks(|sql| conn.execute_batch(sql).unwrap());

    const SQL: &str = "SELECT id, name, score FROM bench_users ORDER BY id ASC";
    const SQL_NOSORT: &str = "SELECT id, name, score FROM bench_users";

    // 1. step-only floor: iterate all rows, never touch columns.
    let step_only = time_n(7, || {
        let mut stmt = conn.prepare(SQL).unwrap();
        let mut rows = stmt.query([]).unwrap();
        let mut n = 0usize;
        while let Some(_row) = rows.next().unwrap() {
            n += 1;
        }
        assert_eq!(n, ROWS);
    });

    // 2. step + get_ref every column (what run_query's input side costs).
    let step_read = time_n(7, || {
        let mut stmt = conn.prepare(SQL_NOSORT).unwrap();
        let mut rows = stmt.query([]).unwrap();
        let mut n = 0usize;
        while let Some(row) = rows.next().unwrap() {
            for i in 0..3 {
                std::hint::black_box(row.get_ref(i).unwrap());
            }
            n += 1;
        }
        assert_eq!(n, ROWS);
    });

    // 3. raw FFI stepping floor: sqlite3_step + sqlite3_column_* direct.
    let raw_ffi = time_n(7, || unsafe {
        use rusqlite::ffi;
        let db = conn.handle();
        let sql = std::ffi::CString::new(SQL_NOSORT).unwrap();
        let mut stmt = std::ptr::null_mut();
        assert_eq!(
            ffi::sqlite3_prepare_v2(db, sql.as_ptr(), -1, &mut stmt, std::ptr::null_mut()),
            ffi::SQLITE_OK
        );
        let mut n = 0usize;
        loop {
            match ffi::sqlite3_step(stmt) {
                ffi::SQLITE_ROW => {
                    for c in 0..3 {
                        match ffi::sqlite3_column_type(stmt, c) {
                            ffi::SQLITE_INTEGER => {
                                std::hint::black_box(ffi::sqlite3_column_int64(stmt, c));
                            }
                            ffi::SQLITE_FLOAT => {
                                std::hint::black_box(ffi::sqlite3_column_double(stmt, c));
                            }
                            ffi::SQLITE_TEXT => {
                                let p = ffi::sqlite3_column_text(stmt, c);
                                let len = ffi::sqlite3_column_bytes(stmt, c) as usize;
                                std::hint::black_box(std::slice::from_raw_parts(p, len));
                            }
                            _ => {}
                        }
                    }
                    n += 1;
                }
                ffi::SQLITE_DONE => break,
                other => panic!("step failed: {other}"),
            }
        }
        ffi::sqlite3_finalize(stmt);
        assert_eq!(n, ROWS);
    });

    // 4/5. full pipeline through the public client: query (build) then encode.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = Client::connect(":memory:").await.unwrap();
        client
            .execute(
                "CREATE TABLE bench_users (id INTEGER, name TEXT, score REAL)",
                vec![],
            )
            .await
            .unwrap();
        let mut stmts = Vec::new();
        insert_chunks(|sql| stmts.push(sql.to_string()));
        for sql in stmts {
            client.execute(&sql, vec![]).await.unwrap();
        }

        let mut q_samples = Vec::new();
        let mut e_samples = Vec::new();
        let mut qb_samples = Vec::new();
        for i in 0..7 {
            // Pad the SQL differently per iteration: same plan, different
            // cache key — these numbers must measure the COLD path.
            let sql = format!("{SQL}{}", " ".repeat(i + 1));
            let t = Instant::now();
            let batch = client.query(&sql, vec![]).await.unwrap();
            q_samples.push(t.elapsed().as_secs_f64() * 1e3);

            let t = Instant::now();
            let bytes = batch.encode();
            e_samples.push(t.elapsed().as_secs_f64() * 1e3);
            std::hint::black_box(bytes.len());

            let sql = format!("{SQL}{}", " ".repeat(i + 100));
            let t = Instant::now();
            let bytes = client.query_bytes_shared(&sql, vec![]).await.unwrap();
            qb_samples.push(t.elapsed().as_secs_f64() * 1e3);
            std::hint::black_box(bytes.len());
        }
        println!("step_only (sort+step, no reads) : {step_only:8.2} ms");
        println!("step+get_ref x3, NO sort        : {step_read:8.2} ms");
        println!("raw FFI step+columns, NO sort   : {raw_ffi:8.2} ms");
        println!("client.query (build batch)      : {:8.2} ms", median(q_samples));
        println!("batch.encode                    : {:8.2} ms", median(e_samples));
        println!("client.query_bytes (e2e native) : {:8.2} ms", median(qb_samples));
    });
}
