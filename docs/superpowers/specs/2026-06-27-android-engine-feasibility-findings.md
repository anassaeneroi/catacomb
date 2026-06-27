# Android Engine Feasibility — Findings

Companion to `2026-06-27-android-engine-feasibility-research.md`.
Each section ends with a verdict: **proven / risky / unsolved**.

## Q1 — yt-dlp on Android

**Verdict: RISKY.** yt-dlp *runs* on-device in shipping apps (Seal, YTDLnis,
SealPlus) — proven. But Catacomb's specific need is the asterisk: **`curl_cffi`
TLS impersonation is documented broken on Android** (yt-dlp #15505 Jan 2026;
#14106 aarch64 Linux), and impersonation is the exact reason Catacomb pins
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

_pending_

## Q3 — POT / Proof-of-Origin

_pending_

## Q4 — Rust core via JNI

_pending_

## Synthesis — go / no-go

_pending_
