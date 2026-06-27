# Q2: JavaScript Runtime (Deno's Job) on Android — Research Report

**Spike:** Catacomb Android feasibility — can a phone-only user run yt-dlp on-device?
**Prior finding (Q1):** Deno has NO official Android (aarch64-linux-android) build.
**Date:** 2026-06-27

---

## 1. What JS interpreters does yt-dlp support, and which work on Android?

### Supported interpreters (as of yt-dlp 2025.11.12+)

yt-dlp's EJS (External JavaScript) system supports exactly five runtimes, in priority order:

| Runtime | Min version | Default? | Android feasibility |
|---|---|---|---|
| Deno | 2.0.0 | YES | No official Android build (Q1 finding) |
| Node.js | 20.0.0 | No (security) | No official Android CLI binary |
| Bun | 1.0.31 | No (security); deprecated after 1.3.14 | No Android build |
| QuickJS | 2023-12-9 (2025-4-26+ for perf) | No (security) | **Buildable via NDK; see below** |
| QuickJS-ng | 0.12.0+ | No (security) | **Official aarch64 Linux binary + NDK cross-compile path** |

PhantomJS is still used by a handful of non-YouTube extractors but is planned for deprecation and is not relevant here.

Source: [EJS Wiki — yt-dlp](https://github.com/yt-dlp/yt-dlp/wiki/EJS)
Source: [Issue #15012 — External JS runtime now required](https://github.com/yt-dlp/yt-dlp/issues/15012)
Source: [Issue #14404 — Upcoming requirements announcement](https://github.com/yt-dlp/yt-dlp/issues/14404)

### Enabling a non-default runtime

Pass `--js-runtimes quickjs` or `--js-runtimes quickjs:/path/to/qjs` on the yt-dlp command line. The `qjs` executable must be discoverable in PATH or its full path specified. yt-dlp also needs the companion `yt-dlp-ejs` package, which is bundled in official executables and PyInstaller builds.

Source: [EJS DeepWiki](https://deepwiki.com/yt-dlp/yt-dlp-wiki/3.3-external-javascript-(ejs)-setup)

---

## 2. What do real Android apps actually use?

### 2a. youtubedl-android (yausername)

youtubedl-android bundles a compiled Python 3.8 + yt-dlp lazy-extractor binary for arm64-v8a, armeabi-v7a, x86, and x86_64. Its README **does not mention** QuickJS, an external JS engine, or EJS support — the bundled yt-dlp version appears to rely on the built-in Python jsinterp path.

Multiple open issues in the Seal app (which consumes youtubedl-android) filed in January–February 2026 report:

> "No supported JavaScript runtime could be found. Only deno is enabled by default; to use another runtime add --js-runtimes RUNTIME[:PATH] to your command/config"

followed by HTTP 403 errors from YouTube. These are unresolved as of the report date, with no maintainer comment addressing a fix.

Sources:
- [youtubedl-android README](https://github.com/yausername/youtubedl-android/blob/master/README.md)
- [Seal issue #2413](https://github.com/JunkFood02/Seal/issues/2413)
- [Seal issue #2428](https://github.com/JunkFood02/Seal/issues/2428)
- [Seal issue #2305 — Will Seal implement a JS runtime?](https://github.com/JunkFood02/Seal/issues/2305)
- [Seal issue #2395 — Problem with JavaScript](https://github.com/JunkFood02/Seal/issues/2395)

**Status: BROKEN for YouTube on current yt-dlp + youtubedl-android without a JS runtime.**

### 2b. QuickJS on Android — availability and yt-dlp driveability

**QuickJS-ng** has documented Android NDK support. The CMake cross-compile invocation is:

```
cmake -B build \
  -DCMAKE_TOOLCHAIN_FILE=$ANDROID_NDK/build/cmake/android.toolchain.cmake \
  -DANDROID_ABI=arm64-v8a \
  -DANDROID_PLATFORM=android-33
cmake --build build
```

NDK 26.0.10792818 or later is required. The result is a `qjs` ELF binary targeting `aarch64-linux-android`. Official QuickJS-ng releases (latest v0.15.1, June 4 2026) ship prebuilt binaries for Linux arm64 (aarch64); **no dedicated Android binary is released**, but the Linux aarch64 build is functionally compatible with Android's Linux kernel ABI when the binary is placed in an app's native library directory and executed as a child process.

Once a `qjs` binary exists on-device, yt-dlp can be pointed to it with `--js-runtimes quickjs:/data/data/<pkg>/nativeLibDir/qjs`. This is the most viable path for Android.

Sources:
- [QuickJS-ng cross-compilation docs](https://www.mintlify.com/quickjs-ng/quickjs/platforms/cross-compilation)
- [QuickJS-ng releases](https://github.com/quickjs-ng/quickjs/releases)
- [yt-dlp QuickJS issue #14431](https://github.com/yt-dlp/yt-dlp/issues/14431)

**Additionally:** there is a purpose-built Android library, **HLahwani/yt-dlp-android** (v1.0.0, May 2026), which embeds QuickJS via Android JNI (NDK) to handle YouTube n-parameter/signature decryption — AAR size ~1.6 MB, covers arm64-v8a, armeabi-v7a, x86_64, x86. This library takes a different architecture from youtubedl-android: instead of bundling the yt-dlp Python binary and shelling out to Python, it reimplements the extraction algorithm in Kotlin/C using QuickJS via JNI, with no WebView required.

Source: [HLahwani/yt-dlp-android](https://github.com/HLahwani/yt-dlp-android)

**Several older QuickJS Android binding libraries** also exist (OpenQuickJS/quickjs-android, taoweiji/quickjs-android, seven332/quickjs-android, dokar3's Kotlin multiplatform binding) but none are purpose-built for yt-dlp JavaScript challenge solving.

### 2c. System Android WebView as JS interpreter

The Android WebView (Blink/V8 under the hood) is NOT used as a yt-dlp interpreter. yt-dlp shells out to an external process (`qjs`, `deno`, `node`) — it cannot call into a WebView. Apps like NewPipe use a pure-Java reimplementation of YouTube's extraction logic (NewPipeExtractor) and never touch yt-dlp or WebView for this purpose.

No yt-dlp issue or community discussion proposes WebView integration for the EJS path.

### 2d. J2V8 / embedded V8

J2V8 is an Android JNI wrapper around Node.js/V8, primarily used historically in React Native. It is not supported by yt-dlp's EJS system, which expects a CLI-invocable `qjs`/`node`/`deno`/`bun` executable, not a library. No evidence found of J2V8 being used as a yt-dlp backend on Android.

### 2e. ytdlp-jsc — embedded QuickJS as a Python plugin

A third-party yt-dlp plugin called **ytdlp-jsc** (by ahaoboy, December 2025) embeds QuickJS as a Python C extension inside a `pip install`-able package, eliminating the need for a separate `qjs` executable. It uses Rust + SWC for JS parsing and QuickJS for execution. It is installed to the yt-dlp plugins directory (`~/.yt-dlp/plugins/`). Benchmarks show it is ~1.44x slower than baseline QuickJS due to startup savings (27 ms vs Deno's 400 ms CLI startup). Platform support is documented for Linux/macOS/Windows; Android-specific builds are not mentioned but the approach (Python C extension with embedded QuickJS) is in principle portable if the Python environment can load native extensions.

Sources:
- [ytdlp-jsc DEV Community post](https://dev.to/ahaoboy/ytdlp-jsc-a-js-challenge-solver-without-runtime-dependencies-3ag3)
- [ahaoboy/ytdlp-ejs GitHub](https://github.com/ahaoboy/ytdlp-ejs)

---

## 3. yt-dlp's pure-Python jsinterp: does it handle current YouTube nsig WITHOUT an external runtime?

**Short answer: No. As of yt-dlp 2025.11.12, the built-in Python jsinterp is explicitly deprecated for YouTube and will not reliably solve modern nsig/n-parameter challenges.**

The timeline:

- **Pre-2025:** yt-dlp used `jsinterp.py` (a pure-Python JS interpreter) to extract and run YouTube's n-parameter decryption function. This was a fragile approach; recurring nsig failures are documented continuously from 2023 onward (issues #10455, #10617, #12398, #12746, #13249, #14707, #14745) spanning multiple player versions.

- **Mid-2025 (issue #14404):** yt-dlp team announced that "drastic changes on YouTube's end" make the native jsinterp solution insufficient. YouTube's JavaScript challenge complexity "far exceeds that of the native jsinterp module." An external JS runtime requirement was announced.

- **November 2025 (yt-dlp 2025.11.12, issue #15012):** External JS runtime support shipped. "Support for YouTube without a JavaScript runtime is now considered 'deprecated'." Without a runtime: limited format availability, severe restrictions for logged-in users, expected to worsen over time, potential complete incompatibility in the future.

The pure-Python jsinterp still exists in the codebase and handles some simpler cases (it manages ES5/ES6 arithmetic, control flow, array/string methods, scope chains), but it cannot handle:
- YouTube's current PO (Proof-of-Origin) token generation
- The n-parameter decipher functions in recent player versions
- Any YouTube feature that requires complex JS pattern matching beyond the interpreter's capabilities

**The "no Deno on Android" problem is NOT moot.** The Python jsinterp fallback produces 403 errors and missing formats on current YouTube, confirmed by real user reports in Seal (January–February 2026).

Sources:
- [Issue #15012](https://github.com/yt-dlp/yt-dlp/issues/15012)
- [Issue #14404](https://github.com/yt-dlp/yt-dlp/issues/14404)
- [jsinterp DeepWiki](https://deepwiki.com/yt-dlp/yt-dlp/5.4-javascript-interpreter)
- [nsig recurring failures tracker](https://errorism.dev/issues/yt-dlp-yt-dlp-youtube-native-nsig-extraction-failed-part-3)

---

## 4. Reliability: Is the JS-challenge path a recurring breakage point on mobile?

**Yes, demonstrably so.**

Evidence:
- The nsig challenge has broken repeatedly with every major YouTube player version since 2023 (at minimum 6 distinct player versions caused failures, documented in separate issues).
- Every time YouTube ships a new player obfuscation, yt-dlp maintainers must patch `jsinterp.py` OR the EJS scripts — and mobile apps that ship a fixed yt-dlp version are stranded until an update.
- The Seal app has at least 4 open issues (2378, 2395, 2405, 2413, 2428) filed in late 2025/early 2026 specifically about the JS runtime requirement causing YouTube failures. None are resolved.
- The HLahwani/yt-dlp-android library (May 2026) notes in its release that player version 25f11721 "uses string-table obfuscation requiring behavioral function discovery" — demonstrating that even a native QuickJS-based solution must continuously track player changes.
- On desktop, the workaround (update yt-dlp + install Deno) is straightforward. On Android, there is no equivalent user-facing update path for the JS runtime itself.

**Mobile is harder than desktop** because: (a) the JS runtime binary cannot be self-updated by the app easily, (b) there is no OS-level Deno/Node to fall back to, and (c) shipping a `qjs` binary inside an APK requires adding a native library build step to the project and managing version pinning.

Sources:
- [Multiple nsig extraction failures 2023-2025](https://github.com/yt-dlp/yt-dlp/issues/13249)
- [Issue #10617 — Broken formats and nsig](https://github.com/yt-dlp/yt-dlp/issues/10617)
- [Seal issues](https://github.com/JunkFood02/Seal/issues)

---

## Summary Comparison Table

| Approach | Android viable? | Effort | Risk |
|---|---|---|---|
| Deno bundled | No (no Android build) | — | Blocked |
| Node.js bundled | No (no Android CLI binary) | — | Blocked |
| yt-dlp pure-Python jsinterp | Partially (deprecated, breaks on modern YT) | None | High — broken now |
| QuickJS-ng NDK cross-compiled `qjs` binary in APK | Yes (proven buildable) | Medium (NDK build step, path plumbing into yt-dlp) | Medium — must track QuickJS-ng + yt-dlp-ejs versions |
| HLahwani/yt-dlp-android (QuickJS JNI, no yt-dlp Python binary) | Yes (May 2026, arm64+x86) | Low if replacing youtubedl-android | Medium — re-implements yt-dlp logic, tracks player changes |
| ytdlp-jsc pip plugin (embedded QuickJS C extension) | Unproven on Android | Medium (depends on Python C extension ABI) | Medium — no Android release, research needed |
| System WebView / J2V8 | Not applicable to yt-dlp EJS | — | Not viable |

---

## Sources

1. [yt-dlp EJS Wiki](https://github.com/yt-dlp/yt-dlp/wiki/EJS)
2. [Issue #15012 — External JS runtime now required](https://github.com/yt-dlp/yt-dlp/issues/15012)
3. [Issue #14404 — Upcoming new requirements for YouTube downloads](https://github.com/yt-dlp/yt-dlp/issues/14404)
4. [EJS DeepWiki](https://deepwiki.com/yt-dlp/yt-dlp-wiki/3.3-external-javascript-(ejs)-setup)
5. [jsinterp DeepWiki](https://deepwiki.com/yt-dlp/yt-dlp/5.4-javascript-interpreter)
6. [JavaScript Challenge Solving DeepWiki](https://deepwiki.com/yt-dlp/yt-dlp/3.4.2-javascript-challenge-solving)
7. [yausername/youtubedl-android](https://github.com/yausername/youtubedl-android)
8. [Seal issue #2305 — Will Seal implement a JS runtime?](https://github.com/JunkFood02/Seal/issues/2305)
9. [Seal issue #2395 — Problem with JavaScript](https://github.com/JunkFood02/Seal/issues/2395)
10. [Seal issue #2413 — No JS runtime found](https://github.com/JunkFood02/Seal/issues/2413)
11. [Seal issue #2428 — No supported JavaScript runtime](https://github.com/JunkFood02/Seal/issues/2428)
12. [HLahwani/yt-dlp-android — QuickJS JNI Android library](https://github.com/HLahwani/yt-dlp-android)
13. [OpenQuickJS/quickjs-android](https://github.com/OpenQuickJS/quickjs-android)
14. [QuickJS-ng releases (v0.15.1, June 2026)](https://github.com/quickjs-ng/quickjs/releases)
15. [QuickJS-ng cross-compilation docs](https://www.mintlify.com/quickjs-ng/quickjs/platforms/cross-compilation)
16. [yt-dlp QuickJS support issue #14431](https://github.com/yt-dlp/yt-dlp/issues/14431)
17. [ytdlp-jsc DEV Community](https://dev.to/ahaoboy/ytdlp-jsc-a-js-challenge-solver-without-runtime-dependencies-3ag3)
18. [ahaoboy/ytdlp-ejs](https://github.com/ahaoboy/ytdlp-ejs)
19. [Improving yt-dlp-ejs with Rust](https://dev.to/ahaoboy/improving-yt-dlp-ejs-with-rust-smaller-and-faster-5cnl)
20. [Hacker News: yt-dlp external JS runtime required](https://news.ycombinator.com/item?id=45898407)
21. [nsig extraction failures (recurring)](https://github.com/yt-dlp/yt-dlp/issues/13249)
22. [Issue #12398 — nsig extraction failed](https://github.com/yt-dlp/yt-dlp/issues/12398)
23. [Alpine Linux yt-dlp-ejs-rt-quickjs package](https://pkgs.alpinelinux.org/package/v3.23/community/aarch64/yt-dlp-ejs-rt-quickjs)
24. [yt-dlp #15197 — unable to find JS runtime](https://github.com/yt-dlp/yt-dlp/issues/15197)

---

## Verdict

**RISKY**

**Recommended JS-runtime approach:** Cross-compile **QuickJS-ng** for Android using the NDK (`ANDROID_ABI=arm64-v8a`, NDK 26+), bundle the resulting `qjs` binary in the APK's native libraries, and point youtubedl-android's yt-dlp invocation at it via `--js-runtimes quickjs:/path/to/qjs`. Alternatively, evaluate replacing youtubedl-android with **HLahwani/yt-dlp-android** (May 2026), which already embeds QuickJS via JNI for all four ABIs in a ~1.6 MB AAR.

The "no Deno on Android" problem is NOT moot: yt-dlp's built-in Python jsinterp was officially deprecated for YouTube in November 2025 (yt-dlp 2025.11.12) and produces 403 errors and missing formats on modern YouTube — confirmed by real user bug reports in the Seal Android app filed January–February 2026. A JavaScript runtime is now a hard dependency for reliable YouTube downloads, not an optimization. QuickJS-ng is the only supported yt-dlp runtime with a clear Android NDK cross-compilation path; it requires effort to build and bundle, but the path is technically proven.
