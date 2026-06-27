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

_pending_

## Q4 — Rust core via JNI

_pending_

## Synthesis — go / no-go

_pending_
