/**
 * Powder ORM — the model layer over the Ncode driver.
 *
 * A {@link PowderTable} is constructed from AOT-compiled metadata (emitted by
 * `powder generate` from `powder.schema.json`): the SQL skeletons for every
 * base operation are precompiled strings, and column identifiers are resolved
 * and quoted at generation time. At runtime no query is parsed or re-planned —
 * fragments are concatenated and parameters bound.
 *
 * Errors are wrapped in {@link PowderError}, which carries the failing SQL and
 * the *caller's* source location (`file:line:col`) so terminals render a
 * clickable "warp" link straight to the offending TS line.
 */

import type { Client } from "./index.js";
import type { NcodeBatch, Scalar } from "./reader.js";

/** Logical column types shared with `powder.schema.json`. */
export type ColumnType = "int" | "float" | "text" | "bool";

export interface ColumnMeta {
  readonly name: string;
  readonly type: ColumnType;
  readonly nullable?: boolean;
  readonly primaryKey?: boolean;
}

/** A relation derived from a foreign key (`posts.user_id -> users.id`). */
export interface RelationMeta {
  /** Property name the related row is attached under (e.g. `user`). */
  readonly name: string;
  readonly localColumn: string;
  readonly foreignColumn: string;
  /** Thunk so mutually-referencing tables can be declared in any order. */
  readonly target: () => TableMeta;
}

/** AOT-compiled table metadata (normally emitted by `powder generate`). */
export interface TableMeta {
  readonly table: string;
  readonly columns: readonly ColumnMeta[];
  /** Precompiled SQL skeletons; runtime only appends bound predicates. */
  readonly sql: {
    readonly selectAll: string;
    readonly insert: string;
    readonly countAll: string;
    readonly deleteAll: string;
    /** column name -> quoted identifier, resolved at generation time. */
    readonly ident: Readonly<Record<string, string>>;
  };
  /** Foreign-key relations loadable via `include`. */
  readonly relations?: readonly RelationMeta[];
}

/** Comparison operators accepted inside a where clause. */
export interface WhereOps<V> {
  eq?: V | null;
  ne?: V | null;
  gt?: V;
  gte?: V;
  lt?: V;
  lte?: V;
  like?: string;
  in?: readonly V[];
}

export type Where<T> = {
  [K in keyof T]?: T[K] | WhereOps<NonNullable<T[K]>> | null;
};

export interface FindOptions<T> {
  where?: Where<T>;
  orderBy?: { [K in keyof T]?: "asc" | "desc" };
  limit?: number;
  offset?: number;
  /** Relations (from the table's foreign keys) to load and attach. */
  include?: Record<string, boolean>;
}

/** A database error mapped back to the caller's source location. */
export class PowderError extends Error {
  /** The SQL that failed. */
  readonly sql: string;
  /** `file:line:col` of the application call site, when resolvable. */
  readonly site?: string;

  constructor(message: string, sql: string, site?: string) {
    super(site ? `${message}\n  query: ${sql}\n  at ${site}` : `${message}\n  query: ${sql}`);
    this.name = "PowderError";
    this.sql = sql;
    this.site = site;
  }
}

const OPS: Record<keyof WhereOps<unknown>, string> = {
  eq: "=",
  ne: "<>",
  gt: ">",
  gte: ">=",
  lt: "<",
  lte: "<=",
  like: "LIKE",
  in: "IN",
};

type Param = number | bigint | string | boolean | null;

/** Capture the first stack frame that lives outside this package / node core. */
function callSite(): string | undefined {
  const stack = new Error().stack;
  if (!stack) return undefined;
  for (const line of stack.split("\n").slice(1)) {
    // Skip this module's own frames and node internals; the first remaining
    // frame is the application call site.
    if (line.includes("node:internal") || /[/\\](dist|ts)[/\\]orm\.(js|ts)/.test(line)) {
      continue;
    }
    const m = line.match(/\(?((?:[A-Za-z]:)?[^():]+?):(\d+):(\d+)\)?\s*$/);
    if (m) return `${m[1].replace(/^file:\/\/\/?/, "")}:${m[2]}:${m[3]}`;
  }
  return undefined;
}

/** Coerce a raw NCB scalar to the model's declared column type. */
function coerce(v: Scalar, type: ColumnType): unknown {
  if (v === null) return null;
  if (type === "bool") return v === true || v === 1 || v === 1n;
  if (typeof v === "bigint") {
    return v >= BigInt(Number.MIN_SAFE_INTEGER) && v <= BigInt(Number.MAX_SAFE_INTEGER)
      ? Number(v)
      : v;
  }
  return v;
}

function toParam(v: unknown): Param {
  if (v === undefined) return null;
  return v as Param;
}

/** Materialize a batch into plain objects following a table's column meta. */
function materialize(batch: NcodeBatch, meta: TableMeta): Record<string, unknown>[] {
  const cols = meta.columns.map((c) => ({ meta: c, col: batch.column(c.name) }));
  const out: Record<string, unknown>[] = new Array(batch.numRows);
  for (let r = 0; r < batch.numRows; r++) {
    const row: Record<string, unknown> = {};
    for (const { meta: cm, col } of cols) {
      row[cm.name] = col ? coerce(col.get(r), cm.type) : null;
    }
    out[r] = row;
  }
  return out;
}

/**
 * A typed table handle: the unified CRUD surface of Powder ORM.
 *
 * @example
 * const users = new PowderTable<User>(client, USERS_META);
 * await users.create({ id: 1, name: "alice", score: 9.5 });
 * const top = await users.findMany({ where: { score: { gte: 5 } }, orderBy: { id: "asc" } });
 */
export class PowderTable<T extends object> {
  constructor(
    private readonly client: Client,
    readonly meta: TableMeta,
  ) {}

  /** Compile a where object into `(fragment, params)`; AOT idents, bound values. */
  private compileWhere(where: Where<T> | undefined): { clause: string; params: Param[] } {
    if (!where) return { clause: "", params: [] };
    const parts: string[] = [];
    const params: Param[] = [];
    for (const [col, cond] of Object.entries(where)) {
      const ident = this.meta.sql.ident[col];
      if (!ident) throw new PowderError(`unknown column \`${col}\``, this.meta.table, callSite());
      if (cond === undefined) continue;
      if (cond === null) {
        parts.push(`${ident} IS NULL`);
      } else if (typeof cond === "object" && !Array.isArray(cond)) {
        for (const [op, value] of Object.entries(cond as WhereOps<unknown>)) {
          const sqlOp = OPS[op as keyof WhereOps<unknown>];
          if (!sqlOp || value === undefined) continue;
          if (op === "in") {
            const list = value as unknown[];
            if (list.length === 0) {
              parts.push("1 = 0"); // IN () matches nothing
            } else {
              parts.push(`${ident} IN (${list.map(() => "?").join(", ")})`);
              params.push(...list.map(toParam));
            }
          } else if (op === "ne" && value === null) {
            parts.push(`${ident} IS NOT NULL`);
          } else if (op === "eq" && value === null) {
            parts.push(`${ident} IS NULL`);
          } else {
            parts.push(`${ident} ${sqlOp} ?`);
            params.push(toParam(value));
          }
        }
      } else {
        parts.push(`${ident} = ?`);
        params.push(toParam(cond));
      }
    }
    return { clause: parts.length ? ` WHERE ${parts.join(" AND ")}` : "", params };
  }

  private compileTail(opts: FindOptions<T>): string {
    let tail = "";
    if (opts.orderBy) {
      const keys = Object.entries(opts.orderBy).filter(([, d]) => d !== undefined);
      if (keys.length) {
        tail += ` ORDER BY ${keys
          .map(([col, dir]) => {
            const ident = this.meta.sql.ident[col];
            if (!ident) throw new PowderError(`unknown column \`${col}\``, this.meta.table, callSite());
            return `${ident} ${dir === "desc" ? "DESC" : "ASC"}`;
          })
          .join(", ")}`;
      }
    }
    if (opts.limit !== undefined) tail += ` LIMIT ${Math.floor(opts.limit)}`;
    if (opts.offset !== undefined) tail += ` OFFSET ${Math.floor(opts.offset)}`;
    return tail;
  }

  private rowsOf(batch: NcodeBatch): T[] {
    return materialize(batch, this.meta) as T[];
  }

  /** Batch-load `include`d relations and attach them to the rows. */
  private async attachRelations(
    rows: T[],
    include: Record<string, boolean>,
    site: string | undefined,
  ): Promise<void> {
    for (const [name, wanted] of Object.entries(include)) {
      if (!wanted) continue;
      const rel = this.meta.relations?.find((r) => r.name === name);
      if (!rel) {
        throw new PowderError(
          `unknown relation \`${name}\` (no foreign key on ${this.meta.table} defines it)`,
          this.meta.table,
          site,
        );
      }
      const target = rel.target();
      const keys = new Set<unknown>();
      for (const row of rows) {
        const v = (row as Record<string, unknown>)[rel.localColumn];
        if (v !== null && v !== undefined) keys.add(v);
      }
      const byKey = new Map<unknown, Record<string, unknown>>();
      const keyList = [...keys];
      const fident = target.sql.ident[rel.foreignColumn] ?? rel.foreignColumn;
      for (let start = 0; start < keyList.length; start += 500) {
        const chunk = keyList.slice(start, start + 500);
        const sql = `${target.sql.selectAll} WHERE ${fident} IN (${chunk.map(() => "?").join(", ")})`;
        const batch = await this.runQuery(sql, chunk as Param[], site);
        for (const trow of materialize(batch, target)) {
          byKey.set(trow[rel.foreignColumn], trow);
        }
      }
      for (const row of rows) {
        const v = (row as Record<string, unknown>)[rel.localColumn];
        (row as Record<string, unknown>)[rel.name] = v == null ? null : byKey.get(v) ?? null;
      }
    }
  }

  private async runQuery(sql: string, params: Param[], site: string | undefined): Promise<NcodeBatch> {
    try {
      return await this.client.query(sql, params);
    } catch (err) {
      throw new PowderError(String((err as Error).message ?? err), sql, site);
    }
  }

  private async runExecute(sql: string, params: Param[], site: string | undefined): Promise<number> {
    try {
      return await this.client.execute(sql, params);
    } catch (err) {
      throw new PowderError(String((err as Error).message ?? err), sql, site);
    }
  }

  /** SELECT rows matching `opts`, materialized as typed objects. */
  async findMany(opts: FindOptions<T> = {}): Promise<T[]> {
    const site = callSite();
    const { clause, params } = this.compileWhere(opts.where);
    const sql = this.meta.sql.selectAll + clause + this.compileTail(opts);
    const rows = this.rowsOf(await this.runQuery(sql, params, site));
    if (opts.include) {
      await this.attachRelations(rows, opts.include, site);
    }
    return rows;
  }

  /** First row matching `opts`, or `null`. */
  async findFirst(opts: FindOptions<T> = {}): Promise<T | null> {
    const rows = await this.findMany({ ...opts, limit: 1 });
    return rows[0] ?? null;
  }

  /** INSERT one row. Missing (nullable) columns are omitted. */
  async create(data: Partial<T>): Promise<number> {
    const site = callSite();
    const keys = Object.keys(data).filter((k) => (data as Record<string, unknown>)[k] !== undefined);
    let sql: string;
    if (keys.length === this.meta.columns.length) {
      // Full-shape insert: use the AOT statement (column order is canonical).
      sql = this.meta.sql.insert;
      const params = this.meta.columns.map((c) => toParam((data as Record<string, unknown>)[c.name]));
      return this.runExecute(sql, params, site);
    }
    const idents = keys.map((k) => {
      const ident = this.meta.sql.ident[k];
      if (!ident) throw new PowderError(`unknown column \`${k}\``, this.meta.table, site);
      return ident;
    });
    sql = `INSERT INTO ${this.meta.table} (${idents.join(", ")}) VALUES (${keys.map(() => "?").join(", ")})`;
    return this.runExecute(sql, keys.map((k) => toParam((data as Record<string, unknown>)[k])), site);
  }

  /** Bulk INSERT with multi-row VALUES, chunked to keep parameter counts sane. */
  async createMany(rows: readonly Partial<T>[], chunkSize = 500): Promise<number> {
    if (rows.length === 0) return 0;
    const site = callSite();
    const keys = Object.keys(rows[0]);
    const idents = keys.map((k) => {
      const ident = this.meta.sql.ident[k];
      if (!ident) throw new PowderError(`unknown column \`${k}\``, this.meta.table, site);
      return ident;
    });
    const rowPh = `(${keys.map(() => "?").join(", ")})`;
    let affected = 0;
    for (let start = 0; start < rows.length; start += chunkSize) {
      const chunk = rows.slice(start, start + chunkSize);
      const sql = `INSERT INTO ${this.meta.table} (${idents.join(", ")}) VALUES ${new Array(chunk.length).fill(rowPh).join(", ")}`;
      const params: Param[] = [];
      for (const row of chunk) {
        for (const k of keys) params.push(toParam((row as Record<string, unknown>)[k]));
      }
      affected += await this.runExecute(sql, params, site);
    }
    return affected;
  }

  /** UPDATE matching rows; returns the affected count. */
  async update(args: { where: Where<T>; data: Partial<T> }): Promise<number> {
    const site = callSite();
    const sets = Object.entries(args.data).filter(([, v]) => v !== undefined);
    if (sets.length === 0) return 0;
    const setSql = sets
      .map(([k]) => {
        const ident = this.meta.sql.ident[k];
        if (!ident) throw new PowderError(`unknown column \`${k}\``, this.meta.table, site);
        return `${ident} = ?`;
      })
      .join(", ");
    const { clause, params } = this.compileWhere(args.where);
    const sql = `UPDATE ${this.meta.table} SET ${setSql}${clause}`;
    return this.runExecute(sql, [...sets.map(([, v]) => toParam(v)), ...params], site);
  }

  /** DELETE matching rows. An empty/omitted where is rejected — use {@link deleteAll}. */
  async delete(where: Where<T>): Promise<number> {
    const site = callSite();
    const { clause, params } = this.compileWhere(where);
    if (!clause) {
      throw new PowderError(
        "delete() requires a non-empty where clause; use deleteAll() to clear the table",
        this.meta.sql.deleteAll,
        site,
      );
    }
    return this.runExecute(this.meta.sql.deleteAll + clause, params, site);
  }

  /** DELETE every row (explicit opt-in). */
  async deleteAll(): Promise<number> {
    return this.runExecute(this.meta.sql.deleteAll, [], callSite());
  }

  /** COUNT rows matching `where`. */
  async count(where?: Where<T>): Promise<number> {
    const site = callSite();
    const { clause, params } = this.compileWhere(where);
    const batch = await this.runQuery(this.meta.sql.countAll + clause, params, site);
    const v = batch.column("n")?.get(0) ?? 0;
    return typeof v === "bigint" ? Number(v) : (v as number);
  }
}
