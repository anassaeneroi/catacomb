# Android Engine Feasibility — Research Spike

- **Date:** 2026-06-27
- **Roadmap item:** 3.2 Android client (first sub-project)
- **Type:** Research spike — produces a written feasibility report + minimal
  emulator reproductions. **No product code.**
- **Status:** Spec (approved design)

## Why this exists

The agreed goal for 3.2 is a **standalone** Android app: a phone-only user can
download videos *on the device*, not merely browse a home server. That moves the
hard problem from UI to engine: can Catacomb's download stack —
`yt-dlp` + a JS runtime (deno's role) + Proof-of-Origin (POT) — run on Android
at all?

The whole Android bet hinges on that answer. Building a Compose UI is wasted
effort if the engine can't run on-device. So before committing to any build
path (Chaquopy, JNI, a JS engine), this spike investigates what has actually
been solved by existing apps, reproduces the cheap parts on the emulator, and
returns a **go / no-go** recommendation plus, if "go", the recommended build
path for the *next* sub-project.

This is explicitly a research deliverable. The bias is desk research, with
on-device reproduction only where it is cheap and materially reduces
uncertainty (the "Reproduce where cheap" policy below).

## Non-goals

- No app, no UI, no Gradle product project, no Rust JNI library shipped.
- Not choosing UI tech (Compose vs. other) — that is a later sub-project.
- Not solving the engine — only determining whether/how it *can* be solved and
  at what cost.
- Not on-real-hardware validation of YouTube login/live flows — anything that
  needs the user's real device or account is flagged, not attempted.

## Environment (already set up)

See the `android-emulator-env` memory for the authoritative, reusable details.
Summary:

- SDK at `~/Android/Sdk`; use `JAVA_HOME=/usr/lib/jvm/java-17-openjdk` for all
  Android tooling (system default JDK 26 is too new for AGP/Gradle).
- AVD `catacomb_test` — Android 14 (API 34), **x86_64**, KVM-accelerated, boots
  headless in seconds. Reachable as `emulator-5554` via `adb`.
- Known gotcha: `sdkmanager` corrupts large downloads in this sandbox; large
  zips were installed via `curl` + manual extraction. Same workaround applies if
  more SDK packages are needed.

**Architecture caveat that shapes the JNI question:** the emulator is x86_64,
but a real phone is `aarch64`. On-device runs here validate *behavior* on
x86_64; they do **not** prove the `aarch64` toolchain. The JNI question must
therefore separately confirm an `aarch64-linux-android` *cross-compile builds*,
even when the artifact actually executed on the emulator is the x86_64 variant.

## Reproduction policy — "reproduce where cheap"

For each research question:

1. **Desk research** is the primary method: how do real apps (Seal,
   youtubedl-android, NewPipe/NewPipeExtractor, Tubular) and yt-dlp's own docs
   handle this?
2. **Reproduce on the emulator only when cheap** and it materially reduces
   uncertainty (e.g. proving yt-dlp's Python imports and runs; proving one Rust
   module loads over JNI).
3. **Flag, don't attempt**, anything requiring the user's real device, a Google
   login, or live-YouTube behavior that won't reproduce headlessly.

Each question ends with a verdict: **proven / risky / unsolved**, with the
evidence behind it.

## Research questions

### Q1 — yt-dlp on Android

- How do real apps run yt-dlp on-device? Compare **Chaquopy** (embedded CPython
  in the APK) vs. **youtubedl-android** (the library Seal/etc. use) vs. any other
  current approach.
- What is the **version/update story**? Catacomb's desktop relies on *nightly*
  yt-dlp for working impersonation; an Android app can't trivially `pip install
  --pre` at runtime. Document how (or whether) on-device yt-dlp gets updated, and
  what staleness costs.
- **Cheap repro target:** get yt-dlp's Python actually importing and executing on
  the emulator — even just `--version` or a metadata-only extract — to prove the
  Python path is real, not theoretical.
- **Verdict:** proven / risky / unsolved, with the recommended mechanism.

### Q2 — JS runtime (deno's job)

- yt-dlp needs a JS interpreter for nsig / signature / challenge solving (deno on
  desktop). What do Android apps use — **QuickJS**, **embedded V8/J2V8**, or the
  **system WebView**'s JS engine? Which does yt-dlp actually accept as its JS
  interpreter on Android?
- Is it reliable against YouTube's *current* challenges, or is it a known weak
  point that breaks periodically?
- **Cheap repro target:** if isolable, run yt-dlp's JS-interpreter path against a
  sample challenge on-device; otherwise document precisely why it can't be
  isolated and what that implies.
- **Verdict:** proven / risky / unsolved.

### Q3 — POT / Proof-of-Origin (biggest unknown)

- How, if at all, do Android apps generate Proof-of-Origin tokens? Is it a
  **WebView-based bgutil** approach, a different provider, or do mobile clients
  sidestep POT because YouTube gates them differently?
- Does YouTube gate the **mobile** client surface the same way the desktop
  clients Catacomb uses get gated? (If mobile clients aren't POT-gated, the
  desktop POT machinery may be unnecessary on-device — a materially different
  and simpler design.)
- **Cheap repro:** likely none without a real device/login — expect this to be
  mostly desk research with explicit flags for what needs the user's hardware.
- **Verdict:** proven / risky / unsolved. **This verdict most strongly gates the
  overall go/no-go.**

### Q4 — Rust core via JNI

- Effort and viability of compiling the shared, UI-free Rust modules
  (`database`, `library`, `vtt`, `fingerprint`, and helpers like `platform` /
  `error_class`) into an Android `.so` and calling them from Kotlin over JNI —
  **vs.** reimplementing that logic in Kotlin.
- Identify which modules port cleanly (pure logic) and which drag in
  desktop-only or subprocess-spawning dependencies that don't belong on-device.
- **Cheap repro target:** cross-compile **one** pure module (candidate: `vtt`,
  the small self-contained WebVTT/SRT parser) to `aarch64-linux-android` to prove
  the toolchain builds, and additionally run an x86_64 build of it over JNI on
  the emulator to prove the call path works end to end. (Per the architecture
  caveat, the build-proof and the run-proof are deliberately separate targets.)
- **Verdict:** proven / risky / unsolved, with a reuse-vs-reimplement
  recommendation.

## Deliverable

A single report. Findings are appended to this file under a "## Findings"
section (or a sibling `2026-06-27-android-engine-feasibility-findings.md` if it
grows large), containing:

- Per-question findings with evidence (links to the apps/docs surveyed, and any
  emulator command transcripts).
- An overall **go / no-go** recommendation for the standalone on-device engine.
- If **go**: the recommended build path and the scope of the *next* sub-project
  (e.g. "Stage-1 engine prototype: Chaquopy yt-dlp + the JS runtime chosen in
  Q2").
- If **no-go** or **partial**: the fallback (e.g. client-to-server app now,
  revisit standalone later) and exactly which unknown blocked it.

## Success criteria

The spike is done when all four questions carry a defensible verdict backed by
evidence, the cheap on-device reproductions have been attempted (or explicitly
documented as not-cheap/blocked), and the report states a clear go/no-go with a
recommended next sub-project. It does **not** require the engine to work — a
well-evidenced "no-go, here's why" is a successful spike.

## Risks / watch-outs

- **Emulator ≠ phone arch.** x86_64 runs prove behavior, not the `aarch64`
  toolchain; keep the JNI build-proof separate from the run-proof.
- **Live-YouTube flakiness.** Challenge/POT behavior changes often and may not
  reproduce headlessly or without a login; such findings are desk-research +
  flagged-for-device, not emulator-proven.
- **Nightly-yt-dlp dependency.** If on-device yt-dlp can only be a pinned/stale
  build, that is itself a partial-no-go signal worth surfacing prominently.
- **Sandbox download corruption.** Any extra SDK/NDK packages needed must use the
  curl + manual-extract workaround, not bare `sdkmanager`.
