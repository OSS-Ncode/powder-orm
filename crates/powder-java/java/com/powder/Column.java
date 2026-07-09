package com.powder;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;

/**
 * A single decoded PCB column. Numeric values are read directly out of the
 * little-endian backing buffer (no per-value object until you ask for one);
 * strings and null-checks go through the offsets / validity buffers, which are
 * themselves slices of the same bytes.
 *
 * <p>The buffer may be a heap {@code byte[]} wrapper or a direct buffer
 * aliasing native memory — the reader is identical either way.
 */
public final class Column {
    private final String name;
    private final DataType type;
    private final int length;
    private final ByteBuffer buf; // little-endian view over the PCB payload
    private final int validityOff; // -1 when the column has no validity bitmap
    private final int buf1Off;
    private final int buf2Off;
    private final int[] utf8Offsets; // only for UTF8 columns

    Column(
            String name,
            DataType type,
            int length,
            ByteBuffer buf,
            int validityOff,
            int buf1Off,
            int buf2Off,
            int[] utf8Offsets) {
        this.name = name;
        this.type = type;
        this.length = length;
        this.buf = buf;
        this.validityOff = validityOff;
        this.buf1Off = buf1Off;
        this.buf2Off = buf2Off;
        this.utf8Offsets = utf8Offsets;
    }

    public String name() {
        return name;
    }

    public DataType type() {
        return type;
    }

    public int length() {
        return length;
    }

    /** Whether the slot at {@code row} holds a value (vs. SQL NULL). */
    public boolean isValid(int row) {
        if (validityOff < 0) {
            return true;
        }
        int b = buf.get(validityOff + (row >> 3)) & 0xFF;
        return (b & (1 << (row & 7))) != 0;
    }

    public long getLong(int row) {
        return buf.getLong(buf1Off + row * 8);
    }

    public double getDouble(int row) {
        return buf.getDouble(buf1Off + row * 8);
    }

    public boolean getBoolean(int row) {
        int b = buf.get(buf1Off + (row >> 3)) & 0xFF;
        return (b & (1 << (row & 7))) != 0;
    }

    public String getString(int row) {
        int start = utf8Offsets[row];
        int end = utf8Offsets[row + 1];
        int n = end - start;
        if (n == 0) {
            return "";
        }
        byte[] chars = new byte[n];
        // Absolute bulk get keeps the shared buffer's position untouched.
        ByteBuffer dup = buf.duplicate();
        dup.position(buf2Off + start);
        dup.get(chars, 0, n);
        return new String(chars, StandardCharsets.UTF_8);
    }

    /** Boxed value at {@code row}, or {@code null} for NULL / out of range. */
    public Object get(int row) {
        if (row < 0 || row >= length || !isValid(row)) {
            return null;
        }
        switch (type) {
            case INT64:
                return getLong(row);
            case FLOAT64:
                return getDouble(row);
            case BOOL:
                return getBoolean(row);
            case UTF8:
                return getString(row);
            default:
                throw new IllegalStateException("unreachable type " + type);
        }
    }
}
