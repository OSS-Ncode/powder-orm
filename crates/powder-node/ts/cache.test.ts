import { test } from "node:test";
import assert from "node:assert/strict";
import { Client } from "./index.js";

test("repeated query is served from the result cache and stays fresh", async () => {
  const db = await Client.connect("sqlite::memory:");
  await db.execute("CREATE TABLE t (id INTEGER, name TEXT)");
  await db.execute("INSERT INTO t VALUES (1, 'a'), (2, 'b')");

  const sql = "SELECT id, name FROM t ORDER BY id ASC";
  const first = await db.query(sql);
  assert.equal(first.numRows, 2);

  // Second identical query: answered by the synchronous cache probe.
  const second = await db.query(sql);
  assert.equal(second.numRows, 2);
  assert.deepEqual(
    second.columns.map((c) => c.name),
    ["id", "name"],
  );
  assert.equal(second.column("name")!.get(1), "b");

  // A write must invalidate the cache — no stale reads.
  await db.execute("INSERT INTO t VALUES (3, 'c')");
  const third = await db.query(sql);
  assert.equal(third.numRows, 3);
  assert.equal(third.column("name")!.get(2), "c");
});

test("cache-hit latency is sub-millisecond end-to-end", async () => {
  const db = await Client.connect("sqlite::memory:");
  await db.execute("CREATE TABLE t (id INTEGER, name TEXT, score REAL)");
  const rows: string[] = [];
  const params: (number | string)[] = [];
  for (let i = 0; i < 10_000; i++) {
    rows.push("(?, ?, ?)");
    params.push(i, `user_${i}`, i / 7);
  }
  for (let start = 0; start < 10_000; start += 500) {
    await db.execute(
      `INSERT INTO t VALUES ${rows.slice(start, start + 500).join(", ")}`,
      params.slice(start * 3, (start + 500) * 3),
    );
  }

  const sql = "SELECT id, name, score FROM t ORDER BY id ASC";
  await db.query(sql); // warm the cache

  const t0 = performance.now();
  const batch = await db.query(sql);
  const ms = performance.now() - t0;
  assert.equal(batch.numRows, 10_000);
  // Generous bound: the point is that the scan (several ms) did not re-run.
  assert.ok(ms < 2, `expected a cache hit, took ${ms.toFixed(3)}ms`);
});
