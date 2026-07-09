# @powder/java — JNI bindings

Java client for the Powder engine. The native layer (Rust, `jni` crate) owns
the async connection and returns query results as the raw PCB byte buffer; the
pure-Java `PcbReader` turns it into typed columns. Mirrors the Node (napi) and
Python (PyO3) bindings.

## Build

```bash
# 1. Native cdylib  ->  <target>/release/powder_java.{dll,so,dylib}
cargo build -p powder-java --release

# 2. Java classes
cd crates/powder-java
javac -d out java/com/powder/*.java java/PowderTest.java

# 3. Run the e2e test (pass the native library path)
java -cp out PowderTest <target>/release/powder_java.dll
#   -> java jni OK (17 checks)
```

On Linux/macOS the library is `libpowder_java.so` / `libpowder_java.dylib`; put
it on `java.library.path` and use `Powder.loadLibraryByName("powder_java")`
instead of an absolute path.

## Usage

```java
import com.powder.*;

Powder.loadLibrary("/path/to/powder_java.dll");
try (Client db = Powder.connect("sqlite::memory:")) {
    db.execute("CREATE TABLE users (id INTEGER, name TEXT, score REAL)");
    db.execute("INSERT INTO users VALUES (?,?,?)", 1L, "alice", 9.5);

    Batch batch = db.run(Query.table("users").select("id", "name").order("id"));
    Column name = batch.column("name");
    for (int r = 0; r < batch.numRows(); r++) {
        System.out.println(name.getString(r));
    }

    // Transactions (nested calls use savepoints).
    db.transaction(tx -> {
        tx.execute("INSERT INTO users VALUES (2, 'bob', 7.0)");
    });
}
```

## Notes

- Bound parameters accept `Long`/`Integer`, `Double`/`Float`, `String`,
  `Boolean`, and `null`. They cross the JNI boundary as a JSON array string, so
  the native surface stays narrow with no object-array reflection.

### Copying vs. zero-copy

| Method | Backing | Boundary copy | Close required |
|---|---|---|---|
| `query(...)` | JVM `byte[]` | one copy | no (close is a no-op) |
| `queryDirect(...)` | direct `ByteBuffer` over native memory | **none** | **yes** |

`queryDirect` hands the JVM a `DirectByteBuffer` aliasing the Rust allocation,
so large result sets never pay the boundary copy. The batch owns that memory —
use try-with-resources and don't read columns after close:

```java
try (Batch b = db.queryDirect("SELECT * FROM users")) {
    // read columns here
}
```

Both paths decode through the same `PcbReader` and produce identical rows;
numeric access is read straight from the little-endian buffer either way.
