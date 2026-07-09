// End-to-end test of the C++ wrapper: RAII client/batch, typed column reads,
// NULLs, non-ASCII text, transactions with savepoints, and error paths.
//
//   cl /std:c++17 /EHsc /W3 test_powder.cpp /link powder_ffi.dll.lib

#include <cstdio>
#include <string>

#include "../include/powder.hpp"

static int checks = 0;

#define CHECK(cond, what)                                    \
    do {                                                     \
        checks++;                                            \
        if (!(cond)) {                                       \
            std::fprintf(stderr, "FAILED: %s\n", what);      \
            return 1;                                        \
        }                                                    \
    } while (0)

int main() {
    try {
        powder::Client db("sqlite::memory:");
        db.execute("CREATE TABLE users (id INTEGER, name TEXT, score REAL, active INTEGER)");
        const int64_t n = db.execute(
            "INSERT INTO users VALUES (?,?,?,?),(?,?,?,?),(?,?,?,?)",
            {int64_t{1}, "alice", 9.5, int64_t{1},
             int64_t{2}, "bob", nullptr, int64_t{0},
             int64_t{3}, std::string("h\xc3\xa9llo \xf0\x9f\x8c\x8d"), -1.25, int64_t{1}});
        CHECK(n == 3, "insert affected 3 rows");

        powder::Batch b = db.query("SELECT id, name, score FROM users ORDER BY id");
        CHECK(b.num_rows() == 3, "3 rows");
        CHECK(b.columns().size() == 3, "3 columns");
        CHECK(b["id"].type() == powder::DataType::Int64, "id is int64");
        CHECK(b["id"].i64(0) == 1 && b["id"].i64(2) == 3, "ids 1..3");
        CHECK(b["name"].str(0) == "alice", "name[0]");
        CHECK(b["name"].str(2) == "h\xc3\xa9llo \xf0\x9f\x8c\x8d", "unicode round-trips");
        CHECK(b["score"].is_valid(0) && !b["score"].is_valid(1), "NULL tracked in validity");
        CHECK(b["score"].f64(0) == 9.5 && b["score"].f64(2) == -1.25, "float reads");

        // Bound-parameter filter.
        powder::Batch f = db.query("SELECT name FROM users WHERE score >= ?", {0.0});
        CHECK(f.num_rows() == 1 && f["name"].str(0) == "alice", "filtered query");

        // Transaction rollback, then nested savepoint semantics.
        try {
            db.transaction([](powder::Client& tx) {
                tx.execute("INSERT INTO users VALUES (4, 'temp', 0, 1)");
                throw std::runtime_error("boom");
            });
        } catch (const std::runtime_error&) {
        }
        CHECK(db.query("SELECT COUNT(*) AS n FROM users")["n"].i64(0) == 3,
              "rollback undid the insert");

        db.transaction([](powder::Client& tx) {
            tx.execute("INSERT INTO users VALUES (5, 'frank', 1.0, 1)");
            try {
                tx.transaction([](powder::Client& inner) {
                    inner.execute("INSERT INTO users VALUES (6, 'ghost', 1.0, 1)");
                    throw std::runtime_error("inner boom");
                });
            } catch (const std::runtime_error&) {
            }
        });
        CHECK(db.query("SELECT COUNT(*) AS n FROM users")["n"].i64(0) == 4,
              "savepoint kept frank, dropped ghost");

        // Error paths.
        bool threw = false;
        try {
            db.query("SELECT * FROM missing");
        } catch (const powder::Error& e) {
            threw = std::string(e.what()).find("missing") != std::string::npos;
        }
        CHECK(threw, "bad SQL throws with the engine message");

        threw = false;
        try {
            b["no_such_column"];
        } catch (const powder::Error&) {
            threw = true;
        }
        CHECK(threw, "unknown column throws");

        // Move semantics: the moved-from client must not double-close.
        powder::Client moved = std::move(db);
        CHECK(moved.query("SELECT 1 AS one")["one"].i64(0) == 1, "moved client still works");
        threw = false;
        try {
            db.execute("SELECT 1"); // NOLINT(bugprone-use-after-move) — deliberate
        } catch (const powder::Error&) {
            threw = true;
        }
        CHECK(threw, "moved-from client is closed");
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAILED: unexpected exception: %s\n", e.what());
        return 1;
    }

    std::printf("cpp binding OK (%d checks)\n", checks);
    return 0;
}
