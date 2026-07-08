import { test } from "node:test";
import assert from "node:assert/strict";
import { Client, PowderError, PowderTable, type TableMeta } from "./index.js";

interface Post {
  id: number;
  user_id: number;
  title: string;
  user?: User | null;
}

const POSTS_META: TableMeta = {
  table: "posts",
  columns: [
    { name: "id", type: "int", primaryKey: true },
    { name: "user_id", type: "int" },
    { name: "title", type: "text" },
  ],
  sql: {
    selectAll: "SELECT id, user_id, title FROM posts",
    insert: "INSERT INTO posts (id, user_id, title) VALUES (?, ?, ?)",
    countAll: "SELECT COUNT(*) AS n FROM posts",
    deleteAll: "DELETE FROM posts",
    ident: { id: "id", user_id: "user_id", title: "title" },
  },
  relations: [
    { name: "user", localColumn: "user_id", foreignColumn: "id", target: () => USERS_META },
  ],
};

interface User {
  id: number;
  name: string | null;
  score: number | null;
  active: boolean;
}

// The shape `powder generate` emits for a table.
const USERS_META: TableMeta = {
  table: "users",
  columns: [
    { name: "id", type: "int", primaryKey: true },
    { name: "name", type: "text", nullable: true },
    { name: "score", type: "float", nullable: true },
    { name: "active", type: "bool" },
  ],
  sql: {
    selectAll: "SELECT id, name, score, active FROM users",
    insert: "INSERT INTO users (id, name, score, active) VALUES (?, ?, ?, ?)",
    countAll: "SELECT COUNT(*) AS n FROM users",
    deleteAll: "DELETE FROM users",
    ident: { id: "id", name: "name", score: "score", active: "active" },
  },
};

async function setup(): Promise<PowderTable<User>> {
  const db = await Client.connect("sqlite::memory:");
  await db.execute(
    "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER NOT NULL)",
  );
  return new PowderTable<User>(db, USERS_META);
}

test("create / findMany / findFirst round-trip with typed coercion", async () => {
  const users = await setup();
  await users.create({ id: 1, name: "alice", score: 9.5, active: true });
  await users.create({ id: 2, name: "bob", score: null, active: false });
  await users.create({ id: 3, name: null, score: 3.25, active: true });

  const all = await users.findMany({ orderBy: { id: "asc" } });
  assert.equal(all.length, 3);
  assert.deepEqual(all[0], { id: 1, name: "alice", score: 9.5, active: true });
  assert.deepEqual(all[1], { id: 2, name: "bob", score: null, active: false });
  assert.equal(typeof all[0].id, "number"); // bigint coerced to number

  const bob = await users.findFirst({ where: { name: "bob" } });
  assert.equal(bob?.id, 2);
  assert.equal(await users.findFirst({ where: { name: "nobody" } }), null);
});

test("where operators compile correctly", async () => {
  const users = await setup();
  await users.createMany([
    { id: 1, name: "a", score: 1, active: true },
    { id: 2, name: "b", score: 2, active: false },
    { id: 3, name: "c", score: 3, active: true },
    { id: 4, name: null, score: null, active: false },
  ]);

  assert.equal((await users.findMany({ where: { score: { gte: 2 } } })).length, 2);
  assert.equal((await users.findMany({ where: { score: { gt: 1, lt: 3 } } })).length, 1);
  assert.equal((await users.findMany({ where: { id: { in: [1, 3] } } })).length, 2);
  assert.equal((await users.findMany({ where: { id: { in: [] } } })).length, 0);
  assert.equal((await users.findMany({ where: { name: null } })).length, 1);
  assert.equal((await users.findMany({ where: { name: { ne: null } } })).length, 3);
  assert.equal((await users.findMany({ where: { name: { like: "a%" } } })).length, 1);
  assert.equal(await users.count(), 4);
  assert.equal(await users.count({ active: true }), 2);
});

test("update and delete return affected counts", async () => {
  const users = await setup();
  await users.createMany([
    { id: 1, name: "a", score: 1, active: true },
    { id: 2, name: "b", score: 2, active: true },
  ]);

  assert.equal(await users.update({ where: { id: 1 }, data: { score: 10 } }), 1);
  assert.equal((await users.findFirst({ where: { id: 1 } }))?.score, 10);

  assert.equal(await users.delete({ id: 2 }), 1);
  assert.equal(await users.count(), 1);

  await assert.rejects(() => users.delete({}), PowderError);
  assert.equal(await users.deleteAll(), 1);
  assert.equal(await users.count(), 0);
});

test("transaction commits on return and rolls back on throw", async () => {
  const db = await Client.connect("sqlite::memory:");
  await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL)");

  await db.transaction(async (tx) => {
    await tx.execute("INSERT INTO t VALUES (1, 'a')");
    await tx.execute("INSERT INTO t VALUES (2, 'b')");
  });
  let batch = await db.query("SELECT COUNT(*) AS n FROM t");
  assert.equal(batch.column("n")!.get(0), 2);

  await assert.rejects(
    db.transaction(async (tx) => {
      await tx.execute("INSERT INTO t VALUES (3, 'c')");
      throw new Error("boom");
    }),
    /boom/,
  );
  batch = await db.query("SELECT COUNT(*) AS n FROM t");
  assert.equal(batch.column("n")!.get(0), 2); // rolled back
});

test("include batch-loads foreign-key relations", async () => {
  const client = await Client.connect("sqlite::memory:");
  await client.execute(
    "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER NOT NULL)",
  );
  await client.execute(
    "CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), title TEXT NOT NULL)",
  );
  const users = new PowderTable<User>(client, USERS_META);
  const posts = new PowderTable<Post>(client, POSTS_META);

  await users.createMany([
    { id: 1, name: "alice", score: 9.5, active: true },
    { id: 2, name: "bob", score: null, active: false },
  ]);
  await posts.createMany([
    { id: 1, user_id: 1, title: "hello" },
    { id: 2, user_id: 1, title: "again" },
    { id: 3, user_id: 2, title: "hi" },
  ]);

  const rows = await posts.findMany({ include: { user: true }, orderBy: { id: "asc" } });
  assert.equal(rows.length, 3);
  assert.equal(rows[0].user?.name, "alice");
  assert.equal(rows[1].user?.name, "alice");
  assert.equal(rows[2].user?.name, "bob");

  // Unknown relation names fail fast.
  await assert.rejects(
    () => posts.findMany({ include: { ghost: true } }),
    (e: PowderError) => e instanceof PowderError && /unknown relation/.test(e.message),
  );
});

test("composite primary keys work through the ORM", async () => {
  const client = await Client.connect("sqlite::memory:");
  await client.execute(
    "CREATE TABLE grades (student INTEGER NOT NULL, course TEXT NOT NULL, grade REAL, PRIMARY KEY (student, course))",
  );
  interface Grade {
    student: number;
    course: string;
    grade: number | null;
  }
  const META: TableMeta = {
    table: "grades",
    columns: [
      { name: "student", type: "int", primaryKey: true },
      { name: "course", type: "text", primaryKey: true },
      { name: "grade", type: "float", nullable: true },
    ],
    sql: {
      selectAll: "SELECT student, course, grade FROM grades",
      insert: "INSERT INTO grades (student, course, grade) VALUES (?, ?, ?)",
      countAll: "SELECT COUNT(*) AS n FROM grades",
      deleteAll: "DELETE FROM grades",
      ident: { student: "student", course: "course", grade: "grade" },
    },
  };
  const grades = new PowderTable<Grade>(client, META);
  await grades.create({ student: 1, course: "math", grade: 4.0 });
  await grades.create({ student: 1, course: "art", grade: 3.5 });
  // The composite key is enforced by the database and surfaces as PowderError.
  await assert.rejects(
    () => grades.create({ student: 1, course: "math", grade: 2.0 }),
    PowderError,
  );
  const row = await grades.findFirst({ where: { student: 1, course: "math" } });
  assert.equal(row?.grade, 4.0);
  assert.equal(await grades.count(), 2);
});

test("PowderError carries SQL and a clickable caller site", async () => {
  const users = await setup();
  // Force a real DB error: duplicate primary key.
  await users.create({ id: 1, name: "x", score: 0, active: true });
  let caught: PowderError | undefined;
  try {
    await users.create({ id: 1, name: "y", score: 0, active: true });
  } catch (e) {
    caught = e as PowderError;
  }
  assert.ok(caught instanceof PowderError);
  assert.match(caught.sql, /INSERT INTO users/);
  // The site must point at THIS test file, not the ORM internals.
  assert.ok(caught.site, "expected a captured call site");
  assert.match(caught.site!, /orm\.test\.(ts|js):\d+:\d+$/);
  assert.match(caught.message, /at .*orm\.test\.(ts|js):\d+:\d+/);

  // Unknown columns are caught before touching the database.
  await assert.rejects(
    () => users.findMany({ where: { ghost: 1 } as never }),
    (e: PowderError) => e instanceof PowderError && /unknown column/.test(e.message),
  );
});
