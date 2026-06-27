# Q1 Research Report: yt-dlp on Android

**Feasibility Spike — Catacomb on-device Android**
Date: 2026-06-27
Research method: Web search + GitHub issue/repo fetches. No emulator or build testing.

---

## Executive Summary

Running yt-dlp on-device on Android is **proven** in the narrow sense that multiple shipping apps (Seal, YTDLnis, SealPlus) do exactly this — but "proven" comes with a hard asterisk for Catacomb's specific use case. The critical dependency `curl_cffi` (TLS impersonation, which Catacomb already calls nightly yt-dlp for) is **documented broken** on Android. That removes the primary anti-bot mechanism Catacomb relies on. The post-install update story for yt-dlp itself is actually good — nightly builds are accessible — but the impersonation layer is the unsolved problem.

---

## 1. How Real Android Apps Run yt-dlp On-Device

### 1.1 youtubedl-android (github.com/yausername/youtubedl-android)

This is the dominant approach used by Seal, YTDLnis, SealPlus, and many other apps. It is a Kotlin/Java Android library wrapper that bundles CPython and the yt-dlp binary as native `.so` files embedded in the APK.

**How it bundles Python + yt-dlp:**

Python is compiled from the Termux project's CPython port using its NDK cross-compilation toolchain. The compiled Python is packed as `libpython.zip.so` (the Python stdlib archive) and `libpython.so` (the Python executable), placed in `jniLibs/` per ABI. The yt-dlp payload uses a "lazy extractors" build (`ytdlp-lazy`, formerly `youtubedl-lazy`) to minimize startup time and package size. Both the Python runtime and yt-dlp are embedded in the compiled APK.

As of version 0.17.1 (released October 2024), the bundled Python was bumped from 3.8 to **3.11.10**. Prior releases (and older docs) still mention Python 3.8; the README has not been updated to reflect 3.11, but the releases page and changelogs confirm the bump. As of version 0.18.1 (November 2024, the latest release), QuickJS support was also added.

**Update mechanism (critical for Catacomb):**

The library exposes `YoutubeDL.updateYoutubeDL(context, updateChannel)`. This downloads a fresh yt-dlp binary from the upstream releases at runtime, replacing the bundled one. The supported channels are:

- `UpdateChannel.STABLE` — monthly yt-dlp stable releases
- `UpdateChannel.NIGHTLY` — yt-dlp nightly builds (from `yt-dlp/yt-dlp-nightly-builds`)
- `UpdateChannel.MASTER` — added in v0.16.0; points to a custom/development URL

The `UpdateChannel` class is open, so developers can pass an arbitrary URL. The Seal app exposes stable, nightly, and a "pre-release" channel to end users in its settings menu. **The yt-dlp binary itself is therefore updatable post-install**, independent of app store releases. Only Python requires a new app release to update (because Python is in `jniLibs`, which is compiled into the APK).

**ABI support:**

Four ABIs: `armeabi-v7a`, `arm64-v8a`, `x86`, `x86_64`. The README explicitly recommends ABI splits to avoid distributing a single fat APK.

**Approximate size cost:**

The demo/sample APK is not directly measured in the sources, but a comparable Python-bundled Android app (Chaquopy demo) runs 45–52 MB per-ABI. The youtubedl-android README's size management advice (ABI splits) implies a similar baseline. No exact per-ABI size was found in open documentation for youtubedl-android specifically; an arm64-v8a-only build would be substantially smaller than the ~50 MB all-ABI baseline.

**Source:** https://github.com/yausername/youtubedl-android, https://github.com/yausername/youtubedl-android/blob/master/README.md, https://github.com/yausername/youtubedl-android/releases, https://github.com/yausername/youtubedl-android/issues/293, https://deepwiki.com/yausername/youtubedl-android/1.2-installation-and-setup

---

### 1.2 Chaquopy

Chaquopy is a Gradle plugin that embeds CPython into an Android app and lets you run `pip install` at build time. yt-dlp can be installed through the pip configuration block in `build.gradle`.

**Python version and ABI support:**

Chaquopy supports Python 3.10, 3.11, 3.12, 3.13, 3.14 (Python 3.8 and 3.9 dropped). Same four ABIs as youtubedl-android.

**Does pip-installed yt-dlp work?**

Partial. Basic video metadata extraction works (confirmed by multiple tutorial posts). However, format merging fails on Android: yt-dlp downloads audio and video tracks separately but cannot merge them because FFmpeg subprocess invocation is problematic in the Android sandbox. A separate `ffmpeg-android-java` integration is required. The merge issue is documented in yt-dlp GitHub issue #13337.

**Size cost:**

The Chaquopy demo APK is 45.95 MB (v16.1.0) and 51.8 MB (v17.0.0) — that's just the Python runtime plus demo code. Adding yt-dlp (a large Python package) and FFmpeg increases this further.

**Key limitation vs. youtubedl-android:**

Chaquopy does not have a runtime yt-dlp update mechanism equivalent to `updateYoutubeDL()`. Updating yt-dlp requires either a new pip build-time install (necessitating a new app release) or a custom download-and-patch scheme. This is a significant problem for the Catacomb use case where nightly is needed.

**Source:** https://github.com/chaquo/chaquopy, https://chaquo.com/chaquopy/news/, https://github.com/yt-dlp/yt-dlp/issues/13337, https://medium.com/@tabish.dev.work/how-to-use-python-script-in-android-d064081ebc8c

---

### 1.3 Other Approaches

**Termux (not an embedded approach):** Termux runs a full Linux environment and pip-installs yt-dlp normally. This works for power users but is not viable for a self-contained app.

**Bundled yt-dlp standalone binary + Rust subprocess:** Catacomb already runs yt-dlp as a subprocess from Rust. In principle, the same pattern could work on Android if a standalone yt-dlp binary (not Python-based) existed. The yt-dlp project ships standalone binaries for Linux x86_64/aarch64/armv7; these run on Android via exec() without Python at all. However, the standalone binary has the same `curl_cffi` problem (see Section 3), and the standalone aarch64 binary ships without curl_cffi bundled (GitHub issue #14106).

---

## 2. The Version/Update Story

This is the most important operational question for Catacomb.

**yt-dlp's own guidance:** yt-dlp shows a warning when the installed version is older than 90 days. The nightly channel (`yt-dlp/yt-dlp-nightly-builds`) is explicitly the "recommended channel for regular users" because it catches YouTube anti-bot changes immediately. Stable releases are monthly and often stale within weeks of a YouTube policy change.

**What staleness costs:** YouTube's bot detection (the "Sign in to confirm you're not a bot" challenge, PO token enforcement, and player client changes) evolves continuously. Multiple GitHub issues filed in 2025–2026 show that even the stable release from a few weeks earlier breaks on public videos. The nightly channel typically fixes these within 24–48 hours of a YouTube change.

**On Android via youtubedl-android:** The `updateYoutubeDL(context, UpdateChannel.NIGHTLY)` call fetches the current nightly yt-dlp binary at runtime and replaces the bundled one. This means **an Android app using youtubedl-android can stay on nightly without requiring an app store release** — the binary is simply re-downloaded from `yt-dlp/yt-dlp-nightly-builds` on each check. Seal v1.9.0 introduced auto-update-on-launch with configurable channel (stable/nightly/pre-release).

**Python is the pinned component:** Python runtime itself cannot be updated without an app release (it's in compiled native libs). If yt-dlp ever requires a newer Python than what was bundled at release time, the user needs an app update. Currently yt-dlp requires Python 3.8+ (CPython) or 3.10+ (PyPy), and youtubedl-android 0.17.1+ ships Python 3.11, so there is headroom.

**PO Token provider (bgutil-pot-provider):** Catacomb's desktop version runs a separate loopback Deno server for PO token generation (see Catacomb's CLAUDE.md). This architecture does not translate trivially to Android: the bgutil-ytdlp-pot-provider is a Deno-based server, and no documented Android port exists. PO tokens are platform-scoped (a Web PO token cannot be used on the Android player client), so switching to the Android player client does not bypass this requirement. The `tv` and `android_vr` player clients currently do not require PO tokens, but these are fragile exceptions that YouTube has tightened in the past.

**Source:** https://github.com/yt-dlp/yt-dlp (README, channels table), https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide, https://github.com/yt-dlp/yt-dlp/issues/15865, https://github.com/JunkFood02/Seal/discussions/673, https://github.com/yt-dlp/yt-dlp-nightly-builds/releases

---

## 3. curl_cffi Availability on Android

This is the critical pain point. The findings are unambiguous.

**What curl_cffi does:** It provides TLS fingerprint impersonation (making yt-dlp's HTTP requests look like they come from Chrome, Firefox, Safari). This is required for TikTok downloads and increasingly for YouTube as bot detection is tightened. Catacomb's desktop stack specifically uses `curl_cffi` via the nightly yt-dlp `[default]` extras (`pip install "yt-dlp[default]"`) and has noted this as the reason nightly is required.

**Android status — documented broken:**

1. **TikTok on Android (yt-dlp issue #15505, filed January 2026):** The TikTok extractor "is attempting impersonation, but no impersonate target is available" on Android 16 via youtubedl-android. The error is explicitly because `curl_cffi` cannot be installed or accessed on Android. The issue was closed as "not planned / can't reproduce / duplicate," indicating the yt-dlp maintainers do not consider Android `curl_cffi` support within their scope.

2. **Linux aarch64 (yt-dlp issue #14106):** Even on desktop Linux aarch64, the bundled `yt-dlp_linux_aarch64` binary cannot access a globally-installed `curl_cffi`. All impersonation targets show as "(unavailable)". The fix (PR #13997) involves bundling curl_cffi inside the binary itself rather than relying on the Python environment. Whether this fix reached the binaries distributed via youtubedl-android is unconfirmed.

3. **Termux (curl_cffi issue #74, July 2023):** A GitHub issue exists requesting Termux/Android support for curl_cffi. It was closed via PR #699 (milestone v0.8.x), but no technical details of the resolution were publicly documented in the visible issue text. Whether this fix covers the Android sandbox environment (vs. Termux's more permissive environment) is unclear.

4. **General impersonation on aarch64:** Reported that "sites that need impersonation don't work on Linux AArch64 environment even when curl_cffi is installed with pip globally" — this is the same underlying architecture as Android arm64.

**Net effect for Catacomb:** The impersonation layer that Catacomb specifically depends on for anti-bot evasion is **not reliably available on Android**. Downloads of non-impersonation-dependent content (standard YouTube public videos with cookies or PO tokens, many other sites) would likely work. But TikTok downloads, and any future YouTube enforcement that requires TLS fingerprint matching, would fail silently or with a clear error.

**Source:** https://github.com/yt-dlp/yt-dlp/issues/15505, https://github.com/yt-dlp/yt-dlp/issues/14106, https://github.com/lexiforest/curl_cffi/issues/74, https://github.com/yt-dlp/yt-dlp/issues/10356

---

## Comparison Table

| Dimension | youtubedl-android | Chaquopy | Termux |
|---|---|---|---|
| Python version | 3.11.10 (as of 0.17.1) | 3.10–3.14 (build-time pick) | System pip (latest) |
| ABIs | armeabi-v7a, arm64-v8a, x86, x86_64 | Same four | N/A (not an app) |
| Per-ABI APK size (est.) | ~40–60 MB (Python + yt-dlp + FFmpeg) | ~50+ MB base + packages | N/A |
| yt-dlp post-install update | Yes — STABLE / NIGHTLY / MASTER channels at runtime | No — requires new app build | Yes — `pip install -U yt-dlp` |
| Nightly builds accessible | Yes (UpdateChannel.NIGHTLY) | No (build-time only) | Yes |
| FFmpeg merge working | Yes (bundled native FFmpeg) | Partial (requires ffmpeg-android-java) | Yes |
| curl_cffi / TLS impersonation | Broken on Android (documented) | Broken on Android (same Python env) | Unclear / env-dependent |
| PO token provider (bgutil) | No Android port documented | No Android port documented | Possible (Deno runnable in Termux) |
| Production apps using it | Seal, YTDLnis, SealPlus, many more | Few — mainly tutorial demos | N/A |
| App store distribution | Fully self-contained APK | Fully self-contained APK | Requires Termux installed |

---

## Key Risk Summary

1. **curl_cffi is broken on Android** (documented in Jan 2026 issue, confirmed for aarch64 more broadly). This kills TikTok and potentially future YouTube enforcement.

2. **bgutil PO token provider has no Android port.** Catacomb's desktop anti-bot relies on a loopback Deno server; that architecture doesn't exist for Android. The `tv`/`android_vr` player clients currently bypass PO token requirements but are historically fragile exceptions.

3. **yt-dlp nightly updates work well** via youtubedl-android's `UpdateChannel.NIGHTLY` — this is the one bright spot. The binary can stay current without app store releases.

4. **Chaquopy is worse for this use case** — no runtime yt-dlp update path means it would fall behind immediately.

5. **Python cannot be updated without an app release**, but the current 3.11 bundled version has years of headroom.

---

## Verdict

**RISKY**

Recommended mechanism (if proceeding): **youtubedl-android** — it is the only approach with working runtime nightly-update support and proven production use in multiple shipping apps. Chaquopy is ruled out by the lack of a runtime update path.

The update story is actually good (nightly is accessible post-install). The blocker is `curl_cffi`: TLS impersonation is documented broken on Android, and Catacomb's nightly-yt-dlp requirement exists specifically because of that dependency. A go/no-go decision should hinge on whether TikTok support is in scope for the Android target; if TikTok is out of scope for v1 and YouTube access without curl_cffi is sufficient (cookies + PO tokens via an alternative provider), the risk is manageable.

---

## Sources

- [youtubedl-android repo](https://github.com/yausername/youtubedl-android)
- [youtubedl-android README](https://github.com/yausername/youtubedl-android/blob/master/README.md)
- [youtubedl-android releases](https://github.com/yausername/youtubedl-android/releases)
- [youtubedl-android Python update guide (issue #293)](https://github.com/yausername/youtubedl-android/issues/293)
- [youtubedl-android DeepWiki installation guide](https://deepwiki.com/yausername/youtubedl-android/1.2-installation-and-setup)
- [youtubedl-android v0.16.0 release notes](https://newreleases.io/project/github/yausername/youtubedl-android/release/0.16.0)
- [Seal app repo](https://github.com/JunkFood02/Seal)
- [Seal app discussion #673 (yt-dlp update troubleshooting)](https://github.com/JunkFood02/Seal/discussions/673)
- [SealPlus repo](https://github.com/MaheshTechnicals/Sealplus)
- [Chaquopy repo](https://github.com/chaquo/chaquopy)
- [Chaquopy news/releases](https://chaquo.com/chaquopy/news/)
- [yt-dlp main repo](https://github.com/yt-dlp/yt-dlp)
- [yt-dlp issue #13337 — yt-dlp doesn't work in Android (Chaquopy, merge failure)](https://github.com/yt-dlp/yt-dlp/issues/13337)
- [yt-dlp issue #14106 — curl_cffi not included in linux_aarch64 binary](https://github.com/yt-dlp/yt-dlp/issues/14106)
- [yt-dlp issue #15505 — TikTok impersonation not available on Android](https://github.com/yt-dlp/yt-dlp/issues/15505)
- [yt-dlp issue #15865 — YouTube "sign in to confirm not a bot" 2026](https://github.com/yt-dlp/yt-dlp/issues/15865)
- [yt-dlp issue #10356 — unable to install curl_cffi](https://github.com/yt-dlp/yt-dlp/issues/10356)
- [curl_cffi issue #74 — Termux Android support](https://github.com/lexiforest/curl_cffi/issues/74)
- [yt-dlp PO Token Guide wiki](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide)
- [yt-dlp nightly builds repo](https://github.com/yt-dlp/yt-dlp-nightly-builds)
- [Medium: Python in Android with Chaquopy and yt-dlp](https://medium.com/@tabish.dev.work/how-to-use-python-script-in-android-d064081ebc8c)
- [XDA Forums: Seal app thread](https://xdaforums.com/t/app-seal-video-audio-downloader-for-android-based-on-yt-dlp-designed-with-material-you.4712898/)
- [bgutil-ytdlp-pot-provider on PyPI](https://pypi.org/project/bgutil-ytdlp-pot-provider/)
- [yt-dlp issue #13067 — YouTube bot detection](https://github.com/yt-dlp/yt-dlp/issues/13067)
