/**
 * Powder — Node.js client.
 *
 * A thin, idiomatic layer over the native napi-rs addon: async `connect`, a
 * `Client` whose `query` resolves to a decoded zero-copy columnar batch, and
 * the fluent {@link Query} builder.
 */
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { readdirSync } from "node:fs";
import { decodeBatch, type PowderBatch } from "./reader.js";
import { Query, type Param } from "./query.js";

export { Query, decodeBatch };
export type { Param, PowderBatch };
export { DataType } from "./reader.js";
export type { PowderColumn, Scalar } from "./reader.js";
export { PowderTable, PowderError, Finder, runNamedQuery } from "./orm.js";
export type {
  ColumnMeta,
  ColumnType,
  FindOptions,
  IncludeMap,
  RelationMeta,
  TableMeta,
  Where,
  WhereOps,
} from "./orm.js";

// The compiled addon (`napi build` emits `powder-node.<platform>.node`). A
// generated `index.node` loader is conventional; fall back to a direct require.
interface NativeClient {
  execute(sql: string, params?: Param[]): Promise<number>;
  query(sql: string, params?: Param[]): Promise<Buffer>;
  /** Sync query-cache probe; non-null only for a fresh repeat on `:memory:`. */
  queryCached(sql: string, params?: Param[]): Buffer | null;
}
interface NativeModule {
  connect(url: string): Promise<NativeClient>;
}

const require = createRequire(import.meta.url);
// The napi platform binary (`index.<triple>.node`) sits at the package root.
// Loading the `.node` directly sidesteps the CJS/ESM loader mismatch.
const pkgRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
// Prebuilt binaries for several platforms may sit side by side; pick the one
// matching this OS (and, when possible, this arch) rather than the first found.
const candidates = readdirSync(pkgRoot).filter(
  (f) => f.startsWith("index.") && f.endsWith(".node"),
);
const binary =
  candidates.find((f) => f.includes(process.platform) && f.includes(process.arch)) ??
  candidates.find((f) => f.includes(process.platform)) ??
  candidates[0];
if (!binary) {
  throw new Error(`no Powder native addon (index.*.node) found in ${pkgRoot}`);
}
const native: NativeModule = require(join(pkgRoot, binary));

/** An async database client backed by the Rust core. */
export class Client {
  /** Transaction nesting depth: 0 = none, 1 = outer BEGIN, >1 = savepoints. */
  private txDepth = 0;

  private constructor(private readonly inner: NativeClient) {}

  /** Open a connection (e.g. `"sqlite::memory:"` or a file path). */
  static async connect(url: string): Promise<Client> {
    return new Client(await native.connect(url));
  }

  /** Run a non-row statement (INSERT/UPDATE/DDL); resolves to rows affected. */
  execute(sql: string, params: Param[] = []): Promise<number> {
    return this.inner.execute(sql, params);
  }

  /** Run a query; resolves to a decoded, zero-copy columnar {@link PowderBatch}.
   * A repeat of an identical read-only query on an unchanged `:memory:`
   * database is answered synchronously from the engine's result cache. */
  async query(sql: string, params: Param[] = []): Promise<PowderBatch> {
    const cached = this.inner.queryCached(sql, params);
    if (cached) return decodeBatch(cached);
    const bytes = await this.inner.query(sql, params);
    return decodeBatch(bytes);
  }

  /** Run a built {@link Query}. */
  async run(query: Query): Promise<PowderBatch> {
    const { sql, params } = query.build();
    return this.query(sql, params);
  }

  /**
   * Run `fn` inside a transaction. The outermost call issues `BEGIN IMMEDIATE`
   * and `COMMIT`/`ROLLBACK`; nested calls use `SAVEPOINT`/`RELEASE`/`ROLLBACK
   * TO`, so an inner transaction that throws rolls back only its own work
   * while an outer one can still commit. Statements issued through this client
   * inside `fn` join the (single-connection, serialized) transaction.
   */
  async transaction<T>(fn: (tx: this) => Promise<T>): Promise<T> {
    const depth = this.txDepth;
    const savepoint = depth > 0 ? `powder_sp_${depth}` : null;
    await this.execute(savepoint ? `SAVEPOINT ${savepoint}` : "BEGIN IMMEDIATE");
    this.txDepth = depth + 1;
    try {
      const result = await fn(this);
      await this.execute(savepoint ? `RELEASE ${savepoint}` : "COMMIT");
      return result;
    } catch (err) {
      try {
        // ROLLBACK TO leaves the savepoint active; release it too so the
        // depth counter and SQLite's savepoint stack stay in lockstep.
        if (savepoint) {
          await this.execute(`ROLLBACK TO ${savepoint}`);
          await this.execute(`RELEASE ${savepoint}`);
        } else {
          await this.execute("ROLLBACK");
        }
      } catch {
        // The original error is the one worth surfacing.
      }
      throw err;
    } finally {
      this.txDepth = depth;
    }
  }
}
