package com.powder;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/**
 * Decoder for the PCB ("Powder Columnar Buffer") wire format — the Java twin
 * of the TypeScript and Python readers. Parses the fixed header + directory
 * and exposes each column over the same backing buffer, which may be a heap
 * {@code byte[]} or a direct buffer aliasing native memory (zero copy).
 */
final class PcbReader {
    // "PCB1" (bytes 0x50 0x43 0x42 0x31) read as a little-endian int.
    private static final int MAGIC = 0x31424350;
    private static final int HEADER_LEN = 24;
    private static final int COLDIR_LEN = 40;

    private PcbReader() {}

    static Batch decode(byte[] bytes) {
        if (bytes == null) {
            throw new IllegalArgumentException("null PCB payload");
        }
        return decode(ByteBuffer.wrap(bytes), 0, 0);
    }

    /**
     * Decode over {@code buf}. When {@code nativeAddress != 0} the batch owns
     * that native allocation and frees it on {@link Batch#close()}.
     */
    static Batch decode(ByteBuffer buf, long nativeAddress, long nativeLength) {
        if (buf == null || buf.capacity() < HEADER_LEN) {
            throw new IllegalArgumentException("buffer smaller than PCB header");
        }
        buf = buf.duplicate().order(ByteOrder.LITTLE_ENDIAN);
        if (buf.getInt(0) != MAGIC) {
            throw new IllegalArgumentException("not a PCB buffer (bad magic)");
        }
        int version = buf.getShort(4) & 0xFFFF;
        if (version != 1) {
            throw new IllegalArgumentException("unsupported PCB version " + version);
        }
        int numColumns = buf.getInt(8);
        int numRows = buf.getInt(12);
        int dirOff = buf.getInt(16);

        List<Column> columns = new ArrayList<>(numColumns);
        for (int c = 0; c < numColumns; c++) {
            int d = dirOff + c * COLDIR_LEN;
            int nameOff = buf.getInt(d);
            int nameLen = buf.getInt(d + 4);
            DataType dtype = DataType.fromCode(buf.get(d + 8) & 0xFF);
            boolean hasValidity = (buf.get(d + 9) & 1) != 0;
            int validityOff = buf.getInt(d + 12);
            int buf1Off = buf.getInt(d + 20);
            int buf2Off = buf.getInt(d + 28);

            byte[] nameBytes = new byte[nameLen];
            ByteBuffer dup = buf.duplicate();
            dup.position(nameOff);
            dup.get(nameBytes, 0, nameLen);
            String name = new String(nameBytes, StandardCharsets.UTF_8);

            int[] utf8Offsets = null;
            if (dtype == DataType.UTF8) {
                utf8Offsets = new int[numRows + 1];
                for (int i = 0; i <= numRows; i++) {
                    utf8Offsets[i] = buf.getInt(buf1Off + i * 4);
                }
            }

            columns.add(
                    new Column(
                            name,
                            dtype,
                            numRows,
                            buf,
                            hasValidity ? validityOff : -1,
                            buf1Off,
                            buf2Off,
                            utf8Offsets));
        }
        return new Batch(numRows, columns, nativeAddress, nativeLength);
    }
}
