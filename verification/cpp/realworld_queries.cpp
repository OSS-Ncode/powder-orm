// Real-world business-query verification against the documented Powder C++
// surface: an e-commerce dataset (customers / products / orders / order_items)
// exercised through raw SQL joins, ORM finders, relations, aggregations,
// named-style raw queries, and nested transactions.

#include <cctype>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

#include "powder.hpp"

static int checks = 0, failures = 0;

#define CHECK(cond, what)                                        \
    do {                                                         \
        checks++;                                                \
        if (!(cond)) {                                           \
            failures++;                                          \
            std::fprintf(stderr, "FAILED: %s\n", what);          \
        } else {                                                 \
            std::printf("ok: %s\n", what);                       \
        }                                                        \
    } while (0)

static bool contains(const std::string& hay, const std::string& needle) {
    return hay.find(needle) != std::string::npos;
}

static const char* SCHEMA = R"({
  "tables": {
    "customers": {
      "columns": {
        "id":     { "type": "int",  "primaryKey": true },
        "name":   { "type": "text" },
        "email":  { "type": "text" },
        "tier":   { "type": "text" },
        "active": { "type": "bool" }
      }
    },
    "products": {
      "columns": {
        "id":    { "type": "int",   "primaryKey": true },
        "name":  { "type": "text" },
        "price": { "type": "float" },
        "stock": { "type": "int" }
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
    },
    "order_items": {
      "columns": {
        "id":         { "type": "int", "primaryKey": true },
        "order_id":   { "type": "int", "references": { "table": "orders", "column": "id" } },
        "product_id": { "type": "int", "references": { "table": "products", "column": "id" } },
        "qty":        { "type": "int" },
        "unit_price": { "type": "float" }
      }
    }
  }
})";

// Dialect-aware transaction helper: BEGIN IMMEDIATE is SQLite-only.
static bool g_sqlite = true;
template <typename Fn>
static void run_tx(powder::Client& db, int depth, Fn&& fn) {
    const std::string sp = depth > 0 ? "vsp_" + std::to_string(depth) : "";
    db.execute(depth > 0 ? "SAVEPOINT " + sp
                         : (g_sqlite ? "BEGIN IMMEDIATE" : "BEGIN"));
    try {
        fn(db);
        db.execute(depth > 0 ? "RELEASE " + sp : "COMMIT");
    } catch (...) {
        if (depth > 0) {
            db.execute("ROLLBACK TO " + sp);
            db.execute("RELEASE " + sp);
        } else {
            db.execute("ROLLBACK");
        }
        throw;
    }
}

int main(int argc, char** argv) {
    try {
        const char* env_url = std::getenv("POWDER_URL");
        const std::string url = argc > 1 ? argv[1] : (env_url ? env_url : "sqlite::memory:");
        g_sqlite = url.rfind("sqlite", 0) == 0;
        std::printf("backend: %s\n", url.c_str());
        powder::Client db(url);

        // ---- DDL + seed (portable SQL; FK는 테이블 레벨 — MySQL은 인라인
        //      REFERENCES를 조용히 무시한다) ------------------------------
        db.execute("DROP TABLE IF EXISTS order_items");
        db.execute("DROP TABLE IF EXISTS orders");
        db.execute("DROP TABLE IF EXISTS products");
        db.execute("DROP TABLE IF EXISTS customers");
        db.execute("CREATE TABLE customers (id BIGINT PRIMARY KEY, name TEXT, email TEXT, tier TEXT, active BOOLEAN)");
        db.execute("CREATE TABLE products (id BIGINT PRIMARY KEY, name TEXT, price DOUBLE PRECISION, stock BIGINT)");
        db.execute("CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT, status TEXT, amount DOUBLE PRECISION, note TEXT, "
                   "FOREIGN KEY (customer_id) REFERENCES customers(id))");
        db.execute("CREATE TABLE order_items (id BIGINT PRIMARY KEY, order_id BIGINT, product_id BIGINT, qty BIGINT, unit_price DOUBLE PRECISION, "
                   "FOREIGN KEY (order_id) REFERENCES orders(id), FOREIGN KEY (product_id) REFERENCES products(id))");

        powder::Orm orm(db, SCHEMA);
        auto customers = orm.table("customers");
        auto products = orm.table("products");
        auto orders = orm.table("orders");
        auto items = orm.table("order_items");

        // ---- Seed via ORM create / create_many ---------------------------
        customers.create(R"({"id":1,"name":"김민준","email":"minjun@corp.kr","tier":"vip","active":true})");
        customers.create_many(R"([
            {"id":2,"name":"이서연","email":"seoyeon@corp.kr","tier":"gold","active":true},
            {"id":3,"name":"박도윤","email":"doyun@corp.kr","tier":"basic","active":true},
            {"id":4,"name":"최지우","email":"jiwoo@old.kr","tier":"basic","active":false},
            {"id":5,"name":"정하은","email":"haeun@corp.kr","tier":"vip","active":true}
        ])");
        CHECK(customers.count() == 5, "seed: 5 customers (create + create_many)");

        // float 컬럼 값은 소수점 필수 — 정수 표기는 i64로 직렬화되어
        // PostgreSQL에서 "error serializing parameter"가 난다.
        products.create_many(R"([
            {"id":10,"name":"노트북","price":1500000.0,"stock":12},
            {"id":11,"name":"모니터","price":350000.0,"stock":40},
            {"id":12,"name":"키보드","price":89000.0,"stock":0},
            {"id":13,"name":"마우스","price":45000.0,"stock":200}
        ])");

        orders.create_many(R"([
            {"id":100,"customer_id":1,"status":"paid","amount":1850000.0,"note":"빠른배송 요청"},
            {"id":101,"customer_id":1,"status":"paid","amount":89000.0,"note":null},
            {"id":102,"customer_id":2,"status":"shipped","amount":350000.0,"note":null},
            {"id":103,"customer_id":3,"status":"pending","amount":45000.0,"note":null},
            {"id":104,"customer_id":5,"status":"paid","amount":700000.0,"note":"법인 세금계산서"},
            {"id":105,"customer_id":2,"status":"cancelled","amount":89000.0,"note":"고객 변심"},
            {"id":106,"customer_id":5,"status":"paid","amount":1500000.0,"note":null}
        ])");

        items.create_many(R"([
            {"id":1000,"order_id":100,"product_id":10,"qty":1,"unit_price":1500000.0},
            {"id":1001,"order_id":100,"product_id":11,"qty":1,"unit_price":350000.0},
            {"id":1002,"order_id":101,"product_id":12,"qty":1,"unit_price":89000.0},
            {"id":1003,"order_id":102,"product_id":11,"qty":1,"unit_price":350000.0},
            {"id":1004,"order_id":103,"product_id":13,"qty":1,"unit_price":45000.0},
            {"id":1005,"order_id":104,"product_id":11,"qty":2,"unit_price":350000.0},
            {"id":1006,"order_id":106,"product_id":10,"qty":1,"unit_price":1500000.0}
        ])");

        // =================================================================
        // 1. 대시보드: 상태별 매출 요약 (raw SQL GROUP BY)
        // =================================================================
        {
            powder::Batch b = db.query(
                "SELECT status, COUNT(*) AS cnt, SUM(amount) AS revenue "
                "FROM orders GROUP BY status ORDER BY revenue DESC");
            CHECK(b.num_rows() == 4, "dashboard: 4 status groups");
            CHECK(b["status"].str(0) == "paid", "dashboard: top revenue status is 'paid'");
            CHECK(b["cnt"].i64(0) == 4, "dashboard: 4 paid orders");
            CHECK(b["revenue"].f64(0) == 4139000.0, "dashboard: paid revenue = 4,139,000");
        }

        // =================================================================
        // 2. 고객별 매출 리포트 (raw SQL JOIN + GROUP BY + HAVING)
        // =================================================================
        {
            powder::Batch b = db.query(
                "SELECT c.name, c.tier, COUNT(o.id) AS orders_cnt, SUM(o.amount) AS total "
                "FROM customers c JOIN orders o ON o.customer_id = c.id "
                "WHERE o.status != 'cancelled' "
                "GROUP BY c.id HAVING SUM(o.amount) >= ? "
                "ORDER BY total DESC", {100000.0});
            CHECK(b.num_rows() == 3, "report: 3 customers over 100k (non-cancelled)");
            CHECK(b["name"].str(0) == "정하은" && b["total"].f64(0) == 2200000.0,
                  "report: top customer 정하은 = 2,200,000");
            CHECK(b["name"].str(1) == "김민준" && b["orders_cnt"].i64(1) == 2,
                  "report: 김민준 has 2 orders");
        }

        // =================================================================
        // 3. 서브쿼리: 한 번도 주문 안 한 고객 (raw SQL NOT IN)
        // =================================================================
        {
            powder::Batch b = db.query(
                "SELECT name FROM customers WHERE id NOT IN (SELECT DISTINCT customer_id FROM orders)");
            CHECK(b.num_rows() == 1 && b["name"].str(0) == "최지우",
                  "subquery: only 최지우 never ordered");
        }

        // =================================================================
        // 4. 재고 없는 상품 중 주문된 것 (raw SQL JOIN + WHERE)
        // =================================================================
        {
            powder::Batch b = db.query(
                "SELECT DISTINCT p.name FROM products p "
                "JOIN order_items i ON i.product_id = p.id WHERE p.stock = 0");
            CHECK(b.num_rows() == 1 && b["name"].str(0) == "키보드",
                  "stockout: 키보드 ordered but out of stock");
        }

        // =================================================================
        // 5. ORM finder: 중첩 AND/OR/NOT + in/like/gte (문서의 Prisma 스타일)
        // =================================================================
        {
            // 활성 고객 중 (vip이거나 이메일이 corp.kr) — 문서 finder.mdx 형태
            std::string rows = customers.find_many(R"({
                "where": {
                    "active": true,
                    "OR": [
                        { "tier": "vip" },
                        { "email": { "like": "%@corp.kr" } }
                    ],
                    "NOT": { "name": { "like": "최%" } }
                },
                "orderBy": { "id": "asc" }
            })");
            CHECK(contains(rows, "김민준") && contains(rows, "정하은") &&
                  contains(rows, "이서연") && contains(rows, "박도윤") &&
                  !contains(rows, "최지우"),
                  "orm-finder: nested OR/NOT/like matches 4 active corp customers");

            // in 연산자
            std::string vips = customers.find_many(R"({"where":{"tier":{"in":["vip","gold"]}},"orderBy":{"id":"asc"}})");
            CHECK(contains(vips, "김민준") && contains(vips, "이서연") &&
                  contains(vips, "정하은") && !contains(vips, "박도윤"),
                  "orm-finder: tier in [vip, gold] -> 3 customers");
        }

        // =================================================================
        // 6. ORM 페이지네이션: limit + offset + count (finder.mdx paginate 의미)
        // =================================================================
        {
            std::string page1 = orders.find_many(R"({"orderBy":{"amount":"desc"},"limit":3,"offset":0})");
            std::string page2 = orders.find_many(R"({"orderBy":{"amount":"desc"},"limit":3,"offset":3})");
            int64_t total = orders.count();
            CHECK(contains(page1, "\"id\":100") && contains(page1, "\"id\":106"),
                  "orm-paginate: page1 has two biggest orders");
            CHECK(!contains(page2, "\"id\":100") && contains(page2, "\"id\":102"),
                  "orm-paginate: page2 disjoint from page1");
            CHECK(total == 7, "orm-paginate: total = 7 (totalPages = ceil(7/3) = 3)");
        }

        // =================================================================
        // 7. ORM 관계: include (배치 로드) + join (belongsTo LEFT JOIN)
        // =================================================================
        {
            std::string with_customer = orders.find_many(
                R"({"where":{"status":"paid"},"include":{"customer":true},"orderBy":{"id":"asc"}})");
            CHECK(contains(with_customer, "\"customer\"") && contains(with_customer, "김민준") &&
                  contains(with_customer, "정하은"),
                  "orm-include: paid orders hydrated with customer objects");

            std::string joined = orders.find_many(
                R"({"where":{"amount":{"gte":1000000}},"join":{"customer":true},"orderBy":{"id":"asc"}})");
            CHECK(contains(joined, "김민준") && contains(joined, "정하은"),
                  "orm-join: belongsTo LEFT JOIN hydrates big orders");

            // 중첩 include: order_items -> order -> customer
            std::string deep = items.find_many(
                R"({"where":{"qty":{"gte":2}},"include":{"order":{"include":{"customer":true}}}})");
            CHECK(contains(deep, "\"order\"") && contains(deep, "정하은"),
                  "orm-include: nested include order->customer");
        }

        // =================================================================
        // 8. ORM groupBy + having + 별칭 (aggregations.mdx 문서 그대로)
        // =================================================================
        {
            std::string g = orders.group_by(R"({
                "by": ["customer_id"],
                "where": { "status": { "ne": "cancelled" } },
                "count": true,
                "sum": ["amount"],
                "having": { "_sum_amount": { "gt": 100000 } },
                "orderBy": { "_sum_amount": "desc" }
            })");
            CHECK(contains(g, "_count") && contains(g, "_sum_amount"),
                  "orm-groupby: alias columns _count/_sum_amount present");
            CHECK(contains(g, "\"customer_id\":5") && contains(g, "\"customer_id\":1") &&
                  contains(g, "\"customer_id\":2") && !contains(g, "\"customer_id\":3"),
                  "orm-groupby: having _sum_amount>100000 keeps 3 groups");
        }

        // =================================================================
        // 9. ORM aggregate: sum/avg/min/max (+ 빈 집합은 null)
        // =================================================================
        {
            std::string s = orders.aggregate("sum", "amount", R"({"status":"paid"})");
            CHECK(contains(s, "4139000"), "orm-aggregate: sum(paid amount)");
            std::string mx = products.aggregate("max", "price");
            CHECK(contains(mx, "1500000"), "orm-aggregate: max product price");
            std::string none = orders.aggregate("avg", "amount", R"({"status":"refunded"})");
            CHECK(none == "null", "orm-aggregate: empty set -> null");
        }

        // =================================================================
        // 10. 업무 흐름: 주문 생성 트랜잭션 (재고 차감 + 주문 + 항목)
        //     중첩 세이브포인트: 안쪽 실패가 바깥 작업을 지우지 않음
        // =================================================================
        {
            run_tx(db, 0, [&](powder::Client& tx) {
                tx.execute("INSERT INTO orders VALUES (?, ?, ?, ?, ?)",
                           {int64_t{107}, int64_t{3}, "paid", 90000.0, nullptr});
                tx.execute("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)",
                           {int64_t{1007}, int64_t{107}, int64_t{13}, int64_t{2}, 45000.0});
                tx.execute("UPDATE products SET stock = stock - ? WHERE id = ?",
                           {int64_t{2}, int64_t{13}});

                // 안쪽: 잘못된 항목 추가 시도 -> 롤백돼야 함
                try {
                    run_tx(db, 1, [&](powder::Client& inner) {
                        inner.execute("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)",
                                      {int64_t{1008}, int64_t{107}, int64_t{12}, int64_t{1}, 89000.0});
                        throw powder::Error("재고 없음 — 취소");
                    });
                } catch (const powder::Error&) {}
            });

            powder::Batch b = db.query("SELECT COUNT(*) AS c FROM order_items WHERE order_id = 107");
            CHECK(b["c"].i64(0) == 1, "tx: outer committed, inner savepoint rolled back");
            powder::Batch st = db.query("SELECT stock FROM products WHERE id = 13");
            CHECK(st["stock"].i64(0) == 198, "tx: stock decremented inside transaction");
        }

        // 전체 롤백: 예외 시 아무것도 남지 않음
        {
            try {
                run_tx(db, 0, [&](powder::Client& tx) {
                    tx.execute("INSERT INTO orders VALUES (999, 1, 'paid', 1.0, NULL)");
                    throw powder::Error("결제 실패");
                });
            } catch (const powder::Error&) {}
            powder::Batch b = db.query("SELECT COUNT(*) AS c FROM orders WHERE id = 999");
            CHECK(b["c"].i64(0) == 0, "tx: full rollback on payment failure");
        }

        // =================================================================
        // 11. ORM update / remove / exists — 운영 업무
        // =================================================================
        {
            // 미결제 오래된 주문 일괄 취소
            int64_t n = orders.update(R"({"status":"pending"})", R"({"status":"cancelled"})");
            CHECK(n == 1, "orm-update: 1 pending order cancelled");

            // 비활성 고객 존재 확인 후 정리 대상 카운트
            CHECK(customers.exists(R"({"active":false})"), "orm-exists: inactive customer exists");

            // FK 강제 확인: 자식(order_items)이 남아 있는 주문 삭제는 거부돼야 함
            bool fk_enforced = false;
            try {
                orders.remove(R"({"id":103})");
            } catch (const powder::Error& e) {
                // SQLite/MySQL say "FOREIGN KEY", Postgres says "foreign key".
                std::string msg = e.what();
                for (auto& ch : msg) ch = static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
                fk_enforced = contains(msg, "foreign key");
            }
            CHECK(fk_enforced, "orm-remove: FK violation rejected (child items exist)");

            // 실무 순서: 항목 먼저 정리 -> 주문 삭제
            int64_t child_removed = items.remove(R"({"order_id":103})");
            int64_t removed = orders.remove(R"({"status":"cancelled","amount":{"lt":50000}})");
            CHECK(child_removed == 1 && removed == 1,
                  "orm-remove: cascade order (items first, then order) succeeds");
        }

        // =================================================================
        // 12. NULL 처리: nullable note 컬럼 (is_valid)
        // =================================================================
        {
            powder::Batch b = db.query("SELECT id, note FROM orders WHERE id IN (100, 101) ORDER BY id");
            CHECK(b["note"].is_valid(0) && b["note"].str(0) == "빠른배송 요청",
                  "null: note present on order 100");
            CHECK(!b["note"].is_valid(1), "null: note NULL on order 101");
        }

        // =================================================================
        // 13. 명명 쿼리 스타일: 문서 topUsers와 동등한 파라미터화 쿼리
        // =================================================================
        {
            powder::Batch b = db.query(
                "SELECT c.id, c.name, SUM(o.amount) AS ltv FROM customers c "
                "JOIN orders o ON o.customer_id = c.id "
                "WHERE c.active = ? AND o.status = ? "
                "GROUP BY c.id HAVING SUM(o.amount) >= ? ORDER BY ltv DESC",
                {true, "paid", 500000.0});
            CHECK(b.num_rows() == 2, "named-style: 2 active customers with paid LTV >= 500k");
            CHECK(b["name"].str(0) == "정하은", "named-style: highest LTV first");
        }

        // =================================================================
        // 14. 한글/이모지 등 non-ASCII 왕복
        // =================================================================
        {
            db.execute("INSERT INTO customers VALUES (?, ?, ?, ?, ?)",
                       {int64_t{6}, "한글🚀고객", "emoji@corp.kr", "basic", true});
            powder::Batch b = db.query("SELECT name FROM customers WHERE id = 6");
            CHECK(b["name"].str(0) == "한글🚀고객", "utf8: Korean + emoji round-trip");
        }

        std::printf("\n%s — %d checks, %d failed\n",
                    failures == 0 ? "REAL-WORLD QUERIES OK" : "SOME QUERIES FAILED",
                    checks, failures);
        return failures == 0 ? 0 : 1;
    } catch (const std::exception& e) {
        std::fprintf(stderr, "UNCAUGHT: %s\n", e.what());
        return 2;
    }
}
