"""Ncode — async, zero-copy columnar database client for Python.

Example
-------
>>> import asyncio, ncode
>>> async def main():
...     db = await ncode.connect("sqlite::memory:")
...     await db.execute("CREATE TABLE t (id INTEGER, name TEXT)")
...     await db.execute("INSERT INTO t VALUES (?, ?)", [1, "alice"])
...     batch = await db.run(ncode.Query.table("t"))
...     return batch.to_rows()
>>> asyncio.run(main())
[{'id': 1, 'name': 'alice'}]
"""

from __future__ import annotations

from typing import Optional, Sequence

from . import _ncode  # native extension (PyO3)
from ._reader import Batch, Column, DataType, decode_batch
from .query import Param, Query

__all__ = [
    "connect",
    "Client",
    "Transaction",
    "Query",
    "Batch",
    "Column",
    "DataType",
    "decode_batch",
    "Param",
]


class Transaction:
    """``async with client.transaction():`` — BEGIN IMMEDIATE on enter,
    COMMIT on clean exit, ROLLBACK when the block raises."""

    def __init__(self, client: "Client"):
        self._client = client

    async def __aenter__(self) -> "Client":
        await self._client.execute("BEGIN IMMEDIATE")
        return self._client

    async def __aexit__(self, exc_type, exc, tb) -> bool:
        if exc_type is None:
            await self._client.execute("COMMIT")
        else:
            try:
                await self._client.execute("ROLLBACK")
            except Exception:  # noqa: BLE001 — surface the original error
                pass
        return False


class Client:
    """An async database client backed by the Rust core."""

    def __init__(self, inner: "_ncode.Client"):
        self._inner = inner

    async def execute(self, sql: str, params: Optional[Sequence[Param]] = None) -> int:
        """Run a non-row statement (INSERT/UPDATE/DDL); returns rows affected."""
        return await self._inner.execute(sql, list(params) if params else None)

    def transaction(self) -> Transaction:
        """Return an async context manager wrapping a transaction."""
        return Transaction(self)

    async def query(
        self, sql: str, params: Optional[Sequence[Param]] = None
    ) -> Batch:
        """Run a query; returns a decoded, zero-copy columnar :class:`Batch`."""
        raw = await self._inner.query(sql, list(params) if params else None)
        return decode_batch(raw)

    async def run(self, query: Query) -> Batch:
        """Run a built :class:`Query`."""
        sql, params = query.build()
        return await self.query(sql, params)


async def connect(url: str) -> Client:
    """Open a connection (e.g. ``"sqlite::memory:"`` or a file path)."""
    inner = await _ncode.connect(url)
    return Client(inner)
