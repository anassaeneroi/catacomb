# Security Audit — yt-offline

**Date:** 2026-05-23 (re-audit after security hardening commit `5999673`)
**Earlier audit:** 2026-05-17 (commit pre-dating session auth, CSP, SHA-256 verify, etc.)
**Scope:** Rust codebase + embedded web UI + dependencies
**Threat model:** Self-hosted personal archiving tool. Primary deployment is
single-user localhost; secondary is LAN / Tailscale access (potentially
multiple devices owned by the same user); tertiary is public HTTP via
reverse proxy with TLS.

---

## Summary

✅ **Overall risk: LOW** for localhost and Tailscale deployments.
⚠️ **MEDIUM** for plain-HTTP LAN exposure (mitigated by SameSite=Strict cookie
+ CSP; defense-in-depth, no transport encryption).

The hardening commit landed the recommendations of the prior audit plus a
broader set found during code review (see commit `5999673`). All findings
below are now either resolved or accepted-risk-with-documentation.

---

## Threat model

| Adversary | Capability | In scope? |
|---|---|---|
| Local user on same machine | Reads files owned by `luna` | Yes — DB chmod, cookies.txt |
| LAN attacker on same subnet | Sees HTTP traffic, can connect to bound ports | Yes — bind policy, auth, CSP |
| Remote attacker via Tailscale tailnet | Owned by same user, but still adversarial in principle | Yes |
| Public internet attacker | Reaches server through misconfigured reverse proxy | Out of scope (not the intended deployment), but defenses still apply |
| Browser-side XSS via injected video metadata | Compromised channel name / description from yt-dlp | Yes — primary XSS surface |
| Supply-chain attack on bundled binaries | Malicious yt-dlp / deno served from GitHub | Yes — SHA-256 verify |

The tool **does not** defend against an attacker with shell access to the
machine, an attacker who can write to the channels directory, or compromise
of upstream YouTube cookies the user has chosen to import.

---

## Findings

### ✅ Command injection — SECURE

All external process invocations use `std::process::Command` /
`tokio::process::Command` with separate `.arg()` calls; no shell
interpretation. Sites audited:

- yt-dlp downloads: `src/downloader.rs:start`, `start_music`, `repair`
- yt-dlp preview: `src/web.rs:get_preview` (now goes through `apply_cookie_flags` and respects bundled-binary setting)
- yt-dlp bundled install: `src/ytdlp_bin.rs:install_command` runs `bash -c` with a static script whose only interpolated values are paths under `~/.local/share/yt-offline/` and constant GitHub URLs — no user input flows in.
- ffmpeg transcoder: `src/web.rs:get_transcode` (kill_on_drop set; child terminates when stream is dropped).

Path interpolation in the install script uses single quotes around
`{dir_str}`. The path is derived from `$HOME` via `std::env::var_os`. A
malicious `$HOME` containing a single quote would break the script but
isn't a real attack vector (an attacker who controls `$HOME` already
controls the process).

### ✅ SQL injection — SECURE

All queries in `src/database.rs` use parameterized statements (`?1` /
`rusqlite::params!`). No string concatenation in SQL. Re-verified after
the `settings` table was added.

### ✅ Path traversal — SECURE

- `/files/*` and `/music-files/*`: served by `tower-http`'s `ServeDir`, which rejects `..` and refuses to follow symlinks out of the served root.
- `/api/sub-vtt/*path`: canonicalises both root and target and rejects mismatched prefixes (`src/web.rs:get_sub_vtt`).
- `/api/transcode/:id`, `/api/chapters/:id`, `/api/metadata/:id`, `/api/description/:id`: all resolve through `library::find_video`, which only returns files indexed by the scanner — direct path injection is impossible.
- `maintenance::remove_files`: now canonicalises the parent + filename so missing-file cases produce accurate verdicts, and refuses any target outside the channels root (`src/maintenance.rs:is_within`).

### ✅ XSS in embedded web UI — DEFENDED

- All user-influenced strings (channel names, video titles, descriptions, uploader names, IDs) are escaped via `esc()` before HTML interpolation.
- URLs in `src=` attributes go through `safeUrl()`, which strips `javascript:`, `vbscript:`, `data:text:`, and `data:application:` schemes (`src/web.rs`).
- `Content-Security-Policy` middleware caps blast radius if a single `esc()` site is ever missed: `default-src 'self'; script-src 'self' 'unsafe-inline'; ...; object-src 'none'; frame-ancestors 'none'`.
- `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer` set globally.

**Known limitation:** the embedded UI uses inline `<script>`, so `'unsafe-inline'` is required in `script-src`. This means a successful XSS injection could execute arbitrary inline JS, but injection itself is blocked at the input-handling layer and CSP still prevents remote script loading or framing.

### ✅ Authentication & sessions — HARDENED

- Argon2-hashed password in the `settings` table (never in `config.toml`).
- 256-bit `rand::thread_rng()` (CSPRNG) session tokens, hex-encoded.
- `Mutex<HashMap<String, Instant>>` of issued tokens, lazily pruned past 30-day TTL — no unbounded growth.
- `Set-Cookie` includes `HttpOnly`, `SameSite=Strict`, `Path=/`, `Max-Age=2592000`. `Secure` is added when `X-Forwarded-Proto: https` is present (so reverse-proxied deployments get it; plain-HTTP LAN does not, which is correct since browsers would otherwise refuse to send the cookie).
- All routes layered behind `auth_middleware` when a password is configured. `/api/login` and unauthenticated `GET /` (serves login page) are the only exceptions.
- `password_required` is cached in an `AtomicBool` so the auth middleware doesn't hit SQLite on every static-file fetch.

### ✅ Login brute-force resistance — RATE LIMITED

- Argon2 verification (~100 ms) imposes a per-attempt cost.
- Plus per-IP rate limiter: 5 failures within the lockout window → 60 s lockout returning HTTP 429 (`src/web.rs:post_login`).
- Successful login resets the counter for that IP.
- Lockout entries garbage-collected on each request.

### ✅ Bundled-binary supply chain — VERIFIED

- `yt-dlp_linux` (or `_macos` / `.exe`) is fetched over HTTPS from the official GitHub release.
- The matching `SHA2-256SUMS` file is fetched from the same release and the binary's SHA-256 is compared. Mismatch aborts the install and deletes the partial binary.
- `deno` is fetched over HTTPS; its SHA-256 is printed in the install log for visual inspection (deno doesn't ship a single SHA256SUMS-equivalent manifest in a stable location). A future improvement would fetch `.sha256sum` per-asset where available.
- Installer sanity-checks that `curl`, `unzip`, and `sha256sum` are present before starting.
- Every bundled-mode launch re-applies `+x` defensively via `ytdlp_bin::ensure_bundled_executable` so a stripped exec bit can't permanently break downloads.

### ✅ Unsafe code — NONE

`grep -rn "unsafe" src/` returns zero results.

### ✅ Body size limits — ENFORCED

A 4 MiB `DefaultBodyLimit` layer is attached to every route. Legitimate
payloads (cookies.txt, JSON settings) are all well under 64 KiB; anything
above 4 MiB is rejected before allocation.

### ⚠️ Plaintext HTTP on LAN — ACCEPTED RISK

When bound to `0.0.0.0` or a LAN IP, the cookie and traffic travel
unencrypted over the network. Defenses:

- `SameSite=Strict` prevents cross-site form submissions from triggering authenticated requests.
- CSP `frame-ancestors 'none'` prevents click-jacking.
- The session cookie is not marked `Secure` because the browser would refuse to send it back on HTTP, breaking login entirely. This is a deliberate trade-off and is documented.

**Mitigation if you need stronger guarantees:** put a reverse proxy with TLS in front (Caddy, nginx, Traefik) and set the `X-Forwarded-Proto` header. The app will then issue cookies with the `Secure` attribute automatically.

### ⚠️ `cookies.txt` and `yt-offline.db` file permissions

- `yt-offline.db`: now `chmod 0600` at open time on Unix (`src/database.rs`). Contains the Argon2 password hash and resume positions. Other local users on the same machine cannot read it.
- `cookies.txt`: not auto-chmodded because the user pastes / file-picks it in, and our `write_cookies()` uses `std::fs::write` which respects the user's umask. **Recommendation in README:** `chmod 600 cookies.txt` after import. (Tracking issue for future automation.)

### ⚠️ Job log echoes yt-dlp stderr verbatim

yt-dlp's stderr is captured into `Job::log` (`src/downloader.rs:spawn_job`).
For routine failures this is fine, but auth-related errors can mention
cookie file paths or impersonation profile names. In a strictly local
deployment this is acceptable; if the UI is shared with someone else,
review logs before sharing.

### ⚠️ TOCTOU on `get_sub_vtt`

After the path-traversal check passes, the file is opened. A symlink swap
between check and read would let a local attacker substitute a different
file — but the attacker would already need write access to the channels
root, at which point they own the data anyway. Accepted.

### ⚠️ Dependency notes

- `paste` v1.0.15 (transitive via egui) is unmaintained. Procedural macro,
  no runtime exposure. Same finding as 2026-05-17 audit; nothing changed
  upstream. Re-check periodically.
- `cargo audit` was not re-run for this audit; recommended before
  publishing release artifacts.

---

## Changes since the 2026-05-17 audit

| Item | Status |
|---|---|
| Bind defaults to `127.0.0.1` | ✅ already in 2026-05-17 |
| `Content-Security-Policy` header | ✅ added (`security_headers` middleware) |
| Optional HTTP password auth | ✅ added (Argon2 + sessions + rate limit) |
| Filter yt-dlp stderr | ⚠️ still echoed (accepted) |
| chmod 600 on cookies.txt | ⚠️ documented; not automated |
| chmod 600 on yt-offline.db | ✅ added (`Database::open`) |
| Session token TTL + GC | ✅ added |
| `Secure` cookie flag for HTTPS | ✅ added (gated on `X-Forwarded-Proto`) |
| Login rate limiting | ✅ added (5 / 60 s per IP) |
| Body size cap | ✅ added (4 MiB) |
| Bundled binary verification | ✅ added (SHA-256 + sanity checks) |
| `safeUrl()` for `src=` attributes | ✅ added |
| Path traversal in maintenance::remove_files | ✅ hardened (parent-canonicalize fallback) |

---

## Recommendations for future hardening

These are nice-to-haves; none are blocking the current deployment.

1. **Auto-`chmod 600` cookies.txt on write.** Reduces user error.
2. **Filter `--cookies` paths from job log.** Trivial regex strip before
   buffering into `Job::log`.
3. **Optional `data:` URL scheme allow-list for `<img>` only.** Useful if
   we ever start showing yt-dlp-supplied thumbnails as inline data URLs.
4. **`cargo audit` in CI.** Once cross-compile CI exists.
5. **Periodic deno SHA-256 pinning.** Currently we print the hash; a
   future enhancement could check it against a small in-repo manifest
   updated by a maintainer.
6. **Account lockout escalation.** Current scheme resets fully after 60 s.
   Consider exponential back-off (60 s → 5 min → 15 min) for repeated
   abuse from the same IP across longer windows.
7. **TLS terminator in-process.** Optional `rustls` integration so the
   app can serve HTTPS without a reverse proxy. Currently considered
   out of scope.

---

## Verification commands

```bash
# Build + tests pass
cargo build --release
cargo test

# No unsafe blocks
grep -rn 'unsafe' src/

# Dependencies up to date
cargo update --dry-run
cargo audit          # requires the cargo-audit binary
```

---

**Auditor:** Claude Code
**Audit method:** Manual code review of every `src/*.rs` plus the embedded
HTML/JS UI; targeted greps for command/SQL/path patterns; verification of
hardening claims against current source.
**Status:** Complete.
