# Troubleshooting

yt-offline classifies failed downloads into one of nine classes and shows
a one-line suggested fix next to the failed job. This page expands on the
most common ones, plus a few non-download issues.

## "Video unavailable. YouTube is requiring a captcha challenge"

**Class:** rate-limited. **Not** a removed video — it's the bot-detection
wall. In order of effectiveness:

1. **Use fresh, logged-in cookies.** Anonymous cookies are the usual
   culprit — see [Anti-bot → cookies](./anti-bot.md#1-be-logged-in-cookies).
   Settings → Cookies warns when your jar is anonymous or expired.
2. **Switch to bundled (nightly) yt-dlp** if you're on system stable.
3. **Enable the POT token provider.**
4. **Try a player-client override** of `tv,mweb` for that channel.
5. If it's a one-off, just wait — yt-offline auto-retries transient
   rate-limits after a cooldown.

## Impersonate targets show "(unavailable)"

`yt-dlp --list-impersonate-targets` lists every target as `(unavailable)`
even though `curl_cffi` is installed.

**Cause:** a yt-dlp ⇄ curl_cffi version gate. Stable yt-dlp caps the
curl_cffi version it accepts; a newer curl_cffi makes it disable *all*
impersonate targets.

**Fix:** use the **bundled** yt-dlp (it installs nightly via `--pre`,
which accepts current curl_cffi), or pin curl_cffi to a compatible
version in your own environment.

## POT provider produces no tokens

You enabled the POT provider and installed it, but downloads still fail
as if no token was generated. yt-dlp logs a *"plugin and HTTP server
major versions are mismatched"* warning.

**Cause:** the yt-dlp plugin came from PyPI (Brainicism's package, which
versions independently) instead of the jim60105 Rust server's release.

**Fix:** re-run the POT **Install/Update** button — yt-offline installs
the version-matched plugin zip from the same release as the server
binary. Don't `pip install bgutil-ytdlp-pot-provider` yourself.

## The youtubetab authentication warning

```
ERROR: [youtube:tab] @Channel: Playlists that require authentication may
not extract correctly without a successful webpage download...
```

Despite the `ERROR:` prefix this is a soft warning, usually a symptom of
the bot-detection issues above (YouTube served an incomplete page). It
does **not** change which videos are found.

**Fix:** for **public** channels, enable **Skip auth check** in that
channel's options (adds `--extractor-args youtubetab:skip=authcheck`) to
silence it. Leave it **off** for members-only/private channels you
archive with cookies — there the warning is a real "your cookies may not
be working" signal.

## "Sign in to confirm you're not a bot"

Same family as the captcha wall. Fix with fresh logged-in cookies + POT;
see [Anti-bot](./anti-bot.md).

## Downloads stall forever

A job sits running with no progress. yt-offline's **hang watchdog**
auto-kills any job silent for 5 minutes and re-queues it, so this should
self-heal. If it recurs on a specific URL, it's usually a server-side
issue with that source; check the job log in the Downloads panel.

## Disk fills up / downloads fail with ENOSPC

yt-offline runs a **disk-full preflight** and refuses to start a download
when the target filesystem has less than ~500 MB free, surfacing it as a
clear "disk full" failure rather than a half-written file. Free space and
retry.

## A whole platform folder shows up as one "channel"

If you see `bandcamp`, `tiktok`, or `channels` listed as a single channel
in the sidebar, your library directory predates the current layout. All
platforms must **nest under** the one `backup.directory`
(`<dir>/channels/`, `<dir>/tiktok/`, …). Move stray creator folders into
their platform's subdir; see [First run → library layout](./first-run.md#the-library-layout).

## The desktop window crashes on maximize

Older builds crashed with a Glutin `EGL_BAD_ALLOC` on NVIDIA + Wayland
when maximized. Current builds use the **wgpu (Vulkan)** renderer, which
handles the resize cleanly. Make sure you have a working Vulkan driver
(`vulkan-icd-loader` + your GPU's Vulkan package), which any desktop with
working graphics already has.

## The desktop window opens but stays blank

The desktop defaults to the wgpu/Vulkan renderer. On some systems with a
broken Vulkan stack, the OS can create the window while wgpu never
presents a frame. Try the OpenGL renderer:

```bash
YT_OFFLINE_RENDERER=glow yt-offline
```

or:

```bash
yt-offline --renderer glow
```

If that works, update/reinstall your Vulkan driver later and switch back
to the default renderer. On NVIDIA + Wayland, prefer the default wgpu
renderer when possible because OpenGL has historically crashed on window
maximize.

## The web UI looks like an old version after an upgrade

The SPA is served `Cache-Control: no-store`, so a hard reload
(Ctrl+Shift+R) always picks up the new binary's UI. If you upgraded the
binary, also **restart the running `--web` process** — the HTML is baked
into the binary at compile time, so the old process keeps serving the old
UI until restarted.

## Where to look next

- **The job log** — every download/transcode job keeps its full yt-dlp /
  ffmpeg output in the Downloads panel (expand the job).
- **`yt-offline.crash.log`** — next to your `yt-offline.db`. A panic in
  any thread (UI, web worker, download) is appended here with a
  timestamp, so it survives a GUI launched without a terminal.
