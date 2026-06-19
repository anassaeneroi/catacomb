# Catacomb

A self-hosted media archive for YouTube and friends. Paste any URL, it
routes the download to the right folder by source, tracks what you've
watched, and plays everything back from a desktop GUI **or** a browser —
even offline, or after the source video is taken down.

Built on [yt-dlp](https://github.com/yt-dlp/yt-dlp); written in Rust as a
single binary that is **both** a desktop app and a headless web server.

## What it backs up

| Platform | Channels | Playlists | Single videos |
|---|---|---|---|
| YouTube | ✅ | ✅ | ✅ |
| TikTok | ✅ | — | ✅ |
| Twitch (VODs + clips) | ✅ | — | ✅ |
| Vimeo | ✅ | ✅ | ✅ |
| Bandcamp | ✅ (artist) | ✅ (albums) | ✅ (tracks) |
| SoundCloud | ✅ | ✅ (sets) | ✅ |
| Odysee | ✅ | — | ✅ |
| Anything else yt-dlp accepts | `other/` | `other/` | `other/` |

## Why it exists

Tartube is the mature open-source yt-dlp GUI and the benchmark in this
space. catacomb matches its feature set while adding things Tartube
doesn't have: a real web UI reachable from any device, a single-binary
distribution with a bundled toolchain, a modern security model
(password-gated UI, Argon2, rate-limited login), a built-in anti-bot
stack (TLS impersonation + Proof-of-Origin tokens), and one-click
post-download format conversion.

## How to read these docs

- New here? **[Installation](./installation.md)** →
  **[First run](./first-run.md)** → **[Downloading](./downloading.md)**.
- Hitting captchas or "Video unavailable"? Go straight to
  **[Anti-bot](./anti-bot.md)** and **[Troubleshooting](./troubleshooting.md)**.
- Hacking on it? **[Architecture](./architecture.md)**.
