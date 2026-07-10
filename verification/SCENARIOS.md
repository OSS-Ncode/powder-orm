# 실무 쿼리 검증 시나리오 (전 언어 공통)

전자상거래 데이터셋으로 문서(`docs-site/content/docs/`)에 적힌 기능을
실무 수준 쿼리로 검증한다. 모든 언어 테스트는 아래 시나리오를 동일하게
구현하고, 통과/실패 카운트를 출력한 뒤 실패 시 비-0 종료 코드를 반환한다.

## 스키마 / 시드

`powder.schema.json` (customers / products / orders / order_items, FK 2단).

DDL은 각 테스트가 직접 실행 (이식 가능한 SQL):

```sql
CREATE TABLE customers (id BIGINT PRIMARY KEY, name TEXT, email TEXT, tier TEXT, active BOOLEAN)
CREATE TABLE products (id BIGINT PRIMARY KEY, name TEXT, price DOUBLE PRECISION, stock BIGINT)
CREATE TABLE orders (id BIGINT PRIMARY KEY, customer_id BIGINT, status TEXT, amount DOUBLE PRECISION, note TEXT,
                     FOREIGN KEY (customer_id) REFERENCES customers(id))
CREATE TABLE order_items (id BIGINT PRIMARY KEY, order_id BIGINT, product_id BIGINT, qty BIGINT, unit_price DOUBLE PRECISION,
                          FOREIGN KEY (order_id) REFERENCES orders(id),
                          FOREIGN KEY (product_id) REFERENCES products(id))
```

> **주의 (DB 매트릭스에서 확인된 사실)**
> - MySQL/MariaDB는 컬럼 인라인 `REFERENCES`를 조용히 무시한다 — FK는 반드시 위처럼 테이블 레벨로.
> - PostgreSQL에서 float 컬럼에 넣는 JSON 숫자는 소수점 필수 (`1500000.0`) — 정수 표기는
>   i64로 직렬화되어 `error serializing parameter N` 발생.
> - `BEGIN IMMEDIATE`는 SQLite 전용 — 서버 DB에서는 `BEGIN`.

시드 (ORM `create` / `createMany` 사용 가능한 언어는 그걸로):

- customers: (1, 김민준, minjun@corp.kr, vip, true), (2, 이서연, seoyeon@corp.kr, gold, true),
  (3, 박도윤, doyun@corp.kr, basic, true), (4, 최지우, jiwoo@old.kr, basic, false),
  (5, 정하은, haeun@corp.kr, vip, true)
- products: (10, 노트북, 1500000, 12), (11, 모니터, 350000, 40), (12, 키보드, 89000, 0), (13, 마우스, 45000, 200)
- orders: (100, 1, paid, 1850000, "빠른배송 요청"), (101, 1, paid, 89000, NULL),
  (102, 2, shipped, 350000, NULL), (103, 3, pending, 45000, NULL),
  (104, 5, paid, 700000, "법인 세금계산서"), (105, 2, cancelled, 89000, "고객 변심"),
  (106, 5, paid, 1500000, NULL)
- order_items: (1000, 100, 10, 1, 1500000), (1001, 100, 11, 1, 350000), (1002, 101, 12, 1, 89000),
  (1003, 102, 11, 1, 350000), (1004, 103, 13, 1, 45000), (1005, 104, 11, 2, 350000), (1006, 106, 10, 1, 1500000)

## 시나리오 (기대값 포함)

1. **대시보드** — raw SQL `GROUP BY status` + SUM: 4개 그룹, 최고 매출 paid(4건, 4,139,000)
2. **고객별 매출 리포트** — raw JOIN + GROUP BY + HAVING(>=100000, 취소 제외):
   3명, 1위 정하은 2,200,000, 김민준 주문 2건
3. **서브쿼리** — 주문 없는 고객 `NOT IN`: 최지우 1명
4. **재고 소진 주문 상품** — JOIN + `stock = 0`: 키보드
5. **ORM finder** — 중첩 `AND/OR/NOT` + `like`: active AND (vip OR %@corp.kr) AND NOT 최%
   → 김민준·이서연·박도윤·정하은 (최지우 제외); `tier in [vip,gold]` → 3명
6. **페이지네이션** — orderBy amount desc, limit 3 / offset: page1에 100·106, page2에 102, 총 7건
7. **관계** — `include {customer}` (paid 주문), `join {customer}` (amount>=1,000,000),
   중첩 include order_items→order→customer (qty>=2 → 정하은)
8. **groupBy+having** — by customer_id, 취소 제외, `_sum_amount > 100000`:
   고객 1·2·5만 남음, `_count`/`_sum_amount` 별칭 존재
9. **aggregate** — sum(paid)=4,139,000 / max(price)=1,500,000 / 빈 집합 avg → null
10. **트랜잭션** — 주문 107 + 항목 + 재고 차감(마우스 200→198) 커밋,
    안쪽 세이브포인트(항목 1008)만 롤백 → 주문 107의 항목 1개;
    별도 트랜잭션 예외 시 전체 롤백 (주문 999 없음)
11. **운영 업무** — update(pending→cancelled)=1건, exists(active=false)=true,
    FK 위반(자식 있는 주문 103 삭제) 오류 확인 후 자식→부모 순 삭제 성공
12. **NULL 처리** — 주문 100 note 존재, 101 note NULL (is_valid/None/null 확인)
13. **명명 쿼리 스타일** — 파라미터화 LTV 쿼리 (active+paid, HAVING>=500000): 2명, 1위 정하은
14. **UTF-8** — "한글🚀고객" 왕복

## 백엔드

- 기본: `sqlite::memory:`
- 테스트는 연결 URL을 인자/환경변수(`POWDER_URL`)로 받아 서버 DB에서도 동일하게 실행.
  서버 DB에서는 시작 시 `DROP TABLE IF EXISTS order_items, orders, products, customers`
  (역순 4문장) 후 DDL 실행.
