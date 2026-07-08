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

import { Client } from "../../crates/ncode-node/dist/index.js";
import { powder } from "./models.js";

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

  // Relations: include batch-loads the FK target and attaches it.
  const posts = await db.posts.findMany({
    include: { user: true },
    orderBy: { id: "asc" },
  });
  for (const post of posts) {
    console.log(`#${post.id} "${post.title}" — by ${post.user?.name ?? "?"}`);
  }

  const active = await db.users.findMany({
    where: { active: true, score: { gte: 5 } },
    orderBy: { score: "desc" },
  });
  console.log("active high scorers:", active.map((u) => u.name));

  // A failing transaction rolls back completely.
  await db
    .$transaction(async (tx) => {
      await tx.users.create({ id: 4, name: "dave", score: 1, active: true });
      throw new Error("changed my mind");
    })
    .catch(() => {});
  console.log("users after rollback:", await db.users.count());

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
