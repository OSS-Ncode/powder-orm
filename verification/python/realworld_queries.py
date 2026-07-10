# Real-world business-query verification for the Powder Python binding.
# Mirrors verification/cpp/realworld_queries.cpp — an e-commerce dataset
# (customers / products / orders / order_items) exercised through raw SQL,
# ORM finders, relations, aggregations, named-style raw queries, and
# nested transactions (savepoints).
#
# Usage: python realworld_queries.py [connection-url]
#        (or POWDER_URL env var; default "sqlite::memory:")

from __future__ import annotations

import asyncio
import os
import sys

import powder
from powder.orm import PowderError

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from powder_models import powder as powder_models  # noqa: E402

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")

checks = 0
failures = 0


def check(cond: bool, what: str) -> None:
    global checks, failures
    checks += 1
    if cond:
        print(f"ok: {what}")
    else:
        failures += 1
        print(f"FAILED: {what}", file=sys.stderr)


DDL = [
    "CREATE TABLE customers (id BIGINT PRIMARY KEY, name TEXT, email TEXT, tier TEXT, active BOOLEAN)",
    "CREATE TABLE products (id BIGINT PRIMARY KEY, name TEXT, price DOUBLE PRECISION, stock BIGINT)",
    "CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT REFERENCES customers(id), status TEXT, amount DOUBLE PRECISION, note TEXT)",
    "CREATE TABLE order_items (id BIGINT PRIMARY KEY, order_id BIGINT REFERENCES orders(id), product_id BIGINT REFERENCES products(id), qty BIGINT, unit_price DOUBLE PRECISION)",
]

DROPS = [
    "DROP TABLE IF EXISTS order_items",
    "DROP TABLE IF EXISTS orders",
    "DROP TABLE IF EXISTS products",
    "DROP TABLE IF EXISTS customers",
]


async def main() -> int:
    url = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("POWDER_URL", "sqlite::memory:")
    client = await powder.connect(url)

    # ---- DDL (drop leftovers in child->parent order, then recreate) ------
    for sql in DROPS:
        await client.execute(sql)
    for sql in DDL:
        await client.execute(sql)

    db = powder_models(client)

    # ---- Seed via ORM create / create_many -------------------------------
    await db.customers.create(
        {"id": 1, "name": "김민준", "email": "minjun@corp.kr", "tier": "vip", "active": True}
    )
    await db.customers.create_many([
        {"id": 2, "name": "이서연", "email": "seoyeon@corp.kr", "tier": "gold", "active": True},
        {"id": 3, "name": "박도윤", "email": "doyun@corp.kr", "tier": "basic", "active": True},
        {"id": 4, "name": "최지우", "email": "jiwoo@old.kr", "tier": "basic", "active": False},
        {"id": 5, "name": "정하은", "email": "haeun@corp.kr", "tier": "vip", "active": True},
    ])
    check(await db.customers.count() == 5, "seed: 5 customers (create + create_many)")

    await db.products.create_many([
        {"id": 10, "name": "노트북", "price": 1500000.0, "stock": 12},
        {"id": 11, "name": "모니터", "price": 350000.0, "stock": 40},
        {"id": 12, "name": "키보드", "price": 89000.0, "stock": 0},
        {"id": 13, "name": "마우스", "price": 45000.0, "stock": 200},
    ])

    await db.orders.create_many([
        {"id": 100, "customer_id": 1, "status": "paid", "amount": 1850000.0, "note": "빠른배송 요청"},
        {"id": 101, "customer_id": 1, "status": "paid", "amount": 89000.0, "note": None},
        {"id": 102, "customer_id": 2, "status": "shipped", "amount": 350000.0, "note": None},
        {"id": 103, "customer_id": 3, "status": "pending", "amount": 45000.0, "note": None},
        {"id": 104, "customer_id": 5, "status": "paid", "amount": 700000.0, "note": "법인 세금계산서"},
        {"id": 105, "customer_id": 2, "status": "cancelled", "amount": 89000.0, "note": "고객 변심"},
        {"id": 106, "customer_id": 5, "status": "paid", "amount": 1500000.0, "note": None},
    ])

    await db.order_items.create_many([
        {"id": 1000, "order_id": 100, "product_id": 10, "qty": 1, "unit_price": 1500000.0},
        {"id": 1001, "order_id": 100, "product_id": 11, "qty": 1, "unit_price": 350000.0},
        {"id": 1002, "order_id": 101, "product_id": 12, "qty": 1, "unit_price": 89000.0},
        {"id": 1003, "order_id": 102, "product_id": 11, "qty": 1, "unit_price": 350000.0},
        {"id": 1004, "order_id": 103, "product_id": 13, "qty": 1, "unit_price": 45000.0},
        {"id": 1005, "order_id": 104, "product_id": 11, "qty": 2, "unit_price": 350000.0},
        {"id": 1006, "order_id": 106, "product_id": 10, "qty": 1, "unit_price": 1500000.0},
    ])

    # ======================================================================
    # 1. 대시보드: 상태별 매출 요약 (raw SQL GROUP BY)
    # ======================================================================
    b = await client.query(
        "SELECT status, COUNT(*) AS cnt, SUM(amount) AS revenue "
        "FROM orders GROUP BY status ORDER BY revenue DESC"
    )
    rows = b.to_rows()
    check(b.num_rows == 4, "dashboard: 4 status groups")
    check(rows[0]["status"] == "paid", "dashboard: top revenue status is 'paid'")
    check(rows[0]["cnt"] == 4, "dashboard: 4 paid orders")
    check(rows[0]["revenue"] == 4139000.0, "dashboard: paid revenue = 4,139,000")

    # ======================================================================
    # 2. 고객별 매출 리포트 (raw SQL JOIN + GROUP BY + HAVING)
    # ======================================================================
    b = await client.query(
        "SELECT c.name, c.tier, COUNT(o.id) AS orders_cnt, SUM(o.amount) AS total "
        "FROM customers c JOIN orders o ON o.customer_id = c.id "
        "WHERE o.status != 'cancelled' "
        "GROUP BY c.id HAVING SUM(o.amount) >= ? "
        "ORDER BY total DESC",
        [100000.0],
    )
    rows = b.to_rows()
    check(b.num_rows == 3, "report: 3 customers over 100k (non-cancelled)")
    check(
        rows[0]["name"] == "정하은" and rows[0]["total"] == 2200000.0,
        "report: top customer 정하은 = 2,200,000",
    )
    check(
        rows[1]["name"] == "김민준" and rows[1]["orders_cnt"] == 2,
        "report: 김민준 has 2 orders",
    )

    # ======================================================================
    # 3. 서브쿼리: 한 번도 주문 안 한 고객 (raw SQL NOT IN)
    # ======================================================================
    b = await client.query(
        "SELECT name FROM customers WHERE id NOT IN (SELECT DISTINCT customer_id FROM orders)"
    )
    check(
        b.num_rows == 1 and b.to_rows()[0]["name"] == "최지우",
        "subquery: only 최지우 never ordered",
    )

    # ======================================================================
    # 4. 재고 없는 상품 중 주문된 것 (raw SQL JOIN + WHERE)
    # ======================================================================
    b = await client.query(
        "SELECT DISTINCT p.name FROM products p "
        "JOIN order_items i ON i.product_id = p.id WHERE p.stock = 0"
    )
    check(
        b.num_rows == 1 and b.to_rows()[0]["name"] == "키보드",
        "stockout: 키보드 ordered but out of stock",
    )

    # ======================================================================
    # 5. ORM finder: 중첩 AND/OR/NOT + in/like (Prisma 스타일)
    # ======================================================================
    found = await db.customers.find_many(
        where={
            "active": True,
            "OR": [
                {"tier": "vip"},
                {"email": {"like": "%@corp.kr"}},
            ],
            "NOT": {"name": {"like": "최%"}},
        },
        order_by={"id": "asc"},
    )
    names = {c.name for c in found}
    check(
        names == {"김민준", "이서연", "박도윤", "정하은"},
        "orm-finder: nested OR/NOT/like matches 4 active corp customers",
    )

    vips = await db.customers.find_many(
        where={"tier": {"in": ["vip", "gold"]}}, order_by={"id": "asc"}
    )
    check(
        [c.name for c in vips] == ["김민준", "이서연", "정하은"],
        "orm-finder: tier in [vip, gold] -> 3 customers",
    )

    # ======================================================================
    # 6. ORM 페이지네이션: limit + offset + count
    # ======================================================================
    page1 = await db.orders.find_many(order_by={"amount": "desc"}, limit=3, offset=0)
    page2 = await db.orders.find_many(order_by={"amount": "desc"}, limit=3, offset=3)
    total = await db.orders.count()
    p1_ids = {o.id for o in page1}
    p2_ids = {o.id for o in page2}
    check(100 in p1_ids and 106 in p1_ids, "orm-paginate: page1 has two biggest orders")
    check(100 not in p2_ids and 102 in p2_ids, "orm-paginate: page2 disjoint from page1")
    check(total == 7, "orm-paginate: total = 7 (totalPages = ceil(7/3) = 3)")

    # ======================================================================
    # 7. ORM 관계: include (배치 로드) + join (belongsTo LEFT JOIN) + 중첩
    # ======================================================================
    with_customer = await db.orders.find_many(
        where={"status": "paid"}, include={"customer": True}, order_by={"id": "asc"}
    )
    cust_names = {o.customer.name for o in with_customer if o.customer}
    check(
        len(with_customer) == 4 and {"김민준", "정하은"} <= cust_names,
        "orm-include: paid orders hydrated with customer objects",
    )

    joined = await db.orders.find_many(
        where={"amount": {"gte": 1000000}}, join={"customer": True}, order_by={"id": "asc"}
    )
    join_names = {o.customer.name for o in joined if o.customer}
    check(
        {"김민준", "정하은"} == join_names,
        "orm-join: belongsTo LEFT JOIN hydrates big orders",
    )

    deep = await db.order_items.find_many(
        where={"qty": {"gte": 2}}, include={"order": {"include": {"customer": True}}}
    )
    check(
        len(deep) == 1
        and deep[0].order is not None
        and deep[0].order.customer is not None
        and deep[0].order.customer.name == "정하은",
        "orm-include: nested include order->customer",
    )

    # ======================================================================
    # 8. ORM groupBy + having + 별칭
    # ======================================================================
    groups = await db.orders.group_by(
        by=["customer_id"],
        where={"status": {"ne": "cancelled"}},
        count=True,
        sum=["amount"],
        having={"_sum_amount": {"gt": 100000}},
        order_by={"_sum_amount": "desc"},
    )
    check(
        groups and "_count" in groups[0] and "_sum_amount" in groups[0],
        "orm-groupby: alias columns _count/_sum_amount present",
    )
    group_ids = {g["customer_id"] for g in groups}
    check(
        group_ids == {1, 2, 5},
        "orm-groupby: having _sum_amount>100000 keeps 3 groups",
    )

    # ======================================================================
    # 9. ORM aggregate: sum/avg/min/max (+ 빈 집합은 None)
    # ======================================================================
    paid_sum = await db.orders.aggregate("sum", "amount", where={"status": "paid"})
    check(paid_sum == 4139000.0, "orm-aggregate: sum(paid amount)")
    max_price = await db.products.aggregate("max", "price")
    check(max_price == 1500000.0, "orm-aggregate: max product price")
    none_avg = await db.orders.aggregate("avg", "amount", where={"status": "refunded"})
    check(none_avg is None, "orm-aggregate: empty set -> None")

    # ======================================================================
    # 10. 트랜잭션: 주문 생성 (재고 차감 + 주문 + 항목)
    #     중첩 세이브포인트: 안쪽 실패가 바깥 작업을 지우지 않음
    # ======================================================================
    async with db.transaction():
        await db.orders.create(
            {"id": 107, "customer_id": 3, "status": "paid", "amount": 90000.0, "note": None}
        )
        await db.order_items.create(
            {"id": 1007, "order_id": 107, "product_id": 13, "qty": 2, "unit_price": 45000.0}
        )
        await client.execute(
            "UPDATE products SET stock = stock - ? WHERE id = ?", [2, 13]
        )
        # 안쪽: 잘못된 항목 추가 시도 -> 세이브포인트 롤백
        try:
            async with db.transaction():
                await db.order_items.create(
                    {"id": 1008, "order_id": 107, "product_id": 12, "qty": 1, "unit_price": 89000.0}
                )
                raise RuntimeError("재고 없음 — 취소")
        except RuntimeError:
            pass

    check(
        await db.order_items.count({"order_id": 107}) == 1,
        "tx: outer committed, inner savepoint rolled back",
    )
    mouse = await db.products.find(13)
    check(mouse is not None and mouse.stock == 198, "tx: stock decremented inside transaction")

    # 전체 롤백: 예외 시 아무것도 남지 않음
    try:
        async with db.transaction():
            await db.orders.create(
                {"id": 999, "customer_id": 1, "status": "paid", "amount": 1.0, "note": None}
            )
            raise RuntimeError("결제 실패")
    except RuntimeError:
        pass
    check(await db.orders.count({"id": 999}) == 0, "tx: full rollback on payment failure")

    # ======================================================================
    # 11. ORM update / delete / exists — 운영 업무
    # ======================================================================
    n = await db.orders.update(where={"status": "pending"}, data={"status": "cancelled"})
    check(n == 1, "orm-update: 1 pending order cancelled")

    check(await db.customers.exists({"active": False}), "orm-exists: inactive customer exists")

    # FK 강제 확인: 자식(order_items)이 남아 있는 주문 삭제는 거부돼야 함
    fk_enforced = False
    try:
        await db.orders.delete({"id": 103})
    except PowderError as e:
        fk_enforced = "foreign key" in str(e).lower()
    check(fk_enforced, "orm-delete: FK violation rejected (child items exist)")

    # 실무 순서: 항목 먼저 정리 -> 주문 삭제
    child_removed = await db.order_items.delete({"order_id": 103})
    removed = await db.orders.delete({"status": "cancelled", "amount": {"lt": 50000}})
    check(
        child_removed == 1 and removed == 1,
        "orm-delete: cascade order (items first, then order) succeeds",
    )

    # ======================================================================
    # 12. NULL 처리: nullable note 컬럼
    # ======================================================================
    b = await client.query("SELECT id, note FROM orders WHERE id IN (100, 101) ORDER BY id")
    note_col = b.column("note")
    check(
        note_col.get(0) == "빠른배송 요청" and b.to_rows()[0]["note"] == "빠른배송 요청",
        "null: note present on order 100",
    )
    check(
        note_col.get(1) is None and b.to_rows()[1]["note"] is None,
        "null: note NULL on order 101",
    )

    # ======================================================================
    # 13. 명명 쿼리 스타일: 파라미터화 LTV 쿼리
    # ======================================================================
    b = await client.query(
        "SELECT c.id, c.name, SUM(o.amount) AS ltv FROM customers c "
        "JOIN orders o ON o.customer_id = c.id "
        "WHERE c.active = ? AND o.status = ? "
        "GROUP BY c.id HAVING SUM(o.amount) >= ? ORDER BY ltv DESC",
        [True, "paid", 500000.0],
    )
    rows = b.to_rows()
    check(b.num_rows == 2, "named-style: 2 active customers with paid LTV >= 500k")
    check(rows[0]["name"] == "정하은", "named-style: highest LTV first")

    # ======================================================================
    # 14. 한글/이모지 등 non-ASCII 왕복
    # ======================================================================
    await client.execute(
        "INSERT INTO customers VALUES (?, ?, ?, ?, ?)",
        [6, "한글🚀고객", "emoji@corp.kr", "basic", True],
    )
    b = await client.query("SELECT name FROM customers WHERE id = 6")
    check(b.to_rows()[0]["name"] == "한글🚀고객", "utf8: Korean + emoji round-trip")

    print(
        f"\n{'REAL-WORLD QUERIES OK' if failures == 0 else 'SOME QUERIES FAILED'} "
        f"— {checks} checks, {failures} failed"
    )
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
