"""Powder ORM tests against a real in-memory database.

Uses the same metadata shape `powder generate` emits, so this doubles as a
contract test for the generated code.
"""

import asyncio
from dataclasses import dataclass
from typing import Optional

import ncode
from ncode.orm import ColumnMeta, PowderError, PowderTable, RelationMeta, TableMeta


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
    db = await ncode.connect("sqlite::memory:")
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
            local_column="user_id",
            foreign_column="id",
            target=lambda: USERS_K_META,
        ),
    ),
)


async def _transactions():
    db = await ncode.connect("sqlite::memory:")
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
    db = await ncode.connect("sqlite::memory:")
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

    rows = await posts.find_many(include={"user": True}, order_by={"id": "asc"})
    assert rows[0].user is not None and rows[0].user.name == "alice", rows[0]
    assert rows[1].user is not None and rows[1].user.name == "bob", rows[1]

    try:
        await posts.find_many(include={"ghost": True})
        raise AssertionError("unknown relation must fail")
    except PowderError as err:
        assert "unknown relation" in str(err)


async def _composite_pk():
    db = await ncode.connect("sqlite::memory:")
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


if __name__ == "__main__":
    test_orm_crud()
    test_orm_click_to_jump()
    test_transactions()
    test_relations_include()
    test_composite_pk()
    print("python orm OK")
