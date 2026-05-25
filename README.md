# yt-offline

![yt-offline icon](icon.png)

A self-hosted media archive for YouTube and friends. Pastes any URL, routes it
to the right folder by source, tracks what you've watched, and lets you play
everything back from a desktop GUI or browser — even when you're offline or
the source video has been taken down.

Built on [yt-dlp](https://github.com/yt-dlp/yt-dlp); written in Rust.

## What it backs up

| Platform                     | Channels    | Playlists   | Single videos |
| ---------------------------- | ----------- | ----------- | ------------- |
| YouTube                      | ✅          | ✅          | ✅            |
| TikTok                       | ✅          | —           | ✅            |
| Twitch (VODs + clips)        | ✅          | —           | ✅            |
| Vimeo                        | ✅          | ✅          | ✅            |
| Bandcamp                     | ✅ (artist) | ✅ (albums) | ✅ (tracks)   |
| SoundCloud                   | ✅          | ✅ (sets)   | ✅            |
| Odysee                       | ✅          | —           | ✅            |
| Anything else yt-dlp accepts | `other/`    | `other/`    | `other/`      |

Each source lands in its own sibling folder under your backup directory
(`channels/` for YouTube, `tiktok/`, `twitch/`, etc.). A `.source-url`
sidecar is dropped in every creator folder so re-checks always know the
exact URL to refresh from.

## Features

### Library

- **Multi-platform sidebar** grouped by source, with platform icons.
- **Search** across your whole library, instant.
- **Sort by:** Newest / Oldest (upload date), Largest / Smallest, Longest /
  Shortest, Title.
- **Watched tracking** + **resume positions** persisted in SQLite.
- **Continue watching** view of partially-played videos.
- **Subtitles** (auto + manual, SRT and WebVTT) + **chapters** + **SponsorBlock**
  marks embedded into the video file.
- **Statistics** — totals, top channels by size/count, weekly downloads
  histogram, upload-year histogram.
- **Maintenance** — finds duplicate video IDs across folders and missing
  sidecars (thumbnail / info.json / description), with one-click repair.

### Playback

- **Desktop GUI** launches your configured player (mpv by default; mpv gets
  resume-position tracking via JSON-IPC).
- **Web UI** plays videos inline in any browser, with subtitle tracks and a
  chapters panel. Optional on-the-fly **ffmpeg transcoding** for browsers
  that can't decode MKV.

### Downloads

- **Bundled yt-dlp + curl_cffi + deno** — one click in Settings sets up a
  self-contained Python venv with `yt-dlp[default]` plus `curl_cffi`
  (browser-impersonation for bot-detection bypass) and `deno` for player-JS
  evaluation. No system installation required.
- Or use **system yt-dlp** — toggleable in Settings.
- **Concurrent-download limit** (default 3) with a visible queue.
- **Quality picker:** Best / 1080p / 720p / 480p / 360p, or **Music mode**
  for audio-only extraction into `music/<artist>/`.
- **Retry hardening:** `--retries 30 --fragment-retries 30
  --retry-sleep linear=1:30:2 --sleep-requests 1` so connection resets self-heal
  instead of failing the job.
- **Scheduler** — periodically re-check every channel for new uploads.
- **Cookies** — paste / file-pick a Netscape `cookies.txt`, or fall back to
  `--cookies-from-browser` for whatever browser you pick.

### Web server

- Single binary, no Node / Python / Docker required (Python only for the
  optional bundled yt-dlp venv).
- **Bind interface picker** — localhost only (default), Tailscale, LAN, or
  all interfaces.
- **Password-gated UI** when enabled — Argon2 hashed, 256-bit session
  tokens, 30-day TTL with lazy pruning, per-IP rate-limit on `/api/login`
  (5 failures → 60s lockout).
- **Security headers:** Content-Security-Policy, X-Frame-Options DENY,
  X-Content-Type-Options nosniff, Referrer-Policy no-referrer.
- **`Secure` cookie flag** when `X-Forwarded-Proto: https` is present
  (i.e. behind a reverse proxy doing TLS).
- 4 MiB request body limit on every route.

### Plex integration

- One click generates a symlink tree your Plex "TV Shows" library can ingest
  directly: `<plex_root>/<show>/Season 2024/<show> - S2024E001 - title.mkv`.
- Writes `.nfo` sidecars per episode (title, season, episode, aired date,
  runtime, plot) plus a show-level `tvshow.nfo`, so Plex's "Personal Media
  (TV Shows)" agent picks up YouTube metadata correctly.
- Symlinks the thumbnail as `<stem>-thumb.jpg` so Plex shows episode art.
- Non-YouTube creators get `<Platform> - <handle>` show folders to avoid
  collisions across sources.

### Themes

Ten themes shared across desktop and web — Dark, Light, Dracula, Trans,
plus three Emo/Scene flavours (Nocturnal, Coffin, Scene Queen).

### Misc

- Native notifications when downloads finish (notify-rust / zbus, no GTK
  build dep).
- `/api/stats`, `/api/ytdlp/update`, `/api/scheduler/run` — every UI button
  has an equivalent JSON endpoint.

## Quick start

```bash
git clone https://codeberg.org/anassaeneroi/yt-offline
cd yt-offline
cargo build --release

# Desktop GUI
./target/release/yt-offline

# Or web server only (headless)
./target/release/yt-offline --web 8080
```

On first run a `config.toml` is created in the working directory. Settings
can also be changed from inside either UI — no manual editing required.

### Bundled yt-dlp setup

1. Open **Settings**.
2. Under **yt-dlp binary**, choose **Bundled** and click **Install**.
3. The installer creates `~/.local/share/yt-offline/venv/` with
   `yt-dlp[default]` + `curl_cffi`, plus `~/.local/share/yt-offline/bin/deno`.
   Progress streams into a regular job entry.

Updates are the same button. Switching to **System** uses whatever `yt-dlp`
is on PATH instead.

## Building

### Arch / CachyOS / Manjaro

```bash
sudo pacman -S --needed rust mpv  # python3 + python-pip already present
git clone https://codeberg.org/anassaeneroi/yt-offline
cd yt-offline
makepkg -si           # or: cargo build --release
```

A `PKGBUILD` is included.

### Debian / Ubuntu

```bash
sudo apt install \
  build-essential pkg-config curl git python3-venv \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libssl-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
cargo build --release
```

`python3-venv` is required for the bundled yt-dlp install path (skipable
if you only ever use system yt-dlp).

### macOS

```bash
xcode-select --install
brew install mpv
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
cargo build --release
```

### Windows

Install [Rust](https://rustup.rs/) (MSVC toolchain), Visual Studio Build
Tools with the **Desktop development with C++** workload, then:

```powershell
winget install mpv.net python.python.3.12
git clone https://codeberg.org/anassaeneroi/yt-offline
cd yt-offline
cargo build --release
```

## Configuration

`config.toml` lives next to the binary (or in the working directory). All
fields are also editable in Settings.

```toml
[backup]
directory      = "/path/to/library/channels"   # YouTube root; siblings hold other platforms
max_concurrent = 3                              # parallel yt-dlp processes
use_bundled_ytdlp = false                       # true = use the venv set up by the Install button

[player]
command = "mpv"      # any executable that takes a file path as last arg
browser = "firefox"  # used by --cookies-from-browser when no cookies.txt is set

[ui]
theme = "dark"       # dark | light | dracula | trans | emo-nocturnal | emo-coffin | emo-scene-queen

[scheduler]
enabled        = false
interval_hours = 24

[web]
port       = 8080
bind       = "127.0.0.1"   # 127.0.0.1 | 0.0.0.0 | a Tailscale/LAN address
transcode  = false          # MKV → MP4 on the fly for browsers (needs ffmpeg)
source_url = "https://codeberg.org/anassaeneroi/yt-offline"  # AGPL §13 footer link

[plex]
library_path = "/path/to/plex/TV/youtube"   # leave unset to disable
```

## Usage

### Desktop GUI

1. Click **⬇ Downloads**, paste any URL — channel, video, playlist, or
   profile from any supported platform. The dialog shows the detected
   source and target folder before you confirm.
2. Choose quality (or **🎵 Music** for audio-only).
3. Pick **Fast mode** to stop at the first already-archived video, or leave
   it off for a full back-fill.

The sidebar groups channels by platform. Right-click any channel for
**Check for new videos** and **Open folder**. The download bar shows the
active job count plus what's queued.

Other top-bar buttons:

- **⟳ Rescan** — re-read the library directory.
- **📊 Stats** — totals, top channels, weekly download histogram.
- **🩺 Maintenance** — find and resolve duplicates / missing sidecars.
- **⚙ Settings** — everything in `config.toml`, plus cookies, password,
  bundled-yt-dlp install/update, and starting the web server.

### Web UI

Start the server (`--web` CLI flag or **Start** button in Settings →
**Web server**), open `http://localhost:8080` (or your configured bind
address). The UI mirrors the desktop one and adds inline video playback.

If a download password is set, you'll be redirected to a login page;
the session cookie is HttpOnly + SameSite=Strict, valid 30 days.

## Library layout

```text
/path/to/library/
├── channels/                    ← YouTube
│   ├── @creator-name/
│   │   ├── .source-url          ← original URL, for re-checks
│   │   ├── archive.txt          ← yt-dlp's downloaded-ID record
│   │   ├── Video Title [abc123].mkv
│   │   ├── Video Title [abc123].webp
│   │   ├── Video Title [abc123].info.json
│   │   ├── Video Title [abc123].description
│   │   └── Video Title [abc123].en.vtt
│   └── @another/
├── tiktok/
├── twitch/
├── vimeo/
├── bandcamp/
├── soundcloud/
├── odysee/
├── other/
├── music/
│   └── <artist>/
│       └── Track Title [xyz].opus
└── yt-offline.db                ← watched + positions + password hash
```

## Troubleshooting

**"Impersonate target ... is not available"**
You're on bundled mode but the venv install hasn't completed. Open
Settings → yt-dlp binary → click **Install** and watch the job for the
"impersonation targets available" line.

**`yt-dlp` not found (system mode)**
Install yt-dlp via your package manager or `pip install yt-dlp`, or
switch to **Bundled** in Settings.

**Web UI says "authentication required" but no password is set**
Sessions cleared after the password was changed. Refresh the page.

**Videos play but seeking jumps to the start**
That's the transcoding mode (`web.transcode = true`). MKV streams can't be
seeked while transcoding live; turn it off if your browser can play MKV
directly.

**Connection-reset / "Recv failure" errors on download**
The defaults already retry 30× with linear backoff. If they're persistent,
your IP is being rate-limited — paste fresh cookies from a browser session
into Settings → Cookies.

**Build fails on Debian/Ubuntu**
Make sure all packages from the build dependencies block are installed,
plus `python3-venv` if you want the bundled yt-dlp install path.

## Security

See [SECURITY_AUDIT.md](SECURITY_AUDIT.md) — the codebase has been audited
end-to-end, covering command/SQL injection, path traversal, XSS, session
management, rate-limiting, supply-chain (bundled binary SHA verification),
and dependency status.

## License

[AGPL-3.0-or-later](LICENSE). When you run this software as a network
service, the AGPL requires that you offer network users access to the
Corresponding Source — the running version, with any modifications.
Set `web.source_url` in config to a public URL where the source is
available; the web UI surfaces it in the footer and `GET /api/settings`.
