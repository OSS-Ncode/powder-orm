/* Real-world business-query verification against the documented Powder C ABI:
 * an e-commerce dataset (customers / products / orders / order_items)
 * exercised through raw SQL joins, ORM finders, relations, aggregations,
 * named-style raw queries, and nested transactions (SAVEPOINT via raw SQL).
 *
 * Connection URL: argv[1], else POWDER_URL, else "sqlite::memory:".
 * Build (MSVC): cl /W3 /utf-8 realworld_queries.c /I ..\..\bindings\c\include
 *               /link powder_ffi.dll.lib
 */
#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "powder.h"

static int checks = 0, failures = 0;

#define CHECK(cond, what)                            \
    do {                                             \
        checks++;                                    \
        if (!(cond)) {                               \
            failures++;                              \
            fprintf(stderr, "FAILED: %s\n", what);   \
        } else {                                     \
            printf("ok: %s\n", what);                \
        }                                            \
    } while (0)

static void die(const char *ctx) {
    const char *err = powder_last_error();
    fprintf(stderr, "UNCAUGHT (%s): %s\n", ctx, err ? err : "unknown powder error");
    exit(2);
}

/* ---- PCB decoding helpers (layout mirrors bindings/cpp/powder.hpp) ------ */

typedef struct {
    unsigned char *buf;
    size_t len;
    uint32_t num_cols;
    uint32_t num_rows;
    uint32_t dir_off;
} Batch;

static uint32_t rd_u32(const unsigned char *p) { uint32_t v; memcpy(&v, p, 4); return v; }
static uint16_t rd_u16(const unsigned char *p) { uint16_t v; memcpy(&v, p, 2); return v; }

static Batch q(PowderClient *db, const char *sql, const char *params_json) {
    Batch b;
    size_t len = 0;
    unsigned char *buf = powder_query(db, sql, params_json, &len);
    if (!buf) die(sql);
    if (len < 24 || memcmp(buf, "PCB1", 4) != 0 || rd_u16(buf + 4) != 1) {
        fprintf(stderr, "UNCAUGHT: not a PCB v1 buffer\n");
        exit(2);
    }
    b.buf = buf;
    b.len = len;
    b.num_cols = rd_u32(buf + 8);
    b.num_rows = rd_u32(buf + 12);
    b.dir_off = rd_u32(buf + 16);
    return b;
}

static void batch_free(Batch *b) {
    if (b->buf) powder_free_buffer(b->buf, b->len);
    b->buf = NULL;
}

/* Column-directory entry offset for `name` (40 bytes per entry). */
static size_t col_dir(const Batch *b, const char *name) {
    size_t nlen = strlen(name);
    uint32_t c;
    for (c = 0; c < b->num_cols; c++) {
        size_t d = (size_t)b->dir_off + (size_t)c * 40;
        uint32_t name_off = rd_u32(b->buf + d);
        uint32_t name_len = rd_u32(b->buf + d + 4);
        if (name_len == nlen && memcmp(b->buf + name_off, name, nlen) == 0) return d;
    }
    fprintf(stderr, "UNCAUGHT: no such column: %s\n", name);
    exit(2);
}

static int64_t col_i64(const Batch *b, const char *name, size_t row) {
    size_t d = col_dir(b, name);
    int64_t v;
    memcpy(&v, b->buf + rd_u32(b->buf + d + 20) + row * 8, 8);
    return v;
}

static double col_f64(const Batch *b, const char *name, size_t row) {
    size_t d = col_dir(b, name);
    double v;
    memcpy(&v, b->buf + rd_u32(b->buf + d + 20) + row * 8, 8);
    return v;
}

static int col_is_valid(const Batch *b, const char *name, size_t row) {
    size_t d = col_dir(b, name);
    uint32_t voff;
    if (!(b->buf[d + 9] & 1)) return 1; /* no validity bitmap: all valid */
    voff = rd_u32(b->buf + d + 12);
    return (b->buf[(size_t)voff + (row >> 3)] >> (row & 7)) & 1;
}

static int col_str_eq(const Batch *b, const char *name, size_t row, const char *expect) {
    size_t d = col_dir(b, name);
    uint32_t buf1 = rd_u32(b->buf + d + 20); /* u32 offsets[num_rows+1] */
    uint32_t buf2 = rd_u32(b->buf + d + 28); /* utf-8 bytes */
    uint32_t s = rd_u32(b->buf + buf1 + row * 4);
    uint32_t e = rd_u32(b->buf + buf1 + (row + 1) * 4);
    size_t elen = strlen(expect);
    return (size_t)(e - s) == elen && memcmp(b->buf + buf2 + s, expect, elen) == 0;
}

/* ---- Fatal-on-error wrappers -------------------------------------------- */

static int64_t xexec(PowderClient *db, const char *sql, const char *params_json) {
    int64_t n = powder_execute(db, sql, params_json);
    if (n < 0) die(sql);
    return n;
}

/* Row-returning ORM op -> malloc'd NUL-terminated JSON string (caller frees). */
static char *orm_find(PowderClient *db, const PowderOrmSchema *sc, const char *op_json) {
    size_t len = 0;
    char *s;
    unsigned char *buf = powder_orm_find_json(db, sc, op_json, &len);
    if (!buf) die(op_json);
    s = (char *)malloc(len + 1);
    if (!s) { fprintf(stderr, "UNCAUGHT: out of memory\n"); exit(2); }
    memcpy(s, buf, len);
    s[len] = '\0';
    powder_free_buffer(buf, len);
    return s;
}

static int64_t orm_exec(PowderClient *db, const PowderOrmSchema *sc, const char *op_json) {
    int64_t n = powder_orm_execute(db, sc, op_json);
    if (n < 0) die(op_json);
    return n;
}

static int contains(const char *hay, const char *needle) {
    return strstr(hay, needle) != NULL;
}

/* ---- Schema (verification/powder.schema.json) --------------------------- */

static const char *SCHEMA =
    "{\"tables\":{"
    "\"customers\":{\"columns\":{"
    "\"id\":{\"type\":\"int\",\"primaryKey\":true},"
    "\"name\":{\"type\":\"text\"},"
    "\"email\":{\"type\":\"text\"},"
    "\"tier\":{\"type\":\"text\"},"
    "\"active\":{\"type\":\"bool\"}}},"
    "\"products\":{\"columns\":{"
    "\"id\":{\"type\":\"int\",\"primaryKey\":true},"
    "\"name\":{\"type\":\"text\"},"
    "\"price\":{\"type\":\"float\"},"
    "\"stock\":{\"type\":\"int\"}}},"
    "\"orders\":{\"columns\":{"
    "\"id\":{\"type\":\"int\",\"primaryKey\":true},"
    "\"customer_id\":{\"type\":\"int\",\"references\":{\"table\":\"customers\",\"column\":\"id\"}},"
    "\"status\":{\"type\":\"text\"},"
    "\"amount\":{\"type\":\"float\"},"
    "\"note\":{\"type\":\"text\",\"nullable\":true}}},"
    "\"order_items\":{\"columns\":{"
    "\"id\":{\"type\":\"int\",\"primaryKey\":true},"
    "\"order_id\":{\"type\":\"int\",\"references\":{\"table\":\"orders\",\"column\":\"id\"}},"
    "\"product_id\":{\"type\":\"int\",\"references\":{\"table\":\"products\",\"column\":\"id\"}},"
    "\"qty\":{\"type\":\"int\"},"
    "\"unit_price\":{\"type\":\"float\"}}}"
    "}}";

int main(int argc, char **argv) {
    const char *url = argc > 1 ? argv[1] : getenv("POWDER_URL");
    PowderClient *db;
    PowderOrmSchema *schema;

    if (!url || !url[0]) url = "sqlite::memory:";
    db = powder_connect(url);
    if (!db) die("connect");

    /* ---- reset (server DBs) + portable DDL ------------------------------ */
    xexec(db, "DROP TABLE IF EXISTS order_items", NULL);
    xexec(db, "DROP TABLE IF EXISTS orders", NULL);
    xexec(db, "DROP TABLE IF EXISTS products", NULL);
    xexec(db, "DROP TABLE IF EXISTS customers", NULL);
    xexec(db, "CREATE TABLE customers (id BIGINT PRIMARY KEY, name TEXT, email TEXT, tier TEXT, active BOOLEAN)", NULL);
    xexec(db, "CREATE TABLE products (id BIGINT PRIMARY KEY, name TEXT, price DOUBLE PRECISION, stock BIGINT)", NULL);
    xexec(db, "CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT REFERENCES customers(id), status TEXT, amount DOUBLE PRECISION, note TEXT)", NULL);
    xexec(db, "CREATE TABLE order_items (id BIGINT PRIMARY KEY, order_id BIGINT REFERENCES orders(id), product_id BIGINT REFERENCES products(id), qty BIGINT, unit_price DOUBLE PRECISION)", NULL);

    schema = powder_orm_schema_new(SCHEMA);
    if (!schema) die("orm schema");

    /* ---- Seed via ORM create / createMany -------------------------------- */
    orm_exec(db, schema,
             "{\"op\":\"create\",\"table\":\"customers\",\"data\":"
             "{\"id\":1,\"name\":\"김민준\",\"email\":\"minjun@corp.kr\",\"tier\":\"vip\",\"active\":true}}");
    orm_exec(db, schema,
             "{\"op\":\"createMany\",\"table\":\"customers\",\"rows\":["
             "{\"id\":2,\"name\":\"이서연\",\"email\":\"seoyeon@corp.kr\",\"tier\":\"gold\",\"active\":true},"
             "{\"id\":3,\"name\":\"박도윤\",\"email\":\"doyun@corp.kr\",\"tier\":\"basic\",\"active\":true},"
             "{\"id\":4,\"name\":\"최지우\",\"email\":\"jiwoo@old.kr\",\"tier\":\"basic\",\"active\":false},"
             "{\"id\":5,\"name\":\"정하은\",\"email\":\"haeun@corp.kr\",\"tier\":\"vip\",\"active\":true}]}");
    CHECK(orm_exec(db, schema, "{\"op\":\"count\",\"table\":\"customers\",\"where\":{}}") == 5,
          "seed: 5 customers (create + createMany)");

    orm_exec(db, schema,
             "{\"op\":\"createMany\",\"table\":\"products\",\"rows\":["
             "{\"id\":10,\"name\":\"노트북\",\"price\":1500000,\"stock\":12},"
             "{\"id\":11,\"name\":\"모니터\",\"price\":350000,\"stock\":40},"
             "{\"id\":12,\"name\":\"키보드\",\"price\":89000,\"stock\":0},"
             "{\"id\":13,\"name\":\"마우스\",\"price\":45000,\"stock\":200}]}");

    orm_exec(db, schema,
             "{\"op\":\"createMany\",\"table\":\"orders\",\"rows\":["
             "{\"id\":100,\"customer_id\":1,\"status\":\"paid\",\"amount\":1850000,\"note\":\"빠른배송 요청\"},"
             "{\"id\":101,\"customer_id\":1,\"status\":\"paid\",\"amount\":89000,\"note\":null},"
             "{\"id\":102,\"customer_id\":2,\"status\":\"shipped\",\"amount\":350000,\"note\":null},"
             "{\"id\":103,\"customer_id\":3,\"status\":\"pending\",\"amount\":45000,\"note\":null},"
             "{\"id\":104,\"customer_id\":5,\"status\":\"paid\",\"amount\":700000,\"note\":\"법인 세금계산서\"},"
             "{\"id\":105,\"customer_id\":2,\"status\":\"cancelled\",\"amount\":89000,\"note\":\"고객 변심\"},"
             "{\"id\":106,\"customer_id\":5,\"status\":\"paid\",\"amount\":1500000,\"note\":null}]}");

    orm_exec(db, schema,
             "{\"op\":\"createMany\",\"table\":\"order_items\",\"rows\":["
             "{\"id\":1000,\"order_id\":100,\"product_id\":10,\"qty\":1,\"unit_price\":1500000},"
             "{\"id\":1001,\"order_id\":100,\"product_id\":11,\"qty\":1,\"unit_price\":350000},"
             "{\"id\":1002,\"order_id\":101,\"product_id\":12,\"qty\":1,\"unit_price\":89000},"
             "{\"id\":1003,\"order_id\":102,\"product_id\":11,\"qty\":1,\"unit_price\":350000},"
             "{\"id\":1004,\"order_id\":103,\"product_id\":13,\"qty\":1,\"unit_price\":45000},"
             "{\"id\":1005,\"order_id\":104,\"product_id\":11,\"qty\":2,\"unit_price\":350000},"
             "{\"id\":1006,\"order_id\":106,\"product_id\":10,\"qty\":1,\"unit_price\":1500000}]}");

    /* =====================================================================
     * 1. Dashboard: revenue summary per status (raw SQL GROUP BY)
     * ===================================================================== */
    {
        Batch b = q(db,
                    "SELECT status, COUNT(*) AS cnt, SUM(amount) AS revenue "
                    "FROM orders GROUP BY status ORDER BY revenue DESC", NULL);
        CHECK(b.num_rows == 4, "dashboard: 4 status groups");
        CHECK(col_str_eq(&b, "status", 0, "paid"), "dashboard: top revenue status is 'paid'");
        CHECK(col_i64(&b, "cnt", 0) == 4, "dashboard: 4 paid orders");
        CHECK(col_f64(&b, "revenue", 0) == 4139000.0, "dashboard: paid revenue = 4,139,000");
        batch_free(&b);
    }

    /* =====================================================================
     * 2. Per-customer revenue report (raw SQL JOIN + GROUP BY + HAVING)
     * ===================================================================== */
    {
        Batch b = q(db,
                    "SELECT c.name, c.tier, COUNT(o.id) AS orders_cnt, SUM(o.amount) AS total "
                    "FROM customers c JOIN orders o ON o.customer_id = c.id "
                    "WHERE o.status != 'cancelled' "
                    "GROUP BY c.id HAVING SUM(o.amount) >= ? "
                    "ORDER BY total DESC", "[100000.0]");
        CHECK(b.num_rows == 3, "report: 3 customers over 100k (non-cancelled)");
        CHECK(col_str_eq(&b, "name", 0, "정하은") && col_f64(&b, "total", 0) == 2200000.0,
              "report: top customer 정하은 = 2,200,000");
        CHECK(col_str_eq(&b, "name", 1, "김민준") && col_i64(&b, "orders_cnt", 1) == 2,
              "report: 김민준 has 2 orders");
        batch_free(&b);
    }

    /* =====================================================================
     * 3. Subquery: customers who never ordered (raw SQL NOT IN)
     * ===================================================================== */
    {
        Batch b = q(db,
                    "SELECT name FROM customers WHERE id NOT IN "
                    "(SELECT DISTINCT customer_id FROM orders)", NULL);
        CHECK(b.num_rows == 1 && col_str_eq(&b, "name", 0, "최지우"),
              "subquery: only 최지우 never ordered");
        batch_free(&b);
    }

    /* =====================================================================
     * 4. Ordered products that are out of stock (raw SQL JOIN + WHERE)
     * ===================================================================== */
    {
        Batch b = q(db,
                    "SELECT DISTINCT p.name FROM products p "
                    "JOIN order_items i ON i.product_id = p.id WHERE p.stock = 0", NULL);
        CHECK(b.num_rows == 1 && col_str_eq(&b, "name", 0, "키보드"),
              "stockout: 키보드 ordered but out of stock");
        batch_free(&b);
    }

    /* =====================================================================
     * 5. ORM finder: nested AND/OR/NOT + in/like
     * ===================================================================== */
    {
        char *rows = orm_find(db, schema,
            "{\"op\":\"findMany\",\"table\":\"customers\",\"where\":{"
            "\"active\":true,"
            "\"OR\":[{\"tier\":\"vip\"},{\"email\":{\"like\":\"%@corp.kr\"}}],"
            "\"NOT\":{\"name\":{\"like\":\"최%\"}}"
            "},\"orderBy\":{\"id\":\"asc\"}}");
        CHECK(contains(rows, "김민준") && contains(rows, "정하은") &&
              contains(rows, "이서연") && contains(rows, "박도윤") &&
              !contains(rows, "최지우"),
              "orm-finder: nested OR/NOT/like matches 4 active corp customers");
        free(rows);

        char *vips = orm_find(db, schema,
            "{\"op\":\"findMany\",\"table\":\"customers\","
            "\"where\":{\"tier\":{\"in\":[\"vip\",\"gold\"]}},\"orderBy\":{\"id\":\"asc\"}}");
        CHECK(contains(vips, "김민준") && contains(vips, "이서연") &&
              contains(vips, "정하은") && !contains(vips, "박도윤"),
              "orm-finder: tier in [vip, gold] -> 3 customers");
        free(vips);
    }

    /* =====================================================================
     * 6. ORM pagination: limit + offset + count
     * ===================================================================== */
    {
        char *page1 = orm_find(db, schema,
            "{\"op\":\"findMany\",\"table\":\"orders\","
            "\"orderBy\":{\"amount\":\"desc\"},\"limit\":3,\"offset\":0}");
        char *page2 = orm_find(db, schema,
            "{\"op\":\"findMany\",\"table\":\"orders\","
            "\"orderBy\":{\"amount\":\"desc\"},\"limit\":3,\"offset\":3}");
        int64_t total = orm_exec(db, schema, "{\"op\":\"count\",\"table\":\"orders\",\"where\":{}}");
        CHECK(contains(page1, "\"id\":100") && contains(page1, "\"id\":106"),
              "orm-paginate: page1 has two biggest orders");
        CHECK(!contains(page2, "\"id\":100") && contains(page2, "\"id\":102"),
              "orm-paginate: page2 disjoint from page1");
        CHECK(total == 7, "orm-paginate: total = 7 (totalPages = ceil(7/3) = 3)");
        free(page1);
        free(page2);
    }

    /* =====================================================================
     * 7. ORM relations: include (batched) + join (belongsTo LEFT JOIN)
     * ===================================================================== */
    {
        char *with_customer = orm_find(db, schema,
            "{\"op\":\"findMany\",\"table\":\"orders\",\"where\":{\"status\":\"paid\"},"
            "\"include\":{\"customer\":true},\"orderBy\":{\"id\":\"asc\"}}");
        CHECK(contains(with_customer, "\"customer\"") && contains(with_customer, "김민준") &&
              contains(with_customer, "정하은"),
              "orm-include: paid orders hydrated with customer objects");
        free(with_customer);

        char *joined = orm_find(db, schema,
            "{\"op\":\"findMany\",\"table\":\"orders\",\"where\":{\"amount\":{\"gte\":1000000}},"
            "\"join\":{\"customer\":true},\"orderBy\":{\"id\":\"asc\"}}");
        CHECK(contains(joined, "김민준") && contains(joined, "정하은"),
              "orm-join: belongsTo LEFT JOIN hydrates big orders");
        free(joined);

        char *deep = orm_find(db, schema,
            "{\"op\":\"findMany\",\"table\":\"order_items\",\"where\":{\"qty\":{\"gte\":2}},"
            "\"include\":{\"order\":{\"include\":{\"customer\":true}}}}");
        CHECK(contains(deep, "\"order\"") && contains(deep, "정하은"),
              "orm-include: nested include order->customer");
        free(deep);
    }

    /* =====================================================================
     * 8. ORM groupBy + having + aliases
     * ===================================================================== */
    {
        char *g = orm_find(db, schema,
            "{\"op\":\"groupBy\",\"table\":\"orders\","
            "\"by\":[\"customer_id\"],"
            "\"where\":{\"status\":{\"ne\":\"cancelled\"}},"
            "\"count\":true,"
            "\"sum\":[\"amount\"],"
            "\"having\":{\"_sum_amount\":{\"gt\":100000}},"
            "\"orderBy\":{\"_sum_amount\":\"desc\"}}");
        CHECK(contains(g, "_count") && contains(g, "_sum_amount"),
              "orm-groupby: alias columns _count/_sum_amount present");
        CHECK(contains(g, "\"customer_id\":5") && contains(g, "\"customer_id\":1") &&
              contains(g, "\"customer_id\":2") && !contains(g, "\"customer_id\":3"),
              "orm-groupby: having _sum_amount>100000 keeps 3 groups");
        free(g);
    }

    /* =====================================================================
     * 9. ORM aggregate: sum/max + empty set -> null
     * ===================================================================== */
    {
        char *s = orm_find(db, schema,
            "{\"op\":\"aggregate\",\"table\":\"orders\",\"fn\":\"sum\","
            "\"column\":\"amount\",\"where\":{\"status\":\"paid\"}}");
        CHECK(strcmp(s, "4139000.0") == 0 || strcmp(s, "4139000") == 0,
              "orm-aggregate: sum(paid amount)");
        free(s);

        char *mx = orm_find(db, schema,
            "{\"op\":\"aggregate\",\"table\":\"products\",\"fn\":\"max\","
            "\"column\":\"price\",\"where\":{}}");
        CHECK(contains(mx, "1500000"), "orm-aggregate: max product price");
        free(mx);

        char *none = orm_find(db, schema,
            "{\"op\":\"aggregate\",\"table\":\"orders\",\"fn\":\"avg\","
            "\"column\":\"amount\",\"where\":{\"status\":\"refunded\"}}");
        CHECK(strcmp(none, "null") == 0, "orm-aggregate: empty set -> null");
        free(none);
    }

    /* =====================================================================
     * 10. Order-creation transaction: stock decrement + order + items.
     *     Inner savepoint failure must not undo outer work.
     *     (No transaction helper in C: BEGIN/SAVEPOINT via powder_execute.)
     * ===================================================================== */
    {
        xexec(db, "BEGIN IMMEDIATE", NULL);
        xexec(db, "INSERT INTO orders VALUES (?, ?, ?, ?, ?)", "[107, 3, \"paid\", 90000.0, null]");
        xexec(db, "INSERT INTO order_items VALUES (?, ?, ?, ?, ?)", "[1007, 107, 13, 2, 45000.0]");
        xexec(db, "UPDATE products SET stock = stock - ? WHERE id = ?", "[2, 13]");

        /* Inner attempt: bad item -> simulated "out of stock" failure,
         * rolled back to the savepoint only. */
        xexec(db, "SAVEPOINT powder_sp_1", NULL);
        xexec(db, "INSERT INTO order_items VALUES (?, ?, ?, ?, ?)", "[1008, 107, 12, 1, 89000.0]");
        xexec(db, "ROLLBACK TO powder_sp_1", NULL);
        xexec(db, "RELEASE powder_sp_1", NULL);

        xexec(db, "COMMIT", NULL);

        {
            Batch b = q(db, "SELECT COUNT(*) AS c FROM order_items WHERE order_id = 107", NULL);
            CHECK(col_i64(&b, "c", 0) == 1, "tx: outer committed, inner savepoint rolled back");
            batch_free(&b);
        }
        {
            Batch st = q(db, "SELECT stock FROM products WHERE id = 13", NULL);
            CHECK(col_i64(&st, "stock", 0) == 198, "tx: stock decremented inside transaction");
            batch_free(&st);
        }
    }

    /* Full rollback: nothing survives a payment failure. */
    {
        xexec(db, "BEGIN IMMEDIATE", NULL);
        xexec(db, "INSERT INTO orders VALUES (999, 1, 'paid', 1.0, NULL)", NULL);
        /* simulated payment failure */
        xexec(db, "ROLLBACK", NULL);

        Batch b = q(db, "SELECT COUNT(*) AS c FROM orders WHERE id = 999", NULL);
        CHECK(col_i64(&b, "c", 0) == 0, "tx: full rollback on payment failure");
        batch_free(&b);
    }

    /* =====================================================================
     * 11. ORM update / delete / exists — operations work
     * ===================================================================== */
    {
        int64_t n = orm_exec(db, schema,
            "{\"op\":\"update\",\"table\":\"orders\","
            "\"where\":{\"status\":\"pending\"},\"data\":{\"status\":\"cancelled\"}}");
        CHECK(n == 1, "orm-update: 1 pending order cancelled");

        char *first = orm_find(db, schema,
            "{\"op\":\"findFirst\",\"table\":\"customers\",\"where\":{\"active\":false},\"limit\":1}");
        CHECK(strcmp(first, "null") != 0, "orm-exists: inactive customer exists");
        free(first);

        /* FK enforcement: deleting an order that still has items must fail. */
        {
            int fk_enforced = 0;
            int64_t r = powder_orm_execute(db, schema,
                "{\"op\":\"delete\",\"table\":\"orders\",\"where\":{\"id\":103}}");
            if (r < 0) {
                /* SQLite/MySQL say "FOREIGN KEY", Postgres "foreign key". */
                const char *err = powder_last_error();
                if (err != NULL) {
                    char low[512];
                    size_t n = strlen(err);
                    if (n >= sizeof low) n = sizeof low - 1;
                    for (size_t i = 0; i < n; i++) low[i] = (char)tolower((unsigned char)err[i]);
                    low[n] = '\0';
                    fk_enforced = strstr(low, "foreign key") != NULL;
                }
            }
            CHECK(fk_enforced, "orm-delete: FK violation rejected (child items exist)");
        }

        /* Real-world order: purge items first, then the order. */
        {
            int64_t child_removed = orm_exec(db, schema,
                "{\"op\":\"delete\",\"table\":\"order_items\",\"where\":{\"order_id\":103}}");
            int64_t removed = orm_exec(db, schema,
                "{\"op\":\"delete\",\"table\":\"orders\","
                "\"where\":{\"status\":\"cancelled\",\"amount\":{\"lt\":50000}}}");
            CHECK(child_removed == 1 && removed == 1,
                  "orm-delete: cascade order (items first, then order) succeeds");
        }
    }

    /* =====================================================================
     * 12. NULL handling: nullable note column (validity bitmap)
     * ===================================================================== */
    {
        Batch b = q(db, "SELECT id, note FROM orders WHERE id IN (100, 101) ORDER BY id", NULL);
        CHECK(col_is_valid(&b, "note", 0) && col_str_eq(&b, "note", 0, "빠른배송 요청"),
              "null: note present on order 100");
        CHECK(!col_is_valid(&b, "note", 1), "null: note NULL on order 101");
        batch_free(&b);
    }

    /* =====================================================================
     * 13. Named-query style: parameterized LTV query
     * ===================================================================== */
    {
        Batch b = q(db,
                    "SELECT c.id, c.name, SUM(o.amount) AS ltv FROM customers c "
                    "JOIN orders o ON o.customer_id = c.id "
                    "WHERE c.active = ? AND o.status = ? "
                    "GROUP BY c.id HAVING SUM(o.amount) >= ? ORDER BY ltv DESC",
                    "[true, \"paid\", 500000.0]");
        CHECK(b.num_rows == 2, "named-style: 2 active customers with paid LTV >= 500k");
        CHECK(col_str_eq(&b, "name", 0, "정하은"), "named-style: highest LTV first");
        batch_free(&b);
    }

    /* =====================================================================
     * 14. Korean + emoji (non-ASCII) round-trip
     * ===================================================================== */
    {
        xexec(db, "INSERT INTO customers VALUES (?, ?, ?, ?, ?)",
              "[6, \"한글🚀고객\", \"emoji@corp.kr\", \"basic\", true]");
        Batch b = q(db, "SELECT name FROM customers WHERE id = 6", NULL);
        CHECK(col_str_eq(&b, "name", 0, "한글🚀고객"), "utf8: Korean + emoji round-trip");
        batch_free(&b);
    }

    powder_orm_schema_free(schema);
    powder_close(db);

    printf("\n%s — %d checks, %d failed\n",
           failures == 0 ? "REAL-WORLD QUERIES OK" : "SOME QUERIES FAILED",
           checks, failures);
    return failures == 0 ? 0 : 1;
}
