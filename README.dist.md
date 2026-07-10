# Powder

**A zero-copy, multi-language ORM on a single Rust core.**

Powder runs one Rust engine under bindings for **9 languages** — TypeScript,
Python, Rust, Java, Kotlin, Go, C, C++, and C#. Query results cross the language
boundary once, as an Arrow-style column buffer (PCB), so there's no per-row
conversion tax — and one `powder.schema.json` derives your typed models,
migrations, and validation.

- ⚡ **Fast** — faster than the raw SQL driver in every language benchmark.
- 🧩 **One core, one bug** — every binding shares the same Rust engine.
- 📄 **Schema is the truth** — DDL, migrations, typed models, relations from one file.
- 🧯 **Fails honestly** — unmappable types and unbounded deletes error, never pass silently.

## Quick look

```ts
const db = powder(await Client.connect("app.db"));

const top = await db.users
  .where("score", ">=", 5)
  .orderBy("score", "desc")
  .limit(10)
  .all();

// relations with no N+1, groupBy/having, nested AND/OR/NOT — all in the ORM
const posts = await db.posts.findMany({ include: { user: true } });
```

## Databases

SQLite · PostgreSQL · MySQL · SQL Server · libSQL · CockroachDB

## 📚 Documentation

Full guides — install, quickstart, per-language usage, per-database notes, and
the ORM reference — live at:

### **https://docs.powder-orm.info/**

## License

MIT
