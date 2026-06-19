# Session handoff — Catacomb

> Working notes for continuing this work in another session/agent (e.g. Codex).
> Written 2026-06-19. Delete or update freely; this is not a tracked spec.

## TL;DR — where things are

- Project: `catacomb` — one Rust binary that is **both** an egui desktop GUI
  and an axum web server wrapping `yt-dlp`. See [CLAUDE.md](CLAUDE.md) for the
  authoritative architecture; [ROADMAP.md](ROADMAP.md) for the plan.
- Recent work has been almost entirely on the **web UI** (`src/web_ui/index.html`,
  a single `include_str!`-baked SPA) plus a few `web.rs`/`database.rs` changes.
- Branch: `main`. Last commit: `2f95e7f` (unified cinematic video player).
- A dev web server is usually running on **:8081** against the user's real
  library. It keeps getting reaped between turns — just restart it (see below).

## How to build / run / test

```bash
cargo build --release                 # ~1.5 min (opt-level=3 + thin LTO)
cargo test --release                  # 128 unit + 11 integration (tests/api.rs); no network
./target/release/catacomb --web 8081  # headless web server (what the user uses)
./target/release/catacomb             # desktop GUI (default)
```

### CRITICAL gotchas (learned this session)

1. **The web SPA is one big embedded file** — `src/web_ui/index.html`, baked in
   at compile time via `include_str!`. **Editing it requires a `cargo build`** to
   take effect. A JS syntax error there will NOT be caught by `cargo build` (the
   HTML is just a string), so after every edit:
   ```bash
   awk '/<script>/{f=1;next}/<\/script>/{f=0}f' src/web_ui/index.html > /tmp/spa.js && node --check /tmp/spa.js
   ```
2. **DB location**: the server opens the DB at `channels_root.join("catacomb.db")`
   (`web.rs` ~L3127), i.e. **`<backup.directory>/catacomb.db`**. For this user
   that's `/mnt/InannaBeloved/youtube-backup/catacomb.db` (~64 MB, real data —
   do NOT clobber). `config.toml` is read from the **process CWD**. A stray
   0-byte `catacomb.db` may sit in the repo dir; it is unused — ignore it.
3. **Offline-first**: this is a self-hosted, possibly-no-internet archiver.
   **Never load fonts/CSS/JS from a CDN.** UI fonts are embedded as base64 woff2
   (SIL OFL) directly in the `<style>` block. Keep it that way.
4. **All 10 themes** are driven by 7 CSS variables (`--bg --panel --card --accent
   --text --muted --border`) set per `.theme-*` class. Any new UI must use those
   vars (and the design-layer tokens below) so every theme inherits it.
5. **Background server reaping**: in the sandbox, `run_in_background` servers get
   killed between turns and shell `&`/`nohup` is unreliable. Just relaunch
   `./target/release/catacomb --web 8081` each time you need it up. `pkill -f`
   first to avoid a port clash.
6. **Verifying UI visually without a browser driver** (no puppeteer/playwright):
   extract the relevant CSS/JS from `index.html` into a `/tmp/*.html` harness with
   a mock payload and screenshot with headless chromium:
   ```bash
   chromium --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
     --window-size=1100,1300 --virtual-time-budget=3000 \
     --screenshot=/tmp/out.png "file:///tmp/harness.html"
   ```
   Note: `--virtual-time-budget` can capture entry animations mid-flight; add a
   "settle" override (disable transitions, force final state) for a clean shot.

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

## What this session shipped (committed)

Newest first (all on `main`):

- `2f95e7f` **Unified cinematic video player.** Replaced native `<video controls>`
  (direct `/files/` playback) with ONE custom player for both direct + transcode.
  Custom scrubber (played/buffered/chapter-ticks/hover tooltip, drag-to-seek via
  pointer events, commits on release), playback-speed popover (0.5–2×), persistent
  volume/speed/captions (localStorage `plPrefs`), CC toggle, PiP, fullscreen,
  auto-hiding controls + center play/pause flash, expanded keyboard
  (space/k j/l ←→ ↑↓ m c p f `<` `>` 0–9). All seeking routes through the
  existing `playerSeek`/`effTime` so the transcode reload-at-offset path is intact.
  CSS prefix `.pl-*`; JS around `playVideo`/`updateVctrl`/`plBindScrubber`.
- `816da05` **Persist login sessions in SQLite.** New `sessions(token, issued_at)`
  table; in-memory map switched `Instant`→`u64` unix secs; insert on login, delete
  on logout, clear on password change, rehydrate at startup via `load_sessions()`.
  Fixes "restart logs everyone out". (`database.rs` + `web.rs`.)
- `dd48e89` **api() 401 → reload to login** instead of a cryptic "error" toast.
- `615c088` **Maintenance modal → "diagnostics console"** (CSS prefix `.mx-*`):
  pulsing health verdict + status tiles + instrument-panel sections. Presentation
  only; all handler ids/classes (`dup-chk` `sim-chk` `at-chk` `dedup-area`
  `autotag-area`, polling) untouched.
- `6f61821` **Stats modal → "observatory" dashboard** (CSS prefix `.sx-*`):
  count-up metric cards, self-drawing SVG area chart (Catmull-Rom), growing
  histogram, ranked leaderboard with Size/Count toggle.
- `c8cb700` **"Cinematheque" reskin**: embedded Instrument Serif + Hanken Grotesk
  (base64 OFL), accent aurora + film-grain atmosphere, cinematic card hover,
  recording-dot wordmark. Design-layer CSS appended after the functional rules so
  it overrides by source order; everything flows through the theme variables.
- `983864f` **macOS osxcross packaging path** (`scripts/package.sh mac` → `.app`
  zip; local-only, not in CI — needs osxcross + SDK).
- `207013e` **Windows shippable**: cross-compiled `.zip` via mingw + console
  reattach fix (`attach_windows_console` in `main.rs`) + Windows build in the
  Forgejo release CI.
- `8b25787` **Library sorting**: download-date sort (file mtime) + grouped sort
  options, both UIs.
- earlier: `015d037` scan leaves one core free; `5ffdb17` download-modal
  diff-aware repaint + Retry-all; `9ed6293` download cancel/retry/queue + dedup
  off-switch.

### Design-layer conventions (web UI)
- CSS is appended near the end of the `<style>` block, AFTER the functional
  rules, so equal-specificity overrides win by source order. Prefixes:
  `.sx-*` = stats observatory, `.mx-*` = maintenance console, `.pl-*` = player.
- Reveal animations gate behind a class added post-layout (e.g. `.sx-go`) so they
  play once per open, not on every re-render. All honor
  `@media (prefers-reduced-motion: reduce)` (a global rule at the end of `<style>`).
- Fonts: `--font-display` (Instrument Serif), `--font-body` (Hanken Grotesk).
  Extra tokens: `--radius`, `--radius-sm`, `--ring`, `--glow`, `--shadow`, `--hair`.

## Uncommitted changes (REVIEW + COMMIT before continuing)

`git status` shows modified-but-uncommitted (mostly docs, made by the user/linter
— do NOT revert, just review and commit):
- Docs/prose: `AGENTS.md`, `CLAUDE.md`, `README.md`, `ROADMAP.md`,
  `SECURITY_AUDIT.md`, `docs/src/{architecture,first-run,installation,packaging}.md`
- Small code touches: `src/database.rs` (~4 lines — schema doc/comment),
  `src/web_ui/index.html` (6/6 — minor).

Run `git diff` to confirm these are intended, then commit them. They look like
documentation catch-up for the session's features.

## Suggested next steps (pick up here)

Highest-leverage, all buildable + verifiable locally:
1. **Login page reskin** — `LOGIN_HTML` in `web.rs` is a plain form and is the one
   surface that still clashes with the cinematheque aesthetic. Small, high impact.
2. **Search overlay** (`openSearch` in the SPA) — could become a command-palette
   experience to match the new players.
3. Commit the pending doc changes (above).

Roadmap "surpass" items still open (see [ROADMAP.md](ROADMAP.md) §3):
- **3.1 macOS binary** — osxcross scaffolding done (`scripts/package.sh mac`);
  needs the toolchain + SDK installed, then verify. Windows already ships.
- **3.2 Android client** (big), **3.8 plugin/scripting hooks** (architectural).
- Phase 4 blue-sky: AI transcript summarisation (FTS transcript index already
  exists), TV-mode layout, multi-user accounts.
- Federation follow-up (3.5): in-UI "add remote" editor (peers are config-only).

## Watch-outs / open questions

- The video player's seek/speed/captions are verified by code-path review +
  static chrome screenshot, NOT live playback (no headless video). Worth a real
  click-through in the app.
- Session persistence: the user's *current* browser cookie predates the
  persistence commit, so they must log in once more; logins after that survive
  restarts. (They may have a download password set or not — it has toggled during
  testing; that's user-driven, not a bug.)
- Don't commit `cookies.txt`, `config.toml`, or `catacomb.db` (all gitignored;
  contain creds / the Argon2 password hash).
