# Session handoff — Catacomb

> Working notes for continuing this work in another session/agent (Copilot, Codex, etc.).
> Updated 2026-07-09. Delete or update freely; this is not a tracked spec.

## RESOLVED (2026-07-09): GUI launches with empty library until manual Rescan

**Root cause (confirmed by reproduction against the real library):** not a data
or render bug — the cold-cache startup scan takes **~64 s** (vs ~96 ms warm) on
the LUKS+btrfs+zstd `/mnt/InannaBeloved` volume, dominated by the directory walk
+ `std::fs::metadata` stat of ~11k sidecar/video files. `info_cache` only saves
JSON *parsing*, not the stats, so it can't shorten a cold scan. The library did
populate on its own after ~64 s (the drain delivers all 89 channels), but the
content area looked empty during the wait, so the user hit ⟳ Rescan — which
appeared to fix it only because the OS cache was warm by then. The old
"database is locked" log line is a separate benign symptom (the web server on
:8081 contends on the same FTS write; `sync_search_index` failing is caught).

**Fixes shipped this session (committed, on `main`):**
- `843fee5` **Scanning state** — `video_list` shows a centered spinner +
  "Scanning library…" while the initial scan is in flight
  (`library_load_rx.is_some() && library.is_empty()`), so a cold scan no longer
  looks broken. Verified live.
- `5a55c15` + `3e6bcfb` **Persistent library snapshot (instant startup)** — the
  scanned `Vec<Channel>` is serialized to a JSON blob in a new
  `library_snapshot(root, json, saved_at)` table after every scan; desktop
  `App::new()` seeds `self.library` from it before spawning the scan thread, so
  a warm launch renders the full library **within ~2 s** (status
  "N videos (refreshing…)") while the authoritative rescan runs and swaps in.
  Deleting the snapshot row falls back to the scanning spinner. Spec +
  plan in `docs/superpowers/specs|plans/2026-07-09-library-snapshot-*`.
  Serde derives added to `Subtitle/Video/Playlist/ChannelMeta/Channel`
  (`#[serde(default)]` for forward-compat).

Original investigation notes (kept for reference):

- **Launch environment**: the daily GUI is the packaged `/usr/bin/catacomb`
  (pacman pkg `catacomb d4bed01-1`, i.e. includes the perf pass), launched
  with **CWD = /home/luna**, which has its own `/home/luna/config.toml`
  pointing at `/mnt/InannaBeloved/youtube-backup/` (real 73 MB DB). Config is
  correct — rescan works against the right library.
- **Code path**: `app.rs::new()` spawns the `catacomb-libscan` thread
  (scan → apply options/folders → `sync_search_index` → `send(library)` →
  `request_repaint`); `update()` drains `library_load_rx` (~app.rs:4855).
  Small scratch libraries populate fine (verified live during the layout-fix
  work), so the mechanism works — this is timing/contention-dependent on the
  big real library.
- **Smoking gun in the user's own launch** (`journalctl --user`, pid 609582,
  launched 03:39:37): `search index sync failed: database is locked` at
  03:40:54 — i.e. the scan took ~77 s and then the FTS sync hit SQLITE_BUSY
  **despite `busy_timeout = 5000`** (present and correct in `database.rs`).
  So some writer held the DB write lock > 5 s during startup. Prime suspect:
  the dedup fingerprint pass (`dedup_enabled = true` in the user's config)
  or another batched writer running concurrently with the libscan thread.
- **Open question**: `sync_search_index` failing is handled (eprintln,
  continue) and `send(library)` comes after it, so that error alone should
  NOT leave the library empty. Either the scan itself returns empty/partial
  under the same lock contention (check `scan_channels_with_cache`'s
  info_cache read/write error handling — the perf pass batched those writes),
  or the send/drain has a separate failure mode. `try_recv().ok()` at
  app.rs:4855 swallows a Disconnected (dead thread) silently — worth making
  that surface an error status.
- **Repro gotcha**: don't launch `/usr/bin/catacomb` under the XWayland
  screenshot recipe — the wgpu backend panics with `Surface::configure:
  Invalid surface` (panic=abort → SIGABRT). That crash is environmental, NOT
  this bug. The repo `target/release` binary renders fine under XWayland.
- **Next steps**: (1) read `scan_channels_with_cache` + batched-write error
  handling for "return empty on DB error" paths; (2) instrument or add a
  status line when the libscan thread dies/errors; (3) reproduce with the
  real library — run the GUI on Wayland natively (no XWayland forcing) with
  stderr captured, or add temporary eprintln timing to the libscan thread;
  (4) check what else writes to the DB in the first ~90 s (fingerprint sync
  runs at startup? scheduler?).
- The user's live instance (pid 609582) was left running; a stray
  `--version`-launched instance from the investigation was killed.

## Feature in flight: federation editor + PeerTube (3-phase project)

Roadmap 3.5 follow-up. What began as an in-UI editor for catacomb peers grew
(user wants PeerTube interop) into a **3-phase project**, decomposed during
brainstorming:

- **Phase 1 — PeerTube client + multi-kind config (backend). DONE + pushed
  (`bf3f2b0`, `e69ebc7`, `f7f36d9`, `00ae19e`).** `config::RemoteKind`
  (catacomb|peertube) + `username` field on `RemoteSection` (non-breaking,
  `#[serde(default)]`). New `src/peertube.rs`: blocking `PeerTubeClient` with
  OAuth2 (password grant + refresh-token renewal, anonymous when no creds),
  `list_channels` / `channel_videos(handle, page)` / `video_media(uuid)` (direct
  MP4, `None` for HLS-only) / `watch_url`. URL parsing detects instance / `/c/`
  channel / `/a/` account targets. Pure mapping fns fixture-tested; HTTP is
  manual-verify. `#[allow(dead_code)]` until phases 2–3 construct it. Spec +
  plan: `docs/superpowers/specs|plans/2026-07-10-peertube-client-backend*`.
- **Phase 2 — kind-aware remote editor (both UIs). NOT STARTED.** The editor
  spec is written (`docs/superpowers/specs/2026-07-10-federation-remote-editor-design.md`,
  commit `112a109`) but predates the kind decision — it must gain a `kind`
  selector + `username` field, and this is where the `RemoteSource` trait
  (catacomb vs peertube dispatch) gets extracted. Editor decisions already
  locked: lives in Settings both UIs; **live-apply** (web needs
  `WebState.remotes` → `RwLock<Vec<Arc<RemoteClient>>>`); **masked/write-only**
  passwords on web (kept-by-URL on blank save), shown in clear on desktop;
  **test-connection** button. Re-brainstorm the editor spec to fold in `kind`
  before writing its plan.
- **Phase 3 — PeerTube browse UI + archive action (both UIs). NOT STARTED.**
  Two-level lazy nav (list channels → click → paginated videos), inline play
  via `video_media` (HLS-only = browse-only), and a per-video "Archive" button
  handing `watch_url` to the downloader.
No spec written yet.

## Shipped this session (2026-07-09): narrow-window layout fix — committed `2c9031b`, pushed

- `23ed80a` (desktop row virtualization, below) is committed + pushed.
- **Narrow-window layout fix** (the follow-up noted below, now RESOLVED): the
  real root cause was NOT the top bar shoving the central panel — the sort
  bar's `right_to_left` chip group inside `video_list`'s toolbar row overflows
  LEFT when the panel is narrow, and egui advances the parent's vertical
  cursor from the overflowed rect, shifting every row below it under the
  sidebar. Fix in `src/app.rs`:
  - `video_list`: sort chips are defined once in `SORT_CHIPS` (visual order);
    `sort_chip_row_width(ui)` measures the group via font metrics and the RTL
    right-aligned layout is used only when it fits (`sort_inline`), otherwise
    the chips render on their own `horizontal_wrapped` row below. Wide-window
    look unchanged.
  - Top bar: `ui.horizontal` → `ui.horizontal_wrapped` (nav buttons wrap to a
    second line instead of going off-screen); the right-aligned status label
    is width-guarded the same way with a truncating-label fallback (an
    overflowing RTL layout spills left over the buttons).
  - `SortMode` now derives `Copy`.
  - Verified by live GUI screenshots at 1000×700 and 1400×800 (scratchpad
    `final2-1000.png` / `final-1400.png`); 134+12 tests pass.
  - Screenshot-harness gotcha: egui renders on events; after `xdotool
    windowsize` you must `windowactivate` + `windowraise` + a couple of
    `mousemove --window` nudges or the capture races a stale frame (the
    "empty second toolbar line" ghost).

## Previous work (2026-07-08): desktop row virtualization — committed as `23ed80a`

- `main` was at `d4bed01` (perf pass phase 1: WAL pragmas, prepare_cached,
  batched scan writes, sort_by_cached_key, panic=abort) — committed + pushed.
- **`src/app.rs`: desktop row virtualization** (the follow-up the perf spec
  deferred; spec at
  `docs/superpowers/specs/2026-07-07-runtime-performance-pass-design.md`):
  - List/Card/Grid all render through `ScrollArea::show_rows` — only the
    visible row range is laid out per frame.
  - Row heights are enforced by fixed-size cells
    (`allocate_ui_with_layout` + `set_min_size`), built from
    `row_text_block_height(ui)` + density-scaled thumb height. Titles
    truncate to one line (`wrap_mode = Truncate`, full title on hover) —
    required for the fixed lattice.
  - List separator is a painted hline inside the cell (the old
    `ui.separator()` added unpredictable height). Grid is a manual
    horizontal run of `cols` fixed cells per lattice row (egui::Grid can't
    be windowed); `cols` is computed before `show_rows` from available width.
- **Verified (live GUI, screenshots in session scratchpad):**
  - List @ 2000 — windowed rendering, uniform lattice, both-direction scroll,
    stable-sort interleave, selection + details pane; @ 24 — exact bottom
    termination, single-line ellipsis truncation.
  - Card — uniform fixed-height cards, single-line truncation, hover ring.
  - Grid — 2 cols @1400px window, 3 cols @1500px, uniform lattice, no clip,
    truncated titles, full button rows. `cols` computed to fit (conservative;
    never overcommits/clips off-screen).
  - `cargo build --release` clean; `cargo test --release` = 134 unit + 12
    integration, 0 failures.
- **Known pre-existing issue — FIXED in the 2026-07-09 session (see top):** at
  the ~1000px window minimum width the central panel content underlapped the
  sidebar. Root cause was the sort bar's overflowing `right_to_left` group,
  not the top header as originally guessed.
- **GUI screenshot recipe on this box (Wayland/KWin):** winit defaults to a
  native Wayland surface that xdotool/`import` can't see. Launch with
  `env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11 DISPLAY=:0` to force XWayland,
  then `xdotool search --pid`, `windowsize`, and `import -window <wid> out.png`
  all work. The window has a hard min size ~1000px (xdotool can't shrink
  below it). Foreground `sleep` is blocked by the harness Bash tool — put
  launch+settle+capture in a script file and run that. `spectacle -b -a` was
  flaky here; `import -window <wid>` is reliable.

## TL;DR — where things are

- Project: **Catacomb** (crate/binary `catacomb`) — one Rust binary that is
  **both** an egui desktop GUI and an axum web server wrapping `yt-dlp`. See
  [CLAUDE.md](CLAUDE.md) for the authoritative architecture; [ROADMAP.md](ROADMAP.md)
  for the plan.
- Branch: `main`. Last commit `c12de9c` (command palette); `5f95bdb` and
  earlier are pushed, **`c12de9c` is committed but NOT yet pushed**. Remote:
  `https://codeberg.org/anassaeneroi/catacomb.git`.
- **Local checkout dir was renamed** `~/code/youtube-backup` → **`~/code/catacomb`**
  (code dir only; the library/backup dir at `/mnt/InannaBeloved/youtube-backup`
  is unchanged). Run the server with the right CWD, e.g.
  `env -C ~/code/catacomb ~/code/catacomb/target/release/catacomb --web 8081`.
- Only `HANDOFF.md` is uncommitted now.
- The project was **renamed yt-offline → Catacomb** (crate, binary, data
  paths with migration, UI, docs, repo URLs). See "The rename" below.
- A dev web server is usually run on **:8081** against the user's real library.
  It gets reaped between turns in this sandbox — just relaunch it (see gotchas).

## How to build / run / test

```bash
cargo build --release                 # ~1.5 min (opt-level=3 + thin LTO) → target/release/catacomb
cargo test --release                  # 128 unit + 11 integration (tests/api.rs); no network
./target/release/catacomb --web 8081  # headless web server (what the user uses)
./target/release/catacomb             # desktop GUI (default)
```

### CRITICAL gotchas

1. **The web SPA is one big embedded file** — `src/web_ui/index.html`, baked in
   at compile time via `include_str!`. **Editing it requires a `cargo build`** to
   take effect. A JS syntax error there will NOT be caught by `cargo build` (the
   HTML is just a string), so after every edit:
   ```bash
   awk '/<script>/{f=1;next}/<\/script>/{f=0}f' src/web_ui/index.html > /tmp/spa.js && node --check /tmp/spa.js
   ```
   `src/web_ui/login.html` is a second, separate embedded page (the login screen).
2. **DB location**: the server opens the DB at `channels_root.join("catacomb.db")`
   (`web.rs` ~L3127 and `app.rs` ~L395), i.e. **`<backup.directory>/catacomb.db`**.
   For this user that's `/mnt/InannaBeloved/youtube-backup/catacomb.db` (~64 MB,
   real data — do NOT clobber). `config.toml` is read from the **process CWD**.
   A stray 0-byte `catacomb.db` may sit in the repo dir; it's unused — ignore it.
3. **Offline-first**: this is a self-hosted, possibly-no-internet archiver.
   **Never load fonts/CSS/JS from a CDN.** UI fonts are embedded as base64 woff2
   (SIL OFL) directly in the `<style>` block. Keep it that way.
4. **All 10 themes** are driven by 7 CSS variables (`--bg --panel --card --accent
   --text --muted --border`) set per `.theme-*` class. Any new UI must use those
   vars (and the design-layer tokens below) so every theme inherits it.
5. **Background server reaping**: in the sandbox, `run_in_background` servers get
   killed between turns and shell `&`/`nohup` is unreliable. Just relaunch
   `./target/release/catacomb --web 8081`; `pkill -f "web 8081"` first.
6. **Verifying UI visually without a browser driver** (no puppeteer/playwright):
   extract the relevant CSS/JS from `index.html` into a `/tmp/*.html` harness with
   a mock payload and screenshot with headless chromium:
   ```bash
   chromium --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
     --window-size=1100,1300 --virtual-time-budget=3000 \
     --screenshot=/tmp/out.png "file:///tmp/harness.html"
   ```
   `--virtual-time-budget` can capture entry animations mid-flight; add a "settle"
   override (disable transitions, force final state) for a clean final shot.

## The rename (yt-offline → Catacomb)

Done and pushed across `404362b` (code), `3f61184` (docs), `5f95bdb` (repo URLs):

- **Crate/binary** `catacomb` (Cargo.toml, deb/rpm assets, PKGBUILD, package.sh,
  `catacomb.desktop`, launch.json, release CI, `tests/api.rs` → `CARGO_BIN_EXE_catacomb`).
- **Data paths**: DB `yt-offline.db` → `catacomb.db`; venv `~/.local/share/yt-offline`
  → `~/.local/share/catacomb`. `migrate_legacy_paths()` in `main.rs` adopts the old
  names on first run (renames DB + WAL/SHM sidecars + venv dir; best-effort, no-op
  once migrated). **Keep this function's OLD-name args as `yt-offline`** — a blunt
  rename sed will wrongly make both sides `catacomb` and break migration (happened
  once; watch for it).
- **Display**: window/tray/title/feed/login/SPA wordmarks → "Catacomb". Env
  override `YT_OFFLINE_RENDERER` → `CATACOMB_RENDERER`.
- Verified: builds, 128+11 tests pass, scratch-dir migration preserved bytes, and
  the live cutover migrated the real 64 MB DB + venv (server served the library).
- Repo URLs point at `…/anassaeneroi/catacomb` (Cargo.toml, PKGBUILD, package.sh).

## Settings flow (the easy thing to get wrong)

Adding a setting touches **five** places (grep an existing one like
`dedup_enabled` or `sponsorblock_mode` end-to-end first):

1. `config.rs` — field + `Default` + `default_with_dir()`.
2. `download_options.rs` — `Option<…>` per-channel override (None = use global).
3. `downloader.rs` — resolver merging global+override + a `pub` field on `Downloader`.
4. Both UIs: `app.rs` (egui Settings + channel-options dialog) AND
   `web_ui/index.html` (Settings modal + channel dialog) AND `web.rs`
   `SettingsPayload` (GET reads config, POST writes config + pushes to live `Downloader`).
5. Seed the `Downloader` field at construction AND on settings-save, in BOTH
   `app.rs` and `web.rs`.

## What this session shipped (committed, newest first)

- `5f95bdb` repo URLs → renamed Codeberg repo.
- `3f61184` / `404362b` **Rename → Catacomb** (see above).
- `6d2261b` **Login page reskin** — `src/web_ui/login.html`: serif wordmark +
  recording dot, aurora + grain, accent focus ring; system-serif (offline-safe).
- `2f95e7f` **Unified cinematic video player.** Replaced native `<video controls>`
  with ONE custom player for both direct + transcode. Custom scrubber
  (played/buffered/chapter-ticks/hover tooltip, drag-to-seek via pointer events,
  commits on release), speed popover (0.5–2×), persistent volume/speed/captions
  (localStorage `plPrefs`), CC toggle, PiP, fullscreen, auto-hiding controls +
  center flash, expanded keyboard (space/k j/l ←→ ↑↓ m c p f `<` `>` 0–9). All
  seeking routes through `playerSeek`/`effTime` (transcode reload-at-offset intact).
  CSS prefix `.pl-*`; JS around `playVideo`/`updateVctrl`/`plBindScrubber`.
- `816da05` **Persist login sessions in SQLite** — `sessions(token, issued_at)`
  table; map switched `Instant`→`u64`; insert/delete/clear + rehydrate at startup.
  Fixes "restart logs everyone out".
- `dd48e89` **api() 401 → reload to login** instead of a cryptic "error" toast.
- `615c088` **Maintenance modal → "diagnostics console"** (CSS `.mx-*`): pulsing
  health verdict + status tiles + instrument panels. Presentation only; handler
  ids/classes untouched.
- `6f61821` **Stats modal → "observatory" dashboard** (CSS `.sx-*`): count-up
  metrics, self-drawing SVG area chart, growing histogram, ranked leaderboard.
- `c8cb700` **"Cinematheque" reskin**: embedded Instrument Serif + Hanken Grotesk
  (base64 OFL), accent aurora + film-grain, cinematic card hover, recording dot.
- `983864f` macOS osxcross packaging; `207013e` Windows shippable .zip + CI;
  `8b25787` library sorting (download-date + grouped options).

### Design-layer conventions (web UI)

- CSS is appended near the end of the `<style>` block, AFTER the functional rules,
  so equal-specificity overrides win by source order. Prefixes: `.sx-*` = stats
  observatory, `.mx-*` = maintenance console, `.pl-*` = player.
- Reveal animations gate behind a class added post-layout (e.g. `.sx-go`) so they
  play once per open, not on re-render. All honor `prefers-reduced-motion` (global
  rule at the end of `<style>`).
- Fonts: `--font-display` (Instrument Serif), `--font-body` (Hanken Grotesk).
  Extra tokens: `--radius`, `--radius-sm`, `--ring`, `--glow`, `--shadow`, `--hair`.

## Command-palette search overlay — SHIPPED (`c12de9c`)

`openSearch()` is now a `.cp-*` command palette (CSS `.cp-*`; JS `openSearch`,
`runCommandSearch`, `renderCommandPalette`, `cpKeydown`, `updateSelected`,
`scrollIntoSelected`, `cpSelect`, `cpOpenRecent`, `cpClearRecents`,
`closeCommandPalette`, `ftsSnippet`). Blurred backdrop, centered input, results
grouped by channel with highlighted FTS snippets (`char(2)`/`char(3)` → `<mark>`
via `ftsSnippet`), ↑↓ nav, Enter to open (`cpSelect`→`selectVideo`), Esc to
close, recents + quick-actions in `localStorage['cp-recents']`. Debounced
queries to `/api/search?limit=60&q=…` with a `cpSeq` guard against stale
responses. Upload date is looked up client-side via `findVideo` (SearchHit has
no date). Triggers: 🔍 header button + `f` hotkey. Verified by build + headless
harness screenshot. Built into the running `:8081` binary.

## Suggested next steps (pick up here)

1. **The ACTIVE BUG at the top** — empty library at GUI launch (root cause
   not yet confirmed; evidence + next steps recorded there).
2. **Resume the remote-editor brainstorm** (see "Feature in flight" above).
3. A fresh web-UI surface to reskin/upgrade, or a roadmap item below.

Roadmap "surpass" items still open (see [ROADMAP.md](ROADMAP.md) §3):

- **3.1 macOS binary** — osxcross scaffolding done (`scripts/package.sh mac`);
  needs the toolchain + SDK installed, then verify. Windows already ships.
- **3.2 Android client** (big), **3.8 plugin/scripting hooks** (architectural).
- Phase 4 blue-sky: AI transcript summarisation (FTS transcript index already
  exists), TV-mode layout, multi-user accounts.
- Federation follow-up (3.5): in-UI "add remote" editor (peers are config-only).

## Watch-outs / open questions

- The video player's seek/speed/captions: verified in real daily use as of
  2026-07-09 (user confirmed) — retired as a watch-out.
- After the rename + session-persistence work, the user's existing browser cookie
  predates both, so they must **log in once more**; logins after that survive
  restarts. A download password may or may not be set (toggled during testing) —
  that's user-driven, not a bug.
- The user's local `config.toml` `source_url` still points at the old
  `…/yt-offline` repo (gitignored, user-specific) — change it in Settings if the
  UI "source" link should match the renamed repo.
- Don't commit `cookies.txt`, `config.toml`, or `catacomb.db` (all gitignored;
  contain creds / the Argon2 password hash).
