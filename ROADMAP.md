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
| Full-text search (titles/desc/transcripts) | ✅ SQLite FTS5 + snippets | ❌ name filter only | We lead |
| Transcript viewer (search + click-to-seek) | ✅ web pane + desktop window | ❌ | We lead |
| Content-aware dedup (perceptual) | ✅ ffmpeg→dHash, duration-bucketed | ❌ ID-only | We lead |
| Maturity / edge cases | ~months | ~years | **Tartube leads** |

Score: 17 ahead, 1 behind, 13 tied. **Phase 1 (Tartube parity) is
complete** and several Phase 3 "surpass" features have landed — the only
thing Tartube still leads on is years-of-edge-cases maturity, which is
time, not a feature.

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

Every Phase 2 item is now done — 2.1 / 2.2 shipped alongside 2.3 / 2.4 /
2.5.

### 2.1 Integration test coverage — DONE

98 unit tests cover parsers/helpers/resolvers; `tests/api.rs` adds 7
end-to-end tests that spawn the **real** `--web` binary against a scratch
tempdir and drive the HTTP API with curl (index/library serving, ETag
304, settings round-trip + persistence, folders CRUD + cycle guard, notes
round-trip, channel-options round-trip + clear, DB backup). Each test
gets its own server/port/tempdir, so they run in parallel — `cargo test`
runs the lot. (A `.forgejo/workflows/test.yml` CI definition exists, but
Codeberg executes Woodpecker, not Forgejo Actions, so it's inert there
without a self-hosted runner; tests run locally.) Stretch left: a
recorded-fixture corpus for the download pipeline and a headless web-UI
test.

### 2.2 Documentation site — DONE

An mdBook under `docs/` (eight pages: introduction, installation,
first-run/config, downloading, anti-bot, troubleshooting, architecture,
packaging), published to Codeberg Pages via `scripts/publish-docs.sh`
(build the book + force-push it to the `pages` branch — Codeberg doesn't
run Forgejo Actions, so publishing is a local one-liner rather than CI).
The anti-bot and troubleshooting pages capture the cookies/curl_cffi/POT/
player-client knowledge; the architecture page documents the
two-front-ends/one-engine design for contributors.

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

### 3.1 Cross-compile macOS + Windows binaries — GROUNDWORK DONE

The compile blockers are cleared: `cargo check --target
x86_64-pc-windows-gnu` is green (only the upstream egui `f32: From<f64>`
warnings). The Linux-only deps are now target-gated in `Cargo.toml` —
`ksni` and `rfd`'s `xdg-portal` backend are `cfg(target_os = "linux")`
only, with `rfd` falling back to its native Win32/AppKit backend
elsewhere. `tray::start` is `cfg`-split: the SNI/ksni implementation on
Linux, a no-op `None` stub on other OSes (windowed-only, exactly the
no-SNI-host behavior). The rest was already portable — `disk_space`
(`statvfs`), `plex` (symlinks; now with a Windows `symlink_file` path),
the mpv IPC `UnixStream`, and the `chmod`/`PermissionsExt` guards are all
`cfg(unix)`-gated. macOS is Unix, so those paths apply there unchanged.

Remaining for a shipped binary: a real per-OS tray backend (e.g.
`tray-icon`) if the tray is wanted off Linux; a CI matrix that actually
*links* (mingw-w64 / an osxcross or macOS runner) and produces artifacts;
and runtime testing on each OS. The hard part — making the tree compile
off Linux — is done.

### 3.2 Android client

Native client over the existing web API. Background download via
WorkManager + JobScheduler. Push notifications via Tailscale-routed
HTTPS or a userland push channel.

### 3.4 Smart auto-tagging — DONE

`autotag.rs` classifies each *unfiled* channel from already-scanned
metadata — source platform plus the median video duration and upload
cadence — into a suggested folder group: "Music" (music platforms or
audio-only channels), "Shorts" (median < 90 s / TikTok), "Long-form &
Podcasts" (median > 25 min), or "Streams & VODs" (Twitch). Mid-length
YouTube is deliberately left unsuggested rather than guessed at. Each
suggestion carries a confidence and a human reason ("median length 76 s;
~1/mo"). Surfaced in both Maintenance views: the web shows per-channel
checkboxes + "Apply → group" (POST /api/autotag/apply creates/reuses the
folder and assigns), desktop shows "Move all → group". Pure arithmetic
over the in-memory library, so it's recomputed on demand with no job.
Never moves anything until you apply.

### 3.5 Federation / multi-host — DONE

A read-only "remote library" mode: configure peers as `[[remote]]` entries
(name, url, optional password) and browse their libraries from this
instance. `remote.rs`'s `RemoteClient` (blocking reqwest, rustls, cookie
jar) fetches the peer's `/api/library`, logging in first if the peer has a
password. Media is **not** proxied — the library JSON's media URLs
(`/files/`, `/music-files/`, `/api/transcode/`, `/api/sub-vtt/`) are
rewritten to absolute peer URLs with the peer's read-only **feed token**
appended, which the peer's auth middleware now accepts for those GET paths.
So only the small library JSON travels through us; video streams straight
from the peer to the browser/mpv. Surfaced in both UIs: the web sidebar
gets a 🌐 Remotes switcher (read-only mode hides the download bar + per-card
mutating actions); desktop gets a 🌐 Remotes screen that fetches off-thread
and plays via mpv on the tokenized URL. Verified end-to-end against a second
local instance (incl. a transcoded stream returning HTTP 200 with the token).
Follow-ups: an in-UI "add remote" editor (today it's config-file only) and a
desktop thumbnail/grid view to match the web fidelity.

### 3.6 Comment viewer enhancements — DONE

The web comment viewer is now threaded with per-thread collapse/expand,
in-comment full-text search (highlighted), sort (top / newest / oldest),
an uploader (OP) badge, and a "new since last visit" highlight + count
(localStorage per-video timestamp). Sentiment chips were dropped as
low-value. (Comments stay a web-only viewer; the desktop captures them
but points you at the web UI.)

### 3.7 Library-wide deduplication — DONE

`maintenance` still finds exact duplicate video IDs; on top of that,
`fingerprint.rs` adds content-aware dedup — ffmpeg keyframe-seek samples
6 frames/video → 9×8 grayscale → 64-bit dHash, grouped within a Hamming
threshold but only inside a duration-tolerance window (sorted sliding
window + union-find) to stay near-linear. A `video_fingerprint` cache
table (keyed by path+mtime like `info_cache`) makes it one-time per video.
Surfaced as a review-only "Similar content" report (background job +
progress) in both the web and desktop Maintenance views; never
auto-deletes. Measured ~0.5 s/video serial (~65 ms across 8 cores).

### 3.8 Plugin / scripting hook

Lua or WASM-based hooks that run on download events: pre-download
filename rewriter, post-download archive uploader, custom metadata
enricher. Inverts the "we hardcode everything" model.

### 3.9 Library-wide full-text search + transcript tooling — DONE

Beyond the original plan. A `video_search` FTS5 index over titles,
channels, descriptions, **and subtitle transcripts** (mtime-gated build,
refreshed after every scan; ranked hits with highlighted snippets),
surfaced as a 🔍 search in both UIs. Plus a searchable, click-to-seek
**transcript viewer** — a pane beside the web player (live-highlights the
current line) and a floating desktop window that seeks mpv over IPC —
backed by a shared `vtt` WebVTT/SRT parser.

## Phase 4 — Stretch / blue-sky

Probably never, or much later.

- A web UI built around a "TV mode" remote-friendly layout.
- AI summarisation of videos (transcript → bullet points).
- Multi-user accounts with per-user watched/positions (currently single-user).
- Integration with Plex / Jellyfin / Kodi as a *source plugin* rather than a
  symlink generator.

## Recently shipped (highlights)

Roughly reverse-chronological. Items that closed out a roadmap line.

- **Transcripts in search** (3.9) — subtitle text folded into the FTS
  index so search matches spoken words; FTS5 column-add migration.
- **Perceptual-hash dedup** (3.7) — `fingerprint.rs` (ffmpeg→dHash,
  duration-bucketed grouping) + `video_fingerprint` cache; background
  "Similar content" review job in both Maintenance views.
- **Library-wide full-text search + transcript viewer** (3.9) — `video_search`
  FTS5 index (titles/channels/descriptions); 🔍 search + a click-to-seek,
  live-highlighting transcript pane (web) / window (desktop) over a shared
  `vtt` parser.
- **Comment viewer enhancements** (3.6) — threaded collapse/expand,
  in-comment search, sort, OP badge, new-since-last-visit.
- **Configurable SponsorBlock** — off / mark / remove, global + per-channel
  (was a hardcoded `--sponsorblock-mark all`).
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
- **Phase 2** is complete: integration tests (2.1), docs site (2.2),
  error recovery (2.3), restore (2.4), stability hardening (2.5).
- **Phase 3** is the "surpass" work now that we're at parity. Shipped so
  far: WebSocket progress (3.3), comment viewer (3.6), perceptual-hash
  dedup (3.7), and full-text search + transcript tooling (3.9). Still open:
  Windows/macOS binaries (3.1), Android (3.2), smart auto-tagging (3.4),
  federation (3.5), plugin hooks (3.8).
- **Phase 4** items might be valuable, but commit to nothing.

Items inside a phase are loosely ordered by user-visible impact, not strict
prerequisite. With the easy "surpass" wins banked, the highest-leverage
remaining moves are **3.1 (Windows/macOS binaries)** for reach and a
**RSS/podcast feed** or **smart auto-tagging (3.4)** for self-contained
features Tartube can't match.
