# Roadmap

## North star

**Surpass [Tartube](https://github.com/axcore/tartube) in every dimension.**

A structured analysis of Tartube's codebase, data model, operations, and
configuration surface lives at [`docs/tartube-spec.md`](docs/tartube-spec.md).
It enumerates the exact features we need to match — every Phase 1 item
below traces back to a specific Tartube subsystem.

Tartube is the mature open-source yt-dlp GUI and the obvious benchmark for a
project in this space. yt-offline has architectural advantages Tartube can't
catch up on quickly (Rust + axum + a real web UI + bundled toolchain + a
modern security model) but trails on feature breadth and years-of-edge-cases
maturity. The plan below closes those gaps first, then pushes past.

## Current state vs Tartube (2026-05-25)

| Area | Us | Tartube | Verdict |
| --- | --- | --- | --- |
| yt-dlp wrapping | ✅ | ✅ | Tied |
| Multi-platform sources | ✅ first-class per-platform routing | ✅ generic | We lead |
| Web UI accessible from any device | ✅ | ❌ desktop only | We lead |
| Single-binary distribution | ✅ Rust binary + venv installer | ❌ Python+GTK deps | We lead |
| Security model (auth, CSP, rate-limit) | ✅ | ❌ never network-facing | We lead |
| Plex export with NFO sidecars | ✅ | ❌ | We lead |
| Bundled curl_cffi for impersonation | ✅ | ❌ user-installed | We lead |
| Themes | ✅ 10 themes | ❌ GTK default | We lead |
| Live-stream recording | ✅ | ✅ recent | Tied |
| Per-channel custom download options | ❌ global only | ✅ deep | **Tartube leads** |
| Folder/group hierarchy | ❌ flat per-platform | ✅ N-level | **Tartube leads** |
| Filter UI (date range, size, watched) | partial sort only | ✅ rich | **Tartube leads** |
| Comments capture | ❌ | ✅ `--write-comments` | **Tartube leads** |
| Per-channel notes / annotations | ❌ | ✅ | **Tartube leads** |
| System tray | ❌ removed early | ✅ | **Tartube leads** |
| Format conversion / re-encode | ❌ remux only | ✅ ffmpeg pipeline | **Tartube leads** |
| Maturity / edge cases | ~weeks | ~years | **Tartube leads** |
| Per-distro packaging | PKGBUILD only | .deb .rpm .pkg.tar.zst | **Tartube leads** |

Score: 8 ahead, 8 behind, 1 tied — i.e. we're already half the way there on
features Tartube has, plus we have a real lead on architecture.

## Phase 1 — Tartube feature parity

The eight items Tartube has that we don't, ordered by how visibly they
change the user experience.

### 1.1 Per-channel custom download options
Tartube's headline feature. Each channel gets its own quality / format /
extractor args / cookies override that take priority over the global config.

- New `channel_settings` SQLite table (channel_path → JSON blob).
- `Downloader::start` consults overrides before applying global defaults.
- Right-click → "Channel options…" opens a per-channel settings dialog.
- Settings cascade: channel override > group override (Phase 1.2) > global.

### 1.2 Folder / group hierarchy
Lets a user organize channels into "Music", "News", "Coding", etc., with
per-group default download options.

- `channel_groups` SQLite table: `(id, parent_id, name)`.
- `Channel` gains `group_id: Option<i64>`.
- Sidebar renders as a tree; drag-and-drop to reorganize (desktop) /
  right-click "Move to group…" (web).
- Group-level download options inherited by member channels unless
  overridden per-channel.

### 1.3 Filter UI (date / size / status)
Today we have sort. Tartube has full filtering with chips and saved sets.

- Filter chips above the video grid: "watched / unwatched / in-progress",
  "today / this week / this month / older", "< 100 MB / > 1 GB", "has
  subtitles", "has chapters".
- Saved filter presets — name a set of chips, restore later.
- Filter state persists across rescans within a session.

### 1.4 Comments capture
yt-dlp's `--write-comments` dumps the full comment tree as JSON sidecar.

- Toggle in download dialog: "Include comments (slow)".
- Stored as `<stem>.comments.json` next to the video.
- New `/api/comments/:id` endpoint returns paginated comments.
- Web video player modal gets a "Comments" tab next to "Chapters".

### 1.5 Per-channel / per-video notes
Free-text user annotations on any channel or video.

- New `notes` table: `(target_kind, target_id, body, updated_at)`.
- Pencil icon on the channel sidebar entry + video card.
- Notes are searchable from the global filter.

### 1.6 System tray
Resurrect the dropped tray module. Minimize to tray, show download
progress overlay, click to open the main window.

- Conditional on a non-headless desktop session.
- Right-click → Show / Hide / Quit / Pause downloads.

### 1.7 Format conversion pipeline
Post-download re-encode option: H.264/AAC mp4 at a configurable CRF, or
audio extraction at a target bitrate. Useful for shrinking large 4K files.

- New `post_process` config field per quality preset.
- ffmpeg job runs after the download completes, replaces the source file
  (or keeps both with an `.original.mkv` suffix).
- Visible in the job log as a separate "transcoding" phase.

### 1.8 Per-distro packaging
Tartube ships .deb, .rpm, .pkg.tar.zst, .exe, .dmg. We ship a PKGBUILD.

- Codeberg CI matrix for `cargo build --release` on each target.
- Helpers to generate .deb (via `cargo-deb`), .rpm (via `cargo-generate-rpm`),
  Windows MSI (via `cargo-wix`), and macOS .app + .dmg.
- GitHub-style release artifacts attached to each tag.

## Phase 2 — Polish where Tartube is mature

Things we win on architecturally but lose on real-world ruggedness.

### 2.1 Integration test coverage
47 unit tests is good for parsers and helpers, useless for end-to-end
correctness. We need real-yt-dlp integration tests against a recorded
fixture corpus.

- Mock-server fixtures for yt-dlp's JSON output.
- `cargo test --features integration` exercises the full download pipeline
  against the mock.
- Headless web-UI tests via `headless_chrome` or `playwright`.

### 2.2 Documentation site
README is the only user-facing doc. Need a real docs site with:

- Per-platform setup guide (deeper than current README sections).
- Troubleshooting playbook for the top 10 yt-dlp errors.
- Architecture page (for contributors).
- Hosted via Codeberg Pages or GitHub Pages mirror.

### 2.3 Error recovery / structured logging
Today: errors land in the job log and that's it. Tartube has a "rescue
recipes" feature where common errors map to documented fixes.

- `Job::failure_reason: Option<ErrorClass>` enum (`RateLimited`,
  `MissingCookies`, `CodecMissing`, `DiskFull`, `Other`).
- UI surfaces the class with a one-click suggested action.
- Anonymous opt-in error telemetry to surface new patterns.

### 2.4 Backup / restore
Tartube has database export. We have nothing.

- `/api/backup` → tar of `yt-offline.db` + `config.toml` + `cookies.txt`.
- `/api/restore` (idempotent merge of watched + positions).
- "Export library snapshot" button in settings.

### 2.5 Stability hardening
Long-running deployments will surface real bugs.

- Replace remaining `.lock().unwrap()` with poisoning-aware accessors.
- Watchdog: if a yt-dlp job hangs past `--socket-timeout * retries`, kill
  and re-queue.
- Disk-full preflight: refuse to start a download when target dir has
  less free space than the average video size on disk.

## Phase 3 — Surpass

Once we're at parity, we push past Tartube on its own ground.

### 3.1 Cross-compile macOS + Windows binaries
The current "later" roadmap item, blocked behind 1.8.

### 3.2 Android client
Native client over the existing web API. Background download via
WorkManager + JobScheduler. Push notifications via Tailscale-routed
HTTPS or a userland push channel.

### 3.3 Real-time progress via WebSocket
Replace the adaptive HTTP poll with a persistent `/ws/progress`
connection. Server pushes job updates; client renders without polling.
Saves the per-second tick and gives instant feedback.

### 3.4 Smart auto-tagging
Cluster channels by uploader frequency, content type, and metadata.
Suggest groups ("looks like a music channel — move to Music?"). Builds
on Phase 1.2's group system.

### 3.5 Federation / multi-host
A "remote library" mode where one yt-offline instance can browse
another's library (read-only) over the same axum API. Useful for a
"home archive + travel laptop" setup.

### 3.6 Comment viewer with diff
The Phase 1.4 comments JSON is just data. We can build the *viewer*
Tartube doesn't have: threaded display, search, "new since last visit"
highlights, sentiment filter.

### 3.7 Library-wide deduplication
Right now `maintenance` finds duplicates by yt-dlp video ID. Surpass by
fingerprinting media bytes (`ffmpeg`-derived perceptual hashes) so the
same video re-uploaded under a different ID gets flagged.

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

## How to read this

- **Phase 1** is the next ~3 months of work if pursued steadily.
- **Phase 2** is concurrent polish — pick up between Phase 1 items.
- **Phase 3** is the year-1 ambition.
- **Phase 4** items might be valuable, but commit to nothing.

Items inside a phase are loosely ordered by user-visible impact, not strict
prerequisite. **1.1 (per-channel options)** is the single biggest gap and
should be tackled first.
