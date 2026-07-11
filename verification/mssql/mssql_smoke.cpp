// SQL Server backend smoke test — T-SQL dialect, integrated auth.
// Covers: DDL, params (int/float/bool/text/null), PCB decode, aggregates,
// JOIN + GROUP BY + HAVING, transactions with savepoints, ORM CRUD,
// Korean/emoji round-trip.
#include <cctype>
#include <cstdio>
#include <string>
#include "powder.hpp"

static int checks = 0, failures = 0;
#define CHECK(cond, what)                                   \
    do {                                                    \
        checks++;                                           \
        if (!(cond)) { failures++; std::fprintf(stderr, "FAILED: %s\n", what); } \
        else { std::printf("ok: %s\n", what); }             \
    } while (0)

static bool contains(const std::string& h, const std::string& n) {
    return h.find(n) != std::string::npos;
}

static const char* SCHEMA = R"({
  "tables": {
    "customers": {
      "columns": {
        "id":     { "type": "int",  "primaryKey": true },
        "name":   { "type": "text" },
        "tier":   { "type": "text" },
        "active": { "type": "bool" }
      }
    },
    "orders": {
      "columns": {
        "id":          { "type": "int", "primaryKey": true },
        "customer_id": { "type": "int", "references": { "table": "customers", "column": "id" } },
        "status":      { "type": "text" },
        "amount":      { "type": "float" },
        "note":        { "type": "text", "nullable": true }
      }
    }
  }
})";

int main(int argc, char** argv) try {
    setvbuf(stdout, nullptr, _IONBF, 0);
    const std::string url = argc > 1 ? argv[1] : "mssql://127.0.0.1:1433/powder_test";
    std::printf("backend: %s\n", url.c_str());
    powder::Client db(url);

    // T-SQL DDL (2008-compatible): no DROP TABLE IF EXISTS. Children first -
    // other suites (CLI migrate) may have left FK-linked tables behind.
    db.execute("IF OBJECT_ID('order_items','U') IS NOT NULL DROP TABLE order_items");
    db.execute("IF OBJECT_ID('products','U') IS NOT NULL DROP TABLE products");
    db.execute("IF OBJECT_ID('orders','U') IS NOT NULL DROP TABLE orders");
    db.execute("IF OBJECT_ID('customers','U') IS NOT NULL DROP TABLE customers");
    db.execute("CREATE TABLE customers (id BIGINT PRIMARY KEY, name NVARCHAR(200), tier NVARCHAR(40), active BIT)");
    db.execute("CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT REFERENCES customers(id), "
               "status NVARCHAR(40), amount FLOAT, note NVARCHAR(400))");

    powder::Orm orm(db, SCHEMA);
    auto customers = orm.table("customers");
    auto orders = orm.table("orders");

    // ORM seed (create / create_many).
    customers.create(R"({"id":1,"name":"김민준","tier":"vip","active":true})");
    customers.create_many(R"([
        {"id":2,"name":"이서연","tier":"gold","active":true},
        {"id":3,"name":"박도윤","tier":"basic","active":false},
        {"id":4,"name":"한글🚀고객","tier":"basic","active":true}
    ])");
    CHECK(customers.count() == 4, "orm: seed 4 customers (create + create_many)");

    orders.create_many(R"([
        {"id":100,"customer_id":1,"status":"paid","amount":1850000.0,"note":"빠른배송 요청"},
        {"id":101,"customer_id":1,"status":"paid","amount":89000,"note":null},
        {"id":102,"customer_id":2,"status":"shipped","amount":350000.0,"note":null},
        {"id":103,"customer_id":3,"status":"cancelled","amount":45000.0,"note":"고객 변심"}
    ])");
    CHECK(orders.count() == 4, "orm: seed 4 orders (int-literal float coerced)");

    // Raw params of every type.
    int64_t n = db.execute("UPDATE orders SET note = ? WHERE id = ?", {"수동 메모", int64_t{102}});
    CHECK(n == 1, "raw: update with text+int params");

    // Dashboard: GROUP BY + SUM (typed decode over TDS).
    {
        powder::Batch b = db.query(
            "SELECT status, COUNT(*) AS cnt, SUM(amount) AS revenue "
            "FROM orders GROUP BY status ORDER BY revenue DESC");
        CHECK(b.num_rows() == 3, "query: 3 status groups");
        CHECK(b["status"].str(0) == "paid" && b["cnt"].i64(0) == 2 &&
              b["revenue"].f64(0) == 1939000.0,
              "query: paid revenue 1,939,000 (COUNT->i64, SUM->f64)");
    }

    // JOIN + HAVING with a float param.
    {
        powder::Batch b = db.query(
            "SELECT c.name, SUM(o.amount) AS total FROM customers c "
            "JOIN orders o ON o.customer_id = c.id "
            "WHERE o.status <> 'cancelled' "
            "GROUP BY c.id, c.name HAVING SUM(o.amount) >= ? ORDER BY total DESC",
            {100000.0});
        CHECK(b.num_rows() == 2 && b["name"].str(0) == "김민준",
              "query: JOIN+HAVING with param, top customer 김민준");
    }

    // TOP (T-SQL's LIMIT).
    {
        powder::Batch b = db.query("SELECT TOP 2 id, amount FROM orders ORDER BY amount DESC");
        CHECK(b.num_rows() == 2 && b["id"].i64(0) == 100, "query: TOP 2 biggest orders");
    }

    // NULL round-trip + BIT decode.
    {
        powder::Batch b = db.query("SELECT id, note, CAST(1 AS BIT) AS flag FROM orders WHERE id IN (?, ?) ORDER BY id",
                                   {int64_t{100}, int64_t{101}});
        CHECK(b["note"].is_valid(0) && b["note"].str(0) == "빠른배송 요청", "null: note present on 100");
        CHECK(!b["note"].is_valid(1), "null: note NULL on 101");
        CHECK(b["flag"].boolean(0), "bit: decodes as bool");
    }

    // ORM finder (no limit — LIMIT is not T-SQL) + update + exists + delete.
    {
        std::string rows = customers.find_many(
            R"({"where":{"active":true,"OR":[{"tier":"vip"},{"tier":"gold"}]},"orderBy":{"id":"asc"}})");
        CHECK(contains(rows, "김민준") && contains(rows, "이서연") && !contains(rows, "박도윤"),
              "orm: nested where OR");
        CHECK(orders.update(R"({"status":"shipped"})", R"({"status":"paid"})") == 1,
              "orm: update shipped -> paid");
        CHECK(customers.exists(R"({"active":false})"), "orm: exists inactive");
        std::string s = orders.aggregate("sum", "amount", R"({"status":"paid"})");
        CHECK(contains(s, "2289000"), "orm: aggregate sum(paid)");
        std::string g = orders.group_by(
            R"({"by":["customer_id"],"count":true,"sum":["amount"],"having":{"_sum_amount":{"gt":100000}},"orderBy":{"_sum_amount":"desc"}})");
        CHECK(contains(g, "_sum_amount") && contains(g, "\"customer_id\":1"), "orm: groupBy+having aliases");
    }

    // FK enforcement.
    {
        bool fk = false;
        try { customers.remove(R"({"id":1})"); }
        catch (const powder::Error& e) {
            std::string m = e.what();
            for (auto& ch : m) ch = (char)std::tolower((unsigned char)ch);
            fk = contains(m, "reference") || contains(m, "foreign");
        }
        CHECK(fk, "fk: deleting referenced customer rejected");
    }

    // Transactions: outer commit, inner savepoint rollback.
    {
        db.execute("BEGIN IMMEDIATE");                 // -> BEGIN TRANSACTION
        db.execute("INSERT INTO orders VALUES (?, ?, ?, ?, ?)",
                   {int64_t{200}, int64_t{2}, "paid", 10000.0, nullptr});
        db.execute("SAVEPOINT sp_1");                  // -> SAVE TRANSACTION sp_1
        db.execute("INSERT INTO orders VALUES (?, ?, ?, ?, ?)",
                   {int64_t{201}, int64_t{2}, "paid", 20000.0, nullptr});
        db.execute("ROLLBACK TO sp_1");                // -> ROLLBACK TRANSACTION sp_1
        db.execute("RELEASE sp_1");                    // -> no-op
        db.execute("COMMIT");
        powder::Batch b = db.query("SELECT COUNT(*) AS c FROM orders WHERE id IN (200, 201)");
        CHECK(b["c"].i64(0) == 1, "tx: outer committed, savepoint rolled back");
    }

    // Full rollback.
    {
        db.execute("BEGIN IMMEDIATE");
        db.execute("INSERT INTO orders VALUES (999, 1, 'paid', 1.0, NULL)");
        db.execute("ROLLBACK");
        powder::Batch b = db.query("SELECT COUNT(*) AS c FROM orders WHERE id = 999");
        CHECK(b["c"].i64(0) == 0, "tx: full rollback");
    }

    // UTF-8 round-trip.
    {
        powder::Batch b = db.query("SELECT name FROM customers WHERE id = 4");
        CHECK(b["name"].str(0) == "한글🚀고객", "utf8: Korean + emoji round-trip");
    }

    std::printf("\n%s — %d checks, %d failed\n",
                failures == 0 ? "MSSQL BACKEND OK" : "MSSQL BACKEND FAILED",
                checks, failures);
    return failures == 0 ? 0 : 1;
} catch (const std::exception& e) {
    std::fprintf(stderr, "UNCAUGHT: %s\n", e.what());
    return 2;
}
