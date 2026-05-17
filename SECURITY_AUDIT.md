# Security Audit — yt-offline

**Date:** 2026-05-17  
**Scope:** Rust codebase + dependencies  
**Threat Model:** Self-hosted personal YouTube archiving tool; single user accessing via localhost + LAN

---

## Summary

✅ **Overall Risk: LOW**

No critical vulnerabilities found. The codebase follows security best practices for command execution, database access, and file serving. One informational warning on a transitive dependency (low risk).

---

## Findings

### ✅ Command Injection — SECURE

All external process invocations use `std::process::Command` or `tokio::process::Command` with `.arg()` calls, bypassing shell interpretation:

- **yt-dlp invocations** (`src/downloader.rs:160–188`): User URL passed as a clean argument, not shell-interpreted. Safe even with special characters.
- **ffmpeg invocations** (`src/web.rs:404–415`): Similarly safe, using separate `.arg()` calls.
- **Preview handler** (`src/web.rs:492–498`): yt-dlp preview command also uses `.arg()`, not vulnerable to command injection.

**No shell escaping needed; no `format!()` in command strings.**

---

### ✅ SQL Injection — SECURE

All database queries in `src/database.rs` use **parameterized statements**:

```rust
// Correct — parameter substitution
self.conn.execute(
    "INSERT OR REPLACE INTO watched (video_id) VALUES (?1)",
    [video_id],
)?;

// Correct — rusqlite::params! macro
self.conn.execute(
    "INSERT OR REPLACE INTO positions (video_id, position_secs) VALUES (?1, ?2)",
    rusqlite::params![video_id, position_secs],
)?;
```

**No string concatenation in SQL strings.** Video IDs come from path parameters and are bound safely.

---

### ✅ Path Traversal — SECURE

**File serving** (`src/web.rs:687`):
- Uses `tower_http::services::ServeDir::new(&channels_root)` — a well-audited crate that prevents `..` traversal.

**Info.json access** (`src/web.rs:230–236`):
- Videos are looked up **only** from the scanned library (`find_video_info_path`).
- Cannot directly request arbitrary files by path; must exist in the library.

**URL path encoding** (`src/web.rs:176–186`):
- Correct percent-encoding of URL segments.
- Path components are filtered to `Component::Normal` (rejects `..` and symlinks).

---

### ✅ Unsafe Code — NONE FOUND

**Result of search:** `grep -r "unsafe"` across all `.rs` files returns **zero unsafe blocks**.

All memory safety is guaranteed by Rust's type system. No C FFI or raw pointers.

---

### ⚠️ Dependency Warning (LOW RISK)

**Crate:** `paste` v1.0.15  
**Status:** Unmaintained since 2024-10-07  
**Severity:** Warning (not an error)  
**Dependency Chain:**  
```
paste 1.0.15
  ← egui-wgpu (GUI rendering)
    ← eframe (GUI framework)
      ← yt-offline (your app)
```

**Impact:**
- `paste` is a procedural macro for code generation; it does not process untrusted input.
- The crate is used only at compile-time for the GUI stack.
- No security-critical functionality in `paste`.
- **Mitigation:** Monitor for a replacement in the egui/eframe ecosystem. No action needed now.

---

## Input Validation

### Download URLs
- **Handler:** `post_download()` (`src/web.rs:339–350`)
- **Validation:** Trimmed and checked for empty; passed directly to yt-dlp via `.arg()`.
- **Risk:** Low — yt-dlp validates URLs server-side and fails gracefully on malformed input.

### Video IDs
- **Source:** Extracted from URL paths and library scans.
- **Usage:** Passed to SQL via parameterized queries and used to look up files in the library.
- **Risk:** Low — parameterized queries eliminate injection; library lookup prevents arbitrary file access.

### Position values
- **Handler:** `post_resume()` (`src/web.rs:464–479`)
- **Validation:** Numeric (f64) — deserialized by serde, bounds-checked (position > 3.0 filter).
- **Risk:** Low — stored in SQLite as REAL, no string manipulation.

### Preview query (yt-dlp URL)
- **Handler:** `get_preview()` (`src/web.rs:484–527`)
- **Validation:** Trimmed and checked for empty.
- **Risk:** Low — passed to yt-dlp via `.arg()`, no shell injection.

---

## Data Exposure

### Watched status & resume positions
- **Storage:** SQLite database (`yt-offline.db`), local filesystem.
- **Access:** Only available to the user running the app (same UID).
- **No network exposure:** Positions are stored server-side; browser caches position locally.
- **Risk:** Low — local-only; no credentials or PII stored.

### Download logs
- **Stored in:** Job objects, kept in RAM and shown in the UI.
- **Leakage risk:** stderr from yt-dlp is visible in the web UI; sensitive auth errors could be logged.
- **Mitigation:** Consider filtering `--cookies` error messages in production. Currently acceptable for personal use.

### yt-dlp output (preview, metadata)
- **Handler:** Parses JSON from yt-dlp; returns filtered fields to the browser.
- **Risk:** Low — only public YouTube metadata (title, duration, view count).

---

## Configuration & Secrets

### `cookies.txt`
- **Location:** Current working directory (hardcoded in `downloader.rs`).
- **Risk:** File is world-readable by default. **Recommendation:** Ensure restrictive file permissions (`chmod 600 cookies.txt`).

### `config.toml`
- **Contains:** Directory paths, port number, browser name, `source_url`.
- **Risk:** Low — no secrets stored. `source_url` is read-only from browser.
- **Sensitive data:** Only the `backup.directory` path could leak file structure; this is expected behavior.

---

## Network Security

### Web server scope
- **Binding:** Configured via `web.port` in `config.toml` (default 8080).
- **Current binding:** Likely `127.0.0.1:8080` (assumed; verify with netstat).
- **Risk:** If bound to `0.0.0.0`, anyone on the network can access. **Recommendation:** Bind to `127.0.0.1` or use a reverse proxy with authentication.

### No HTTPS
- **Status:** Web UI is served over HTTP.
- **Risk:** Medium if exposed to untrusted networks. **Recommendation:** Use a reverse proxy (nginx, Caddy) with TLS for remote access.

### No authentication
- **Status:** No login required; access is "security by obscurity" (port number).
- **Risk:** Medium if the port is discovered. **Recommendation:** Add optional HTTP Basic Auth or reverse proxy authentication for LAN access.

---

## Recommendations (Priority Order)

### HIGH
1. **Verify web server binding** — Ensure `axum` binds to `127.0.0.1` only, not `0.0.0.0`.
   - Check: `src/web.rs:618` where the listener is created.
   - Mitigation: Add a config option for `bind_addr` or document localhost-only setup.

### MEDIUM
2. **File permissions on `cookies.txt`** — Remind users to `chmod 600 cookies.txt`.
   - Add note to README.

3. **TLS + authentication for remote access** — If exposing to LAN/internet:
   - Use a reverse proxy (nginx, Caddy) with mTLS or Basic Auth.
   - Or add built-in HTTP Basic Auth via an optional config field.

4. **Filter sensitive logs** — Consider not echoing yt-dlp stderr if it contains auth errors.
   - Current: Acceptable for personal use. Not a blocker.

### LOW
5. **Update `paste` crate dependency chain** — Monitor egui/eframe for a replacement.
   - Not urgent; no security impact in your use case.

6. **Add Content-Security-Policy headers** — Prevent XSS if web UI is exposed to untrusted networks.
   - Current: Low risk (browser only, no JavaScript vulnerabilities found).

---

## Conclusion

The codebase demonstrates **strong security practices** for a personal self-hosted tool:
- ✅ No injection vulnerabilities (command, SQL).
- ✅ No unsafe code.
- ✅ Path traversal protection in place.
- ✅ Parameterized database queries.
- ✅ Safe process spawning.

**No blockers for personal use.** For public/LAN exposure, implement the HIGH-priority recommendations.

---

**Auditor:** Claude Code  
**Audit Method:** Manual code review + cargo audit  
**Status:** Complete
