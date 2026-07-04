# Catacomb Android

The Catacomb Android app: a Compose UI over a **bundled on-device yt-dlp engine**
(Python + yt-dlp + ffmpeg + aria2c via [youtubedl-android]) plus a **shared Rust
core** (`vtt`, `error_class`, `platform`) called over JNI.

## What lives here

- **`app/`** — the Gradle app (Kotlin + Jetpack Compose + Material3). The real,
  installable app.
- **`rust/catacomb_core/`** — the Rust `cdylib` (Phase 4) reused verbatim from the
  desktop's pure modules, cross-compiled to a JNI `.so`. See its own README.
- **`demo/`** — a tiny dependency-free Java app that only exercises the Rust core
  (a minimal, Gradle-free reference; predates the full app).

## Features

- **Bundled yt-dlp** — real on-device download engine, no server needed. Extracts
  its Python environment on first launch.
- **Intuitive navigation** — bottom navigation bar: **Download**, **Files**,
  **Settings**.
- **All desktop themes** — the 19 palettes from `src/theme.rs` (Dark, Light,
  Dracula, Trans, the Emo/goth set, Cyberpunk, Synthwave, Vaporwave, Nord,
  Gruvbox, Tokyo Night, Paper, Honey, Candlelight) selectable as live swatches;
  the choice is persisted.
- **Live platform detection** — as you type a URL, the Rust core resolves its
  platform (chip shows icon + name + folder).
- **Improved progress indicator** — animated determinate bar with %, ETA, live
  yt-dlp status line, success/error end states, plus a scrollable log.

## Build

```bash
./build-apk.sh      # -> app/build/outputs/apk/debug/app-debug.apk  (~130 MB)
```

`build-apk.sh`:
- runs Gradle on a JDK <= 21 (the system JDK may be newer and break R8/d8;
  override with `TOOL_JAVA_HOME`),
- builds the Rust native libs first if missing (`rust/catacomb_core/build.sh`),
- writes `local.properties` (SDK path) and a stable `debug.keystore`,
- runs `./gradlew assembleDebug`.

Toolchain (pinned): Gradle 8.9 (wrapper), AGP 8.5.2, Kotlin 1.9.24, Compose
compiler 1.5.14, Compose BOM 2024.06.00, compileSdk 34, minSdk 24. ABIs:
arm64-v8a (devices) + x86_64 (emulator) — the intersection of what the Rust core
and youtubedl-android ship.

## Install & run

```bash
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n com.catacomb.spike/.MainActivity
```

Verified on an Android 14 (API 34) x86_64 emulator: installs, launches, the
bundled engine initialises ("yt-dlp ready"), the Rust `.so` loads (live platform
detection), theme switching re-themes the whole app and persists across restart,
and all three screens render.

## Status vs the Stage-1 plan

| Phase | What | Status |
|-------|------|--------|
| 1 | App shell + yt-dlp-android integration | Done (this app) |
| 2 | WebView-BotGuard POT generation | Not done — needs a real device + Google session |
| 3 | Token passing + anti-bot test | Not done — device-only |
| 4 | Rust core -> Android `.so` via JNI | Done (`rust/catacomb_core`) |
| 5 | On-device Go/No-Go | Partial — app verified on emulator; anti-bot download test is device-only |

The engine is bundled and initialises on-device; a *successful YouTube download*
still depends on the anti-bot/POT question (Phases 2-3), which needs a physical
device with a signed-in Google session — the open item from the feasibility
research.

[youtubedl-android]: https://github.com/junkfood02/youtubedl-android
