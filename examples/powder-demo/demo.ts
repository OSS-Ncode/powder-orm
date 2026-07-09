/**
 * Powder ORM end-to-end demo.
 *
 * Workflow this file exercises (the same one `docs/ORM.md` documents):
 *   1. powder init                        -> powder.schema.json (users + posts FK)
 *   2. powder migrate --db demo.db       -> CREATE TABLE ... (FK, dependency order)
 *   3. powder seed --db demo.db --file seed.json
 *   4. powder validate --db demo.db      -> build gate (exit 1 on drift)
 *   5. powder generate --ts models.ts    -> AOT-compiled typed models + relations
 *   6. this file                         -> typed CRUD, include, $transaction
 *
 * Run: npx tsc -p tsconfig.json && node demo.js
 */

import { Client } from "../../crates/powder-node/dist/index.js";
import { PowderTable } from "../../crates/powder-node/dist/index.js";
import { powder, type Posts, type Users } from "./models.js";

async function main() {
  const client = await Client.connect("sqlite::memory:");
  await client.execute(
    "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL, active INTEGER NOT NULL)",
  );
  await client.execute(
    "CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, title TEXT NOT NULL, FOREIGN KEY (user_id) REFERENCES users(id))",
  );

  const db = powder(client);

  // Transactions: all-or-nothing seeding.
  await db.$transaction(async (tx) => {
    await tx.users.createMany([
      { id: 1, name: "alice", score: 9.5, active: true },
      { id: 2, name: "bob", score: null, active: false },
      { id: 3, name: "carol", score: 7.25, active: true },
    ]);
    await tx.posts.createMany([
      { id: 1, user_id: 1, title: "hello powder" },
      { id: 2, user_id: 1, title: "second post" },
      { id: 3, user_id: 3, title: "carol writes" },
    ]);
  });

  // belongsTo via include (2 queries) or a single LEFT JOIN — same result.
  const posts = await db.posts.findMany({
    include: { user: true },
    orderBy: { id: "asc" },
  });
  for (const post of posts) {
    console.log(`#${post.id} "${post.title}" — by ${post.user?.name ?? "?"}`);
  }
  const joined = await db.posts.findMany({ join: { user: true }, orderBy: { id: "asc" } });
  console.log("via JOIN:", joined.map((p) => `${p.title}/${p.user?.name}`).join(", "));

  // hasMany reverse relation: each user's posts.
  const authors = await db.users.findMany({ include: { posts: true }, orderBy: { id: "asc" } });
  for (const u of authors) {
    console.log(`${u.name} wrote: [${(u.posts ?? []).map((p) => p.title).join(", ")}]`);
  }

  // Nested transactions: inner rolls back via savepoint, outer commits.
  await db.$transaction(async (tx) => {
    await tx.users.create({ id: 4, name: "dave", score: 3, active: true });
    await tx
      .$transaction(async (inner) => {
        await inner.posts.create({ id: 4, user_id: 4, title: "draft" });
        throw new Error("discard draft");
      })
      .catch(() => {});
  });
  console.log("after nested tx — users:", await db.users.count(), "posts:", await db.posts.count());

  // Beginner-friendly: find() by primary key, and a chainable query.
  console.log("find(2):", (await db.users.find(2))?.name);
  const active = await db.users
    .where({ active: true })
    .where({ score: { gte: 5 } })
    .orderBy("score", "desc")
    .all();
  console.log("active high scorers:", active.map((u) => u.name));

  // Custom named query from powder.schema.json — SQL compiled at generate time.
  const top = await db.$queries.topUsers({ active: true, minScore: 5 });
  console.log("named query topUsers:", top.map((u) => `${u.name}(${u.score})`).join(", "));

  // Nested include: post -> its author -> that author's posts.
  const deep = await db.posts.findMany({
    include: { user: { include: { posts: true } } },
    orderBy: { id: "asc" },
    limit: 1,
  });
  console.log(
    `nested include: post "${deep[0].title}" by ${deep[0].user?.name},`,
    `who wrote ${deep[0].user?.posts?.length} posts`,
  );

  // User-defined table methods: graft your own helpers with $extend and
  // call them like built-ins (db.posts.byUser / db.users.top).
  const xdb = db.$extend({
    posts: {
      async byUser(this: PowderTable<Posts>, userId: number): Promise<Posts[]> {
        return this.where({ user_id: userId }).orderBy("id").all();
      },
    },
    users: {
      async top(this: PowderTable<Users>, n: number): Promise<Users[]> {
        return this.orderBy("score", "desc").limit(n).all();
      },
    },
  });
  const mine = await xdb.posts.byUser(1);
  const best = await xdb.users.top(1);
  console.log(`$extend: user 1 has ${mine.length} posts; top scorer is ${best[0]?.name}`);

  // Beginner syntax v2: 3-arg where + aggregates + paginate.
  const rich = await db.users.where("score", ">=", 5).count();
  const avgScore = await db.users.where({ active: true }).avg("score");
  const page = await db.users.orderBy("id").paginate(1, 2);
  console.log(`where("score",">=",5) -> ${rich}, avg=${avgScore}, page 1/${page.totalPages}`);

  // Click-to-jump: the PowderError below points at THIS file and line.
  try {
    await db.users.create({ id: 1, name: "dup", active: true });
  } catch (err) {
    console.log("\nexpected failure (note the clickable `at <file>:<line>`):");
    console.log(String((err as Error).message));
  }
}

main().then(
  () => console.log("\npowder demo OK"),
  (err) => {
    console.error(err);
    throw err;
  },
);
