// Package powder is the Go client for the Powder engine.
//
// It talks to the Rust core through the stable C ABI exported by the
// powder-ffi crate (powder_ffi.dll / libpowder_ffi.so / .dylib). Query results
// arrive as the zero-copy PCB columnar buffer and are decoded in pure Go.
//
// Load the native library once, then connect:
//
//	if err := powder.Load("/path/to/powder_ffi.dll"); err != nil { ... }
//	db, err := powder.Connect("sqlite::memory:")
//	defer db.Close()
//
//	batch, err := db.Query("SELECT id, name FROM users WHERE id >= ?", 2)
//	name := batch.Column("name")
//	for r := 0; r < batch.NumRows(); r++ { fmt.Println(name.String(r)) }
package powder

import (
	"encoding/json"
	"errors"
	"fmt"
	"sync"
)

// Client is a database connection. It is safe for use from one goroutine at a
// time; the Rust core serializes access to its single connection.
type Client struct {
	handle  uintptr
	mu      sync.Mutex
	txDepth int
}

// Connect opens a connection (e.g. "sqlite::memory:" or a file path).
// Call Load first.
func Connect(url string) (*Client, error) {
	if err := ensureLoaded(); err != nil {
		return nil, err
	}
	h, err := nativeConnect(url)
	if err != nil {
		return nil, err
	}
	return &Client{handle: h}, nil
}

// Close releases the connection. Safe to call more than once.
func (c *Client) Close() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.handle == 0 {
		return nil
	}
	nativeClose(c.handle)
	c.handle = 0
	return nil
}

// marshalParams encodes bound parameters as the JSON array the C ABI expects.
// Supported: integers, floats, string, bool, nil.
func marshalParams(params []any) (string, error) {
	if len(params) == 0 {
		return "[]", nil
	}
	for _, p := range params {
		switch p.(type) {
		case nil, bool, string,
			int, int8, int16, int32, int64,
			uint, uint8, uint16, uint32, uint64,
			float32, float64:
		default:
			return "", fmt.Errorf("powder: unsupported parameter type %T", p)
		}
	}
	b, err := json.Marshal(params)
	if err != nil {
		return "", fmt.Errorf("powder: cannot encode parameters: %w", err)
	}
	return string(b), nil
}

// Exec runs a non-row statement (INSERT/UPDATE/DDL) and returns rows affected.
func (c *Client) Exec(sql string, params ...any) (int64, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.handle == 0 {
		return 0, errors.New("powder: client is closed")
	}
	pjson, err := marshalParams(params)
	if err != nil {
		return 0, err
	}
	return nativeExecute(c.handle, sql, pjson)
}

// Query runs a query and decodes the PCB result into a Batch.
func (c *Client) Query(sql string, params ...any) (*Batch, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.handle == 0 {
		return nil, errors.New("powder: client is closed")
	}
	pjson, err := marshalParams(params)
	if err != nil {
		return nil, err
	}
	// nativeQuery copies the PCB bytes into a Go slice and frees the native
	// allocation, so the returned Batch is owned by the Go GC.
	data, err := nativeQuery(c.handle, sql, pjson)
	if err != nil {
		return nil, err
	}
	return DecodePCB(data)
}

// Run executes a built Query.
func (c *Client) Run(q *Query) (*Batch, error) {
	return c.Query(q.SQL(), q.Params()...)
}

// Transaction runs fn in a transaction. The outermost call issues
// BEGIN IMMEDIATE + COMMIT/ROLLBACK; nested calls use SAVEPOINT / RELEASE /
// ROLLBACK TO, so an inner transaction that fails rolls back only its own work
// while an outer one can still commit.
func (c *Client) Transaction(fn func(tx *Client) error) (err error) {
	depth := c.txDepth
	savepoint := ""
	if depth > 0 {
		savepoint = fmt.Sprintf("powder_sp_%d", depth)
	}

	begin := "BEGIN IMMEDIATE"
	if savepoint != "" {
		begin = "SAVEPOINT " + savepoint
	}
	if _, err = c.Exec(begin); err != nil {
		return err
	}
	c.txDepth = depth + 1
	defer func() { c.txDepth = depth }()

	// A panic in fn must not leave the transaction open.
	defer func() {
		if r := recover(); r != nil {
			c.rollback(savepoint)
			panic(r)
		}
	}()

	if err = fn(c); err != nil {
		c.rollback(savepoint)
		return err
	}
	commit := "COMMIT"
	if savepoint != "" {
		commit = "RELEASE " + savepoint
	}
	_, err = c.Exec(commit)
	return err
}

func (c *Client) rollback(savepoint string) {
	// Errors here are secondary; the original failure is what callers want.
	if savepoint != "" {
		_, _ = c.Exec("ROLLBACK TO " + savepoint)
		_, _ = c.Exec("RELEASE " + savepoint)
		return
	}
	_, _ = c.Exec("ROLLBACK")
}
