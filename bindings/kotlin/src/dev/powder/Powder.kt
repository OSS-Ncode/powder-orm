// Kotlin binding for the Powder engine.
//
// Layers an idiomatic, ORM-style API over the Java (JNI) binding: a chainable
// query DSL with infix where-operators, safe identifier handling, and
// transaction blocks. One dependency: the com.powder classes + powder_java
// native library.
//
//   val db = Database.connect("sqlite::memory:", libPath)
//   db.from("users")
//     .where { ("score" gte 5.0) and ("active" eq true) }
//     .orderBy("score", Order.DESC)
//     .limit(10)
//     .all()

package dev.powder

import com.powder.Batch
import com.powder.Client
import com.powder.Powder as Native

/** Sort direction for [TableRef.orderBy]. */
enum class Order { ASC, DESC }

/** Thrown for misuse of the DSL itself (engine errors keep their own type). */
class PowderDslException(message: String) : RuntimeException(message)

private val IDENT = Regex("[A-Za-z_][A-Za-z0-9_]*")

private fun ident(name: String): String {
    if (!IDENT.matches(name)) {
        throw PowderDslException("invalid identifier: `$name`")
    }
    return name
}

// ---------------------------------------------------------------------------
// Where DSL — builds parameterized SQL; values never enter the SQL text.
// ---------------------------------------------------------------------------

/** A boolean predicate tree with its bound parameters. */
sealed class Cond {
    internal abstract fun sql(): String
    internal abstract fun params(): List<Any?>

    internal class Cmp(private val column: String, private val op: String, private val value: Any?) : Cond() {
        override fun sql(): String {
            val col = ident(column)
            return when {
                value == null && op == "=" -> "$col IS NULL"
                value == null && op == "<>" -> "$col IS NOT NULL"
                else -> "$col $op ?"
            }
        }

        override fun params(): List<Any?> = if (value == null) emptyList() else listOf(value)
    }

    internal class InList(private val column: String, private val values: List<Any?>) : Cond() {
        override fun sql(): String {
            val col = ident(column)
            // Empty IN () is always false — keep the SQL valid and honest.
            if (values.isEmpty()) return "1 = 0"
            return "$col IN (${values.joinToString(", ") { "?" }})"
        }

        override fun params(): List<Any?> = values
    }

    internal class Bool(private val op: String, private val lhs: Cond, private val rhs: Cond) : Cond() {
        override fun sql(): String = "(${lhs.sql()} $op ${rhs.sql()})"
        override fun params(): List<Any?> = lhs.params() + rhs.params()
    }
}

infix fun String.eq(v: Any?): Cond = Cond.Cmp(this, "=", v)
infix fun String.ne(v: Any?): Cond = Cond.Cmp(this, "<>", v)
infix fun String.gt(v: Any): Cond = Cond.Cmp(this, ">", v)
infix fun String.gte(v: Any): Cond = Cond.Cmp(this, ">=", v)
infix fun String.lt(v: Any): Cond = Cond.Cmp(this, "<", v)
infix fun String.lte(v: Any): Cond = Cond.Cmp(this, "<=", v)
infix fun String.like(pattern: String): Cond = Cond.Cmp(this, "LIKE", pattern)
infix fun String.inList(values: List<Any?>): Cond = Cond.InList(this, values)

infix fun Cond.and(other: Cond): Cond = Cond.Bool("AND", this, other)
infix fun Cond.or(other: Cond): Cond = Cond.Bool("OR", this, other)

// ---------------------------------------------------------------------------
// TableRef — the chainable, immutable query surface.
// ---------------------------------------------------------------------------

/**
 * A query in progress against one table. Every step returns a NEW TableRef,
 * so partial queries can be shared safely:
 *
 *   val active = db.from("users").where { "active" eq true }
 *   val top = active.orderBy("score", Order.DESC).limit(5).all()
 */
class TableRef internal constructor(
    private val client: Client,
    private val table: String,
    private val selectCols: List<String> = emptyList(),
    private val cond: Cond? = null,
    private val order: List<Pair<String, Order>> = emptyList(),
    private val limitN: Long? = null,
    private val offsetN: Long? = null,
) {
    private fun copy(
        selectCols: List<String> = this.selectCols,
        cond: Cond? = this.cond,
        order: List<Pair<String, Order>> = this.order,
        limitN: Long? = this.limitN,
        offsetN: Long? = this.offsetN,
    ) = TableRef(client, table, selectCols, cond, order, limitN, offsetN)

    /** Columns to return (default `*`). */
    fun select(vararg columns: String): TableRef = copy(selectCols = columns.map(::ident))

    /** Add a predicate; repeated calls AND together. */
    fun where(build: () -> Cond): TableRef = where(build())

    fun where(condition: Cond): TableRef =
        copy(cond = if (cond == null) condition else cond and condition)

    fun orderBy(column: String, direction: Order = Order.ASC): TableRef =
        copy(order = order + (ident(column) to direction))

    fun limit(n: Long): TableRef = copy(limitN = n)
    fun offset(n: Long): TableRef = copy(offsetN = n)

    // -- reads ---------------------------------------------------------------

    /** Run the query and return the raw columnar batch. */
    fun batch(): Batch {
        val (sql, params) = buildSelect(selectCols.ifEmpty { listOf("*") }.joinToString(", "))
        return client.query(sql, *params.toTypedArray())
    }

    /** All rows as maps (copies; convenient for small result sets). */
    fun all(): List<Map<String, Any?>> = batch().toRows()

    /** The first row, or null. */
    fun first(): Map<String, Any?>? = limit(1).all().firstOrNull()

    /** Row count for the current predicate (ignores select/order/limit). */
    fun count(): Long {
        val (whereSql, params) = whereClause()
        val batch = client.query("SELECT COUNT(*) AS n FROM ${ident(table)}$whereSql", *params.toTypedArray())
        return batch.column("n").getLong(0)
    }

    /** Look up one row by equality on the given columns: `find("id" to 1)`. */
    fun find(vararg key: Pair<String, Any?>): Map<String, Any?>? {
        if (key.isEmpty()) throw PowderDslException("find() needs at least one column = value pair")
        var c: Cond? = null
        for ((col, v) in key) {
            val next = col eq v
            c = if (c == null) next else c and next
        }
        return where(c!!).first()
    }

    // -- writes ---------------------------------------------------------------

    /** INSERT one row: `insert("id" to 1, "name" to "alice")`. */
    fun insert(vararg values: Pair<String, Any?>): Long {
        if (values.isEmpty()) throw PowderDslException("insert() needs at least one column")
        val cols = values.joinToString(", ") { ident(it.first) }
        val marks = values.joinToString(", ") { "?" }
        return client.execute(
            "INSERT INTO ${ident(table)} ($cols) VALUES ($marks)",
            *values.map { it.second }.toTypedArray(),
        )
    }

    /**
     * UPDATE rows matching the predicate. Refuses to run without a where()
     * — a full-table update must be spelled out with [updateAll].
     */
    fun update(vararg set: Pair<String, Any?>): Long {
        if (cond == null) {
            throw PowderDslException("update() without where() would touch every row; use updateAll()")
        }
        return updateAll(*set)
    }

    /** UPDATE with the current predicate (or every row when none). */
    fun updateAll(vararg set: Pair<String, Any?>): Long {
        if (set.isEmpty()) throw PowderDslException("update needs at least one column")
        val assignments = set.joinToString(", ") { "${ident(it.first)} = ?" }
        val (whereSql, whereParams) = whereClause()
        return client.execute(
            "UPDATE ${ident(table)} SET $assignments$whereSql",
            *(set.map { it.second } + whereParams).toTypedArray(),
        )
    }

    /** DELETE rows matching the predicate; refuses without where(). */
    fun delete(): Long {
        if (cond == null) {
            throw PowderDslException("delete() without where() would drop every row; use deleteAll()")
        }
        return deleteAll()
    }

    /** DELETE with the current predicate (or every row when none). */
    fun deleteAll(): Long {
        val (whereSql, params) = whereClause()
        return client.execute("DELETE FROM ${ident(table)}$whereSql", *params.toTypedArray())
    }

    // -- SQL assembly ----------------------------------------------------------

    private fun whereClause(): Pair<String, List<Any?>> =
        if (cond == null) "" to emptyList() else " WHERE ${cond.sql()}" to cond.params()

    private fun buildSelect(cols: String): Pair<String, List<Any?>> {
        val sb = StringBuilder("SELECT ").append(cols).append(" FROM ").append(ident(table))
        val (whereSql, params) = whereClause()
        sb.append(whereSql)
        if (order.isNotEmpty()) {
            sb.append(" ORDER BY ")
            sb.append(order.joinToString(", ") { (c, d) -> "$c ${d.name}" })
        }
        if (limitN != null) sb.append(" LIMIT ").append(limitN)
        if (offsetN != null) {
            if (limitN == null) sb.append(" LIMIT -1")
            sb.append(" OFFSET ").append(offsetN)
        }
        return sb.toString() to params
    }
}

// ---------------------------------------------------------------------------
// Database — the connection handle.
// ---------------------------------------------------------------------------

/**
 * An open Powder connection with the Kotlin ORM-style surface.
 * Close it (or use `use { }`) to release the native connection.
 */
class Database private constructor(private val client: Client) : AutoCloseable {
    companion object {
        /**
         * Connect to a database. When [libPath] is given, the native
         * powder_java library is loaded from that absolute path first.
         */
        @JvmStatic
        @JvmOverloads
        fun connect(url: String, libPath: String? = null): Database {
            if (libPath != null) {
                Native.loadLibrary(libPath)
            }
            return Database(Native.connect(url))
        }
    }

    /** Raw SQL escape hatch: non-row statement, returns rows affected. */
    fun execute(sql: String, vararg params: Any?): Long = client.execute(sql, *params)

    /** Raw SQL escape hatch: query, returns the columnar batch. */
    fun query(sql: String, vararg params: Any?): Batch = client.query(sql, *params)

    /** Start an ORM-style chain on a table. */
    fun from(table: String): TableRef = TableRef(client, ident(table))

    /**
     * Run [body] in a transaction: COMMIT on return, ROLLBACK on throw.
     * Nested calls use savepoints (the Java binding's semantics).
     */
    fun <T> transaction(body: (Database) -> T): T {
        var result: T? = null
        client.transaction { result = body(this) }
        @Suppress("UNCHECKED_CAST")
        return result as T
    }

    override fun close() = client.close()
}
