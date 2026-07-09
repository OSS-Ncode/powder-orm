// End-to-end test of the Kotlin binding and its ORM-style DSL.
//
//   kotlinc -cp <powder-java-classes> src/dev/powder/Powder.kt test/KotlinTest.kt -d out
//   kotlin  -cp "out;<powder-java-classes>" KotlinTestKt <path-to-powder_java.dll>

import dev.powder.Database
import dev.powder.Order
import dev.powder.PowderDslException
import dev.powder.and
import dev.powder.eq
import dev.powder.gte
import dev.powder.inList
import dev.powder.like
import dev.powder.ne
import dev.powder.or

var checks = 0

fun check(cond: Boolean, what: String) {
    checks++
    if (!cond) {
        System.err.println("FAILED: $what")
        kotlin.system.exitProcess(1)
    }
}

fun main(args: Array<String>) {
    require(args.isNotEmpty()) { "usage: KotlinTestKt <path-to-powder_java-lib>" }

    Database.connect("sqlite::memory:", args[0]).use { db ->
        db.execute("CREATE TABLE users (id INTEGER, name TEXT, score REAL, active INTEGER)")

        // ORM-style inserts.
        db.from("users").insert("id" to 1, "name" to "alice", "score" to 9.5, "active" to 1)
        db.from("users").insert("id" to 2, "name" to "bob", "score" to null, "active" to 0)
        db.from("users").insert("id" to 3, "name" to "héllo 🌍", "score" to -1.25, "active" to 1)
        check(db.from("users").count() == 3L, "3 rows inserted")

        // Chainable reads with the infix where DSL.
        val top = db.from("users")
            .select("id", "name", "score")
            .where { ("score" gte 0.0) and ("active" eq 1) }
            .orderBy("score", Order.DESC)
            .limit(5)
            .all()
        check(top.size == 1 && top[0]["name"] == "alice", "filtered + ordered read")

        // Shared partial query stays immutable.
        val active = db.from("users").where { "active" eq 1 }
        val ordered = active.orderBy("id", Order.ASC)
        check(active.count() == 2L, "base ref unchanged by chaining")
        check(ordered.all().map { it["id"] } == listOf(1L, 3L), "ordered ids")

        // NULL handling: eq null -> IS NULL, ne null -> IS NOT NULL.
        check(db.from("users").where { "score" eq null }.count() == 1L, "IS NULL")
        check(db.from("users").where { "score" ne null }.count() == 2L, "IS NOT NULL")

        // OR grouping and IN lists (empty IN is always false).
        val either = db.from("users")
            .where { ("name" like "ali%") or ("id" inList listOf(3L)) }
            .orderBy("id")
            .all()
        check(either.map { it["id"] } == listOf(1L, 3L), "OR + IN")
        check(db.from("users").where { "id" inList emptyList<Any?>() }.count() == 0L, "empty IN")

        // find() by key; first() on no match.
        val bob = db.from("users").find("id" to 2)
        check(bob != null && bob["name"] == "bob", "find by pk")
        check(db.from("users").find("id" to 99) == null, "find miss is null")

        // Unicode round-trips through the PCB reader.
        check(db.from("users").find("id" to 3)!!["name"] == "héllo 🌍", "unicode")

        // update/delete refuse to run without where().
        var guarded = false
        try { db.from("users").update("score" to 0.0) } catch (e: PowderDslException) { guarded = true }
        check(guarded, "update without where is guarded")
        guarded = false
        try { db.from("users").delete() } catch (e: PowderDslException) { guarded = true }
        check(guarded, "delete without where is guarded")

        // Targeted update + delete.
        check(db.from("users").where { "id" eq 2 }.update("score" to 5.0) == 1L, "update one row")
        check(db.from("users").find("id" to 2)!!["score"] == 5.0, "updated value visible")
        check(db.from("users").where { "id" eq 2 }.delete() == 1L, "delete one row")
        check(db.from("users").count() == 2L, "row gone")

        // Injection safety: identifiers are validated, values parameterized.
        guarded = false
        try { db.from("users; DROP TABLE users").count() } catch (e: PowderDslException) { guarded = true }
        check(guarded, "malicious table name rejected")
        check(
            db.from("users").where { "name" eq "x' OR '1'='1" }.count() == 0L,
            "malicious value stays a value",
        )

        // Transactions: rollback undoes, nested savepoint keeps outer work.
        try {
            db.transaction { tx ->
                tx.from("users").insert("id" to 7, "name" to "temp", "score" to 0.0, "active" to 1)
                throw RuntimeException("boom")
            }
        } catch (e: RuntimeException) { /* expected */ }
        check(db.from("users").count() == 2L, "rollback undid the insert")

        db.transaction { tx ->
            tx.from("users").insert("id" to 8, "name" to "frank", "score" to 1.0, "active" to 1)
            try {
                tx.transaction { inner ->
                    inner.from("users").insert("id" to 9, "name" to "ghost", "score" to 1.0, "active" to 1)
                    throw RuntimeException("inner boom")
                }
            } catch (e: RuntimeException) { /* expected */ }
        }
        check(db.from("users").count() == 3L, "savepoint kept frank, dropped ghost")

        // Raw escape hatch still available, typed column access included.
        val batch = db.query("SELECT COUNT(*) AS n FROM users WHERE active = ?", 1L)
        check(batch.column("n").getLong(0) == 3L, "raw query escape hatch")
    }

    println("kotlin binding OK ($checks checks)")
}
