# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`yt-offline` — a single Rust binary that is **both** a desktop GUI (eframe/egui)
and a headless web server (axum), wrapping `yt-dlp` to archive YouTube/TikTok/
Twitch/etc. AGPL-3.0. North-star goal and feature-parity tracking live in
[ROADMAP.md](ROADMAP.md); a structured analysis of the Tartube benchmark is in
[docs/tartube-spec.md](docs/tartube-spec.md).

## Commands

```bash
cargo build --release           # the binary (release profile is opt-level=3 + thin LTO, ~1-2 min)
cargo test --release            # all unit tests (fast; no network)
cargo test --release <name>     # single test by substring, e.g. `cargo test --release subs_disabled`
./target/release/yt-offline           # desktop GUI mode (default)
./target/release/yt-offline --web 8080  # headless web server on a port

scripts/package.sh [deb|rpm|appimage|all]   # build distro packages → dist/ (see docs/PACKAGING.md)
```

There is no separate lint step; `cargo build` warnings are the lint. The egui
dependency emits ~39 `f32: From<f64>` fallback warnings on a clean build — those
are upstream, not from this code.

### Running / verifying against a real library

The app reads `config.toml` and `cookies.txt` **from the process working
directory**, not a fixed path. To smoke-test the web server in isolation, make a
scratch dir with a minimal `config.toml` (`[backup]\ndirectory = "..."`) and run
`--web <port>` from inside it. `/api/*` endpoints require auth when a password is
set in the target library's DB.

## Architecture

### Two front-ends, one engine

`main.rs` dispatches: `--web` → `web::run()` (axum, blocks forever); otherwise
`app::App` (eframe). **Both share `downloader::Downloader`**, the single source
of truth for yt-dlp job lifecycle. `Downloader` is *not* async — it spawns an OS
thread per `yt-dlp` process, streams stdout/stderr back over an `mpsc` channel
into each `Job`'s log buffer, and the caller must pump `Downloader::poll()`
regularly (the egui frame loop and a web background task both do this). When you
add download behavior, it goes in `Downloader` and is automatically available to
both UIs.

`poll()` also drives the cross-cutting job machinery: **auto-retry** of transient
failures (rate-limit/network) with cooldown + adaptive throttle, and the
**post-download ffmpeg transcode** pass. These work by capturing specs onto the
`Job` at `start()` time (`RetrySpec`, `ConvertSpec`) and acting on them when the
job transitions state — the `pending_*_spec` fields on `Downloader` are stashed
by `start()` and consumed by `enqueue()`/`spawn_job()` so the four non-download
enqueue paths (repair/music/yt-dlp-update/pot-update) stay untouched.

### Settings flow (the easy thing to get wrong)

Almost every feature has the same five-touchpoint shape — miss one and it
silently half-works:

1. `config.rs` — a field/section + its `Default` and the `default_with_dir()` constructor.
2. `download_options.rs` — an `Option<…>` per-channel override (None = defer to global).
3. `downloader.rs` — a resolver that merges global config + per-channel override into yt-dlp/ffmpeg args, plus a `pub` field on `Downloader` holding the global default.
4. **Both** UIs render the global setting *and* the per-channel override: desktop in `app.rs` (egui Settings screen + the channel-options dialog), web in `web_ui/index.html` (Settings modal + channel-options dialog) **and** `web.rs`'s `SettingsPayload` struct (GET reads from config, POST writes config + pushes the value onto the live `Downloader`).
5. Seed the `Downloader` field at construction **and** on settings-save, in **both** `app.rs` and `web.rs`.

Grep an existing setting end-to-end before adding one — `subtitle_defaults`,
`youtube_player_clients`, and `convert_defaults` are complete worked examples.

### Filesystem layout invariant

`platform::platform_root(channels_root, platform)` = `channels_root.join(dir_name)`.
**All** platforms (including YouTube, whose `dir_name` is `channels`) nest under
the one configured `backup.directory`. `library_root == channels_root` now (a
historical two-level split was removed). `.source-url` sidecars in each creator
folder let channel re-checks recover the exact URL. Library scanning
(`library.rs`) is parallel and consults a `(path, mtime)` SQLite cache to skip
re-parsing unchanged `info.json` sidecars.

### Persistence

`database.rs` wraps an r2d2 SQLite pool (file-backed; `Database` is cheaply
`Clone` — the pool is an `Arc`, so the parallel scanner takes its own handle).
Schema lives in `init_schema()`; new columns are added via idempotent
`ALTER TABLE … ADD COLUMN` that swallows the duplicate-column error (no migration
framework). The web UI holds library/notes snapshots in memory and mutating
endpoints mirror DB writes onto those caches + `bump_library_version()` (the
ETag) so `/api/library` stays consistent without a rescan.

### Web UI is one embedded file

`web_ui/index.html` is the entire SPA (HTML+CSS+JS), `include_str!`-baked into
the binary at compile time — editing it requires a rebuild to take effect. It's
served with `Cache-Control: no-store` so binary upgrades don't strand stale tabs.
Progress streams over `/ws/progress` (WebSocket) with HTTP-poll fallback.

### Bundled toolchain & anti-bot

`ytdlp_bin.rs` manages an optional self-contained venv at
`~/.local/share/yt-offline/` (nightly `yt-dlp[default]` via `--pre` + `curl_cffi`
for TLS impersonation + bundled `deno`). `pot_provider.rs` runs `bgutil-pot` (a
loopback HTTP server) for YouTube Proof-of-Origin tokens; **its yt-dlp plugin
must come from the same release as the server binary, not PyPI** (version skew
silently produces no tokens — see the module doc). `error_class.rs` pattern-
matches yt-dlp stderr into actionable classes (the captcha "Video unavailable"
wall is classified RateLimited, not NotFound — order matters in `classify()`).

## Conventions

- **Never commit** `cookies.txt` (live session creds), `config.toml` (user-
  specific), or `yt-offline.db` (contains the Argon2 password hash). All
  gitignored.
- Redact the absolute cookies path out of any log line surfaced to the UI/API
  (`redact_sensitive` in `downloader.rs`) — it leaks `$HOME`.
- `app.rs` and `web.rs` are large (~3–4k lines) because each owns a full UI; new
  desktop code goes in `app.rs`, web handlers in `web.rs`, shared logic in the
  focused modules (`downloader`, `database`, `library`, `platform`, …).
- Tray (`ksni`) and file dialogs (`rfd` xdg-portal) are Linux-only/no-GTK by
  design; keep that posture (it's why packaging avoids a GTK dep). Windows/macOS
  are not yet first-class — the tray would need a per-OS backend.
