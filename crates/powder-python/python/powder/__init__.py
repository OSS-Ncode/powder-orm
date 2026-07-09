"""Powder — async, zero-copy columnar database client for Python.

Example
-------
>>> import asyncio, powder
>>> async def main():
...     db = await powder.connect("sqlite::memory:")
...     await db.execute("CREATE TABLE t (id INTEGER, name TEXT)")
...     await db.execute("INSERT INTO t VALUES (?, ?)", [1, "alice"])
...     batch = await db.run(powder.Query.table("t"))
...     return batch.to_rows()
>>> asyncio.run(main())
[{'id': 1, 'name': 'alice'}]
"""

from __future__ import annotations

from typing import Optional, Sequence

from . import _powder  # native extension (PyO3)
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
    """``async with client.transaction():`` transaction scope.

    The outermost scope uses ``BEGIN IMMEDIATE`` + ``COMMIT``/``ROLLBACK``;
    nested scopes use ``SAVEPOINT``/``RELEASE``/``ROLLBACK TO`` so an inner
    block that raises rolls back only its own work while an outer block can
    still commit.
    """

    def __init__(self, client: "Client"):
        self._client = client
        self._savepoint: Optional[str] = None

    async def __aenter__(self) -> "Client":
        depth = self._client._tx_depth
        if depth > 0:
            self._savepoint = f"powder_sp_{depth}"
            await self._client.execute(f"SAVEPOINT {self._savepoint}")
        else:
            await self._client.execute("BEGIN IMMEDIATE")
        self._client._tx_depth = depth + 1
        return self._client

    async def __aexit__(self, exc_type, exc, tb) -> bool:
        self._client._tx_depth -= 1
        sp = self._savepoint
        if exc_type is None:
            await self._client.execute(f"RELEASE {sp}" if sp else "COMMIT")
        else:
            try:
                if sp:
                    await self._client.execute(f"ROLLBACK TO {sp}")
                    await self._client.execute(f"RELEASE {sp}")
                else:
                    await self._client.execute("ROLLBACK")
            except Exception:  # noqa: BLE001 — surface the original error
                pass
        return False


class Client:
    """An async database client backed by the Rust core."""

    def __init__(self, inner: "_powder.Client"):
        self._inner = inner
        #: Transaction nesting depth (0 = none).
        self._tx_depth = 0

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
    inner = await _powder.connect(url)
    return Client(inner)
