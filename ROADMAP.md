# Roadmap

## North star

**Surpass [Tartube](https://github.com/axcore/tartube) in every dimension.**

A structured analysis of Tartube's codebase, data model, operations, and
configuration surface lives at [`docs/tartube-spec.md`](docs/tartube-spec.md);
it's what the Phase 1 parity work traced back to.

Tartube is the mature open-source yt-dlp GUI and the obvious benchmark for a
project in this space. yt-offline has architectural advantages Tartube can't
catch up on quickly (Rust + axum + a real web UI + bundled toolchain + a
modern security model). **As of 2026-06 we're at feature parity** — every
Tartube subsystem is matched or led; the remaining gap is years-of-edge-
cases maturity. The plan below is now mostly "surpass" work.

## Current state vs Tartube (2026-06-07)

| Area | Us | Tartube | Verdict |
| --- | --- | --- | --- |
| yt-dlp wrapping | ✅ | ✅ | Tied |
| Multi-platform sources | ✅ first-class per-platform routing | ✅ generic | We lead |
| Web UI accessible from any device | ✅ | ❌ desktop only | We lead |
| Single-binary distribution | ✅ Rust binary + venv installer | ❌ Python+GTK deps | We lead |
| Per-distro packaging | ✅ .deb / .rpm / AppImage + PKGBUILD + CI | ✅ .deb .rpm .pkg.tar.zst | Tied |
| Security model (auth, CSP, rate-limit) | ✅ | ❌ never network-facing | We lead |
| Plex export with NFO sidecars | ✅ | ❌ | We lead |
| Anti-bot stack (impersonation + POT + nightly) | ✅ curl_cffi + bgutil-pot + nightly yt-dlp | ❌ user-installed | We lead |
| Cookie freshness / anonymous-jar warning | ✅ | ❌ | We lead |
| Auto-retry + adaptive throttle on rate-limit | ✅ | ❌ | We lead |
| Configurable YouTube player clients | ✅ global + per-channel | ❌ | We lead |
| Themes | ✅ 10 themes | ❌ GTK default | We lead |
| Live-stream recording | ✅ | ✅ recent | Tied |
| WebSocket real-time progress | ✅ | ❌ polling | We lead |
| Mobile-responsive web UI | ✅ | ❌ desktop only | We lead |
| Per-channel custom download options | ✅ JSON-blob overrides | ✅ deep | Tied |
| Subtitle controls (auto / embed / convert) | ✅ global + per-channel | ✅ | Tied |
| Folder/group hierarchy | ✅ N-level nesting | ✅ N-level | Tied |
| Filter UI (date / size / watched + presets) | ✅ chip-style + named presets | ✅ rich + presets | Tied |
| Format conversion / re-encode | ✅ remux / H.264 / audio post-pass | ✅ ffmpeg pipeline | Tied |
| Comments capture | ✅ `--write-comments` + viewer | ✅ raw JSON | We lead |
| System tray | ✅ ksni (Linux SNI) | ✅ GTK | Tied |
| Library backup + restore | ✅ DB snapshot + idempotent import | ✅ | Tied |
| Per-channel notes / annotations | ✅ searchable | ✅ | Tied |
| Error classification + suggested fixes | ✅ 9-class + hints | ✅ rescue recipes | Tied |
| Stability hardening | ✅ crash log + disk-full preflight + hang watchdog + poison-recover locks | partial | We lead |
| Desktop UI scale | ✅ global egui zoom, persisted | ✅ GTK native | Tied |
| Maturity / edge cases | ~months | ~years | **Tartube leads** |

Score: 14 ahead, 1 behind, 13 tied. **Phase 1 (Tartube parity) is
complete** — the only thing Tartube still leads on is years-of-edge-cases
maturity, which is time, not a feature. Everything below is "surpass"
territory plus test/doc investment.

## Phase 1 — COMPLETE

All Tartube parity items shipped. For the record, the last to land:

- **1.7 Format conversion** — post-download ffmpeg pass with three modes
  (remux→mp4, re-encode→H.264/AAC at a CRF, audio extraction), global
  `[convert]` config + per-channel override, keep-original toggle,
  surfaced as a distinct transcode job.
- **1.3+ Filter presets** — name a chip-filter set and re-apply it from
  the filter row; persisted in localStorage.

See **Recently shipped** for the full list.

## Phase 2 — Polish where Tartube is mature

Things we win on architecturally but lose on real-world ruggedness.

The two genuinely-open items are **2.1 integration tests** and **2.2 docs
site** — both investment rather than features. 2.3 / 2.4 / 2.5 are done.

### 2.1 Integration test coverage

98 unit tests are good for parsers/helpers/resolvers, but don't cover
end-to-end correctness. We need real-yt-dlp integration tests against a
recorded fixture corpus.

- Mock-server fixtures for yt-dlp's JSON output.
- `cargo test --features integration` exercises the full download pipeline
  against the mock.
- Headless web-UI tests via `headless_chrome` or `playwright`.

### 2.2 Documentation site

README is the only user-facing doc. Need a real docs site with:

- Per-platform setup guide (deeper than current README sections).
- Troubleshooting playbook for the top 10 yt-dlp errors.
- Architecture page (for contributors).
- Hosted via Codeberg Pages.

### 2.3 Error recovery / structured logging — DONE

Shipped a 9-class error classifier (`RateLimited`, `MembersOnly`,
`Geoblocked`, `NotFound`, `CodecMissing`, `DiskFull`, `NetworkError`,
`BadCookies`, `Other`) with a one-line suggested fix per class, surfaced
in both UIs. Remaining stretch: opt-in anonymous error telemetry to
surface new patterns.

### 2.4 Library restore — DONE

`POST /api/restore/db` + file pickers in both UIs do an idempotent merge
(watched / positions / flags / folders / notes), schema-validated.

### 2.5 Stability hardening — DONE

crash.log panic hook · disk-full preflight (synthetic DiskFull job) ·
auto-retry + adaptive throttle on transient failures · hang watchdog
(SIGKILL a job silent for 5 min, classified retryable so it re-queues) ·
`util::LockExt::lock_recover()` recovers poisoned `WebState` mutexes
instead of cascading one handler's panic into a dead server.

## Phase 3 — Surpass

Once we're at parity, we push past Tartube on its own ground.

### 3.1 Cross-compile macOS + Windows binaries

The Linux packaging (1.8) is done; this is the natural next reach. Blocked
on abstracting the Linux-only bits behind a per-OS backend — the `ksni`
tray and the `rfd` xdg-portal file dialog have no Windows/macOS path yet.
Once the tray is a trait with per-OS impls, the rest of the stack
(eframe/wgpu, axum, rusqlite-bundled) already cross-compiles.

### 3.2 Android client

Native client over the existing web API. Background download via
WorkManager + JobScheduler. Push notifications via Tailscale-routed
HTTPS or a userland push channel.

### 3.4 Smart auto-tagging

Cluster channels by uploader frequency, content type, and metadata.
Suggest groups ("looks like a music channel — move to Music?"). Builds
on Phase 1.2's group system.

### 3.5 Federation / multi-host

A "remote library" mode where one yt-offline instance can browse
another's library (read-only) over the same axum API. Useful for a
"home archive + travel laptop" setup.

### 3.6 Comment viewer enhancements

The comments capture (1.4) already ships a viewer. Surpass Tartube
(which only dumps the raw JSON) with:

- Threaded display with collapse/expand at any depth.
- Full-text search within a video's comments.
- "New since last visit" highlights.
- Sentiment / keyword filter chips.

### 3.7 Library-wide deduplication

Right now `maintenance` finds duplicates only by yt-dlp video ID — it
catches "you downloaded this exact video twice," but not the same content
re-uploaded under a *different* ID (a reupload, a cross-platform mirror, a
re-encode). Surpass by perceptual-hashing sampled frames (ffmpeg → dHash)
and grouping videos whose fingerprints match within a Hamming threshold,
even across IDs/resolutions/encodings. A new `video_fingerprint` cache
table (keyed by path+mtime like `info_cache`) makes it a one-time cost per
video; bucket by duration first to keep the comparison sub-quadratic.

### 3.8 Plugin / scripting hook

Lua or WASM-based hooks that run on download events: pre-download
filename rewriter, post-download archive uploader, custom metadata
enricher. Inverts the "we hardcode everything" model.

## Phase 4 — Stretch / blue-sky

Probably never, or much later.

- A web UI built around a "TV mode" remote-friendly layout.
- AI summarisation of videos (transcript → bullet points).
- Multi-user accounts with per-user watched/positions (currently single-user).
- Integration with Plex / Jellyfin / Kodi as a *source plugin* rather than a
  symlink generator.

## Recently shipped (highlights)

Roughly reverse-chronological. Items that closed out a roadmap line.

- **Hang watchdog + poison-recover locks** (2.5) — SIGKILL a yt-dlp/ffmpeg
  job silent for 5 min (classified retryable so it re-queues); recover
  poisoned `WebState` mutexes instead of cascading a panic into a dead
  server.
- **Filter presets** (1.3+) — save/apply/delete named chip-filter sets.
- **Desktop UI scale** — global egui zoom (whole UI, not just cards),
  Settings slider + Ctrl +/-/0, persisted.
- **Format conversion** (1.7) — post-download ffmpeg remux / H.264 /
  audio pass, global + per-channel, keep-original toggle.
- **Anti-bot stack** — POT token provider (bgutil-pot, version-matched
  plugin), nightly yt-dlp for working curl_cffi impersonation, dropped
  the captcha-prone forced `player_client=web`, auto-retry + adaptive
  throttle on rate-limit, configurable player clients (global +
  per-channel), cookie freshness / anonymous-jar warning.
- **Subtitle controls** — global `[subtitles]` config + per-channel
  overrides (download / auto / embed / convert-format / langs).
- **N-level folder nesting** (1.2+) — `parent_id` tree, recursive
  sidebar, move-folder-into-folder with cycle prevention.
- **Per-distro packaging** (1.8) — .deb / .rpm / AppImage via
  scripts/package.sh + Forgejo CI release artifacts.
- **Per-channel / per-video notes** (1.5) — searchable annotations.
- **Library restore** (2.4) — idempotent backup import.
- **Error classification** (2.3) — 9-class classifier + suggested fixes.
- **Crash log + disk-full preflight** (2.5) — panic→crash.log, statvfs
  guard before download.
- **wgpu renderer** — fixed NVIDIA+Wayland crash-on-maximize.
- **Performance pass** — info.json mtime cache, thumbnail worker pool,
  /api/library body cache, opt-level=3 + thin LTO.
- **System tray** (1.6) — ksni-based SNI tray, minimize-to-tray opt-in.
- **Filter chips** (1.3) — watch / date / size / has-subs / has-chapters,
  AND together, persisted to localStorage.
- **Web downloads modal** — bottom #jobs bar replaced with a ⬇ button
  in the header opening a full modal. `d` keyboard shortcut.
- **Desktop screens refactor** — Settings/Stats/Maintenance moved from
  floating windows to full CentralPanel views with vertical scroll.
- **WebSocket job progress** (3.3) — replaced HTTP polling.
- **Mobile-responsive web UI** — proper media queries at 640px / 380px.
- **Library backup** (2.4 — backup direction) — DB download from settings.
- **Theme contrast fixes** — every theme now passes per-state fg_stroke
  contrast checks.
- **Shuffle play** — random unwatched video on the desktop + web.
- **Keyboard shortcuts** — `/` `r` `d` `?` in the web UI.
- **Bulk tagging + channel-name search** — multi-select + flag bulk-set.
- **Channel folders + per-folder Check all** (1.2) — one-level grouping.
- **Per-channel download options** (1.1) — JSON-blob overrides applied
  on scheduled re-checks.
- **Per-video state flags + smart folders + comments capture** (1.3/1.4)
  — favourite / bookmark / waiting / archive flags as smart-folder views;
  `--write-comments` with viewer tab.

## How to read this

- **Phase 1** (Tartube parity) is **complete** — kept above for the record.
- **Phase 2** is down to test coverage (2.1) and a docs site (2.2).
- **Phase 3** is the "surpass" work now that we're at parity.
- **Phase 4** items might be valuable, but commit to nothing.

Items inside a phase are loosely ordered by user-visible impact, not strict
prerequisite. With parity reached, the highest-leverage next moves are
**3.1 (Windows/macOS binaries)** for reach and **3.7 (perceptual-hash
dedup)** / **3.6 (comment-viewer)** for features Tartube can't match.
