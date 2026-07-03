# Catacomb Android (Stage-1 prototype)

This directory holds the Android Stage-1 feasibility prototype described in
[`docs/superpowers/specs/2026-06-27-android-stage1-prototype-plan.md`](../docs/superpowers/specs/2026-06-27-android-stage1-prototype-plan.md).

## Status by phase

| Phase | What | Status |
|-------|------|--------|
| 1 | App shell + `yt-dlp-android` integration | Not started (needs Compose deps + the `yt-dlp-android` release) |
| 2 | WebView-BotGuard POT generation | Blocked here — needs a real device + genuine Google session |
| 3 | Token passing + anti-bot test | Blocked here — device-only |
| 4 | **Rust core → Android `.so` via JNI** | ✅ **Implemented & verified** |
| 5 | On-device integration test → Go/No-Go | Device-only |

Phases 2/3/5 require a physical Android device with a logged-in Google session
(the WebView anti-bot test), so they can't be executed in a headless CI/dev
environment. Phase 4 — the "shared Rust core" leg the feasibility research
marked **PROVEN** — is fully implemented and verified here.

## Installable demo APK (`demo/`)

[`demo/`](demo/) is a minimal, dependency-free app (plain Android Views, no
Gradle/AGP/Compose) that calls the Phase-4 JNI core on-device and shows the
results. Build it with `demo/build-apk.sh` (drives `aapt2`/`d8`/`zipalign`/
`apksigner` directly) → `demo/out/catacomb-spike-debug.apk`. Verified on an
Android 14 x86_64 emulator: the app launches, loads `libcatacomb_core.so`, and
the JNI calls return correct JSON. See [`demo/README.md`](demo/README.md).

## Phase 4: `rust/catacomb_core`

A `cdylib` that reuses Catacomb's **pure** modules verbatim (`vtt`,
`error_class`, `platform`) via `#[path]` includes — no logic fork — and exposes
them to Kotlin over JNI. See [`rust/catacomb_core/src/lib.rs`](rust/catacomb_core/src/lib.rs).

Exposed entry points (all String-in / String-out), bound in
[`app/src/main/java/com/catacomb/spike/RustCore.kt`](app/src/main/java/com/catacomb/spike/RustCore.kt):

| Kotlin | Returns |
|--------|---------|
| `RustCore.vttParse(vtt)` | JSON `[{start, text}, …]` |
| `RustCore.classifyError(log)` | JSON `{class, label, hint}` |
| `RustCore.platformFromUrl(url)` | JSON `{dir_name, display_name, icon}` |
| `RustCore.platformDirName(url)` | plain folder name, e.g. `channels` |

### Build the native libs

```bash
cd rust/catacomb_core
./build.sh                 # arm64-v8a + x86_64 (release) → app/src/main/jniLibs/<abi>/
./build.sh arm64-v8a       # single ABI
API=24 ./build.sh          # override min-API linker (default 24)
```

The script locates the NDK from `$ANDROID_NDK_HOME`, `$ANDROID_NDK_ROOT`, or the
newest `ndk/<ver>` under `$ANDROID_HOME` / `$ANDROID_SDK_ROOT` / `~/Android/Sdk`.
Only the target linker is configured — every dependency (`serde`, `serde_json`,
`jni`) is pure Rust, so there's no C toolchain to set up.

Outputs (git-ignored — rebuild rather than commit binaries):
`app/src/main/jniLibs/{arm64-v8a,x86_64}/libcatacomb_core.so`.

### Verify

```bash
cd rust/catacomb_core

# 1. Logic reachable through the crate (runs the shared modules' own tests too):
cargo test

# 2. Exported JNI symbols in the shipped (arm64) .so:
NDK=~/Android/Sdk/ndk/26.3.11579264
$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf --dyn-syms \
  target/aarch64-linux-android/release/libcatacomb_core.so | grep Java_com_catacomb
# → Java_com_catacomb_spike_RustCore_{vttParse,classifyError,platformFromUrl,platformDirName}
```

An end-to-end JVM round-trip (build a host `.so`, `System.load` it, call each
native method) was used during development to confirm the bridge works at
runtime, not just that the symbols resolve.

## Notes

- The JNI method names in `RustCore.kt` **must** match the Rust
  `Java_com_catacomb_spike_RustCore_<name>` exports exactly.
- The `.so` is built with `panic = "abort"` and every entry point wraps its body
  in `catch_unwind`, so a Rust panic can never unwind into the JVM (UB).
