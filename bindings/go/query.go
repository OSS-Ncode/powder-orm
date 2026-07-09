package powder

import "strings"

// Query is a fluent, injection-safe SELECT builder — the Go mirror of the
// Rust/TypeScript/Java builders. Values go through bound parameters; only
// identifiers you supply are interpolated.
//
//	q := powder.Table("users").Select("id", "name").
//		Filter("score >= ?", 5.0).OrderBy("score", "DESC").Limit(10)
//	batch, err := db.Run(q)
type Query struct {
	table   string
	cols    []string
	wheres  []string
	params  []any
	orderBy string
	limit   *int64
	offset  *int64
}

// Table starts a query against the named table.
func Table(name string) *Query { return &Query{table: name} }

// Select sets the projected columns (defaults to "*").
func (q *Query) Select(columns ...string) *Query {
	q.cols = columns
	return q
}

// Filter adds a WHERE predicate; supply one "?" per parameter. Repeated calls
// are ANDed together.
func (q *Query) Filter(predicate string, params ...any) *Query {
	q.wheres = append(q.wheres, predicate)
	q.params = append(q.params, params...)
	return q
}

// OrderBy sets the sort column and direction ("ASC" or "DESC").
func (q *Query) OrderBy(column, direction string) *Query {
	dir := "ASC"
	if strings.EqualFold(direction, "DESC") {
		dir = "DESC"
	}
	q.orderBy = column + " " + dir
	return q
}

func (q *Query) Limit(n int64) *Query {
	q.limit = &n
	return q
}

func (q *Query) Offset(n int64) *Query {
	q.offset = &n
	return q
}

// SQL renders the statement.
func (q *Query) SQL() string {
	var b strings.Builder
	b.WriteString("SELECT ")
	if len(q.cols) == 0 {
		b.WriteString("*")
	} else {
		b.WriteString(strings.Join(q.cols, ", "))
	}
	b.WriteString(" FROM ")
	b.WriteString(q.table)
	if len(q.wheres) > 0 {
		b.WriteString(" WHERE ")
		b.WriteString(strings.Join(q.wheres, " AND "))
	}
	if q.orderBy != "" {
		b.WriteString(" ORDER BY ")
		b.WriteString(q.orderBy)
	}
	if q.limit != nil {
		b.WriteString(" LIMIT ")
		b.WriteString(itoa(*q.limit))
	}
	if q.offset != nil {
		b.WriteString(" OFFSET ")
		b.WriteString(itoa(*q.offset))
	}
	return b.String()
}

// Params returns the bound parameters, in placeholder order.
func (q *Query) Params() []any { return q.params }

func itoa(n int64) string {
	if n == 0 {
		return "0"
	}
	neg := n < 0
	var buf [20]byte
	i := len(buf)
	u := uint64(n)
	if neg {
		u = uint64(-n)
	}
	for u > 0 {
		i--
		buf[i] = byte('0' + u%10)
		u /= 10
	}
	if neg {
		i--
		buf[i] = '-'
	}
	return string(buf[i:])
}
