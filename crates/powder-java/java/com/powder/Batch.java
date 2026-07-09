package com.powder;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * A decoded columnar result set.
 *
 * <p>When produced by the zero-copy path ({@link Client#queryDirect}) the batch
 * aliases a native allocation; {@link #close()} releases it. Batches from the
 * copying path ({@link Client#query}) are backed by a JVM {@code byte[]} and
 * closing them is a no-op, so {@code try (Batch b = ...)} is always safe.
 */
public final class Batch implements AutoCloseable {
    private final int numRows;
    private final List<Column> columns;
    private final Map<String, Column> byName;
    private long nativeAddress; // 0 when JVM-owned
    private final long nativeLength;

    Batch(int numRows, List<Column> columns, long nativeAddress, long nativeLength) {
        this.numRows = numRows;
        this.columns = columns;
        this.nativeAddress = nativeAddress;
        this.nativeLength = nativeLength;
        this.byName = new LinkedHashMap<>();
        for (Column c : columns) {
            byName.put(c.name(), c);
        }
    }

    public int numRows() {
        return numRows;
    }

    public List<Column> columns() {
        return columns;
    }

    public Column column(String name) {
        return byName.get(name);
    }

    /** Whether this batch aliases native memory that must be released. */
    public boolean isDirect() {
        return nativeAddress != 0;
    }

    /** Row-oriented view (copies) — convenient for small result sets. */
    public List<Map<String, Object>> toRows() {
        List<Map<String, Object>> rows = new ArrayList<>(numRows);
        for (int r = 0; r < numRows; r++) {
            Map<String, Object> row = new LinkedHashMap<>();
            for (Column c : columns) {
                row.put(c.name(), c.get(r));
            }
            rows.add(row);
        }
        return rows;
    }

    /**
     * Release the native allocation, if any. Idempotent. Columns must not be
     * read after close on a direct batch.
     */
    @Override
    public void close() {
        if (nativeAddress != 0) {
            PowderNative.freeBuffer(nativeAddress, nativeLength);
            nativeAddress = 0;
        }
    }
}
