import com.powder.Batch;
import com.powder.Client;
import com.powder.Orm;
import com.powder.Powder;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Real-world business-query verification for the Java (JNI) binding — the 14
 * scenarios from verification/SCENARIOS.md, mirroring the C++ reference
 * (verification/cpp/realworld_queries.cpp).
 *
 * Usage: java RealworldQueries <path-to-powder_java.dll> [connection-url]
 * URL fallback: POWDER_URL env var, then "sqlite::memory:".
 */
public class RealworldQueries {
    static int checks = 0, failures = 0;

    static void check(boolean cond, String what) {
        checks++;
        if (cond) {
            System.out.println("ok: " + what);
        } else {
            failures++;
            System.err.println("FAILED: " + what);
        }
    }

    static LinkedHashMap<String, Object> row(Object... kv) {
        LinkedHashMap<String, Object> m = new LinkedHashMap<>();
        for (int i = 0; i < kv.length; i += 2) {
            m.put((String) kv[i], kv[i + 1]);
        }
        return m;
    }

    static boolean anyName(List<Map<String, Object>> rows, String name) {
        return rows.stream().anyMatch(r -> name.equals(r.get("name")));
    }

    static boolean anyId(List<Map<String, Object>> rows, long id) {
        return rows.stream().anyMatch(r -> Long.valueOf(id).equals(r.get("id")));
    }

    static String readSchema() throws Exception {
        String env = System.getenv("POWDER_SCHEMA");
        Path[] candidates = {
            env != null ? Path.of(env) : null,
            Path.of("powder.schema.json"),
            Path.of("..", "powder.schema.json"),
        };
        for (Path p : candidates) {
            if (p != null && Files.exists(p)) {
                return Files.readString(p);
            }
        }
        throw new IllegalStateException("powder.schema.json not found (cwd or parent, or set POWDER_SCHEMA)");
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 1) {
            throw new IllegalArgumentException("usage: RealworldQueries <path-to-native-lib> [connection-url]");
        }
        Powder.loadLibrary(args[0]);
        String url = args.length >= 2 ? args[1]
                : System.getenv("POWDER_URL") != null ? System.getenv("POWDER_URL")
                : "sqlite::memory:";
        String schemaJson = readSchema();

        try (Client db = Powder.connect(url)) {
            // ---- reset + DDL (portable SQL from SCENARIOS.md) ---------------
            db.execute("DROP TABLE IF EXISTS order_items");
            db.execute("DROP TABLE IF EXISTS orders");
            db.execute("DROP TABLE IF EXISTS products");
            db.execute("DROP TABLE IF EXISTS customers");
            db.execute("CREATE TABLE customers (id BIGINT PRIMARY KEY, name TEXT, email TEXT, tier TEXT, active BOOLEAN)");
            db.execute("CREATE TABLE products (id BIGINT PRIMARY KEY, name TEXT, price DOUBLE PRECISION, stock BIGINT)");
            db.execute("CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT REFERENCES customers(id), status TEXT, amount DOUBLE PRECISION, note TEXT)");
            db.execute("CREATE TABLE order_items (id BIGINT PRIMARY KEY, order_id BIGINT REFERENCES orders(id), product_id BIGINT REFERENCES products(id), qty BIGINT, unit_price DOUBLE PRECISION)");

            try (Orm orm = db.orm(schemaJson)) {
                Orm.Table customers = orm.table("customers");
                Orm.Table products = orm.table("products");
                Orm.Table orders = orm.table("orders");
                Orm.Table items = orm.table("order_items");

                // ---- Seed via ORM create / createMany -----------------------
                customers.create(row("id", 1, "name", "김민준", "email", "minjun@corp.kr", "tier", "vip", "active", true));
                customers.createMany(List.of(
                        row("id", 2, "name", "이서연", "email", "seoyeon@corp.kr", "tier", "gold", "active", true),
                        row("id", 3, "name", "박도윤", "email", "doyun@corp.kr", "tier", "basic", "active", true),
                        row("id", 4, "name", "최지우", "email", "jiwoo@old.kr", "tier", "basic", "active", false),
                        row("id", 5, "name", "정하은", "email", "haeun@corp.kr", "tier", "vip", "active", true)));
                check(customers.count(null) == 5, "seed: 5 customers (create + createMany)");

                products.createMany(List.of(
                        row("id", 10, "name", "노트북", "price", 1500000, "stock", 12),
                        row("id", 11, "name", "모니터", "price", 350000, "stock", 40),
                        row("id", 12, "name", "키보드", "price", 89000, "stock", 0),
                        row("id", 13, "name", "마우스", "price", 45000, "stock", 200)));

                orders.createMany(List.of(
                        row("id", 100, "customer_id", 1, "status", "paid", "amount", 1850000, "note", "빠른배송 요청"),
                        row("id", 101, "customer_id", 1, "status", "paid", "amount", 89000, "note", null),
                        row("id", 102, "customer_id", 2, "status", "shipped", "amount", 350000, "note", null),
                        row("id", 103, "customer_id", 3, "status", "pending", "amount", 45000, "note", null),
                        row("id", 104, "customer_id", 5, "status", "paid", "amount", 700000, "note", "법인 세금계산서"),
                        row("id", 105, "customer_id", 2, "status", "cancelled", "amount", 89000, "note", "고객 변심"),
                        row("id", 106, "customer_id", 5, "status", "paid", "amount", 1500000, "note", null)));

                items.createMany(List.of(
                        row("id", 1000, "order_id", 100, "product_id", 10, "qty", 1, "unit_price", 1500000),
                        row("id", 1001, "order_id", 100, "product_id", 11, "qty", 1, "unit_price", 350000),
                        row("id", 1002, "order_id", 101, "product_id", 12, "qty", 1, "unit_price", 89000),
                        row("id", 1003, "order_id", 102, "product_id", 11, "qty", 1, "unit_price", 350000),
                        row("id", 1004, "order_id", 103, "product_id", 13, "qty", 1, "unit_price", 45000),
                        row("id", 1005, "order_id", 104, "product_id", 11, "qty", 2, "unit_price", 350000),
                        row("id", 1006, "order_id", 106, "product_id", 10, "qty", 1, "unit_price", 1500000)));

                // =============================================================
                // 1. 대시보드: 상태별 매출 요약 (raw SQL GROUP BY)
                // =============================================================
                {
                    Batch b = db.query(
                            "SELECT status, COUNT(*) AS cnt, SUM(amount) AS revenue "
                            + "FROM orders GROUP BY status ORDER BY revenue DESC");
                    check(b.numRows() == 4, "dashboard: 4 status groups");
                    check("paid".equals(b.column("status").getString(0)), "dashboard: top revenue status is 'paid'");
                    check(b.column("cnt").getLong(0) == 4, "dashboard: 4 paid orders");
                    check(b.column("revenue").getDouble(0) == 4139000.0, "dashboard: paid revenue = 4,139,000");
                }

                // =============================================================
                // 2. 고객별 매출 리포트 (raw SQL JOIN + GROUP BY + HAVING)
                // =============================================================
                {
                    Batch b = db.query(
                            "SELECT c.name, c.tier, COUNT(o.id) AS orders_cnt, SUM(o.amount) AS total "
                            + "FROM customers c JOIN orders o ON o.customer_id = c.id "
                            + "WHERE o.status != 'cancelled' "
                            + "GROUP BY c.id HAVING SUM(o.amount) >= ? "
                            + "ORDER BY total DESC", 100000.0);
                    check(b.numRows() == 3, "report: 3 customers over 100k (non-cancelled)");
                    check("정하은".equals(b.column("name").getString(0)) && b.column("total").getDouble(0) == 2200000.0,
                            "report: top customer 정하은 = 2,200,000");
                    check("김민준".equals(b.column("name").getString(1)) && b.column("orders_cnt").getLong(1) == 2,
                            "report: 김민준 has 2 orders");
                }

                // =============================================================
                // 3. 서브쿼리: 한 번도 주문 안 한 고객 (raw SQL NOT IN)
                // =============================================================
                {
                    Batch b = db.query(
                            "SELECT name FROM customers WHERE id NOT IN (SELECT DISTINCT customer_id FROM orders)");
                    check(b.numRows() == 1 && "최지우".equals(b.column("name").getString(0)),
                            "subquery: only 최지우 never ordered");
                }

                // =============================================================
                // 4. 재고 없는 상품 중 주문된 것 (raw SQL JOIN + WHERE)
                // =============================================================
                {
                    Batch b = db.query(
                            "SELECT DISTINCT p.name FROM products p "
                            + "JOIN order_items i ON i.product_id = p.id WHERE p.stock = 0");
                    check(b.numRows() == 1 && "키보드".equals(b.column("name").getString(0)),
                            "stockout: 키보드 ordered but out of stock");
                }

                // =============================================================
                // 5. ORM finder: 중첩 AND/OR/NOT + in/like
                // =============================================================
                {
                    List<Map<String, Object>> rows = customers.findMany(row(
                            "where", row(
                                    "active", true,
                                    "OR", List.of(
                                            row("tier", "vip"),
                                            row("email", row("like", "%@corp.kr"))),
                                    "NOT", row("name", row("like", "최%"))),
                            "orderBy", row("id", "asc")));
                    check(anyName(rows, "김민준") && anyName(rows, "정하은")
                            && anyName(rows, "이서연") && anyName(rows, "박도윤")
                            && !anyName(rows, "최지우"),
                            "orm-finder: nested OR/NOT/like matches 4 active corp customers");

                    List<Map<String, Object>> vips = customers.findMany(row(
                            "where", row("tier", row("in", List.of("vip", "gold"))),
                            "orderBy", row("id", "asc")));
                    check(vips.size() == 3 && anyName(vips, "김민준") && anyName(vips, "이서연")
                            && anyName(vips, "정하은") && !anyName(vips, "박도윤"),
                            "orm-finder: tier in [vip, gold] -> 3 customers");
                }

                // =============================================================
                // 6. ORM 페이지네이션: limit + offset + count
                // =============================================================
                {
                    List<Map<String, Object>> page1 = orders.findMany(row(
                            "orderBy", row("amount", "desc"), "limit", 3, "offset", 0));
                    List<Map<String, Object>> page2 = orders.findMany(row(
                            "orderBy", row("amount", "desc"), "limit", 3, "offset", 3));
                    long total = orders.count(null);
                    check(anyId(page1, 100) && anyId(page1, 106),
                            "orm-paginate: page1 has two biggest orders");
                    check(!anyId(page2, 100) && anyId(page2, 102),
                            "orm-paginate: page2 disjoint from page1");
                    check(total == 7, "orm-paginate: total = 7 (totalPages = ceil(7/3) = 3)");
                }

                // =============================================================
                // 7. ORM 관계: include (배치 로드) + join + 중첩 include
                // =============================================================
                {
                    List<Map<String, Object>> withCustomer = orders.findMany(row(
                            "where", row("status", "paid"),
                            "include", row("customer", true),
                            "orderBy", row("id", "asc")));
                    boolean hydrated = !withCustomer.isEmpty()
                            && withCustomer.stream().allMatch(r -> r.get("customer") instanceof Map);
                    boolean names = withCustomer.stream().anyMatch(r ->
                                    "김민준".equals(((Map<?, ?>) r.get("customer")).get("name")))
                            && withCustomer.stream().anyMatch(r ->
                                    "정하은".equals(((Map<?, ?>) r.get("customer")).get("name")));
                    check(hydrated && names, "orm-include: paid orders hydrated with customer objects");

                    List<Map<String, Object>> joined = orders.findMany(row(
                            "where", row("amount", row("gte", 1000000)),
                            "join", row("customer", true),
                            "orderBy", row("id", "asc")));
                    boolean joinNames = joined.stream().anyMatch(r ->
                                    "김민준".equals(((Map<?, ?>) r.get("customer")).get("name")))
                            && joined.stream().anyMatch(r ->
                                    "정하은".equals(((Map<?, ?>) r.get("customer")).get("name")));
                    check(joinNames, "orm-join: belongsTo LEFT JOIN hydrates big orders");

                    List<Map<String, Object>> deep = items.findMany(row(
                            "where", row("qty", row("gte", 2)),
                            "include", row("order", row("include", row("customer", true)))));
                    boolean nested = deep.size() == 1 && deep.get(0).get("order") instanceof Map
                            && "정하은".equals(((Map<?, ?>) ((Map<?, ?>) deep.get(0).get("order")).get("customer")).get("name"));
                    check(nested, "orm-include: nested include order->customer");
                }

                // =============================================================
                // 8. ORM groupBy + having + 별칭
                // =============================================================
                {
                    List<Map<String, Object>> g = orders.groupBy(row(
                            "by", List.of("customer_id"),
                            "where", row("status", row("ne", "cancelled")),
                            "count", true,
                            "sum", List.of("amount"),
                            "having", row("_sum_amount", row("gt", 100000)),
                            "orderBy", row("_sum_amount", "desc")));
                    boolean aliases = !g.isEmpty() && g.get(0).containsKey("_count") && g.get(0).containsKey("_sum_amount");
                    check(aliases, "orm-groupby: alias columns _count/_sum_amount present");
                    boolean groups = g.size() == 3
                            && g.stream().anyMatch(r -> Long.valueOf(5).equals(r.get("customer_id")))
                            && g.stream().anyMatch(r -> Long.valueOf(1).equals(r.get("customer_id")))
                            && g.stream().anyMatch(r -> Long.valueOf(2).equals(r.get("customer_id")))
                            && g.stream().noneMatch(r -> Long.valueOf(3).equals(r.get("customer_id")));
                    check(groups, "orm-groupby: having _sum_amount>100000 keeps 3 groups");
                }

                // =============================================================
                // 9. ORM aggregate: sum/max (+ 빈 집합은 null)
                // =============================================================
                {
                    Double s = orders.aggregate("sum", "amount", row("status", "paid"));
                    check(s != null && s == 4139000.0, "orm-aggregate: sum(paid amount)");
                    Double mx = products.aggregate("max", "price", null);
                    check(mx != null && mx == 1500000.0, "orm-aggregate: max product price");
                    Double none = orders.aggregate("avg", "amount", row("status", "refunded"));
                    check(none == null, "orm-aggregate: empty set -> null");
                }

                // =============================================================
                // 10. 트랜잭션: 주문 생성 + 재고 차감, 안쪽 세이브포인트만 롤백
                // =============================================================
                {
                    db.transaction(tx -> {
                        tx.execute("INSERT INTO orders VALUES (?, ?, ?, ?, ?)",
                                107L, 3L, "paid", 90000.0, null);
                        tx.execute("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)",
                                1007L, 107L, 13L, 2L, 45000.0);
                        tx.execute("UPDATE products SET stock = stock - ? WHERE id = ?", 2L, 13L);

                        try {
                            tx.transaction(inner -> {
                                inner.execute("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)",
                                        1008L, 107L, 12L, 1L, 89000.0);
                                throw new RuntimeException("재고 없음 — 취소");
                            });
                        } catch (RuntimeException ignored) {
                        }
                    });

                    Batch b = db.query("SELECT COUNT(*) AS c FROM order_items WHERE order_id = 107");
                    check(b.column("c").getLong(0) == 1, "tx: outer committed, inner savepoint rolled back");
                    Batch st = db.query("SELECT stock FROM products WHERE id = 13");
                    check(st.column("stock").getLong(0) == 198, "tx: stock decremented inside transaction");

                    // 전체 롤백: 예외 시 아무것도 남지 않음
                    try {
                        db.transaction(tx -> {
                            tx.execute("INSERT INTO orders VALUES (999, 1, 'paid', 1.0, NULL)");
                            throw new RuntimeException("결제 실패");
                        });
                    } catch (RuntimeException ignored) {
                    }
                    Batch rb = db.query("SELECT COUNT(*) AS c FROM orders WHERE id = 999");
                    check(rb.column("c").getLong(0) == 0, "tx: full rollback on payment failure");
                }

                // =============================================================
                // 11. ORM update / delete / exists — 운영 업무
                // =============================================================
                {
                    long n = orders.update(row("status", "pending"), row("status", "cancelled"));
                    check(n == 1, "orm-update: 1 pending order cancelled");

                    check(customers.exists(row("active", false)), "orm-exists: inactive customer exists");

                    // FK 강제: 자식(order_items)이 남아 있는 주문 삭제는 거부돼야 함
                    boolean fkEnforced = false;
                    try {
                        orders.delete(row("id", 103));
                    } catch (RuntimeException e) {
                        // SQLite/MySQL say "FOREIGN KEY", Postgres "foreign key".
                        fkEnforced = e.getMessage() != null
                                && e.getMessage().toLowerCase().contains("foreign key");
                    }
                    check(fkEnforced, "orm-delete: FK violation rejected (child items exist)");

                    long childRemoved = items.delete(row("order_id", 103));
                    long removed = orders.delete(row("status", "cancelled", "amount", row("lt", 50000)));
                    check(childRemoved == 1 && removed == 1,
                            "orm-delete: cascade order (items first, then order) succeeds");
                }

                // =============================================================
                // 12. NULL 처리: nullable note 컬럼 (isValid)
                // =============================================================
                {
                    Batch b = db.query("SELECT id, note FROM orders WHERE id IN (100, 101) ORDER BY id");
                    check(b.column("note").isValid(0) && "빠른배송 요청".equals(b.column("note").getString(0)),
                            "null: note present on order 100");
                    check(!b.column("note").isValid(1) && b.column("note").get(1) == null,
                            "null: note NULL on order 101");
                }

                // =============================================================
                // 13. 명명 쿼리 스타일: 파라미터화 LTV 쿼리
                // =============================================================
                {
                    Batch b = db.query(
                            "SELECT c.id, c.name, SUM(o.amount) AS ltv FROM customers c "
                            + "JOIN orders o ON o.customer_id = c.id "
                            + "WHERE c.active = ? AND o.status = ? "
                            + "GROUP BY c.id HAVING SUM(o.amount) >= ? ORDER BY ltv DESC",
                            true, "paid", 500000.0);
                    check(b.numRows() == 2, "named-style: 2 active customers with paid LTV >= 500k");
                    check("정하은".equals(b.column("name").getString(0)), "named-style: highest LTV first");
                }

                // =============================================================
                // 14. 한글/이모지 등 non-ASCII 왕복
                // =============================================================
                {
                    db.execute("INSERT INTO customers VALUES (?, ?, ?, ?, ?)",
                            6L, "한글🚀고객", "emoji@corp.kr", "basic", true);
                    Batch b = db.query("SELECT name FROM customers WHERE id = 6");
                    check("한글🚀고객".equals(b.column("name").getString(0)), "utf8: Korean + emoji round-trip");
                }
            }
        }

        System.out.printf("%n%s — %d checks, %d failed%n",
                failures == 0 ? "REAL-WORLD QUERIES OK" : "SOME QUERIES FAILED", checks, failures);
        System.exit(failures == 0 ? 0 : 1);
    }
}
