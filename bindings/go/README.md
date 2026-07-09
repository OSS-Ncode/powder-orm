# powder-go — Go bindings

Go client for the Powder engine. It calls the Rust core through the stable C
ABI exported by the `powder-ffi` crate, and decodes the zero-copy PCB columnar
buffer in pure Go.

- **Windows**: no C toolchain needed — the library is bound with `syscall`, so
  the package builds with `CGO_ENABLED=0`.
- **Linux / macOS**: `dlopen`s the shared library via cgo.

## Build & test

```bash
# 1. Native C-ABI library
cargo build -p powder-ffi --release
#    -> <target>/release/powder_ffi.dll | libpowder_ffi.so | libpowder_ffi.dylib

# 2. Run the Go tests against it
cd bindings/go
POWDER_LIB=<target>/release/powder_ffi.dll go test ./...
```

`POWDER_LIB` points the tests at the native library; without it they skip.

## Usage

```go
import powder "github.com/powder/powder-go"

if err := powder.Load("/path/to/powder_ffi.dll"); err != nil { panic(err) }

db, err := powder.Connect("sqlite::memory:")
if err != nil { panic(err) }
defer db.Close()

db.Exec("CREATE TABLE users (id INTEGER, name TEXT, score REAL)")
db.Exec("INSERT INTO users VALUES (?,?,?)", 1, "alice", 9.5)

// Fluent builder, or raw SQL with bound parameters.
batch, err := db.Run(powder.Table("users").Select("id", "name").OrderBy("id", "ASC"))
name := batch.Column("name")
for r := 0; r < batch.NumRows(); r++ {
    fmt.Println(name.String(r))
}

// Transactions; nested calls use savepoints.
err = db.Transaction(func(tx *powder.Client) error {
    _, err := tx.Exec("INSERT INTO users VALUES (2, 'bob', 7.0)")
    return err
})
```

## Notes

- Bound parameters accept Go integers, floats, `string`, `bool`, and `nil`.
  They cross the ABI as a JSON array string, keeping the C surface to plain
  pointers and integers.
- `Column` reads values straight out of the little-endian PCB payload; nothing
  is materialized until you ask for it. `Batch.Rows()` is the convenience
  (copying) view.
- The PCB payload is copied once from native memory into a Go slice, which the
  GC then owns — Go cannot safely hold a pointer into a foreign allocation.
- `Client` serializes its own calls; the Rust core owns a single connection.
