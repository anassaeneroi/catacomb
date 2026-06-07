# Staying ahead of YouTube's bot detection

YouTube increasingly fingerprints and rate-limits automated clients.
yt-offline ships a layered defense; understanding the layers makes the
difference between "everything downloads" and "constant captchas."

In rough order of impact:

## 1. Be logged in (cookies)

This is the single biggest factor. **Anonymous requests get captcha-
walled the hardest.** Provide a `cookies.txt` exported from a browser
where you are **signed in to YouTube**, or point yt-offline at the
browser profile directly.

A *valid* logged-in jar contains the auth cookies `SID`, `SAPISID`,
`__Secure-1PSID`, `__Secure-3PSID`, `LOGIN_INFO`, etc. A jar with only
`VISITOR_INFO1_LIVE`, `PREF`, `YSC` is **anonymous** — it's not signed in
and actually makes detection *worse* than no cookies. yt-offline's
Settings → Cookies panel warns you when your jar is anonymous or expired.

Two ways to supply cookies:

- **Export a `cookies.txt`** with a browser extension like *Get
  cookies.txt LOCALLY*, then paste/upload it in Settings → Cookies. A
  file in the working directory takes precedence over the browser option.
- **Read the browser profile live** by setting the cookie browser to a
  yt-dlp `--cookies-from-browser` spec. Plain `firefox`/`chrome`/`brave`
  works for default profiles; for a non-default profile (e.g. Brave's
  beta channel) use the full form:

  ```
  brave:/home/you/.config/BraveSoftware/Brave-Origin-Beta
  ```

  The path is the **profile root** (yt-dlp appends the `Default`
  subdirectory itself). The advantage: cookies are read fresh from the
  live session each download, so they never go stale.

> Cookies are session credentials — yt-offline never commits or transmits
> `cookies.txt` unprompted, and redacts the cookie path out of any log
> line shown in the UI.

## 2. TLS impersonation (curl_cffi)

yt-dlp's `--impersonate` makes requests carry a real browser's TLS
fingerprint (via `curl_cffi`), so the connection doesn't *look* like a
script. The bundled install sets this up automatically and yt-offline
picks an impersonation target per platform.

If impersonation silently does nothing, it's almost always a
**yt-dlp ⇄ curl_cffi version mismatch** — which is exactly why the
bundled install uses **nightly** yt-dlp (it accepts current curl_cffi;
stable lags and disables all impersonate targets when a newer curl_cffi
is present). See
[Troubleshooting → impersonation](./troubleshooting.md#impersonate-targets-show-unavailable).

## 3. POT tokens (Proof-of-Origin)

YouTube increasingly binds a per-video **Proof-of-Origin token** to
playback; without one, format URLs come back empty. yt-offline can run
[bgutil-pot](https://github.com/jim60105/bgutil-ytdlp-pot-provider-rs), a
loopback HTTP server that mints these tokens, and point yt-dlp at it.

Enable **Settings → Use POT token provider** (requires the bundled
yt-dlp; the matching plugin installs into its venv) and click **Install**.

> **Version-skew footgun:** the yt-dlp plugin must come from the *same
> release* as the bgutil-pot server binary — **not** the PyPI package,
> which versions independently and silently produces no tokens on a
> mismatch. yt-offline's installer handles this by unpacking the
> version-matched plugin zip from the server's release.

## 4. Player-client selection

YouTube cracks down on different internal "player clients" over time —
the `web` client is currently the most captcha-prone, while `tv` and
`mweb` are the least. yt-offline no longer forces `web`; it lets yt-dlp
pick good defaults. If a specific channel keeps hitting captchas, set a
client override (global or per-channel):

```
tv,mweb
```

## 5. Throttling

A burst of ~30 rapid requests is a classic trip-wire. yt-offline inserts
a small jittered pause between videos (a fixed cadence looks robotic; a
random one looks human) and, after any rate-limit hit, triples the
sleeps for the rest of the batch before recovering.

---

**TL;DR for a clean setup:** bundled (nightly) yt-dlp + fresh
**logged-in** cookies + POT provider enabled. That combination resolves
the vast majority of captcha / "Video unavailable" failures.
