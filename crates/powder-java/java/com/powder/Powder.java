package com.powder;

/** Entry point for the Powder Java client. */
public final class Powder {
    private Powder() {}

    /**
     * Load the native Powder library (the {@code powder_java} cdylib) by
     * absolute path. Call once before {@link #connect}. Use this rather than
     * {@link System#loadLibrary} when the library is not on
     * {@code java.library.path}.
     */
    public static void loadLibrary(String absolutePath) {
        System.load(absolutePath);
    }

    /** Load the native library by name from {@code java.library.path}. */
    public static void loadLibraryByName(String name) {
        System.loadLibrary(name);
    }

    /** Open a connection (e.g. {@code "sqlite::memory:"} or a file path). */
    public static Client connect(String url) {
        return Client.connect(url);
    }
}
