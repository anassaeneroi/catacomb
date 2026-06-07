# Architecture

For contributors. The repo's `CLAUDE.md` is the terse version of this;
read both.

## Two front-ends, one engine

`main.rs` dispatches: `--web` → `web::run()` (axum, blocks forever);
otherwise `app::App` (eframe/egui desktop GUI). **Both share
`downloader::Downloader`**, the single source of truth for the yt-dlp job
lifecycle.

`Downloader` is **not** async — it spawns an OS thread per yt-dlp process,
streams stdout/stderr back over an `mpsc` channel into each `Job`'s log
buffer, and the caller pumps `Downloader::poll()` regularly (the egui
frame loop and a web background task both do). Anything you add to
`Downloader` is automatically available to both UIs.

`poll()` also drives the cross-cutting job machinery: auto-retry of
transient failures (cooldown + adaptive throttle), the hang watchdog, and
the post-download ffmpeg transcode pass. These work by capturing specs
onto the `Job` at `start()` time (`RetrySpec`, `ConvertSpec`) and acting
on them when the job changes state.

## The settings flow (the easy thing to get wrong)

Almost every configurable feature has the same five-touchpoint shape —
miss one and it silently half-works:

1. `config.rs` — a field/section + its `Default` + the `default_with_dir`
   constructor.
2. `download_options.rs` — an `Option<…>` per-channel override (None =
   defer to global).
3. `downloader.rs` — a resolver merging global config + per-channel
   override into yt-dlp/ffmpeg args, plus a `pub` field on `Downloader`
   holding the global default.
4. **Both** UIs render the global setting *and* the per-channel override:
   desktop in `app.rs`, web in `web_ui/index.html` **and** `web.rs`'s
   `SettingsPayload` (GET reads config, POST writes config + pushes onto
   the live `Downloader`).
5. Seed the `Downloader` field at construction **and** on settings-save,
   in **both** `app.rs` and `web.rs`.

`subtitle_defaults`, `youtube_player_clients`, and `convert_defaults` are
complete worked examples — grep one end-to-end before adding a setting.

## Filesystem layout

`platform::platform_root(channels_root, platform)` =
`channels_root.join(dir_name)`. **All** platforms (including YouTube,
whose `dir_name` is `channels`) nest under the one configured
`backup.directory`. `.source-url` sidecars in each creator folder let
re-checks recover the exact URL. Library scanning (`library.rs`) is
parallel and consults a `(path, mtime)` SQLite cache to skip re-parsing
unchanged `info.json` sidecars.

## Persistence

`database.rs` wraps an r2d2 SQLite pool. `Database` is cheaply `Clone`
(the pool is an `Arc`), so the parallel scanner takes its own handle.
Schema lives in `init_schema()`; new columns are added via idempotent
`ALTER TABLE … ADD COLUMN` that swallows the duplicate-column error (no
migration framework). The web UI keeps library/notes snapshots in memory;
mutating endpoints mirror DB writes onto those caches and bump a version
counter (the `/api/library` ETag) so reads stay consistent without a
rescan.

The long-lived `WebState` mutexes are accessed via
`util::LockExt::lock_recover()`, which recovers a poisoned lock instead of
cascading one handler's panic into a dead server.

## Web UI is one embedded file

`web_ui/index.html` is the entire SPA (HTML+CSS+JS), `include_str!`-baked
into the binary at compile time — editing it requires a rebuild. Served
`Cache-Control: no-store` so binary upgrades don't strand stale tabs.
Progress streams over `/ws/progress` (WebSocket) with an HTTP-poll
fallback.

## Anti-bot subsystems

`ytdlp_bin.rs` manages the optional self-contained venv at
`~/.local/share/yt-offline/` (nightly `yt-dlp[default]` + `curl_cffi` +
bundled `deno`). `pot_provider.rs` runs `bgutil-pot` for Proof-of-Origin
tokens — its yt-dlp plugin must come from the same release as the server
binary. `error_class.rs` pattern-matches yt-dlp stderr into actionable
classes (order matters in `classify()`: the captcha "Video unavailable"
wall is RateLimited, not NotFound).

## Tests

- Unit tests are inline `#[cfg(test)]` modules (parsers, resolvers, the
  error classifier, DB merge logic).
- `tests/api.rs` spawns the **real** `--web` binary against a scratch dir
  and drives the HTTP API with curl — genuine end-to-end coverage of the
  axum + SQLite + config stack.

`cargo test` runs both. `.forgejo/workflows/test.yml` runs them on every
push.

## Platform support

Tray (`ksni`) and file dialogs (`rfd` xdg-portal) are Linux-only / no-GTK
by design — that's why packaging avoids a GTK dependency. Windows/macOS
aren't first-class yet: the tray needs a per-OS backend before a clean
cross-build. The rest (eframe/wgpu, axum, rusqlite-bundled) already
compiles cross-platform, and `ytdlp_bin` already has `cfg!(windows)`
branches.
