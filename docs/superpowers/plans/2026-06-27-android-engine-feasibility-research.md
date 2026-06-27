# Android Engine Feasibility Research — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce an evidence-backed go/no-go decision on a *standalone*
on-device Android download engine for Catacomb (yt-dlp + JS runtime + POT),
plus a recommended next sub-project if "go".

**Architecture:** This is a **research spike**, not a feature build. Each task
investigates one question via desk research, performs a *cheap* emulator
reproduction where it materially reduces uncertainty, records evidence in a
findings file, and commits. The final task synthesizes a go/no-go verdict. No
product code, no UI, no shipped Rust JNI library.

**Tech Stack (under investigation, not committed):** Android SDK/NDK, the
`catacomb_test` emulator (API 34, x86_64), `adb`, Chaquopy / youtubedl-android,
an embeddable JS engine (QuickJS / WebView), Rust `aarch64-linux-android`
cross-compile + JNI.

## Global Constraints

- **Spec authority:** `docs/superpowers/specs/2026-06-27-android-engine-feasibility-research.md`. Every task traces to a research question (Q1–Q4) or the synthesis.
- **No product code.** No app, no Gradle product project, no UI, no shipped JNI lib. Throwaway experiment artifacts only, kept under `/tmp/.../scratchpad` or a clearly-marked `android-spike/` scratch dir — **not** committed into the crate.
- **Only the findings doc is committed** to the repo: `docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md`.
- **Android tooling env:** always export `JAVA_HOME=/usr/lib/jvm/java-17-openjdk` and `ANDROID_HOME=~/Android/Sdk` (system JDK 26 breaks AGP/Gradle). See the `android-emulator-env` memory.
- **Emulator:** AVD `catacomb_test`, reachable as `emulator-5554`. It gets reaped between turns — relaunch headless before any `adb` work (see Task 0).
- **Reproduce-where-cheap policy:** desk research is primary; run on-device proofs only when cheap and uncertainty-reducing; **flag** (do not attempt) anything needing the user's real device, a Google login, or live-YouTube behavior that won't reproduce headlessly.
- **Arch caveat:** emulator is x86_64; a real phone is aarch64. On-device runs prove *behavior*, not the aarch64 toolchain. Keep build-proof (aarch64 compiles) separate from run-proof (x86_64 executes).
- **Sandbox download corruption:** large SDK/NDK zips must be fetched via `curl` + manual extract, never bare `sdkmanager` (it truncates them).
- **Verdict vocabulary:** every question ends in exactly one of **proven / risky / unsolved**, with the evidence behind it.

---

### Task 0: Bootstrap the findings doc + confirm the emulator is live

**Files:**
- Create: `docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md`

**Interfaces:**
- Consumes: nothing.
- Produces: the findings file with one `## Q1`…`## Q4` + `## Synthesis`
  heading per later task to append under. Section headings are the contract
  every later task writes into.

- [ ] **Step 1: Relaunch the emulator headless and wait for full boot**

```bash
export ANDROID_HOME=~/Android/Sdk
export PATH="$ANDROID_HOME/platform-tools:$PATH"
pkill -f "avd catacomb_test" 2>/dev/null; sleep 1
nohup "$ANDROID_HOME/emulator/emulator" -avd catacomb_test \
  -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect \
  -no-snapshot -memory 2048 > /tmp/emu.log 2>&1 &
adb wait-for-device
until [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do sleep 2; done
adb shell getprop ro.build.version.sdk | tr -d '\r'
```

Expected: prints `34` once boot completes (usually < 60s with KVM).

- [ ] **Step 2: Create the findings skeleton**

```markdown
# Android Engine Feasibility — Findings

Companion to `2026-06-27-android-engine-feasibility-research.md`.
Each section ends with a verdict: **proven / risky / unsolved**.

## Q1 — yt-dlp on Android

## Q2 — JS runtime (deno's job)

## Q3 — POT / Proof-of-Origin

## Q4 — Rust core via JNI

## Synthesis — go / no-go
```

- [ ] **Step 3: Commit**

```bash
cd /home/luna/code/catacomb
git add docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md
git commit -m "docs(android-spike): scaffold feasibility findings doc"
```

---

### Task 1: Q1 — yt-dlp on Android

**Files:**
- Modify: `docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md` (the `## Q1` section)

**Interfaces:**
- Consumes: a booted emulator (Task 0).
- Produces: Q1 findings + verdict; specifically a named recommended mechanism
  ("Chaquopy" | "youtubedl-android" | "other") that Task 5 synthesis reads.

- [ ] **Step 1: Desk research — how real apps run yt-dlp on-device**

Investigate and capture, with source links:
- **youtubedl-android** (yausername) — the lib Seal/Tubular-adjacent apps use: how it bundles Python, how it ships/updates the yt-dlp payload (its `updateYoutubeDL`/channel mechanism).
- **Chaquopy** — embedding CPython + pip in an APK: what it costs (APK size, supported ABIs, Python version), and whether `pip`-installed yt-dlp works at runtime.
- **Seal** app specifically: which of the above it uses and any documented yt-dlp version-pinning.
Record: bundling mechanism, supported ABIs, approximate size cost.

- [ ] **Step 2: Capture the version/update story (Catacomb-specific worry)**

Catacomb desktop relies on **nightly** yt-dlp for working `curl_cffi`
impersonation (see `ytdlp_bin.rs`). Document: can an Android yt-dlp be updated
post-install (youtubedl-android's download channel? in-app pip?) or is it
pinned/stale, and what staleness costs for impersonation. State this explicitly
— a pinned-only answer is a partial-no-go signal.

- [ ] **Step 3: Cheap emulator repro — prove a Python yt-dlp can run on-device**

Goal: prove yt-dlp's Python actually imports/executes on Android, not just in
theory. Cheapest path that avoids a full Gradle app: push a CPython-for-Android
+ yt-dlp wheel via `adb` into an app-data dir, or run the youtubedl-android
sample. Minimum acceptable proof = `yt-dlp --version` (or `python -m yt_dlp
--version`) executing under Android and printing a version. Capture the exact
commands and output.

```bash
# illustrative shape — actual mechanism determined in Step 1:
adb shell "cd /data/local/tmp && ./python -m yt_dlp --version"
```

Expected: a yt-dlp version string printed from within the Android environment.
If this proves not-cheap (needs a full app build), STOP, document why, and mark
the repro "deferred to Stage-1 prototype" rather than sinking hours here.

- [ ] **Step 4: Write Q1 findings + verdict**

Append to the `## Q1` section: mechanism comparison table, the update-story
paragraph, the repro transcript (or the documented reason it was deferred), and
a one-line **verdict: proven / risky / unsolved** with the recommended
mechanism.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md
git commit -m "docs(android-spike): Q1 yt-dlp-on-Android findings + verdict"
```

---

### Task 2: Q2 — JS runtime (deno's job)

**Files:**
- Modify: `docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md` (the `## Q2` section)

**Interfaces:**
- Consumes: Q1's chosen yt-dlp mechanism (the JS runtime must integrate with it).
- Produces: Q2 findings + verdict; a named JS-runtime recommendation
  ("QuickJS" | "system WebView JS" | "embedded V8" | "none viable") that Task 5
  reads.

- [ ] **Step 1: Desk research — what JS interpreter yt-dlp uses on Android**

yt-dlp needs JS for nsig/signature/challenge solving (deno on desktop).
Investigate and capture, with links:
- yt-dlp's `--exec`/`jsinterp` and external-interpreter support: which interpreters it accepts (deno, node, quickjs?), and what youtubedl-android wires in.
- **QuickJS on Android** (e.g. quickjs-android bindings) vs. **system WebView** JS vs. **J2V8/embedded V8**: availability, size, and whether yt-dlp can drive them.
- Known fragility: is the JS-challenge path a recurring break point on mobile?

- [ ] **Step 2: Cheap emulator repro (if isolable)**

If yt-dlp's JS path can be exercised in isolation on-device (e.g. running its
bundled jsinterp against a sample nsig challenge), do it via `adb` and capture
output. If it can't be isolated without the full extractor flow, document
precisely why and what that implies — do not force it.

- [ ] **Step 3: Write Q2 findings + verdict**

Append to `## Q2`: the interpreter-options comparison, integration notes with
Q1's mechanism, the repro (or why-not), and **verdict: proven / risky /
unsolved** with the recommended runtime.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md
git commit -m "docs(android-spike): Q2 JS-runtime findings + verdict"
```

---

### Task 3: Q3 — POT / Proof-of-Origin (biggest unknown)

**Files:**
- Modify: `docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md` (the `## Q3` section)

**Interfaces:**
- Consumes: Q1/Q2 context (POT generation may need the JS runtime / a WebView).
- Produces: Q3 findings + verdict — the verdict that most strongly gates the
  overall go/no-go in Task 5.

- [ ] **Step 1: Desk research — POT on mobile**

Investigate and capture, with links:
- How `bgutil-ytdlp-pot-provider` / bgutil-pot work, and whether a **WebView-based** POT generation path exists for Android (no Node sidecar).
- Crucially: **does YouTube gate the mobile client surface the same way** the desktop clients Catacomb uses get gated? If mobile/tv/embedded clients aren't POT-gated, the desktop POT machinery may be unnecessary on-device — a materially simpler design. Cite yt-dlp issues / NewPipeExtractor discussion.
- What Seal/NewPipe do about POT today (or don't).

- [ ] **Step 2: Identify what needs the user's real device**

POT/login behavior likely won't reproduce headlessly. Explicitly list which
checks need the user's real device or a Google login, and frame them as
flagged-for-device rather than blockers on the spike.

- [ ] **Step 3: Write Q3 findings + verdict**

Append to `## Q3`: the POT-on-mobile landscape, the "is mobile gated like
desktop?" answer (the pivotal one), the flagged-for-device list, and **verdict:
proven / risky / unsolved**. Note explicitly how this verdict propagates to
synthesis.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md
git commit -m "docs(android-spike): Q3 POT/Proof-of-Origin findings + verdict"
```

---

### Task 4: Q4 — Rust core via JNI

**Files:**
- Modify: `docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md` (the `## Q4` section)
- Throwaway scratch only (NOT committed): an `android-spike/` dir under the scratchpad for the cross-compile experiment.

**Interfaces:**
- Consumes: the existing crate's module layout (`src/vtt.rs` is the chosen
  pure-logic probe).
- Produces: Q4 findings + verdict + a reuse-vs-reimplement recommendation and a
  per-module port-cleanliness list.

- [ ] **Step 1: Desk research + module triage**

Read the current crate to classify modules as **pure-logic (portable)** vs.
**desktop/subprocess-bound (not portable)**. Candidates to confirm portable:
`vtt`, `library`, `database`, `fingerprint`, `platform`, `error_class`. For each,
note dependencies that would block an Android `.so` (subprocess spawning, GTK/
egui, OS-specific calls). Capture the table.

- [ ] **Step 2: Install the NDK + Rust android targets (curl workaround if needed)**

```bash
export JAVA_HOME=/usr/lib/jvm/java-17-openjdk
export ANDROID_HOME=~/Android/Sdk
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --install "ndk;26.3.11579264" 2>&1 | tail -3
rustup target add aarch64-linux-android x86_64-linux-android
```

Expected: NDK present under `$ANDROID_HOME/ndk/`; both Rust targets installed.
If the NDK zip corrupts (sandbox bug), curl it from the repo manifest and
extract manually per the Global Constraints.

- [ ] **Step 3: Build-proof — cross-compile `vtt` to aarch64 (does the toolchain build?)**

In the throwaway scratch dir, create a minimal `cdylib` crate that depends on a
copy of `vtt.rs` (or a path-dep into the repo, read-only) exposing one JNI fn,
and build it for aarch64. This proves the *toolchain*, per the arch caveat.

```bash
cd /tmp/claude-1000/-home-luna-code-catacomb/<session>/scratchpad/android-spike
# configure CC/AR/linker from the NDK toolchain for the target, then:
cargo build --release --target aarch64-linux-android
ls target/aarch64-linux-android/release/*.so
```

Expected: a `.so` is produced for aarch64. Capture the command + result.

- [ ] **Step 4: Run-proof — call an x86_64 build over JNI on the emulator**

Build the same lib for `x86_64-linux-android`, push it + a tiny test harness via
`adb`, and invoke the JNI function on the emulator to prove the call path works
end-to-end. (x86_64 because that's the emulator ABI.) If a full JNI harness is
not-cheap, downgrade to: push the `.so` and confirm it `dlopen`s / `readelf`
shows the expected symbol, and document the JNI call as deferred.

```bash
adb push target/x86_64-linux-android/release/libcatacomb_spike.so /data/local/tmp/
adb shell "cd /data/local/tmp && readelf -d ./libcatacomb_spike.so | head"
```

Expected: the lib loads / exposes the symbol on-device. Capture output.

- [ ] **Step 5: Write Q4 findings + verdict**

Append to `## Q4`: the module port-cleanliness table, the aarch64 build-proof
result, the x86_64 run/load proof, the effort estimate, a **reuse-vs-reimplement
recommendation**, and **verdict: proven / risky / unsolved**.

- [ ] **Step 6: Commit (findings only; scratch dir stays out of git)**

```bash
cd /home/luna/code/catacomb
git status --short   # confirm only the findings doc is staged; no android-spike/ leaked in
git add docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md
git commit -m "docs(android-spike): Q4 Rust-core-via-JNI findings + verdict"
```

---

### Task 5: Synthesis — go/no-go + recommended next sub-project

**Files:**
- Modify: `docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md` (the `## Synthesis` section)

**Interfaces:**
- Consumes: the four verdicts (Q1–Q4).
- Produces: the spike's terminal deliverable — a go/no-go recommendation.

- [ ] **Step 1: Tabulate the four verdicts**

Append a table: Question | Verdict | One-line reason. Pull each verbatim from
Q1–Q4.

- [ ] **Step 2: Write the go/no-go recommendation**

Apply the decision logic from the spec:
- **Go (standalone engine)** only if Q1, Q2, Q3 are each at worst *risky* with a credible path, and Q3 (POT) isn't *unsolved*.
- **Partial / pivot** if Q3 is *unsolved* or yt-dlp can only be stale: recommend a **client-to-server app first** (reuses the existing web API per `remote.rs`), revisit standalone later. Name the exact blocking unknown.
- **No-go** if multiple cores are *unsolved*.
State the recommendation in one paragraph, then the **recommended next
sub-project** with a one-line scope (e.g. "Stage-1 engine prototype: Q1
mechanism + Q2 runtime" OR "Client-to-server Android app against the web API").

- [ ] **Step 3: Update roadmap pointer**

Add a one-line note under ROADMAP.md §3.2 pointing at the findings doc and the
verdict, so the roadmap reflects the decision.

```bash
# edit ROADMAP.md 3.2 to reference docs/.../findings.md and the go/no-go outcome
```

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-06-27-android-engine-feasibility-findings.md ROADMAP.md
git commit -m "docs(android-spike): synthesis — go/no-go + next sub-project; link from roadmap"
```

- [ ] **Step 5: Tear down the emulator**

```bash
pkill -f "avd catacomb_test" 2>/dev/null; echo "emulator stopped"
```

---

## Self-Review

**Spec coverage:**
- Q1 → Task 1 ✅; Q2 → Task 2 ✅; Q3 → Task 3 ✅; Q4 → Task 4 ✅.
- Spec "Deliverable" (findings file, per-question evidence, go/no-go, next
  sub-project) → Task 0 (scaffold) + Tasks 1–4 (evidence) + Task 5 (go/no-go +
  next) ✅.
- Spec "reproduce where cheap" → each repro step has an explicit "if not cheap,
  document and defer" escape ✅.
- Spec arch caveat (build-proof vs run-proof) → Task 4 Steps 3 & 4 are split
  exactly along that line ✅.
- Spec "success = even a well-evidenced no-go" → Task 5 Step 2 handles no-go /
  pivot explicitly ✅.
- Spec non-goal "no committed product code" → Global Constraints + Task 4 scratch
  dir kept out of git, with a `git status` guard ✅.

**Placeholder scan:** No TBD/TODO. Repro commands that depend on a Step-1 finding
(the yt-dlp bundling mechanism) are marked "illustrative shape — actual mechanism
determined in Step 1", which is honest for a research spike rather than a fake
concrete command. The `<session>` token in the scratch path is a real
filesystem placeholder the executor substitutes, not a planning gap.

**Type consistency:** Findings section headings (`## Q1`…`## Q4`, `## Synthesis`)
are defined once in Task 0 Step 2 and referenced identically by every later
task. Verdict vocabulary (proven/risky/unsolved) is uniform. AVD name
`catacomb_test`, target triples `aarch64-linux-android` / `x86_64-linux-android`,
and the findings filename are identical across all tasks.
