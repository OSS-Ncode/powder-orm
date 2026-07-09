import { test } from "node:test";
import assert from "node:assert/strict";
import { Client } from "./index.js";
import { decodeBatch, DataType } from "./reader.js";

/** Reach the raw PCB bytes the client would decode (private native handle). */
async function pcbBytes(db: Client, sql: string): Promise<Uint8Array> {
  const inner = (db as unknown as { inner: { query(sql: string, p: unknown[]): Promise<Uint8Array> } })
    .inner;
  return inner.query(sql, []);
}

/** Copy `src` into a fresh ArrayBuffer at byte offset `shift` and return a
 * Uint8Array view over it — forces every "aligned" fast path off. */
function misalign(src: Uint8Array, shift: number): Uint8Array {
  const backing = new Uint8Array(src.byteLength + shift + 8);
  backing.set(src, shift);
  return new Uint8Array(backing.buffer, shift, src.byteLength);
}

test("int64 wide values fall back to bigint, safe ones stay number", async () => {
  const db = await Client.connect("sqlite::memory:");
  await db.execute("CREATE TABLE t (v INTEGER)");
  // tiny; just under 2^53 (safe but outside the fast hi-half window);
  // 2^62 (→ bigint); most-negative wide value (→ bigint).
  await db.execute(
    "INSERT INTO t VALUES (1), (9007199254740991), (4611686018427387904), (-4611686018427387904)",
  );
  const rows = (await db.query("SELECT v FROM t ORDER BY rowid")).toRows();
  assert.equal(rows[0].v, 1);
  assert.equal(rows[1].v, 9007199254740991);
  assert.equal(rows[2].v, 4611686018427387904n);
  assert.equal(rows[3].v, -4611686018427387904n);
});

test("decodeBatch on a misaligned buffer takes the DataView fallback paths", async () => {
  const db = await Client.connect("sqlite::memory:");
  await db.execute("CREATE TABLE t (i INTEGER, f REAL, s TEXT)");
  await db.execute(
    "INSERT INTO t VALUES (4611686018427387904, 1.5, 'héllo wörld — 긴 비ASCII 문자열을 강제하는 행'), (7, 2.5, 'plain')",
  );
  const bytes = await pcbBytes(db, "SELECT i, f, s FROM t ORDER BY rowid");

  for (const shift of [1, 4]) {
    const skewed = decodeBatch(misalign(bytes, shift));
    assert.equal(skewed.numRows, 2);
    assert.equal(skewed.column("i")!.get(0), 4611686018427387904n);
    assert.equal(skewed.column("i")!.get(1), 7);
    assert.equal(skewed.column("f")!.get(0), 1.5);
    assert.equal(skewed.column("f")!.get(1), 2.5);
    assert.equal(
      skewed.column("s")!.get(0),
      "héllo wörld — 긴 비ASCII 문자열을 강제하는 행",
    );
    assert.equal(skewed.column("s")!.get(1), "plain");
  }
});

test("bad magic and unsupported version are rejected clearly", () => {
  const junk = new Uint8Array(64);
  assert.throws(() => decodeBatch(junk), /bad magic/);

  const v = new DataView(junk.buffer);
  v.setUint32(0, 0x31424350, true); // "PCB1"
  v.setUint16(4, 9, true); // bogus version
  assert.throws(() => decodeBatch(junk), /unsupported PCB version 9/);
});

test("unsupported column type code is rejected", async () => {
  const db = await Client.connect("sqlite::memory:");
  await db.execute("CREATE TABLE t (v INTEGER)");
  await db.execute("INSERT INTO t VALUES (1)");
  const bytes = new Uint8Array(await pcbBytes(db, "SELECT v FROM t"));
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const dirOff = view.getUint32(16, true);
  view.setUint8(dirOff + 8, 200); // corrupt the type code in the directory
  assert.throws(() => decodeBatch(bytes), /unsupported PCB type code 200/);
});

test("column accessors: isValid, toArray, unknown column, bounds", async () => {
  const db = await Client.connect("sqlite::memory:");
  await db.execute("CREATE TABLE t (v REAL, b INTEGER)");
  await db.execute("INSERT INTO t VALUES (1.5, 1), (NULL, 0)");
  const batch = await db.query("SELECT v, b FROM t ORDER BY rowid");

  const vcol = batch.column("v")!;
  assert.equal(vcol.isValid(0), true);
  assert.equal(vcol.isValid(1), false);
  assert.deepEqual(vcol.toArray(), [1.5, null]);
  assert.equal(vcol.get(-1), null);
  assert.equal(vcol.get(99), null);
  assert.equal(batch.column("nope"), undefined);
  assert.equal(vcol.type, DataType.Float64);
});

test("long non-ASCII strings and memoized repeat reads", async () => {
  const long = "가나다라마바사아자차카타파하".repeat(5); // > 32 bytes, non-ASCII
  const db = await Client.connect("sqlite::memory:");
  await db.execute("CREATE TABLE t (s TEXT)");
  await db.execute("INSERT INTO t VALUES (?), ('short')", [long]);
  const batch = await db.query("SELECT s FROM t ORDER BY rowid");
  const col = batch.column("s")!;
  const first = col.get(0);
  assert.equal(first, long);
  assert.equal(col.get(0), first); // memoized second read
  assert.deepEqual(batch.toRows().map((r) => r.s), [long, "short"]);
});
