// Real-world business-query verification of the Kotlin binding: the shared
// e-commerce scenarios (verification/SCENARIOS.md) exercised through raw SQL,
// the chainable from() DSL, and the schema-aware ORM (powder.schema.json).
//
//   kotlinc -cp <powder-java-classes> ../../bindings/kotlin/src/dev/powder/Powder.kt RealworldQueries.kt -d out
//   java -cp "out;<powder-java-classes>;<kotlin-stdlib.jar>" RealworldQueriesKt <powder_java.dll> [schema.json] [url]

import dev.powder.Database
import dev.powder.Order
import dev.powder.and
import dev.powder.eq
import java.io.File

var checks = 0
var failures = 0

fun check(cond: Boolean, what: String) {
    checks++
    if (cond) {
        println("ok: $what")
    } else {
        failures++
        System.err.println("FAILED: $what")
    }
}

fun main(args: Array<String>) {
    require(args.isNotEmpty()) { "usage: RealworldQueriesKt <path-to-powder_java-lib> [schema.json] [url]" }
    val libPath = args[0]
    val schemaPath = if (args.size > 1) args[1] else "../powder.schema.json"
    val url = when {
        args.size > 2 -> args[2]
        else -> System.getenv("POWDER_URL") ?: "sqlite::memory:"
    }
    val schemaJson = File(schemaPath).readText(Charsets.UTF_8)

    Database.connect(url, libPath).use { db ->
        // ---- reset + DDL (portable SQL, children first) -------------------
        db.execute("DROP TABLE IF EXISTS order_items")
        db.execute("DROP TABLE IF EXISTS orders")
        db.execute("DROP TABLE IF EXISTS products")
        db.execute("DROP TABLE IF EXISTS customers")
        db.execute("CREATE TABLE customers (id BIGINT PRIMARY KEY, name TEXT, email TEXT, tier TEXT, active BOOLEAN)")
        db.execute("CREATE TABLE products (id BIGINT PRIMARY KEY, name TEXT, price DOUBLE PRECISION, stock BIGINT)")
        db.execute(
            "CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT REFERENCES customers(id), " +
                "status TEXT, amount DOUBLE PRECISION, note TEXT)"
        )
        db.execute(
            "CREATE TABLE order_items (id BIGINT PRIMARY KEY, order_id BIGINT REFERENCES orders(id), " +
                "product_id BIGINT REFERENCES products(id), qty BIGINT, unit_price DOUBLE PRECISION)"
        )

        db.orm(schemaJson).use { orm ->
            val customers = orm.table("customers")
            val products = orm.table("products")
            val orders = orm.table("orders")
            val items = orm.table("order_items")

            // ---- seed via ORM create / createMany -------------------------
            customers.create(mapOf("id" to 1, "name" to "김민준", "email" to "minjun@corp.kr", "tier" to "vip", "active" to true))
            customers.createMany(listOf(
                mapOf("id" to 2, "name" to "이서연", "email" to "seoyeon@corp.kr", "tier" to "gold", "active" to true),
                mapOf("id" to 3, "name" to "박도윤", "email" to "doyun@corp.kr", "tier" to "basic", "active" to true),
                mapOf("id" to 4, "name" to "최지우", "email" to "jiwoo@old.kr", "tier" to "basic", "active" to false),
                mapOf("id" to 5, "name" to "정하은", "email" to "haeun@corp.kr", "tier" to "vip", "active" to true),
            ))
            check(customers.count() == 5L, "seed: 5 customers (create + createMany)")

            products.createMany(listOf(
                mapOf("id" to 10, "name" to "노트북", "price" to 1500000, "stock" to 12),
                mapOf("id" to 11, "name" to "모니터", "price" to 350000, "stock" to 40),
                mapOf("id" to 12, "name" to "키보드", "price" to 89000, "stock" to 0),
                mapOf("id" to 13, "name" to "마우스", "price" to 45000, "stock" to 200),
            ))
            orders.createMany(listOf(
                mapOf("id" to 100, "customer_id" to 1, "status" to "paid", "amount" to 1850000, "note" to "빠른배송 요청"),
                mapOf("id" to 101, "customer_id" to 1, "status" to "paid", "amount" to 89000, "note" to null),
                mapOf("id" to 102, "customer_id" to 2, "status" to "shipped", "amount" to 350000, "note" to null),
                mapOf("id" to 103, "customer_id" to 3, "status" to "pending", "amount" to 45000, "note" to null),
                mapOf("id" to 104, "customer_id" to 5, "status" to "paid", "amount" to 700000, "note" to "법인 세금계산서"),
                mapOf("id" to 105, "customer_id" to 2, "status" to "cancelled", "amount" to 89000, "note" to "고객 변심"),
                mapOf("id" to 106, "customer_id" to 5, "status" to "paid", "amount" to 1500000, "note" to null),
            ))
            items.createMany(listOf(
                mapOf("id" to 1000, "order_id" to 100, "product_id" to 10, "qty" to 1, "unit_price" to 1500000),
                mapOf("id" to 1001, "order_id" to 100, "product_id" to 11, "qty" to 1, "unit_price" to 350000),
                mapOf("id" to 1002, "order_id" to 101, "product_id" to 12, "qty" to 1, "unit_price" to 89000),
                mapOf("id" to 1003, "order_id" to 102, "product_id" to 11, "qty" to 1, "unit_price" to 350000),
                mapOf("id" to 1004, "order_id" to 103, "product_id" to 13, "qty" to 1, "unit_price" to 45000),
                mapOf("id" to 1005, "order_id" to 104, "product_id" to 11, "qty" to 2, "unit_price" to 350000),
                mapOf("id" to 1006, "order_id" to 106, "product_id" to 10, "qty" to 1, "unit_price" to 1500000),
            ))

            // ================================================================
            // 1. 대시보드: 상태별 매출 요약 (raw SQL GROUP BY)
            // ================================================================
            run {
                val b = db.query(
                    "SELECT status, COUNT(*) AS cnt, SUM(amount) AS revenue " +
                        "FROM orders GROUP BY status ORDER BY revenue DESC"
                )
                check(b.numRows() == 4, "dashboard: 4 status groups")
                check(b.column("status").getString(0) == "paid", "dashboard: top revenue status is 'paid'")
                check(b.column("cnt").getLong(0) == 4L, "dashboard: 4 paid orders")
                check(b.column("revenue").getDouble(0) == 4139000.0, "dashboard: paid revenue = 4,139,000")
            }

            // ================================================================
            // 2. 고객별 매출 리포트 (raw SQL JOIN + GROUP BY + HAVING)
            // ================================================================
            run {
                val b = db.query(
                    "SELECT c.name, c.tier, COUNT(o.id) AS orders_cnt, SUM(o.amount) AS total " +
                        "FROM customers c JOIN orders o ON o.customer_id = c.id " +
                        "WHERE o.status != 'cancelled' " +
                        "GROUP BY c.id HAVING SUM(o.amount) >= ? " +
                        "ORDER BY total DESC",
                    100000.0,
                )
                check(b.numRows() == 3, "report: 3 customers over 100k (non-cancelled)")
                check(
                    b.column("name").getString(0) == "정하은" && b.column("total").getDouble(0) == 2200000.0,
                    "report: top customer 정하은 = 2,200,000",
                )
                check(
                    b.column("name").getString(1) == "김민준" && b.column("orders_cnt").getLong(1) == 2L,
                    "report: 김민준 has 2 orders",
                )
            }

            // ================================================================
            // 3. 서브쿼리: 한 번도 주문 안 한 고객 (raw SQL NOT IN)
            // ================================================================
            run {
                val b = db.query(
                    "SELECT name FROM customers WHERE id NOT IN (SELECT DISTINCT customer_id FROM orders)"
                )
                check(b.numRows() == 1 && b.column("name").getString(0) == "최지우", "subquery: only 최지우 never ordered")
            }

            // ================================================================
            // 4. 재고 없는 상품 중 주문된 것 (raw JOIN + DSL 확인)
            // ================================================================
            run {
                val b = db.query(
                    "SELECT DISTINCT p.name FROM products p " +
                        "JOIN order_items i ON i.product_id = p.id WHERE p.stock = 0"
                )
                check(b.numRows() == 1 && b.column("name").getString(0) == "키보드", "stockout: 키보드 ordered but out of stock")

                // 같은 사실을 DSL로 교차 확인 (stock = 0 상품 + 그 상품의 주문 항목)
                val out = db.from("products").where { "stock" eq 0 }.all()
                check(out.size == 1 && out[0]["name"] == "키보드", "stockout(DSL): only 키보드 has stock = 0")
                val ordered = db.from("order_items").where { "product_id" eq (out[0]["id"] as Long) }.count()
                check(ordered == 1L, "stockout(DSL): 키보드 appears in 1 order item")
            }

            // ================================================================
            // 5. ORM finder: 중첩 AND/OR/NOT + like/in
            // ================================================================
            run {
                val rows = customers.findMany(
                    where = mapOf(
                        "active" to true,
                        "OR" to listOf(
                            mapOf("tier" to "vip"),
                            mapOf("email" to mapOf("like" to "%@corp.kr")),
                        ),
                        "NOT" to mapOf("name" to mapOf("like" to "최%")),
                    ),
                    orderBy = mapOf("id" to "asc"),
                )
                val names = rows.map { it["name"] }
                check(
                    names == listOf<Any?>("김민준", "이서연", "박도윤", "정하은"),
                    "orm-finder: nested OR/NOT/like matches 4 active corp customers",
                )

                val vips = customers.findMany(
                    where = mapOf("tier" to mapOf("in" to listOf("vip", "gold"))),
                    orderBy = mapOf("id" to "asc"),
                ).map { it["name"] }
                check(
                    vips == listOf<Any?>("김민준", "이서연", "정하은"),
                    "orm-finder: tier in [vip, gold] -> 3 customers",
                )
            }

            // ================================================================
            // 6. 페이지네이션 (DSL orderBy + limit + offset + count)
            // ================================================================
            run {
                val byAmount = db.from("orders").orderBy("amount", Order.DESC).limit(3)
                val page1 = byAmount.offset(0).all().map { it["id"] }
                val page2 = byAmount.offset(3).all().map { it["id"] }
                check(page1.contains(100L) && page1.contains(106L), "paginate: page1 has two biggest orders")
                check(!page2.contains(100L) && page2.contains(102L), "paginate: page2 disjoint from page1")
                check(db.from("orders").count() == 7L, "paginate: total = 7 (totalPages = ceil(7/3) = 3)")
            }

            // ================================================================
            // 7. ORM 관계: include (배치 로드) + join (belongsTo LEFT JOIN)
            // ================================================================
            run {
                val withCustomer = orders.findMany(
                    where = mapOf("status" to "paid"),
                    include = mapOf("customer" to true),
                    orderBy = mapOf("id" to "asc"),
                )
                val hydrated = withCustomer.map { (it["customer"] as? Map<*, *>)?.get("name") }
                check(
                    withCustomer.size == 4 && hydrated.contains("김민준") && hydrated.contains("정하은"),
                    "orm-include: paid orders hydrated with customer objects",
                )

                val joined = orders.findMany(
                    where = mapOf("amount" to mapOf("gte" to 1000000)),
                    join = mapOf("customer" to true),
                    orderBy = mapOf("id" to "asc"),
                )
                val joinedNames = joined.map { (it["customer"] as? Map<*, *>)?.get("name") }
                check(
                    joinedNames.contains("김민준") && joinedNames.contains("정하은"),
                    "orm-join: belongsTo LEFT JOIN hydrates big orders",
                )

                val deep = items.findMany(
                    where = mapOf("qty" to mapOf("gte" to 2)),
                    include = mapOf("order" to mapOf("include" to mapOf("customer" to true))),
                )
                val deepCustomer = ((deep.firstOrNull()?.get("order") as? Map<*, *>)?.get("customer") as? Map<*, *>)?.get("name")
                check(deep.size == 1 && deepCustomer == "정하은", "orm-include: nested include order->customer")
            }

            // ================================================================
            // 8. ORM groupBy + having + 별칭
            // ================================================================
            run {
                val g = orders.groupBy(
                    by = listOf("customer_id"),
                    where = mapOf("status" to mapOf("ne" to "cancelled")),
                    count = true,
                    sum = listOf("amount"),
                    having = mapOf("_sum_amount" to mapOf("gt" to 100000)),
                    orderBy = mapOf("_sum_amount" to "desc"),
                )
                check(
                    g.isNotEmpty() && g[0].containsKey("_count") && g[0].containsKey("_sum_amount"),
                    "orm-groupby: alias columns _count/_sum_amount present",
                )
                val ids = g.map { it["customer_id"] }
                check(
                    ids.toSet() == setOf<Any?>(1L, 2L, 5L),
                    "orm-groupby: having _sum_amount>100000 keeps customers 1/2/5 only",
                )
            }

            // ================================================================
            // 9. ORM aggregate: sum / max / 빈 집합 -> null
            // ================================================================
            run {
                check(orders.aggregate("sum", "amount", mapOf("status" to "paid")) == 4139000.0, "orm-aggregate: sum(paid amount) = 4,139,000")
                check(products.aggregate("max", "price") == 1500000.0, "orm-aggregate: max product price = 1,500,000")
                check(orders.aggregate("avg", "amount", mapOf("status" to "refunded")) == null, "orm-aggregate: empty set -> null")
            }

            // ================================================================
            // 10. 트랜잭션: 주문 생성 + 재고 차감, 중첩 세이브포인트, 전체 롤백
            // ================================================================
            run {
                db.transaction { tx ->
                    tx.execute("INSERT INTO orders VALUES (?, ?, ?, ?, ?)", 107L, 3L, "paid", 90000.0, null)
                    tx.execute("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)", 1007L, 107L, 13L, 2L, 45000.0)
                    tx.execute("UPDATE products SET stock = stock - ? WHERE id = ?", 2L, 13L)

                    // 안쪽: 잘못된 항목 추가 시도 -> 세이브포인트 롤백
                    try {
                        tx.transaction { inner ->
                            inner.execute("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)", 1008L, 107L, 12L, 1L, 89000.0)
                            throw RuntimeException("재고 없음 — 취소")
                        }
                    } catch (e: RuntimeException) { /* expected */ }
                }
                val itemCount = db.from("order_items").where { "order_id" eq 107 }.count()
                check(itemCount == 1L, "tx: outer committed, inner savepoint rolled back")
                val stock = db.from("products").find("id" to 13)!!["stock"]
                check(stock == 198L, "tx: stock decremented inside transaction (200 -> 198)")

                // 전체 롤백: 예외 시 아무것도 남지 않음
                try {
                    db.transaction { tx ->
                        tx.execute("INSERT INTO orders VALUES (999, 1, 'paid', 1.0, NULL)")
                        throw RuntimeException("결제 실패")
                    }
                } catch (e: RuntimeException) { /* expected */ }
                check(db.from("orders").where { "id" eq 999 }.count() == 0L, "tx: full rollback on payment failure")
            }

            // ================================================================
            // 11. ORM update / delete / exists — 운영 업무 (+ FK 강제)
            // ================================================================
            run {
                check(
                    orders.update(mapOf("status" to "pending"), mapOf("status" to "cancelled")) == 1L,
                    "orm-update: 1 pending order cancelled",
                )
                check(customers.exists(mapOf("active" to false)), "orm-exists: inactive customer exists")

                // FK 강제: 자식(order_items)이 남은 주문 103 삭제는 거부돼야 함
                var fkEnforced = false
                try {
                    orders.delete(mapOf("id" to 103))
                } catch (e: RuntimeException) {
                    fkEnforced = (e.message ?: "").contains("FOREIGN KEY", ignoreCase = true)
                }
                check(fkEnforced, "orm-delete: FK violation rejected (child items exist)")

                // 실무 순서: 항목 먼저 정리 -> 주문 삭제
                val childRemoved = items.delete(mapOf("order_id" to 103))
                val removed = orders.delete(mapOf("status" to "cancelled", "amount" to mapOf("lt" to 50000)))
                check(childRemoved == 1L && removed == 1L, "orm-delete: cascade order (items first, then order) succeeds")
            }

            // ================================================================
            // 12. NULL 처리: nullable note 컬럼 (isValid + DSL IS NULL)
            // ================================================================
            run {
                val b = db.query("SELECT id, note FROM orders WHERE id IN (100, 101) ORDER BY id")
                check(
                    b.column("note").isValid(0) && b.column("note").getString(0) == "빠른배송 요청",
                    "null: note present on order 100",
                )
                check(!b.column("note").isValid(1), "null: note NULL on order 101")
                check(
                    db.from("orders").where { ("id" eq 101) and ("note" eq null) }.count() == 1L,
                    "null(DSL): eq null renders IS NULL",
                )
            }

            // ================================================================
            // 13. 명명 쿼리 스타일: 파라미터화 LTV 쿼리
            // ================================================================
            run {
                val b = db.query(
                    "SELECT c.id, c.name, SUM(o.amount) AS ltv FROM customers c " +
                        "JOIN orders o ON o.customer_id = c.id " +
                        "WHERE c.active = ? AND o.status = ? " +
                        "GROUP BY c.id HAVING SUM(o.amount) >= ? ORDER BY ltv DESC",
                    true, "paid", 500000.0,
                )
                check(b.numRows() == 2, "named-style: 2 active customers with paid LTV >= 500k")
                check(b.column("name").getString(0) == "정하은", "named-style: highest LTV first")
            }

            // ================================================================
            // 14. 한글/이모지 등 non-ASCII 왕복
            // ================================================================
            run {
                db.execute(
                    "INSERT INTO customers VALUES (?, ?, ?, ?, ?)",
                    6L, "한글🚀고객", "emoji@corp.kr", "basic", true,
                )
                check(
                    db.from("customers").find("id" to 6)!!["name"] == "한글🚀고객",
                    "utf8: Korean + emoji round-trip (DSL find)",
                )
                val b = db.query("SELECT name FROM customers WHERE id = ?", 6L)
                check(b.column("name").getString(0) == "한글🚀고객", "utf8: Korean + emoji round-trip (raw query)")
            }
        }
    }

    println()
    println("${if (failures == 0) "REAL-WORLD QUERIES OK" else "SOME QUERIES FAILED"} — $checks checks, $failures failed")
    kotlin.system.exitProcess(if (failures == 0) 0 else 1)
}
