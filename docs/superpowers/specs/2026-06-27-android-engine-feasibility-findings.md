# Android Engine Feasibility — Findings

Companion to `2026-06-27-android-engine-feasibility-research.md`.
Each section ends with a verdict: **proven / risky / unsolved**.

## Q1 — yt-dlp on Android

**Verdict: RISKY.** yt-dlp *runs* on-device in shipping apps (Seal, YTDLnis,
SealPlus) — proven. But Catacomb's specific need is the asterisk: **`curl_cffi`
TLS impersonation is documented broken on Android** (yt-dlp issues #15505 Jan
2026 and #14106 aarch64 Linux), and impersonation is the exact reason Catacomb pins
nightly yt-dlp. The yt-dlp *binary* update story is good — **youtubedl-android**
(yausername) exposes `UpdateChannel.NIGHTLY` to pull fresh builds post-install,
no app-store release, across all four ABIs. Chaquopy is ruled out: no runtime
update path.

- **Recommended mechanism:** youtubedl-android (only approach with runtime
  nightly updates + production proof).
- **Risk carried forward:** the anti-bot layer (impersonation + the Deno-based
  bgutil POT loopback, which has no Android port) needs a rethink — feeds Q2/Q3.
- Full evidence + sources: `q1-yt-dlp-android-report.md`.

## Q2 — JS runtime (deno's job)

**Verdict: RISKY.** An external JS runtime is now a **hard dependency** —
yt-dlp **deprecated its built-in Python jsinterp for YouTube in 2025.11.12**
because YouTube's n-sig/PO-token JS exceeds what it can handle, so "no Deno on
Android" cannot be sidestepped by the pure-Python path. A concrete on-device
path exists: cross-compile **QuickJS-ng** via NDK (arm64-v8a, NDK 26+) and pass
`--js-runtimes quickjs:/path/to/qjs`; or use **HLahwani/yt-dlp-android** (May
2026), a purpose-built lib already embedding QuickJS via JNI across all four
ABIs (~1.6 MB AAR). It's "risky" because this is the live breakage frontier —
Seal users have hit 403/missing-format bugs since Jan 2026 on the deprecated
path.

- **Recommended approach:** QuickJS-ng (`--js-runtimes quickjs:...`), or adopt
  HLahwani/yt-dlp-android which bundles it.
- **Note vs Q1:** this newer lib may supersede youtubedl-android as the
  mechanism since it solves Q1+Q2 together — weigh in synthesis.
- Full evidence + sources: `q2-js-runtime-android-report.md`.

## Q3 — POT / Proof-of-Origin

**Verdict: RISKY** (the go/no-go gate — and it does NOT block).
**POT is required** on the mobile surface; you can't reliably skip it for a
general archiver. The shortcut clients are closing: `android_vr` dropped to
360p (SABR A/B, Mar 2026) and `tv` was removed from yt-dlp defaults after
YouTube enforced LOGIN_REQUIRED. **But there is a production-proven path that is
strictly better than Catacomb's desktop design:** YTDLnis (since v1.8.3, Mar
2025) runs YouTube's BotGuard JS challenge **inside the Android WebView** and
passes the resulting web GVS PO token to yt-dlp as an extractor arg. This
**collapses Q2 and Q3 into one mechanism** — the WebView *is* the JS runtime —
and **eliminates the Deno/Node sidecar entirely**.

- **Recommended anti-bot approach:** WebView-based BotGuard/POT generation (à la
  YTDLnis), no sidecar.
- **Caveats:** tokens are video-bound + ~12h-lived (a WebView attestation call
  per download); native `android`-client DroidGuard tokens remain **unsolved**
  (no open tooling outside genuine Play Services). Fragile cat-and-mouse — must
  track YouTube changes.
- **Flagged-for-device:** actual token minting + a real download need a device/
  WebView/Google session; not reproducible headlessly here.
- Full evidence + sources: `q3-pot-android-report.md`.

## Q4 — Rust core via JNI

**Verdict: PROVEN (toolchain), with a verification caveat on the binding layer.**
The controller ran a real cross-compile build-proof here (not recalled): the
crate's pure `vtt.rs` module, vendored into a minimal `cdylib`, compiled with
NDK r26d to **both** `aarch64-linux-android` and `x86_64-linux-android` `.so`
files for Android 34, each exporting the expected C-ABI symbol
(`catacomb_spike_cue_count`, defined `T`). A matching NDK-clang Android client
harness also built. So Rust-module → Android `.so` is demonstrably real.

- **Module triage (controller, from the crate):** cleanly portable — `vtt`
  (pure std), `autotag` (pure arithmetic), `error_class`/`platform` (serde
  only), `library` (effectively). Portable-with-effort — `database`
  (rusqlite/r2d2; SQLite runs on Android). Not a clean port — `fingerprint`
  (spawns `ffmpeg`; needs ffmpeg-kit). The download engine modules
  (`downloader`, `ytdlp_bin`, `pot_provider`) stay off-device by design.
- **Recommended toolchain:** `cargo-ndk` + **uniffi** (Kotlin codegen) over the
  raw `jni` crate — avoids JNI signature drift past ~10 functions. *(Toolchain
  recommendation is from desk research that hit a web rate-limit and leans on
  knowledge through 2025-08; treat the uniffi-vs-jni specifics as
  documented-but-verify. The build-proof above is hard evidence.)*
- **Reuse vs reimplement:** reuse the pure modules. Effort ~5 dev-days for a
  first `.so` exposing 5–6 modules to Kotlin.
- **Run-proof status:** on-device dlopen+call was **blocked by the sandbox
  reaping the emulator** (a harness limitation, not an Android/Rust one);
  downgraded per plan to symbol-export verification, which passed. The full
  dlopen+invoke is deferred to the Stage-1 prototype.
- Full evidence + sources: `q4-rust-jni-android-report.md`.

## Synthesis — go / no-go

_pending_
