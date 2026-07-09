package com.powder;

/**
 * Shared liveness token between a direct {@link Batch} and its {@link Column}s.
 * Reading a column after the batch released its native allocation would be a
 * use-after-free; the token turns that into an {@link IllegalStateException}.
 */
final class Lifetime {
    volatile boolean closed;

    void check() {
        if (closed) {
            throw new IllegalStateException("batch is closed; native memory was released");
        }
    }
}
