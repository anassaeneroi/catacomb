# First run & configuration

## The config file

catacomb reads `config.toml` **from its working directory** (the
directory you launch it from), not a fixed path. The same goes for
`cookies.txt`. Everything in `config.toml` is also editable in Settings;
edits there are written back to the file.

```toml
[backup]
directory          = "/path/to/library"   # the umbrella dir; all platforms nest under it
max_concurrent     = 3                     # parallel yt-dlp processes
use_bundled_ytdlp  = false                 # true = use the managed venv (see below)
use_pot_provider   = false                 # YouTube Proof-of-Origin tokens (see Anti-bot)
youtube_player_clients = ""                # e.g. "tv,mweb" to route around captchas

[player]
command = "mpv"          # any executable taking a file path as its last arg
browser = "firefox"      # cookie source when no cookies.txt is set (see Anti-bot)

[ui]
theme       = "dark"     # dark | light | dracula | trans | emo-nocturnal | emo-coffin | emo-scene-queen
ui_scale    = 1.0        # global zoom for the whole desktop UI

[scheduler]
enabled        = false
interval_hours = 24      # auto re-check every channel for new uploads

[web]
port      = 8080
bind      = "127.0.0.1"  # 127.0.0.1 | 0.0.0.0 | a Tailscale/LAN address
transcode = false        # MKV → MP4 on the fly for browsers that can't decode MKV

[subtitles]
enabled        = true
auto_generated = true    # include machine captions
embed          = false   # also embed into the container
format         = ""      # "" = native; "srt" for Plex compatibility
langs          = ""      # "" = all; "en" or "en,ja" to filter

[convert]
mode           = ""      # "" / "remux-mp4" / "h264-mp4" / "audio"
crf            = 23       # for h264-mp4 (lower = bigger/better)
preset         = "medium"
audio_format   = "mp3"    # for audio mode
keep_original  = false    # keep <name>.original.<ext> after converting

[plex]
library_path = "/path/to/plex/TV/youtube"   # leave unset to disable
```

## The library layout

Everything nests under the one `backup.directory`:

```text
<backup.directory>/
  channels/        ← YouTube creators
  tiktok/  twitch/  vimeo/  bandcamp/  soundcloud/  odysee/  other/
  music/           ← audio-only "Music mode" downloads, by artist
  archive.txt      ← yt-dlp's global download archive
  cookies.txt      ← optional, if you set one
  catacomb.db    ← plain SQLite app state (watched/positions/flags/notes, password hash, sessions, caches)
```

Each creator folder gets a hidden `.source-url` sidecar so re-checks
always know the exact URL to refresh from.

## Bundled vs system yt-dlp

In **Settings → yt-dlp binary** you choose:

- **System** — uses whatever `yt-dlp` is on your `PATH`.
- **Bundled** — click **Install** and catacomb builds a self-contained
  venv at `~/.local/share/catacomb/`: nightly `yt-dlp[default]` +
  `curl_cffi` (TLS impersonation) + a bundled `deno` (player-JS). The
  same button updates it later.

The bundled path is recommended — it installs **nightly** yt-dlp, which
keeps pace with YouTube's frequent anti-bot changes (stable lags). It's
also required for the [POT token provider](./anti-bot.md#3-pot-tokens-proof-of-origin).

## The two front-ends

- `catacomb` — desktop GUI (eframe/egui).
- `catacomb --web [PORT]` — headless web server. Bind to `127.0.0.1`
  (default) for localhost-only, a Tailscale address for your tailnet, or
  `0.0.0.0` for the LAN. **Set a password** (Settings) before exposing it
  beyond localhost — the UI and all `/api` routes are then gated behind an
  Argon2-hashed, rate-limited login.
