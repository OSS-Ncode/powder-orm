// Real-world business-query verification of the C# Powder binding, mirroring
// verification/cpp/realworld_queries.cpp: an e-commerce dataset exercised via
// raw SQL (Client.Query/Execute + Batch) and the ORM (db.Orm(schemaJson)).
//
//   POWDER_LIB=<path>\powder_ffi.dll dotnet run [connection-url]

using System.Text.Json.Nodes;
using Powder;

int checks = 0, failures = 0;

void Check(bool cond, string what)
{
    checks++;
    if (cond)
    {
        Console.WriteLine($"ok: {what}");
    }
    else
    {
        failures++;
        Console.Error.WriteLine($"FAILED: {what}");
    }
}

static bool HasId(JsonArray rows, long id) =>
    rows.Any(r => (long?)r!["id"] == id);

string url = args.Length > 0 ? args[0]
    : Environment.GetEnvironmentVariable("POWDER_URL") ?? "sqlite::memory:";

// Locate verification/powder.schema.json by walking up from the binary.
string? schemaPath = null;
for (var dir = new DirectoryInfo(AppContext.BaseDirectory); dir != null; dir = dir.Parent)
{
    string candidate = Path.Combine(dir.FullName, "powder.schema.json");
    if (File.Exists(candidate)) { schemaPath = candidate; break; }
}
if (schemaPath == null)
{
    Console.Error.WriteLine("powder.schema.json not found above " + AppContext.BaseDirectory);
    return 2;
}
string schemaJson = File.ReadAllText(schemaPath);

try
{
    using var db = Client.Connect(url);

    // ---- DDL (portable SQL; drop children first on server backends) -------
    foreach (var t in new[] { "order_items", "orders", "products", "customers" })
        db.Execute($"DROP TABLE IF EXISTS {t}");

    db.Execute("CREATE TABLE customers (id BIGINT PRIMARY KEY, name TEXT, email TEXT, tier TEXT, active BOOLEAN)");
    db.Execute("CREATE TABLE products (id BIGINT PRIMARY KEY, name TEXT, price DOUBLE PRECISION, stock BIGINT)");
    db.Execute("CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT REFERENCES customers(id), status TEXT, amount DOUBLE PRECISION, note TEXT)");
    db.Execute("CREATE TABLE order_items (id BIGINT PRIMARY KEY, order_id BIGINT REFERENCES orders(id), product_id BIGINT REFERENCES products(id), qty BIGINT, unit_price DOUBLE PRECISION)");

    using var orm = db.Orm(schemaJson);
    var customers = orm.Table("customers");
    var products = orm.Table("products");
    var orders = orm.Table("orders");
    var items = orm.Table("order_items");

    // ---- Seed via ORM Create / CreateMany ----------------------------------
    customers.Create(new { id = 1, name = "김민준", email = "minjun@corp.kr", tier = "vip", active = true });
    customers.CreateMany(new object[]
    {
        new { id = 2, name = "이서연", email = "seoyeon@corp.kr", tier = "gold", active = true },
        new { id = 3, name = "박도윤", email = "doyun@corp.kr", tier = "basic", active = true },
        new { id = 4, name = "최지우", email = "jiwoo@old.kr", tier = "basic", active = false },
        new { id = 5, name = "정하은", email = "haeun@corp.kr", tier = "vip", active = true },
    });
    Check(customers.Count() == 5, "seed: 5 customers (Create + CreateMany)");

    products.CreateMany(new object[]
    {
        new { id = 10, name = "노트북", price = 1500000.0, stock = 12 },
        new { id = 11, name = "모니터", price = 350000.0, stock = 40 },
        new { id = 12, name = "키보드", price = 89000.0, stock = 0 },
        new { id = 13, name = "마우스", price = 45000.0, stock = 200 },
    });

    orders.CreateMany(new object[]
    {
        new { id = 100, customer_id = 1, status = "paid", amount = 1850000.0, note = (string?)"빠른배송 요청" },
        new { id = 101, customer_id = 1, status = "paid", amount = 89000.0, note = (string?)null },
        new { id = 102, customer_id = 2, status = "shipped", amount = 350000.0, note = (string?)null },
        new { id = 103, customer_id = 3, status = "pending", amount = 45000.0, note = (string?)null },
        new { id = 104, customer_id = 5, status = "paid", amount = 700000.0, note = (string?)"법인 세금계산서" },
        new { id = 105, customer_id = 2, status = "cancelled", amount = 89000.0, note = (string?)"고객 변심" },
        new { id = 106, customer_id = 5, status = "paid", amount = 1500000.0, note = (string?)null },
    });

    items.CreateMany(new object[]
    {
        new { id = 1000, order_id = 100, product_id = 10, qty = 1, unit_price = 1500000.0 },
        new { id = 1001, order_id = 100, product_id = 11, qty = 1, unit_price = 350000.0 },
        new { id = 1002, order_id = 101, product_id = 12, qty = 1, unit_price = 89000.0 },
        new { id = 1003, order_id = 102, product_id = 11, qty = 1, unit_price = 350000.0 },
        new { id = 1004, order_id = 103, product_id = 13, qty = 1, unit_price = 45000.0 },
        new { id = 1005, order_id = 104, product_id = 11, qty = 2, unit_price = 350000.0 },
        new { id = 1006, order_id = 106, product_id = 10, qty = 1, unit_price = 1500000.0 },
    });

    // ====================================================================
    // 1. 대시보드: 상태별 매출 요약 (raw SQL GROUP BY)
    // ====================================================================
    {
        var b = db.Query(
            "SELECT status, COUNT(*) AS cnt, SUM(amount) AS revenue " +
            "FROM orders GROUP BY status ORDER BY revenue DESC");
        Check(b.NumRows == 4, "dashboard: 4 status groups");
        Check(b["status"].GetString(0) == "paid", "dashboard: top revenue status is 'paid'");
        Check(b["cnt"].GetInt64(0) == 4, "dashboard: 4 paid orders");
        Check(b["revenue"].GetDouble(0) == 4139000.0, "dashboard: paid revenue = 4,139,000");
    }

    // ====================================================================
    // 2. 고객별 매출 리포트 (raw SQL JOIN + GROUP BY + HAVING)
    // ====================================================================
    {
        var b = db.Query(
            "SELECT c.name, c.tier, COUNT(o.id) AS orders_cnt, SUM(o.amount) AS total " +
            "FROM customers c JOIN orders o ON o.customer_id = c.id " +
            "WHERE o.status != 'cancelled' " +
            "GROUP BY c.id HAVING SUM(o.amount) >= ? " +
            "ORDER BY total DESC", 100000.0);
        Check(b.NumRows == 3, "report: 3 customers over 100k (non-cancelled)");
        Check(b["name"].GetString(0) == "정하은" && b["total"].GetDouble(0) == 2200000.0,
              "report: top customer 정하은 = 2,200,000");
        Check(b["name"].GetString(1) == "김민준" && b["orders_cnt"].GetInt64(1) == 2,
              "report: 김민준 has 2 orders");
    }

    // ====================================================================
    // 3. 서브쿼리: 한 번도 주문 안 한 고객 (raw SQL NOT IN)
    // ====================================================================
    {
        var b = db.Query(
            "SELECT name FROM customers WHERE id NOT IN (SELECT DISTINCT customer_id FROM orders)");
        Check(b.NumRows == 1 && b["name"].GetString(0) == "최지우",
              "subquery: only 최지우 never ordered");
    }

    // ====================================================================
    // 4. 재고 없는 상품 중 주문된 것 (raw SQL JOIN + WHERE)
    // ====================================================================
    {
        var b = db.Query(
            "SELECT DISTINCT p.name FROM products p " +
            "JOIN order_items i ON i.product_id = p.id WHERE p.stock = 0");
        Check(b.NumRows == 1 && b["name"].GetString(0) == "키보드",
              "stockout: 키보드 ordered but out of stock");
    }

    // ====================================================================
    // 5. ORM finder: 중첩 AND/OR/NOT + in/like
    // ====================================================================
    {
        var rows = customers.FindMany(new
        {
            where = new
            {
                active = true,
                OR = new object[]
                {
                    new { tier = "vip" },
                    new { email = new { like = "%@corp.kr" } },
                },
                NOT = new { name = new { like = "최%" } },
            },
            orderBy = new { id = "asc" },
        });
        var names = rows.Select(r => (string?)r!["name"]).ToList();
        Check(rows.Count == 4 &&
              names.Contains("김민준") && names.Contains("이서연") &&
              names.Contains("박도윤") && names.Contains("정하은") &&
              !names.Contains("최지우"),
              "orm-finder: nested OR/NOT/like matches 4 active corp customers");

        var vips = customers.FindMany(new
        {
            where = new { tier = new { @in = new[] { "vip", "gold" } } },
            orderBy = new { id = "asc" },
        });
        var vipNames = vips.Select(r => (string?)r!["name"]).ToList();
        Check(vips.Count == 3 &&
              vipNames.Contains("김민준") && vipNames.Contains("이서연") &&
              vipNames.Contains("정하은") && !vipNames.Contains("박도윤"),
              "orm-finder: tier in [vip, gold] -> 3 customers");
    }

    // ====================================================================
    // 6. ORM 페이지네이션: limit + offset + count
    // ====================================================================
    {
        var page1 = orders.FindMany(new { orderBy = new { amount = "desc" }, limit = 3, offset = 0 });
        var page2 = orders.FindMany(new { orderBy = new { amount = "desc" }, limit = 3, offset = 3 });
        long total = orders.Count();
        Check(page1.Count == 3 && HasId(page1, 100) && HasId(page1, 106),
              "orm-paginate: page1 has two biggest orders");
        Check(!HasId(page2, 100) && HasId(page2, 102),
              "orm-paginate: page2 disjoint from page1");
        Check(total == 7, "orm-paginate: total = 7 (totalPages = ceil(7/3) = 3)");
    }

    // ====================================================================
    // 7. ORM 관계: include (배치 로드) + join (belongsTo LEFT JOIN) + 중첩 include
    // ====================================================================
    {
        var withCustomer = orders.FindMany(new
        {
            where = new { status = "paid" },
            include = new { customer = true },
            orderBy = new { id = "asc" },
        });
        var custNames = withCustomer.Select(r => (string?)r!["customer"]?["name"]).ToList();
        Check(withCustomer.Count == 4 && withCustomer.All(r => r!["customer"] is JsonObject) &&
              custNames.Contains("김민준") && custNames.Contains("정하은"),
              "orm-include: paid orders hydrated with customer objects");

        var joined = orders.FindMany(new
        {
            where = new { amount = new { gte = 1000000 } },
            join = new { customer = true },
            orderBy = new { id = "asc" },
        });
        var joinedNames = joined.Select(r => (string?)r!["customer"]?["name"]).ToList();
        Check(joined.Count == 2 && joinedNames.Contains("김민준") && joinedNames.Contains("정하은"),
              "orm-join: belongsTo LEFT JOIN hydrates big orders");

        var deep = items.FindMany(new
        {
            where = new { qty = new { gte = 2 } },
            include = new { order = new { include = new { customer = true } } },
        });
        Check(deep.Count == 1 &&
              (string?)deep[0]!["order"]?["customer"]?["name"] == "정하은",
              "orm-include: nested include order->customer");
    }

    // ====================================================================
    // 8. ORM groupBy + having + 별칭
    // ====================================================================
    {
        var g = orders.GroupBy(new
        {
            by = new[] { "customer_id" },
            where = new { status = new { ne = "cancelled" } },
            count = true,
            sum = new[] { "amount" },
            having = new { _sum_amount = new { gt = 100000 } },
            orderBy = new { _sum_amount = "desc" },
        });
        Check(g.Count > 0 && g.All(r => ((JsonObject)r!).ContainsKey("_count") &&
                                        ((JsonObject)r!).ContainsKey("_sum_amount")),
              "orm-groupby: alias columns _count/_sum_amount present");
        var groupIds = g.Select(r => (long?)r!["customer_id"]).ToList();
        Check(g.Count == 3 &&
              groupIds.Contains(1) && groupIds.Contains(2) && groupIds.Contains(5) &&
              !groupIds.Contains(3),
              "orm-groupby: having _sum_amount>100000 keeps 3 groups");
    }

    // ====================================================================
    // 9. ORM aggregate: sum/max (+ 빈 집합은 null)
    // ====================================================================
    {
        Check(orders.Aggregate("sum", "amount", new { status = "paid" }) == 4139000.0,
              "orm-aggregate: sum(paid amount)");
        Check(products.Aggregate("max", "price") == 1500000.0,
              "orm-aggregate: max product price");
        Check(orders.Aggregate("avg", "amount", new { status = "refunded" }) == null,
              "orm-aggregate: empty set -> null");
    }

    // ====================================================================
    // 10. 업무 흐름: 주문 생성 트랜잭션 + 중첩 세이브포인트 + 전체 롤백
    // ====================================================================
    {
        db.Transaction(tx =>
        {
            tx.Execute("INSERT INTO orders VALUES (?, ?, ?, ?, ?)",
                       107L, 3L, "paid", 90000.0, null);
            tx.Execute("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)",
                       1007L, 107L, 13L, 2L, 45000.0);
            tx.Execute("UPDATE products SET stock = stock - ? WHERE id = ?", 2L, 13L);

            // 안쪽: 잘못된 항목 추가 시도 -> 세이브포인트 롤백
            try
            {
                tx.Transaction(inner =>
                {
                    inner.Execute("INSERT INTO order_items VALUES (?, ?, ?, ?, ?)",
                                  1008L, 107L, 12L, 1L, 89000.0);
                    throw new InvalidOperationException("재고 없음 — 취소");
                });
            }
            catch (InvalidOperationException) { }
        });

        var b = db.Query("SELECT COUNT(*) AS c FROM order_items WHERE order_id = 107");
        Check(b["c"].GetInt64(0) == 1, "tx: outer committed, inner savepoint rolled back");
        var st = db.Query("SELECT stock FROM products WHERE id = 13");
        Check(st["stock"].GetInt64(0) == 198, "tx: stock decremented inside transaction");

        // 전체 롤백: 예외 시 아무것도 남지 않음
        try
        {
            db.Transaction(tx =>
            {
                tx.Execute("INSERT INTO orders VALUES (999, 1, 'paid', 1.0, NULL)");
                throw new InvalidOperationException("결제 실패");
            });
        }
        catch (InvalidOperationException) { }
        var rb = db.Query("SELECT COUNT(*) AS c FROM orders WHERE id = 999");
        Check(rb["c"].GetInt64(0) == 0, "tx: full rollback on payment failure");
    }

    // ====================================================================
    // 11. ORM update / delete / exists — 운영 업무 (FK 강제 확인)
    // ====================================================================
    {
        long n = orders.Update(new { status = "pending" }, new { status = "cancelled" });
        Check(n == 1, "orm-update: 1 pending order cancelled");

        Check(customers.Exists(new { active = false }), "orm-exists: inactive customer exists");

        // 자식(order_items)이 남아 있는 주문 103 삭제는 FK 위반으로 거부돼야 함
        bool fkEnforced = false;
        try
        {
            orders.Delete(new { id = 103 });
        }
        catch (PowderException e)
        {
            fkEnforced = e.Message.Contains("FOREIGN KEY", StringComparison.OrdinalIgnoreCase);
        }
        Check(fkEnforced, "orm-delete: FK violation rejected (child items exist)");

        // 실무 순서: 항목 먼저 정리 -> 주문 삭제
        long childRemoved = items.Delete(new { order_id = 103 });
        long removed = orders.Delete(new { status = "cancelled", amount = new { lt = 50000 } });
        Check(childRemoved == 1 && removed == 1,
              "orm-delete: cascade order (items first, then order) succeeds");
    }

    // ====================================================================
    // 12. NULL 처리: nullable note 컬럼 (IsValid / boxed null)
    // ====================================================================
    {
        var b = db.Query("SELECT id, note FROM orders WHERE id IN (100, 101) ORDER BY id");
        Check(b["note"].IsValid(0) && b["note"].GetString(0) == "빠른배송 요청",
              "null: note present on order 100");
        Check(!b["note"].IsValid(1) && b["note"].Get(1) == null,
              "null: note NULL on order 101");
    }

    // ====================================================================
    // 13. 명명 쿼리 스타일: 파라미터화 LTV 쿼리
    // ====================================================================
    {
        var b = db.Query(
            "SELECT c.id, c.name, SUM(o.amount) AS ltv FROM customers c " +
            "JOIN orders o ON o.customer_id = c.id " +
            "WHERE c.active = ? AND o.status = ? " +
            "GROUP BY c.id HAVING SUM(o.amount) >= ? ORDER BY ltv DESC",
            true, "paid", 500000.0);
        Check(b.NumRows == 2, "named-style: 2 active customers with paid LTV >= 500k");
        Check(b["name"].GetString(0) == "정하은", "named-style: highest LTV first");
    }

    // ====================================================================
    // 14. 한글/이모지 등 non-ASCII 왕복
    // ====================================================================
    {
        db.Execute("INSERT INTO customers VALUES (?, ?, ?, ?, ?)",
                   6L, "한글🚀고객", "emoji@corp.kr", "basic", true);
        var b = db.Query("SELECT name FROM customers WHERE id = 6");
        Check(b["name"].GetString(0) == "한글🚀고객", "utf8: Korean + emoji round-trip");
    }
}
catch (Exception e)
{
    Console.Error.WriteLine($"UNCAUGHT: {e}");
    return 2;
}

Console.WriteLine();
Console.WriteLine($"{(failures == 0 ? "REAL-WORLD QUERIES OK" : "SOME QUERIES FAILED")} — {checks} checks, {failures} failed");
return failures == 0 ? 0 : 1;
