"""Powder ORM tests against a real in-memory database.

Uses the same metadata shape `powder generate` emits, so this doubles as a
contract test for the generated code.
"""

import asyncio
from dataclasses import dataclass
from typing import Optional

import powder
from powder.orm import (
    ColumnMeta,
    PowderError,
    PowderTable,
    RelationMeta,
    TableMeta,
    run_named_query,
)


@dataclass
class User:
    id: int
    name: Optional[str]
    score: Optional[float]
    active: bool


USERS_META = TableMeta(
    table="users",
    columns=[
        ColumnMeta(name="id", type="int", primary_key=True),
        ColumnMeta(name="name", type="text", nullable=True),
        ColumnMeta(name="score", type="float", nullable=True),
        ColumnMeta(name="active", type="bool"),
    ],
    select_all="SELECT id, name, score, active FROM users",
    insert="INSERT INTO users (id, name, score, active) VALUES (?, ?, ?, ?)",
    count_all="SELECT COUNT(*) AS n FROM users",
    delete_all="DELETE FROM users",
    ident={"id": "id", "name": "name", "score": "score", "active": "active"},
)


async def _setup() -> PowderTable:
    db = await powder.connect("sqlite::memory:")
    await db.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER NOT NULL)"
    )
    return PowderTable(db, USERS_META, User)


async def _crud():
    users = await _setup()
    await users.create({"id": 1, "name": "alice", "score": 9.5, "active": True})
    await users.create(User(id=2, name="bob", score=None, active=False))
    await users.create_many(
        [
            {"id": 3, "name": "carol", "score": 3.0, "active": True},
            {"id": 4, "name": None, "score": None, "active": False},
        ]
    )

    rows = await users.find_many(order_by={"id": "asc"})
    assert len(rows) == 4, rows
    assert rows[0] == User(id=1, name="alice", score=9.5, active=True), rows[0]
    assert rows[1].score is None and rows[1].active is False

    # where operators
    assert len(await users.find_many(where={"score": {"gte": 3}})) == 2
    assert len(await users.find_many(where={"id": {"in": [1, 3]}})) == 2
    assert len(await users.find_many(where={"id": {"in": []}})) == 0
    assert len(await users.find_many(where={"name": None})) == 1
    assert len(await users.find_many(where={"name": {"ne": None}})) == 3
    assert len(await users.find_many(where={"name": {"like": "a%"}})) == 1
    assert await users.count() == 4
    assert await users.count({"active": True}) == 2

    first = await users.find_first(where={"active": True}, order_by={"score": "desc"})
    assert first is not None and first.id == 1

    # update / delete
    assert await users.update(where={"id": 1}, data={"score": 10.0}) == 1
    assert (await users.find_first(where={"id": 1})).score == 10.0
    assert await users.delete({"id": 4}) == 1
    try:
        await users.delete({})
        raise AssertionError("empty delete() must be rejected")
    except PowderError:
        pass
    assert await users.delete_all() == 3
    assert await users.count() == 0


async def _click_to_jump():
    users = await _setup()
    await users.create({"id": 1, "name": "x", "score": 0.0, "active": True})
    try:
        await users.create({"id": 1, "name": "y", "score": 0.0, "active": True})
        raise AssertionError("duplicate pk must fail")
    except PowderError as err:
        assert "INSERT INTO users" in err.sql, err.sql
        assert err.site and "test_orm.py" in err.site, err.site
        assert "at " in str(err) and "test_orm.py" in str(err), str(err)

    # Unknown columns are caught before touching the database.
    try:
        await users.find_many(where={"ghost": 1})
        raise AssertionError("unknown column must fail")
    except PowderError as err:
        assert "unknown column" in str(err)


from dataclasses import field
from typing import List


@dataclass(kw_only=True)
class Post:
    id: int
    user_id: int
    title: str
    user: Optional["UserK"] = None


@dataclass(kw_only=True)
class UserK:
    id: int
    name: Optional[str] = None
    posts: List[Post] = field(default_factory=list)


USERS_K_META = TableMeta(
    table="users",
    columns=[
        ColumnMeta(name="id", type="int", primary_key=True),
        ColumnMeta(name="name", type="text", nullable=True),
    ],
    select_all="SELECT id, name FROM users",
    insert="INSERT INTO users (id, name) VALUES (?, ?)",
    count_all="SELECT COUNT(*) AS n FROM users",
    delete_all="DELETE FROM users",
    ident={"id": "id", "name": "name"},
    relations=(
        RelationMeta(
            name="posts",
            kind="hasMany",
            local_columns=["id"],
            foreign_columns=["user_id"],
            target=lambda: POSTS_META,
        ),
    ),
)

POSTS_META = TableMeta(
    table="posts",
    columns=[
        ColumnMeta(name="id", type="int", primary_key=True),
        ColumnMeta(name="user_id", type="int"),
        ColumnMeta(name="title", type="text"),
    ],
    select_all="SELECT id, user_id, title FROM posts",
    insert="INSERT INTO posts (id, user_id, title) VALUES (?, ?, ?)",
    count_all="SELECT COUNT(*) AS n FROM posts",
    delete_all="DELETE FROM posts",
    ident={"id": "id", "user_id": "user_id", "title": "title"},
    relations=(
        RelationMeta(
            name="user",
            kind="belongsTo",
            local_columns=["user_id"],
            foreign_columns=["id"],
            target=lambda: USERS_K_META,
        ),
    ),
)


async def _transactions():
    db = await powder.connect("sqlite::memory:")
    await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL)")

    async with db.transaction():
        await db.execute("INSERT INTO t VALUES (1, 'a')")
        await db.execute("INSERT INTO t VALUES (2, 'b')")
    batch = await db.query("SELECT COUNT(*) AS n FROM t")
    assert batch.column("n").get(0) == 2

    try:
        async with db.transaction():
            await db.execute("INSERT INTO t VALUES (3, 'c')")
            raise RuntimeError("boom")
    except RuntimeError:
        pass
    batch = await db.query("SELECT COUNT(*) AS n FROM t")
    assert batch.column("n").get(0) == 2, "rollback must undo the insert"


async def _relations():
    db = await powder.connect("sqlite::memory:")
    await db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
    await db.execute(
        "CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), title TEXT NOT NULL)"
    )
    row_types = {"users": UserK, "posts": Post}
    users = PowderTable(db, USERS_K_META, UserK, row_types=row_types)
    posts = PowderTable(db, POSTS_META, Post, row_types=row_types)

    await users.create_many([{"id": 1, "name": "alice"}, {"id": 2, "name": "bob"}])
    await posts.create_many(
        [
            {"id": 1, "user_id": 1, "title": "hello"},
            {"id": 2, "user_id": 2, "title": "hi"},
        ]
    )

    # belongsTo via include.
    rows = await posts.find_many(include={"user": True}, order_by={"id": "asc"})
    assert rows[0].user is not None and rows[0].user.name == "alice", rows[0]
    assert rows[1].user is not None and rows[1].user.name == "bob", rows[1]

    # belongsTo via a single LEFT JOIN.
    joined = await posts.find_many(join={"user": True}, order_by={"id": "asc"})
    assert joined[0].user is not None and joined[0].user.name == "alice", joined[0]
    assert joined[1].user is not None and joined[1].user.name == "bob", joined[1]

    # hasMany reverse include.
    with_posts = await users.find_many(include={"posts": True}, order_by={"id": "asc"})
    assert [p.title for p in with_posts[0].posts] == ["hello"], with_posts[0]
    assert [p.title for p in with_posts[1].posts] == ["hi"], with_posts[1]

    # create() from a dataclass whose relation field (posts) is populated must
    # not try to insert the relation as a column.
    await users.create(UserK(id=9, name="zed"))
    assert await users.count() == 3

    # hasMany cannot be joined.
    try:
        await users.find_many(join={"posts": True})
        raise AssertionError("hasMany join must fail")
    except PowderError as err:
        assert "hasMany" in str(err)

    try:
        await posts.find_many(include={"ghost": True})
        raise AssertionError("unknown relation must fail")
    except PowderError as err:
        assert "unknown relation" in str(err)


async def _composite_fk_relation():
    db = await powder.connect("sqlite::memory:")
    await db.execute(
        "CREATE TABLE orders (id INTEGER NOT NULL, year INTEGER NOT NULL, total REAL, PRIMARY KEY (id, year))"
    )
    await db.execute(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, order_id INTEGER NOT NULL, order_year INTEGER NOT NULL, "
        "FOREIGN KEY (order_id, order_year) REFERENCES orders(id, year))"
    )
    orders_meta = TableMeta(
        table="orders",
        columns=[
            ColumnMeta(name="id", type="int", primary_key=True),
            ColumnMeta(name="year", type="int", primary_key=True),
            ColumnMeta(name="total", type="float", nullable=True),
        ],
        select_all="SELECT id, year, total FROM orders",
        insert="INSERT INTO orders (id, year, total) VALUES (?, ?, ?)",
        count_all="SELECT COUNT(*) AS n FROM orders",
        delete_all="DELETE FROM orders",
        ident={"id": "id", "year": "year", "total": "total"},
    )
    items_meta = TableMeta(
        table="items",
        columns=[
            ColumnMeta(name="id", type="int", primary_key=True),
            ColumnMeta(name="order_id", type="int"),
            ColumnMeta(name="order_year", type="int"),
        ],
        select_all="SELECT id, order_id, order_year FROM items",
        insert="INSERT INTO items (id, order_id, order_year) VALUES (?, ?, ?)",
        count_all="SELECT COUNT(*) AS n FROM items",
        delete_all="DELETE FROM items",
        ident={"id": "id", "order_id": "order_id", "order_year": "order_year"},
        relations=(
            RelationMeta(
                name="order",
                kind="belongsTo",
                local_columns=["order_id", "order_year"],
                foreign_columns=["id", "year"],
                target=lambda: orders_meta,
            ),
        ),
    )
    orders = PowderTable(db, orders_meta)
    items = PowderTable(db, items_meta)
    await orders.create_many([{"id": 1, "year": 2026, "total": 100.0}, {"id": 1, "year": 2025, "total": 50.0}])
    await items.create_many(
        [{"id": 1, "order_id": 1, "order_year": 2026}, {"id": 2, "order_id": 1, "order_year": 2025}]
    )
    via_include = await items.find_many(include={"order": True}, order_by={"id": "asc"})
    assert via_include[0]["order"]["total"] == 100.0, via_include[0]
    assert via_include[1]["order"]["total"] == 50.0, via_include[1]
    via_join = await items.find_many(join={"order": True}, order_by={"id": "asc"})
    assert via_join[0]["order"]["total"] == 100.0, via_join[0]
    assert via_join[1]["order"]["total"] == 50.0, via_join[1]


async def _nested_include():
    db = await powder.connect("sqlite::memory:")
    await db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
    await db.execute(
        "CREATE TABLE posts (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), title TEXT NOT NULL)"
    )
    row_types = {"users": UserK, "posts": Post}
    users = PowderTable(db, USERS_K_META, UserK, row_types=row_types)
    posts = PowderTable(db, POSTS_META, Post, row_types=row_types)
    await users.create_many([{"id": 1, "name": "alice"}, {"id": 2, "name": "bob"}])
    await posts.create_many(
        [
            {"id": 1, "user_id": 1, "title": "hello"},
            {"id": 2, "user_id": 1, "title": "again"},
            {"id": 3, "user_id": 2, "title": "hi"},
        ]
    )

    # posts -> user -> posts, two levels deep.
    rows = await posts.find_many(
        include={"user": {"include": {"posts": True}}}, order_by={"id": "asc"}
    )
    assert rows[0].user.name == "alice"
    assert [p.title for p in rows[0].user.posts] == ["hello", "again"], rows[0].user
    assert [p.title for p in rows[2].user.posts] == ["hi"]

    try:
        await posts.find_many(include={"user": {"include": {"ghost": True}}})
        raise AssertionError("nested unknown relation must fail")
    except PowderError as err:
        assert "unknown relation" in str(err)


async def _beginner_api():
    db = await powder.connect("sqlite::memory:")
    await db.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER NOT NULL)"
    )
    users = PowderTable(db, USERS_META)
    await users.create_many(
        [
            {"id": 1, "name": "alice", "score": 9.5, "active": True},
            {"id": 2, "name": "bob", "score": 2.0, "active": True},
            {"id": 3, "name": "carol", "score": 7.0, "active": False},
        ]
    )

    # find() by single-column primary key, and by dict.
    assert (await users.find(2))["name"] == "bob"
    assert await users.find(99) is None
    assert (await users.find({"name": "carol"}))["id"] == 3

    # Chainable finder; each step returns a fresh Finder.
    base = users.where(active=True)
    top = await base.order_by("score", "desc").limit(1).all()
    assert [u["name"] for u in top] == ["alice"]
    assert await base.count() == 2
    assert (await base.order_by("score", "asc").first())["name"] == "bob"
    assert len(await base.all()) == 2  # `base` was not mutated

    # where() merges; later calls override the same column.
    assert await users.where(active=True).where(active=False).count() == 1
    assert len(await users.all()) == 3


async def _named_query():
    db = await powder.connect("sqlite::memory:")
    await db.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER NOT NULL)"
    )
    users = PowderTable(db, USERS_META)
    await users.create_many(
        [
            {"id": 1, "name": "alice", "score": 9.5, "active": True},
            {"id": 2, "name": "bob", "score": 2.0, "active": True},
            {"id": 3, "name": "carol", "score": 8.0, "active": False},
        ]
    )

    sql = "SELECT id, name, score, active FROM users WHERE active = ? AND score >= ? ORDER BY score DESC"
    rows = await run_named_query(
        db, sql, ["active", "minScore"], {"active": True, "minScore": 5.0}, meta=USERS_META
    )
    assert [r["name"] for r in rows] == ["alice"]
    assert rows[0]["active"] is True  # typed via meta

    # A param used twice binds twice, in order.
    twice = await run_named_query(db, "SELECT id FROM users WHERE id > ? OR id < ?", ["x", "x"], {"x": 2})
    assert sorted(int(r["id"]) for r in twice) == [1, 3]

    # Missing arguments fail fast.
    try:
        await run_named_query(db, sql, ["active", "minScore"], {"active": True}, meta=USERS_META)
        raise AssertionError("missing param must fail")
    except PowderError as err:
        assert "missing parameter `minScore`" in str(err)


async def _nested_transactions():
    db = await powder.connect("sqlite::memory:")
    await db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")

    # Inner rolls back (savepoint), outer commits.
    async with db.transaction():
        await db.execute("INSERT INTO t VALUES (1)")
        try:
            async with db.transaction():
                await db.execute("INSERT INTO t VALUES (2)")
                raise RuntimeError("inner boom")
        except RuntimeError:
            pass
        await db.execute("INSERT INTO t VALUES (3)")
    batch = await db.query("SELECT id FROM t ORDER BY id")
    col = batch.column("id")
    assert [col.get(0), col.get(1)] == [1, 3], "savepoint should undo only the inner insert"

    # Inner commits, outer rolls back -> everything undone.
    await db.execute("DELETE FROM t")
    try:
        async with db.transaction():
            async with db.transaction():
                await db.execute("INSERT INTO t VALUES (9)")
            raise RuntimeError("outer boom")
    except RuntimeError:
        pass
    n = (await db.query("SELECT COUNT(*) AS n FROM t")).column("n").get(0)
    assert n == 0


async def _composite_pk():
    db = await powder.connect("sqlite::memory:")
    await db.execute(
        "CREATE TABLE g (student INTEGER NOT NULL, course TEXT NOT NULL, grade REAL, PRIMARY KEY (student, course))"
    )
    meta = TableMeta(
        table="g",
        columns=[
            ColumnMeta(name="student", type="int", primary_key=True),
            ColumnMeta(name="course", type="text", primary_key=True),
            ColumnMeta(name="grade", type="float", nullable=True),
        ],
        select_all="SELECT student, course, grade FROM g",
        insert="INSERT INTO g (student, course, grade) VALUES (?, ?, ?)",
        count_all="SELECT COUNT(*) AS n FROM g",
        delete_all="DELETE FROM g",
        ident={"student": "student", "course": "course", "grade": "grade"},
    )
    g = PowderTable(db, meta)
    await g.create({"student": 1, "course": "math", "grade": 4.0})
    await g.create({"student": 1, "course": "art", "grade": 3.5})
    try:
        await g.create({"student": 1, "course": "math", "grade": 2.0})
        raise AssertionError("composite pk must be enforced")
    except PowderError:
        pass
    assert await g.count() == 2


def test_orm_crud():
    asyncio.run(_crud())


def test_orm_click_to_jump():
    asyncio.run(_click_to_jump())


def test_transactions():
    asyncio.run(_transactions())


def test_relations_include():
    asyncio.run(_relations())


def test_composite_pk():
    asyncio.run(_composite_pk())


def test_composite_fk_relation():
    asyncio.run(_composite_fk_relation())


def test_nested_transactions():
    asyncio.run(_nested_transactions())


def test_nested_include():
    asyncio.run(_nested_include())


def test_beginner_api():
    asyncio.run(_beginner_api())


def test_named_query():
    asyncio.run(_named_query())


if __name__ == "__main__":
    test_orm_crud()
    test_orm_click_to_jump()
    test_transactions()
    test_relations_include()
    test_composite_pk()
    test_composite_fk_relation()
    test_nested_transactions()
    test_nested_include()
    test_beginner_api()
    test_named_query()
    print("python orm OK")
