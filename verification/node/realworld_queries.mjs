// Real-world business-query verification for the Powder Node bindings (plain JS).
// Mirrors verification/cpp/realworld_queries.cpp and SCENARIOS.md: 14 scenarios
// against an e-commerce dataset via raw SQL, the generated models (compiled to
// dist/powder_models.js by `npm run build`), relations, aggregations,
// named-query style, and nested transactions.

import { Client, PowderError, runNamedQuery } from "@powder/node";
import { powder } from "./dist/powder_models.js";

let checks = 0;
let failures = 0;

function check(cond, what) {
  checks++;
  if (cond) {
    console.log(`ok: ${what}`);
  } else {
    failures++;
    console.error(`FAILED: ${what}`);
  }
}

const num = (v) => (typeof v === "bigint" ? Number(v) : v);

async function main() {
  const url = process.argv[2] ?? process.env.POWDER_URL ?? "sqlite::memory:";
  const client = await Client.connect(url);

  // ---- Reset (reverse FK order) + portable DDL ---------------------------
  for (const t of ["order_items", "orders", "products", "customers"]) {
    await client.execute(`DROP TABLE IF EXISTS ${t}`);
  }
  await client.execute(
    "CREATE TABLE customers (id BIGINT PRIMARY KEY, name TEXT, email TEXT, tier TEXT, active BOOLEAN)",
  );
  await client.execute(
    "CREATE TABLE products (id BIGINT PRIMARY KEY, name TEXT, price DOUBLE PRECISION, stock BIGINT)",
  );
  await client.execute(
    "CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT REFERENCES customers(id), status TEXT, amount DOUBLE PRECISION, note TEXT)",
  );
  await client.execute(
    "CREATE TABLE order_items (id BIGINT PRIMARY KEY, order_id BIGINT REFERENCES orders(id), product_id BIGINT REFERENCES products(id), qty BIGINT, unit_price DOUBLE PRECISION)",
  );

  const db = powder(client);

  // ---- Seed via ORM create / createMany ----------------------------------
  await db.customers.create({ id: 1, name: "김민준", email: "minjun@corp.kr", tier: "vip", active: true });
  await db.customers.createMany([
    { id: 2, name: "이서연", email: "seoyeon@corp.kr", tier: "gold", active: true },
    { id: 3, name: "박도윤", email: "doyun@corp.kr", tier: "basic", active: true },
    { id: 4, name: "최지우", email: "jiwoo@old.kr", tier: "basic", active: false },
    { id: 5, name: "정하은", email: "haeun@corp.kr", tier: "vip", active: true },
  ]);
  check((await db.customers.count()) === 5, "seed: 5 customers (create + createMany)");

  await db.products.createMany([
    { id: 10, name: "노트북", price: 1500000, stock: 12 },
    { id: 11, name: "모니터", price: 350000, stock: 40 },
    { id: 12, name: "키보드", price: 89000, stock: 0 },
    { id: 13, name: "마우스", price: 45000, stock: 200 },
  ]);

  await db.orders.createMany([
    { id: 100, customer_id: 1, status: "paid", amount: 1850000, note: "빠른배송 요청" },
    { id: 101, customer_id: 1, status: "paid", amount: 89000, note: null },
    { id: 102, customer_id: 2, status: "shipped", amount: 350000, note: null },
    { id: 103, customer_id: 3, status: "pending", amount: 45000, note: null },
    { id: 104, customer_id: 5, status: "paid", amount: 700000, note: "법인 세금계산서" },
    { id: 105, customer_id: 2, status: "cancelled", amount: 89000, note: "고객 변심" },
    { id: 106, customer_id: 5, status: "paid", amount: 1500000, note: null },
  ]);

  await db.order_items.createMany([
    { id: 1000, order_id: 100, product_id: 10, qty: 1, unit_price: 1500000 },
    { id: 1001, order_id: 100, product_id: 11, qty: 1, unit_price: 350000 },
    { id: 1002, order_id: 101, product_id: 12, qty: 1, unit_price: 89000 },
    { id: 1003, order_id: 102, product_id: 11, qty: 1, unit_price: 350000 },
    { id: 1004, order_id: 103, product_id: 13, qty: 1, unit_price: 45000 },
    { id: 1005, order_id: 104, product_id: 11, qty: 2, unit_price: 350000 },
    { id: 1006, order_id: 106, product_id: 10, qty: 1, unit_price: 1500000 },
  ]);

  // ========================================================================
  // 1. 대시보드: 상태별 매출 요약 (raw SQL GROUP BY)
  // ========================================================================
  {
    const b = await client.query(
      "SELECT status, COUNT(*) AS cnt, SUM(amount) AS revenue FROM orders GROUP BY status ORDER BY revenue DESC",
    );
    check(b.numRows === 4, "dashboard: 4 status groups");
    check(b.column("status")?.get(0) === "paid", "dashboard: top revenue status is 'paid'");
    check(num(b.column("cnt")?.get(0)) === 4, "dashboard: 4 paid orders");
    check(num(b.column("revenue")?.get(0)) === 4139000, "dashboard: paid revenue = 4,139,000");
  }

  // ========================================================================
  // 2. 고객별 매출 리포트 (raw SQL JOIN + GROUP BY + HAVING)
  // ========================================================================
  {
    const b = await client.query(
      "SELECT c.name, c.tier, COUNT(o.id) AS orders_cnt, SUM(o.amount) AS total " +
        "FROM customers c JOIN orders o ON o.customer_id = c.id " +
        "WHERE o.status != 'cancelled' " +
        "GROUP BY c.id HAVING SUM(o.amount) >= ? ORDER BY total DESC",
      [100000],
    );
    check(b.numRows === 3, "report: 3 customers over 100k (non-cancelled)");
    check(
      b.column("name")?.get(0) === "정하은" && num(b.column("total")?.get(0)) === 2200000,
      "report: top customer 정하은 = 2,200,000",
    );
    check(
      b.column("name")?.get(1) === "김민준" && num(b.column("orders_cnt")?.get(1)) === 2,
      "report: 김민준 has 2 orders",
    );
  }

  // ========================================================================
  // 3. 서브쿼리: 한 번도 주문 안 한 고객 (raw SQL NOT IN)
  // ========================================================================
  {
    const b = await client.query(
      "SELECT name FROM customers WHERE id NOT IN (SELECT DISTINCT customer_id FROM orders)",
    );
    check(b.numRows === 1 && b.column("name")?.get(0) === "최지우", "subquery: only 최지우 never ordered");
  }

  // ========================================================================
  // 4. 재고 없는 상품 중 주문된 것 (raw SQL JOIN + WHERE)
  // ========================================================================
  {
    const b = await client.query(
      "SELECT DISTINCT p.name FROM products p JOIN order_items i ON i.product_id = p.id WHERE p.stock = 0",
    );
    check(b.numRows === 1 && b.column("name")?.get(0) === "키보드", "stockout: 키보드 ordered but out of stock");
  }

  // ========================================================================
  // 5. ORM finder: 중첩 AND/OR/NOT + in/like (문서의 Prisma 스타일)
  // ========================================================================
  {
    const rows = await db.customers.findMany({
      where: {
        active: true,
        OR: [{ tier: "vip" }, { email: { like: "%@corp.kr" } }],
        NOT: { name: { like: "최%" } },
      },
      orderBy: { id: "asc" },
    });
    const names = rows.map((r) => r.name);
    check(
      names.length === 4 &&
        ["김민준", "이서연", "박도윤", "정하은"].every((n) => names.includes(n)) &&
        !names.includes("최지우"),
      "orm-finder: nested OR/NOT/like matches 4 active corp customers",
    );

    const vips = await db.customers.findMany({
      where: { tier: { in: ["vip", "gold"] } },
      orderBy: { id: "asc" },
    });
    const vipNames = vips.map((r) => r.name);
    check(
      vipNames.length === 3 &&
        ["김민준", "이서연", "정하은"].every((n) => vipNames.includes(n)),
      "orm-finder: tier in [vip, gold] -> 3 customers",
    );
  }

  // ========================================================================
  // 6. ORM 페이지네이션: limit + offset + Finder.paginate
  // ========================================================================
  {
    const page1 = await db.orders.findMany({ orderBy: { amount: "desc" }, limit: 3, offset: 0 });
    const page2 = await db.orders.findMany({ orderBy: { amount: "desc" }, limit: 3, offset: 3 });
    const ids1 = page1.map((o) => o.id);
    const ids2 = page2.map((o) => o.id);
    check(ids1.includes(100) && ids1.includes(106), "orm-paginate: page1 has two biggest orders");
    check(!ids2.includes(100) && ids2.includes(102), "orm-paginate: page2 disjoint from page1");
    const paged = await db.orders.orderBy("amount", "desc").paginate(1, 3);
    check(
      paged.total === 7 && paged.totalPages === 3 && paged.rows.length === 3,
      "orm-paginate: paginate() total = 7, totalPages = 3",
    );
  }

  // ========================================================================
  // 7. ORM 관계: include (배치 로드) + join (belongsTo LEFT JOIN) + 중첩
  // ========================================================================
  {
    const withCustomer = await db.orders.findMany({
      where: { status: "paid" },
      include: { customer: true },
      orderBy: { id: "asc" },
    });
    const custNames = withCustomer.map((o) => o.customer?.name);
    check(
      withCustomer.length === 4 && custNames.includes("김민준") && custNames.includes("정하은"),
      "orm-include: paid orders hydrated with customer objects",
    );

    const joined = await db.orders.findMany({
      where: { amount: { gte: 1000000 } },
      join: { customer: true },
      orderBy: { id: "asc" },
    });
    const joinedNames = joined.map((o) => o.customer?.name);
    check(
      joined.length === 2 && joinedNames.includes("김민준") && joinedNames.includes("정하은"),
      "orm-join: belongsTo LEFT JOIN hydrates big orders",
    );

    const deep = await db.order_items.findMany({
      where: { qty: { gte: 2 } },
      include: { order: { include: { customer: true } } },
    });
    check(
      deep.length === 1 && deep[0].order?.customer?.name === "정하은",
      "orm-include: nested include order->customer",
    );
  }

  // ========================================================================
  // 8. ORM groupBy + having + 별칭 (aggregations.mdx 문서 그대로)
  // ========================================================================
  {
    const g = await db.orders.groupBy({
      by: ["customer_id"],
      where: { status: { ne: "cancelled" } },
      count: true,
      sum: ["amount"],
      having: { _sum_amount: { gt: 100000 } },
      orderBy: { _sum_amount: "desc" },
    });
    check(
      g.length > 0 && "_count" in g[0] && "_sum_amount" in g[0],
      "orm-groupby: alias columns _count/_sum_amount present",
    );
    const groupIds = g.map((r) => num(r.customer_id));
    check(
      g.length === 3 && [5, 1, 2].every((id) => groupIds.includes(id)) && !groupIds.includes(3),
      "orm-groupby: having _sum_amount>100000 keeps 3 groups",
    );
  }

  // ========================================================================
  // 9. ORM aggregate: sum/max (+ 빈 집합은 null)
  // ========================================================================
  {
    check(
      (await db.orders.aggregate("sum", "amount", { status: "paid" })) === 4139000,
      "orm-aggregate: sum(paid amount) = 4,139,000",
    );
    check((await db.products.aggregate("max", "price")) === 1500000, "orm-aggregate: max product price");
    check(
      (await db.orders.aggregate("avg", "amount", { status: "refunded" })) === null,
      "orm-aggregate: empty set -> null",
    );
  }

  // ========================================================================
  // 10. 트랜잭션: 주문 생성 + 재고 차감, 중첩 세이브포인트, 전체 롤백
  // ========================================================================
  {
    await db.$transaction(async (tx) => {
      await tx.orders.create({ id: 107, customer_id: 3, status: "paid", amount: 90000, note: null });
      await tx.order_items.create({ id: 1007, order_id: 107, product_id: 13, qty: 2, unit_price: 45000 });
      await client.execute("UPDATE products SET stock = stock - ? WHERE id = ?", [2, 13]);

      // 안쪽 세이브포인트: 잘못된 항목 추가 시도 -> 롤백돼야 함
      try {
        await tx.$transaction(async (inner) => {
          await inner.order_items.create({ id: 1008, order_id: 107, product_id: 12, qty: 1, unit_price: 89000 });
          throw new Error("재고 없음 — 취소");
        });
      } catch {
        // expected
      }
    });
    check(
      (await db.order_items.count({ order_id: 107 })) === 1,
      "tx: outer committed, inner savepoint rolled back",
    );
    const mouse = await db.products.find(13);
    check(mouse?.stock === 198, "tx: stock decremented inside transaction");

    // 전체 롤백: 예외 시 아무것도 남지 않음
    try {
      await db.$transaction(async (tx) => {
        await tx.orders.create({ id: 999, customer_id: 1, status: "paid", amount: 1, note: null });
        throw new Error("결제 실패");
      });
    } catch {
      // expected
    }
    check((await db.orders.count({ id: 999 })) === 0, "tx: full rollback on payment failure");
  }

  // ========================================================================
  // 11. ORM update / delete / exists — 운영 업무 (FK 강제 포함)
  // ========================================================================
  {
    check(
      (await db.orders.update({ where: { status: "pending" }, data: { status: "cancelled" } })) === 1,
      "orm-update: 1 pending order cancelled",
    );
    check(await db.customers.exists({ active: false }), "orm-exists: inactive customer exists");

    let fkEnforced = false;
    try {
      await db.orders.delete({ id: 103 });
    } catch (err) {
      fkEnforced = err instanceof PowderError && /FOREIGN KEY/i.test(err.message);
    }
    check(fkEnforced, "orm-delete: FK violation rejected (child items exist)");

    const childRemoved = await db.order_items.delete({ order_id: 103 });
    const removed = await db.orders.delete({ status: "cancelled", amount: { lt: 50000 } });
    check(childRemoved === 1 && removed === 1, "orm-delete: cascade order (items first, then order) succeeds");
  }

  // ========================================================================
  // 12. NULL 처리: nullable note 컬럼
  // ========================================================================
  {
    const b = await client.query("SELECT id, note FROM orders WHERE id IN (100, 101) ORDER BY id");
    check(b.column("note")?.get(0) === "빠른배송 요청", "null: note present on order 100");
    check(b.column("note")?.get(1) === null, "null: note NULL on order 101");
    const o101 = await db.orders.find(101);
    check(o101 !== null && o101.note === null, "null: ORM materializes NULL note as null");
  }

  // ========================================================================
  // 13. 명명 쿼리 스타일: 파라미터화 LTV 쿼리 (runNamedQuery)
  // ========================================================================
  {
    const rows = await runNamedQuery(
      client,
      "SELECT c.id, c.name, SUM(o.amount) AS ltv FROM customers c " +
        "JOIN orders o ON o.customer_id = c.id " +
        "WHERE c.active = ? AND o.status = ? " +
        "GROUP BY c.id HAVING SUM(o.amount) >= ? ORDER BY ltv DESC",
      ["active", "status", "min"],
      { active: true, status: "paid", min: 500000 },
    );
    check(rows.length === 2, "named-style: 2 active customers with paid LTV >= 500k");
    check(rows[0].name === "정하은", "named-style: highest LTV first");
  }

  // ========================================================================
  // 14. 한글/이모지 등 non-ASCII 왕복
  // ========================================================================
  {
    await db.customers.create({ id: 6, name: "한글🚀고객", email: "emoji@corp.kr", tier: "basic", active: true });
    const c = await db.customers.find(6);
    check(c?.name === "한글🚀고객", "utf8: Korean + emoji round-trip");
  }

  console.log(
    `\n${failures === 0 ? "REAL-WORLD QUERIES OK" : "SOME QUERIES FAILED"} — ${checks} checks, ${failures} failed`,
  );
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error("UNCAUGHT:", err);
  process.exit(2);
});
