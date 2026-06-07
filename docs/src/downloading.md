# Downloading

## Starting a download

Paste any supported URL into the download bar (desktop) or the ⬇
Downloads modal (web). yt-offline classifies the URL by platform, routes
it to the right folder, and starts yt-dlp. A channel/playlist URL pulls
the whole thing; a single-video URL pulls just that one.

**Quality picker:** Best / 1080p / 720p / 480p / 360p, or **Music mode**
for audio-only extraction into `music/<artist>/`.

**Fast mode** stops at the first already-downloaded video (quick routine
re-checks). Turn it off for a full gap-filling scan.

## Per-channel options

Right-click a channel (or use the ⚙ on its sidebar row) for overrides
that apply to scheduled re-checks and the "Check for new videos" action:

- Quality cap, audio-only, bandwidth cap, min/max file size, date cutoff.
- A free-form `--match-filter` (e.g. `duration > 60 & view_count > 100`).
- Subtitle overrides, YouTube player-client override, post-download
  convert mode — each defaulting to the global setting.
- **Skip auth check** — silences yt-dlp's "playlists that require
  authentication" warning for **public** channels (see
  [Troubleshooting](./troubleshooting.md#the-youtubetab-authentication-warning)).

Per-channel options ride along in library backup/restore.

## Subtitles

Global defaults (Settings → Subtitles) + per-channel overrides control:
download on/off, auto-generated captions, embedding into the container,
language filter, and format conversion (`srt` is the most
Plex/player-compatible). Subtitles are written as sidecar files and
optionally embedded.

## Format conversion

A post-download ffmpeg pass (Settings → Format conversion, or per
channel):

- **Remux → mp4** — instant container change, no re-encode (device/Plex
  compatibility).
- **Re-encode → H.264 mp4** — shrink large 4K files at a chosen CRF +
  x264 preset.
- **Extract audio** — mp3 / m4a / opus / flac.

It runs as a distinct transcode job after the download. **Keep original**
preserves `<name>.original.<ext>` alongside the converted file; otherwise
the source is removed once the convert succeeds.

## Resilience

Downloads are hardened against YouTube's flakiness automatically:

- **Retry + backoff** on connection resets (`--retries 30`, linear
  retry-sleep).
- **Jittered throttle** between videos so a long channel scan doesn't
  look robotic and trip the captcha wall.
- **Auto-retry** of transient (rate-limit / network / captcha) failures
  after a cooldown, with adaptive slow-down for the rest of the batch.
- **Hang watchdog** kills a job that produces no output for 5 minutes (a
  wedged request) and re-queues it.

Failures are classified and shown with a one-line suggested fix — see
[Troubleshooting](./troubleshooting.md).

## Scheduler

Enable it (Settings → Auto-check channels) to re-check every channel for
new uploads on an interval. Each channel uses its own stored options.
There's also a per-folder "Check all" action.
