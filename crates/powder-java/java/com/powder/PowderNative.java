package com.powder;

/**
 * Raw JNI entry points into the Powder Rust core. Package-private: application
 * code uses {@link Client}. Load the native library once via
 * {@link Powder#loadLibrary(String)} before calling {@link Client#connect}.
 */
final class PowderNative {
    private PowderNative() {}

    static native long connect(String url);

    static native long execute(long handle, String sql, String paramsJson);

    /** PCB payload copied into a JVM {@code byte[]}. */
    static native byte[] query(long handle, String sql, String paramsJson);

    /**
     * PCB payload as a direct {@link java.nio.ByteBuffer} aliasing native
     * memory — no boundary copy. Must be released with {@link #freeBuffer}.
     */
    static native java.nio.ByteBuffer queryDirect(long handle, String sql, String paramsJson);

    /** Native address behind a direct buffer returned by {@link #queryDirect}. */
    static native long bufferAddress(java.nio.ByteBuffer buffer);

    /** Reclaim the allocation behind a {@link #queryDirect} buffer. */
    static native void freeBuffer(long address, long length);

    static native void close(long handle);
}
