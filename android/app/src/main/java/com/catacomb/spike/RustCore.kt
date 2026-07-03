package com.catacomb.spike

/**
 * Kotlin binding for the Catacomb Rust core (`libcatacomb_core.so`).
 *
 * This is the JVM side of the Phase-4 JNI bridge proven in
 * `android/rust/catacomb_core`. Each `external fun` resolves to a
 * `#[no_mangle] pub extern "system" fn Java_com_catacomb_spike_RustCore_<name>`
 * symbol in the shared library, so the method names here **must** match the
 * Rust exports exactly (verified via `llvm-readelf --dyn-syms`).
 *
 * The `.so` must be on the app's `jniLibs` path per ABI, e.g.
 * `app/src/main/jniLibs/arm64-v8a/libcatacomb_core.so`
 * (produced by `android/rust/catacomb_core/build.sh`).
 *
 * All calls cross into Rust, which reuses the *same* modules the desktop/web
 * build uses (`vtt`, `error_class`, `platform`) — no logic fork.
 */
object RustCore {
    init {
        // Loads libcatacomb_core.so from the ABI-specific jniLibs folder.
        System.loadLibrary("catacomb_core")
    }

    /**
     * Parse WebVTT/SRT subtitle text into a JSON array of cues:
     * `[{"start": <seconds:Double>, "text": <String>}, …]`.
     * Returns `"[]"` for empty/unparseable input.
     */
    external fun vttParse(vtt: String): String

    /**
     * Classify a yt-dlp failure log (one log entry per line) into JSON:
     * `{"class": <kebab-id>, "label": <String>, "hint": <String>}`.
     * `class` is one of: rate-limited, members-only, geo-blocked, not-found,
     * codec-missing, disk-full, network-error, bad-cookies, other.
     */
    external fun classifyError(log: String): String

    /**
     * Resolve a media URL to its platform descriptor JSON:
     * `{"dir_name": <String>, "display_name": <String>, "icon": <String>}`.
     */
    external fun platformFromUrl(url: String): String

    /**
     * The on-disk backup-folder name for a URL's platform
     * (e.g. `"channels"` for YouTube, `"tiktok"`, …, `"other"`).
     */
    external fun platformDirName(url: String): String
}
