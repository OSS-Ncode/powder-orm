// End-to-end test of the C# binding: connect, execute, typed columnar reads,
// NULLs, unicode, transactions with savepoints, error paths.
//
//   POWDER_LIB=<path-to-powder_ffi.dll> dotnet run

using Powder;

int checks = 0;

void Check(bool cond, string what)
{
    checks++;
    if (!cond)
    {
        Console.Error.WriteLine($"FAILED: {what}");
        Environment.Exit(1);
    }
}

using (var db = Client.Connect("sqlite::memory:"))
{
    db.Execute("CREATE TABLE users (id INTEGER, name TEXT, score REAL, active INTEGER)");
    long n = db.Execute(
        "INSERT INTO users VALUES (?,?,?,?),(?,?,?,?),(?,?,?,?)",
        1L, "alice", 9.5, 1L,
        2L, "bob", null, 0L,
        3L, "héllo 🌍", -1.25, 1L);
    Check(n == 3, "insert affected 3 rows");

    var batch = db.Query("SELECT id, name, score FROM users ORDER BY id");
    Check(batch.NumRows == 3, "3 rows");
    Check(batch.Columns.Count == 3, "3 columns");
    Check(batch["id"].Type == DataType.Int64, "id is int64");
    Check(batch["id"].GetInt64(0) == 1 && batch["id"].GetInt64(2) == 3, "ids 1..3");
    Check(batch["name"].GetString(0) == "alice", "name[0]");
    Check(batch["name"].GetString(2) == "héllo 🌍", "unicode round-trips");
    Check(batch["score"].IsValid(0) && !batch["score"].IsValid(1), "NULL tracked in validity");
    Check(Math.Abs(batch["score"].GetDouble(2) - (-1.25)) < 1e-12, "float reads");
    Check(batch["score"].Get(1) == null, "boxed NULL is null");

    // Bound parameters + ToRows.
    var f = db.Query("SELECT name, score FROM users WHERE score >= ?", 0.0);
    Check(f.NumRows == 1, "filtered query");
    var rows = f.ToRows();
    Check((string?)rows[0]["name"] == "alice" && (double?)rows[0]["score"] == 9.5, "ToRows values");

    // Transaction rollback.
    try
    {
        db.Transaction(tx =>
        {
            tx.Execute("INSERT INTO users VALUES (4, 'temp', 0, 1)");
            throw new InvalidOperationException("boom");
        });
    }
    catch (InvalidOperationException) { }
    Check(db.Query("SELECT COUNT(*) AS n FROM users")["n"].GetInt64(0) == 3,
          "rollback undid the insert");

    // Nested savepoints: inner rolls back, outer commits.
    db.Transaction(tx =>
    {
        tx.Execute("INSERT INTO users VALUES (5, 'frank', 1.0, 1)");
        try
        {
            tx.Transaction(inner =>
            {
                inner.Execute("INSERT INTO users VALUES (6, 'ghost', 1.0, 1)");
                throw new InvalidOperationException("inner boom");
            });
        }
        catch (InvalidOperationException) { }
    });
    Check(db.Query("SELECT COUNT(*) AS n FROM users")["n"].GetInt64(0) == 4,
          "savepoint kept frank, dropped ghost");

    // Error paths.
    bool threw = false;
    try { db.Query("SELECT * FROM missing"); }
    catch (PowderException e) { threw = e.Message.Contains("missing"); }
    Check(threw, "bad SQL throws with the engine message");

    threw = false;
    try { _ = batch["no_such_column"]; }
    catch (PowderException) { threw = true; }
    Check(threw, "unknown column throws");
}

// Disposed client rejects use.
var closed = Client.Connect("sqlite::memory:");
closed.Dispose();
closed.Dispose(); // idempotent
bool rejected = false;
try { closed.Execute("SELECT 1"); }
catch (PowderException) { rejected = true; }
Check(rejected, "disposed client rejects use");

Console.WriteLine($"csharp binding OK ({checks} checks)");
