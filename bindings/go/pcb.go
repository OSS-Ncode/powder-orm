package powder

import (
	"encoding/binary"
	"fmt"
	"math"
)

// DataType is one of the four physical column types carried by PCB.
type DataType uint8

const (
	Int64 DataType = iota
	Float64
	Bool
	Utf8
)

func (d DataType) String() string {
	switch d {
	case Int64:
		return "int64"
	case Float64:
		return "float64"
	case Bool:
		return "bool"
	case Utf8:
		return "utf8"
	}
	return fmt.Sprintf("DataType(%d)", uint8(d))
}

const (
	pcbMagic     = 0x31424350 // "PCB1" as a little-endian uint32
	pcbHeaderLen = 24
	pcbColDirLen = 40
)

// Column is a single decoded PCB column. Values are read straight out of the
// little-endian payload; nothing is materialized until you ask for it.
type Column struct {
	name        string
	dtype       DataType
	length      int
	data        []byte
	validityOff int // -1 when the column has no validity bitmap
	buf1Off     int
	buf2Off     int
	utf8Offsets []uint32 // only for Utf8 columns
}

func (c *Column) Name() string   { return c.name }
func (c *Column) Type() DataType { return c.dtype }
func (c *Column) Len() int       { return c.length }

// IsValid reports whether the slot at row holds a value (vs. SQL NULL).
func (c *Column) IsValid(row int) bool {
	if c.validityOff < 0 {
		return true
	}
	b := c.data[c.validityOff+(row>>3)]
	return b&(1<<(uint(row)&7)) != 0
}

func (c *Column) Int64(row int) int64 {
	return int64(binary.LittleEndian.Uint64(c.data[c.buf1Off+row*8:]))
}

func (c *Column) Float64(row int) float64 {
	return math.Float64frombits(binary.LittleEndian.Uint64(c.data[c.buf1Off+row*8:]))
}

func (c *Column) Bool(row int) bool {
	b := c.data[c.buf1Off+(row>>3)]
	return b&(1<<(uint(row)&7)) != 0
}

func (c *Column) String(row int) string {
	start := int(c.utf8Offsets[row])
	end := int(c.utf8Offsets[row+1])
	return string(c.data[c.buf2Off+start : c.buf2Off+end])
}

// Get returns the boxed value at row, or nil for NULL / out of range.
func (c *Column) Get(row int) any {
	if row < 0 || row >= c.length || !c.IsValid(row) {
		return nil
	}
	switch c.dtype {
	case Int64:
		return c.Int64(row)
	case Float64:
		return c.Float64(row)
	case Bool:
		return c.Bool(row)
	case Utf8:
		return c.String(row)
	}
	return nil
}

// Batch is a decoded columnar result set.
type Batch struct {
	numRows int
	columns []*Column
	byName  map[string]*Column
}

func (b *Batch) NumRows() int       { return b.numRows }
func (b *Batch) Columns() []*Column { return b.columns }

// Column returns the named column, or nil.
func (b *Batch) Column(name string) *Column { return b.byName[name] }

// Rows returns a row-oriented view (copies) — handy for small result sets.
func (b *Batch) Rows() []map[string]any {
	rows := make([]map[string]any, b.numRows)
	for r := 0; r < b.numRows; r++ {
		row := make(map[string]any, len(b.columns))
		for _, c := range b.columns {
			row[c.name] = c.Get(r)
		}
		rows[r] = row
	}
	return rows
}

// DecodePCB parses a PCB payload. The batch borrows data; do not mutate it.
func DecodePCB(data []byte) (*Batch, error) {
	if len(data) < pcbHeaderLen {
		return nil, fmt.Errorf("powder: buffer smaller than PCB header")
	}
	if binary.LittleEndian.Uint32(data[0:]) != pcbMagic {
		return nil, fmt.Errorf("powder: not a PCB buffer (bad magic)")
	}
	if v := binary.LittleEndian.Uint16(data[4:]); v != 1 {
		return nil, fmt.Errorf("powder: unsupported PCB version %d", v)
	}
	numColumns := int(binary.LittleEndian.Uint32(data[8:]))
	numRows := int(binary.LittleEndian.Uint32(data[12:]))
	dirOff := int(binary.LittleEndian.Uint32(data[16:]))

	batch := &Batch{
		numRows: numRows,
		columns: make([]*Column, 0, numColumns),
		byName:  make(map[string]*Column, numColumns),
	}
	for i := 0; i < numColumns; i++ {
		d := dirOff + i*pcbColDirLen
		if d+pcbColDirLen > len(data) {
			return nil, fmt.Errorf("powder: truncated PCB directory")
		}
		nameOff := int(binary.LittleEndian.Uint32(data[d:]))
		nameLen := int(binary.LittleEndian.Uint32(data[d+4:]))
		dtype := DataType(data[d+8])
		hasValidity := data[d+9]&1 != 0
		validityOff := int(binary.LittleEndian.Uint32(data[d+12:]))
		buf1Off := int(binary.LittleEndian.Uint32(data[d+20:]))
		buf2Off := int(binary.LittleEndian.Uint32(data[d+28:]))

		if dtype > Utf8 {
			return nil, fmt.Errorf("powder: unsupported PCB type code %d", uint8(dtype))
		}
		col := &Column{
			name:    string(data[nameOff : nameOff+nameLen]),
			dtype:   dtype,
			length:  numRows,
			data:    data,
			buf1Off: buf1Off,
			buf2Off: buf2Off,
		}
		col.validityOff = -1
		if hasValidity {
			col.validityOff = validityOff
		}
		if dtype == Utf8 {
			col.utf8Offsets = make([]uint32, numRows+1)
			for k := 0; k <= numRows; k++ {
				col.utf8Offsets[k] = binary.LittleEndian.Uint32(data[buf1Off+k*4:])
			}
		}
		batch.columns = append(batch.columns, col)
		batch.byName[col.name] = col
	}
	return batch, nil
}
