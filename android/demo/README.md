# Catacomb Rust-core demo APK

A minimal, installable Android app that exercises the Phase-4 Rust JNI core
(`libcatacomb_core.so`) **on-device**. It's intentionally dependency-free — plain
Android Views, no Gradle / AGP / AndroidX / Compose — so it builds against
`android.jar` alone by driving the SDK build-tools directly. This sidesteps
AGP's toolchain constraints (e.g. a very new system JDK) and needs no network.

This is **not** the full Stage-1 app (no `yt-dlp-android`, no WebView-BotGuard —
those are Phases 1/2/3/5 and are device-with-Google-session territory). It's a
focused proof that the shared Rust modules run on Android through JNI.

## What it does

Three buttons call into Rust and show the JSON that comes back:

- **Detect platform** → `RustCore.platformFromUrl(url)` + `platformDirName(url)`
- **Classify error** → `RustCore.classifyError(log)`
- **Parse sample VTT** → `RustCore.vttParse(sample)`

## Build

```bash
# 1. Build the native libs (arm64-v8a + x86_64) if not already present:
../rust/catacomb_core/build.sh

# 2. Build the signed, installable APK:
./build-apk.sh            # → out/catacomb-spike-debug.apk
```

Requirements (auto-detected): an Android SDK with `platforms;android-34` and
`build-tools;34.0.0`, and a JDK ≤ 21 for the SDK tools (R8/`d8` and `apksigner`
choke on bleeding-edge JDKs; the script prefers `/usr/lib/jvm/java-17-openjdk`,
override with `TOOL_JAVA_HOME`).

The pipeline is: `aapt2 link` (manifest + resources) → `javac` (against
`android.jar`) → `d8` (dex) → `zip` in `classes.dex` + `lib/<abi>/*.so` →
`zipalign` → `apksigner` (debug key). Output is v2+v3 signed, minSdk 24.

## Install & run

```bash
adb install -r out/catacomb-spike-debug.apk
adb shell am start -n com.catacomb.spike/.MainActivity
```

Verified on an Android 14 (API 34) x86_64 emulator: the app launches, the `.so`
loads, and the startup probe + button calls return correct JSON
(`{"dir_name":"channels","display_name":"YouTube","icon":"▶"}` for a YouTube URL).

## Notes

- `src/com/catacomb/spike/RustCore.java` is the Java twin of the Kotlin
  `../app/.../RustCore.kt`; both bind the same `Java_com_catacomb_spike_RustCore_*`
  native symbols, so the identical `.so` serves either. Only the Java one is
  compiled by this manual build (no `kotlinc` needed).
- `out/` (APK, `.idsig`, debug keystore) is git-ignored — rebuild rather than
  commit binaries.
