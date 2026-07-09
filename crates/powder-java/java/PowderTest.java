import com.powder.Batch;
import com.powder.Client;
import com.powder.Column;
import com.powder.DataType;
import com.powder.Powder;
import com.powder.Query;

/**
 * End-to-end test of the Java (JNI) binding: async engine + PCB reader +
 * transactions. Run with the native library path as the single argument:
 *
 * <pre>javac -d out java/com/powder/*.java java/PowderTest.java
 * java -cp out PowderTest /path/to/powder_java.dll</pre>
 */
public class PowderTest {
    static int checks = 0;

    static void check(boolean cond, String what) {
        checks++;
        if (!cond) {
            throw new AssertionError("FAILED: " + what);
        }
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 1) {
            throw new IllegalArgumentException("usage: PowderTest <path-to-native-lib>");
        }
        Powder.loadLibrary(args[0]);

        try (Client db = Powder.connect("sqlite::memory:")) {
            db.execute("CREATE TABLE users (id INTEGER, name TEXT, score REAL, active INTEGER)");
            long n = db.execute(
                    "INSERT INTO users VALUES (?,?,?,?),(?,?,?,?),(?,?,?,?)",
                    1L, "alice", 9.5, 1L,
                    2L, "bob", null, 0L,
                    3L, "héllo 🌍", -1.25, 1L);
            check(n == 3, "insert affected 3 rows, got " + n);

            Batch batch = db.run(
                    Query.table("users").select("id", "name", "score").order("id", "ASC"));
            check(batch.numRows() == 3, "3 rows");

            Column id = batch.column("id");
            check(id.type() == DataType.INT64, "id is int64");
            check(id.getLong(0) == 1 && id.getLong(2) == 3, "ids 1..3");

            Column name = batch.column("name");
            check(name.type() == DataType.UTF8, "name is utf8");
            check(name.getString(0).equals("alice"), "name[0] alice");
            check(name.getString(2).equals("héllo 🌍"), "utf8 preserved");

            Column score = batch.column("score");
            check(score.type() == DataType.FLOAT64, "score is float64");
            check(score.getDouble(0) == 9.5, "score[0] 9.5");
            check(score.get(1) == null, "score[1] NULL via validity bitmap");
            check(score.getDouble(2) == -1.25, "score[2] -1.25");

            // Bound parameters + a filtered query.
            Batch filtered = db.query("SELECT name FROM users WHERE id >= ? ORDER BY id", 2L);
            check(filtered.numRows() == 2, "2 rows id>=2");
            check(filtered.column("name").getString(0).equals("bob"), "bob first");

            // Zero-copy path: a direct ByteBuffer aliasing native memory.
            try (Batch direct = db.queryDirect(
                    "SELECT id, name, score FROM users ORDER BY id ASC")) {
                check(direct.isDirect(), "queryDirect returns a native-backed batch");
                check(direct.numRows() == 3, "direct: 3 rows");
                check(direct.column("id").getLong(2) == 3, "direct: ids");
                check(direct.column("name").getString(2).equals("héllo 🌍"), "direct: utf8");
                check(direct.column("score").get(1) == null, "direct: NULL preserved");
                check(direct.column("score").getDouble(0) == 9.5, "direct: float64");
                // The two paths must decode to identical rows.
                Batch copied = db.query("SELECT id, name, score FROM users ORDER BY id ASC");
                check(copied.toRows().equals(direct.toRows()), "direct == copied rows");
                check(!copied.isDirect(), "query() is JVM-backed");
                copied.close(); // no-op, but always safe
            }
            // Closing twice is idempotent and must not double-free.
            Batch d2 = db.queryDirect("SELECT id FROM users");
            d2.close();
            d2.close();
            check(!d2.isDirect(), "closed batch releases its native buffer");

            // toRows view.
            check(batch.toRows().get(0).get("name").equals("alice"), "toRows name");

            // Transaction commit.
            db.transaction(tx -> {
                tx.execute("INSERT INTO users VALUES (4, 'dave', 3.0, 1)");
            });
            check(count(db) == 4, "commit added a row");

            // Transaction rollback.
            try {
                db.transaction(tx -> {
                    tx.execute("INSERT INTO users VALUES (5, 'erin', 1.0, 1)");
                    throw new RuntimeException("boom");
                });
            } catch (RuntimeException ignored) {
            }
            check(count(db) == 4, "rollback undid the insert");

            // Nested transaction: inner rolls back (savepoint), outer commits.
            db.transaction(tx -> {
                tx.execute("INSERT INTO users VALUES (6, 'frank', 1.0, 1)");
                try {
                    tx.transaction(inner -> {
                        inner.execute("INSERT INTO users VALUES (7, 'ghost', 1.0, 1)");
                        throw new RuntimeException("inner boom");
                    });
                } catch (RuntimeException ignored) {
                }
            });
            check(count(db) == 5, "savepoint kept frank, dropped ghost");

            // --- error branches --------------------------------------------

            // Invalid SQL through queryDirect must not leak or wedge.
            boolean threw = false;
            try {
                db.queryDirect("SELECT * FROM no_such_table");
            } catch (RuntimeException e) {
                threw = e.getMessage() != null && e.getMessage().contains("no_such_table");
            }
            check(threw, "queryDirect propagates SQL errors");

            // Invalid SQL through the copying path too.
            threw = false;
            try {
                db.query("SELECT * FROM no_such_table");
            } catch (RuntimeException e) {
                threw = true;
            }
            check(threw, "query propagates SQL errors");

            // Failed transaction body that is NOT a RuntimeException is wrapped.
            threw = false;
            try {
                db.transaction(tx -> { throw new Exception("checked boom"); });
            } catch (RuntimeException e) {
                threw = e.getCause() != null && "checked boom".equals(e.getCause().getMessage());
            }
            check(threw, "checked exceptions from a tx body are wrapped");

            // Reading a direct batch after close is rejected; double close ok.
            Batch direct = db.queryDirect("SELECT id FROM users ORDER BY id");
            check(direct.isDirect(), "direct batch aliases native memory");
            direct.close();
            direct.close(); // idempotent
            threw = false;
            try {
                direct.column("id").getLong(0);
            } catch (RuntimeException e) {
                threw = true;
            }
            check(threw, "closed direct batch refuses reads");
        }

        // Every operation on a closed client fails fast (checkOpen branches).
        Client closed = Powder.connect("sqlite::memory:");
        closed.close();
        int rejected = 0;
        try { closed.execute("SELECT 1"); } catch (IllegalStateException e) { rejected++; }
        try { closed.query("SELECT 1"); } catch (IllegalStateException e) { rejected++; }
        try { closed.queryDirect("SELECT 1"); } catch (IllegalStateException e) { rejected++; }
        check(rejected == 3, "closed client rejects execute/query/queryDirect");

        System.out.println("java jni OK (" + checks + " checks)");
    }

    static long count(Client db) {
        return db.query("SELECT COUNT(*) AS n FROM users").column("n").getLong(0);
    }
}
