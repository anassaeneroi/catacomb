package com.catacomb.spike;

/**
 * JNI binding for the Catacomb Rust core ({@code libcatacomb_core.so}).
 *
 * <p>Java twin of {@code RustCore.kt}. Only one is compiled per build system —
 * the Kotlin object is for the future Gradle/Compose app; this Java class is
 * what the manually-built demo APK (see {@code android/demo/}) actually
 * compiles. Both resolve to the identical native symbols
 * {@code Java_com_catacomb_spike_RustCore_<name>} in the shared library, so the
 * same {@code .so} serves either.
 */
public final class RustCore {
    static {
        // Loads libcatacomb_core.so from the APK's lib/<abi>/ directory.
        System.loadLibrary("catacomb_core");
    }

    private RustCore() {}

    /** VTT/SRT text → JSON array {@code [{"start":Double,"text":String}, …]}. */
    public static native String vttParse(String vtt);

    /** yt-dlp failure log (one entry per line) → JSON {@code {class,label,hint}}. */
    public static native String classifyError(String log);

    /** URL → platform descriptor JSON {@code {dir_name,display_name,icon}}. */
    public static native String platformFromUrl(String url);

    /** URL → backup-folder name (e.g. {@code channels} for YouTube). */
    public static native String platformDirName(String url);
}
