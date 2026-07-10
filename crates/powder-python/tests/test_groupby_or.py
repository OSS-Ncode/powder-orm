"""OR/nested WHERE + group_by/having for the Python ORM.

Mirrors the TypeScript contract; same metadata shape `powder generate` emits.
"""

import asyncio
from dataclasses import dataclass
from typing import Optional

import pytest

import powder
from powder.orm import ColumnMeta, PowderError, PowderTable, TableMeta


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


@dataclass
class OrderRow:
    id: int
    user_id: int
    amount: float


ORDERS_META = TableMeta(
    table="orders",
    columns=[
        ColumnMeta(name="id", type="int", primary_key=True),
        ColumnMeta(name="user_id", type="int"),
        ColumnMeta(name="amount", type="float"),
    ],
    select_all="SELECT id, user_id, amount FROM orders",
    insert="INSERT INTO orders (id, user_id, amount) VALUES (?, ?, ?)",
    count_all="SELECT COUNT(*) AS n FROM orders",
    delete_all="DELETE FROM orders",
    ident={"id": "id", "user_id": "user_id", "amount": "amount"},
)


async def _users() -> PowderTable:
    db = await powder.connect("sqlite::memory:")
    await db.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER NOT NULL)"
    )
    t = PowderTable(db, USERS_META, User)
    await t.create_many(
        [
            {"id": 1, "name": "alice", "score": 95, "active": True},
            {"id": 2, "name": "bob", "score": 40, "active": True},
            {"id": 3, "name": "vip_carol", "score": 10, "active": False},
        ]
    )
    return t


async def _orders() -> PowderTable:
    db = await powder.connect("sqlite::memory:")
    await db.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, amount REAL)")
    t = PowderTable(db, ORDERS_META, OrderRow)
    await t.create_many(
        [
            {"id": 1, "user_id": 1, "amount": 30},
            {"id": 2, "user_id": 1, "amount": 80},
            {"id": 3, "user_id": 2, "amount": 40},
            {"id": 4, "user_id": 2, "amount": 5},
        ]
    )
    return t


def _id(rows):
    return [r.id if hasattr(r, "id") else r["id"] for r in rows]


# --- OR / nested WHERE ------------------------------------------------------


def test_where_or_not_nested():
    async def run():
        users = await _users()

        rows = await users.find_many(
            where={"OR": [{"score": {"gte": 90}}, {"name": {"like": "vip%"}}]},
            order_by={"id": "asc"},
        )
        assert _id(rows) == [1, 3]

        not_rows = await users.find_many(where={"NOT": {"active": True}}, order_by={"id": "asc"})
        assert _id(not_rows) == [3]

        # active AND (score>=90 OR (id=3 AND score=10))
        mixed = await users.find_many(
            where={
                "active": True,
                "OR": [{"score": {"gte": 90}}, {"AND": [{"id": 3}, {"score": 10}]}],
            },
            order_by={"id": "asc"},
        )
        assert _id(mixed) == [1]  # id3 is inactive so excluded by the AND active=True

        assert len(await users.find_many(where={"OR": []})) == 0

    asyncio.run(run())


def test_where_unknown_column_in_or_raises():
    async def run():
        users = await _users()
        with pytest.raises(PowderError):
            await users.find_many(where={"OR": [{"nope": 1}]})

    asyncio.run(run())


# --- group_by / having ------------------------------------------------------


def test_group_by_count_sum_having():
    async def run():
        orders = await _orders()

        rows = await orders.group_by(
            by=["user_id"], count=True, sum=["amount"], order_by={"user_id": "asc"}
        )
        assert rows == [
            {"user_id": 1, "_count": 2, "_sum_amount": 110.0},
            {"user_id": 2, "_count": 2, "_sum_amount": 45.0},
        ]

        filt = await orders.group_by(
            by=["user_id"],
            sum=["amount"],
            having={"_sum_amount": {"gt": 100}},
            order_by={"_sum_amount": "desc"},
        )
        assert filt == [{"user_id": 1, "_sum_amount": 110.0}]

    asyncio.run(run())


def test_group_by_errors():
    async def run():
        orders = await _orders()
        with pytest.raises(PowderError):
            await orders.group_by(by=[])
        with pytest.raises(PowderError):
            await orders.group_by(
                by=["user_id"], sum=["amount"], having={"_sum_amount": {"like": 1}}
            )

    asyncio.run(run())
