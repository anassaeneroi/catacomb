//! HTTP server providing a browser-based library UI and download API.
//!
//! # Architecture
//!
//! All mutable server state lives in [`WebState`], wrapped in `Arc<WebState>`
//! and shared across axum handlers.  Mutable fields use `Mutex` or `AtomicBool`
//! so they can be updated by concurrent requests without blocking the async
//! runtime.
//!
//! The server can be started in two ways:
//!
//! * **Standalone mode** — `serve(config)` blocks until the process exits.
//! * **GUI-embedded mode** — `run_with_shutdown(config)` spawns a background
//!   Tokio runtime and returns a `Sender<()>` the GUI can use to stop the server.
//!
//! # AGPL §13 compliance
//!
//! This software is licensed under the GNU Affero General Public License v3.
//! Any deployment that serves the web UI to network users must make the
//! Corresponding Source available.  Set `web.source_url` in `config.toml` to
//! a URL where the source can be obtained; the URL is displayed in the UI footer
//! and returned by `GET /api/settings`.

use std::collections::{HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::{
    body::{Body, Bytes},
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::SaltString;

use crate::config::Config;
use crate::database::Database;
use crate::downloader::{DownloadQuality, Downloader, JobState};
use crate::platform::classify_url;
use crate::library;
use crate::maintenance;

// ── Shared state ──────────────────────────────────────────────────────────────

/// Serialisable snapshot of a single download job, sent to the browser.
#[derive(Clone, Serialize)]
pub struct JobSnapshot {
    pub label: String,
    pub url: String,
    /// One of `"running"`, `"done"`, or `"failed"`.
    pub state: &'static str,
    pub progress: f32,
    pub last_line: String,
    /// Classification of the failure, if `state == "failed"`. One of
    /// `rate-limited`, `members-only`, `geo-blocked`, `not-found`,
    /// `codec-missing`, `disk-full`, `network-error`, `bad-cookies`, `other`,
    /// or `null` while still running / on success. Drives the suggested
    /// action hint in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<crate::error_class::ErrorClass>,
    /// Human-readable one-line suggested action paired with `error_class`.
    /// Empty when `error_class` is `Other` or `None`.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub error_hint: &'static str,
}

/// All mutable state shared across axum handlers via `Arc<WebState>`.
pub struct WebState {
    /// Scanned channel/playlist/video tree, refreshed after each completed download.
    pub library: Mutex<Vec<library::Channel>>,
    /// Active and recently finished yt-dlp jobs.
    pub downloader: Mutex<Downloader>,
    /// Set of video IDs the user has marked as watched (persisted in SQLite).
    pub watched: Mutex<HashSet<String>>,
    /// Bookmark / favourite / waiting / archive flag sets, hydrated from the
    /// `video_flags` SQLite table at startup. Each set holds the video IDs
    /// with the named flag enabled. Drives the smart-folder sidebar entries
    /// and the per-card action icons.
    pub flags: Mutex<crate::database::VideoFlagsBundle>,
    /// Last known playback position per video ID in seconds (persisted in SQLite).
    pub positions: Mutex<HashMap<String, f64>>,
    /// SQLite handle. Internally backed by an `r2d2` pool, so concurrent
    /// handlers each check out their own connection without serializing on
    /// an external mutex.
    pub db: Database,
    /// YouTube channels directory (the legacy `channels/` folder, kept for
    /// backward compat). Other platforms live as siblings under [`library_root`].
    pub channels_root: PathBuf,
    /// Parent of `channels_root`. Backs the `/files/` static-file mount so
    /// non-YouTube platforms are reachable at `/files/<platform>/<creator>/...`.
    pub library_root: PathBuf,
    pub config_path: PathBuf,
    pub config: Mutex<Config>,
    /// Whether to transcode MKV→mp4 on the fly for playback (requires ffmpeg).
    pub transcode: AtomicBool,
    /// Active session tokens mapped to their issued-at `Instant`. Tokens older
    /// than [`SESSION_TTL`] are rejected and pruned lazily on each touch.
    pub sessions: Mutex<HashMap<String, std::time::Instant>>,
    /// When the last scheduled channel check ran; used to compute the next due time.
    pub last_scheduled_check: Mutex<Option<std::time::Instant>>,
    /// Cached "is password required" — refreshed when the password is changed.
    /// Avoids a DB hit on every request through `auth_middleware`.
    pub password_required_cache: AtomicBool,
    /// Monotonically-incremented version counter; serves as the ETag for
    /// `/api/library`. Bumped on any state change that would alter the
    /// JSON response (rescan, watched toggle, resume position, maintenance
    /// remove). Combined with `If-None-Match` short-circuits the megabytes
    /// of library JSON when nothing has changed.
    pub library_version: AtomicU64,
    /// Per-IP failure tracker for [`post_login`]. Each entry is the number of
    /// recent failures and the instant the lockout (if any) expires.
    pub login_attempts: Mutex<HashMap<std::net::IpAddr, LoginAttempt>>,
    /// Push channel for `/ws/progress` subscribers. A background tokio task
    /// ticks every 500 ms while jobs are active and broadcasts a fresh
    /// [`ProgressResponse`] snapshot here; the WebSocket handler forwards
    /// each message to its client. The `/api/progress` HTTP endpoint stays
    /// available as a fallback for clients that can't open a socket.
    pub progress_tx: tokio::sync::broadcast::Sender<String>,
    /// Cached serialized body of `/api/library` keyed by the
    /// `library_version` it was built against. On hit, handlers ship
    /// the cached bytes without re-walking the channel tree or
    /// re-serializing. The Arc lets concurrent responses share one
    /// allocation. Cleared / replaced lazily on the next miss.
    pub library_body_cache: Mutex<Option<(u64, std::sync::Arc<String>)>>,
}

/// Failed-login tracking entry. After [`LOGIN_LOCKOUT_AFTER`] failures from
/// the same IP, further attempts are rejected until [`LoginAttempt::until`].
pub struct LoginAttempt {
    pub failures: u32,
    pub until: Option<std::time::Instant>,
}

/// How long a session token is valid for after login.
pub const SESSION_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 3600);
/// Failures per IP before /api/login starts returning 429.
pub const LOGIN_LOCKOUT_AFTER: u32 = 5;
/// How long the lockout lasts once tripped.
pub const LOGIN_LOCKOUT_DURATION: std::time::Duration = std::time::Duration::from_secs(60);

impl WebState {
    fn job_snapshots(dl: &Downloader) -> Vec<JobSnapshot> {
        dl.jobs
            .iter()
            .map(|j| JobSnapshot {
                label: j.label.clone(),
                url: j.url.clone(),
                state: match j.state {
                    JobState::Running => "running",
                    JobState::Done => "done",
                    JobState::Failed => "failed",
                },
                progress: j.progress,
                last_line: j.log.back().cloned().unwrap_or_default(),
                // Skip `Other` so the badge doesn't get a useless generic
                // label — the raw log line is still shown for that case.
                error_class: j.failure_class.filter(|c|
                    *c != crate::error_class::ErrorClass::Other
                ),
                error_hint: j.failure_class.map(|c| c.hint()).unwrap_or(""),
            })
            .collect()
    }
}

// ── API types ─────────────────────────────────────────────────────────────────
// These types are serialised to JSON and consumed by the browser UI.

/// Response body for `GET /api/library`.
#[derive(Serialize)]
struct LibraryResponse {
    channels: Vec<ChannelInfo>,
    folders: Vec<crate::database::FolderRecord>,
}

/// JSON representation of a single channel sent to the browser.
#[derive(Serialize)]
struct ChannelInfo {
    name: String,
    /// dir_name() of the channel's source platform. Used by the UI to render
    /// the platform icon and group entries.
    platform: &'static str,
    /// Human-readable platform name (e.g. "YouTube", "TikTok").
    platform_label: &'static str,
    /// Platform icon used in the sidebar.
    platform_icon: &'static str,
    /// Original URL the channel was downloaded from, if a `.source-url` sidecar
    /// exists. Used by the UI's "Check for new videos" action to avoid relying
    /// on a folder-name heuristic.
    source_url: Option<String>,
    /// Folder id from the user's channel-organisation tree. `None` when the
    /// channel is "Unfiled" (no row in `channel_assignments`).
    folder_id: Option<i64>,
    total_videos: usize,
    size_bytes: u64,
    subscriber_count: Option<u64>,
    uploader: Option<String>,
    channel_url: Option<String>,
    /// Thumbnail URL for the channel overview grid — first available video thumbnail.
    thumb_url: Option<String>,
    playlists: Vec<PlaylistInfo>,
    videos: Vec<VideoInfo>,
}

/// JSON representation of a playlist within a channel.
#[derive(Serialize)]
struct PlaylistInfo {
    name: String,
    videos: Vec<VideoInfo>,
}

/// JSON representation of a single video sent to the browser.
///
/// `resume_pos` is only set when the user has played more than 3 seconds
/// so that the "continue watching" list stays meaningful.
#[derive(Serialize)]
struct VideoInfo {
    id: String,
    title: String,
    duration_secs: Option<f64>,
    file_size: Option<u64>,
    /// Upload date as `YYYYMMDD` (yt-dlp's native format).
    upload_date: Option<String>,
    /// Filesystem mtime as a UNIX timestamp. Drives the Recent-additions feed.
    mtime_unix: Option<u64>,
    has_video: bool,
    has_live_chat: bool,
    watched: bool,
    /// Smart-folder flags. Populated from the in-memory
    /// [`WebState::flags`] sets at response-build time.
    bookmark: bool,
    favourite: bool,
    waiting: bool,
    archive: bool,
    video_url: Option<String>,
    thumb_url: Option<String>,
    subtitles: Vec<SubtitleInfo>,
    has_chapters: bool,
    resume_pos: Option<f64>,
}

/// A single subtitle track URL for a video.
#[derive(Serialize)]
struct SubtitleInfo {
    lang: String,
    /// Human-readable label (e.g. "English"), shown in the track selector.
    label: String,
    url: String,
}

/// JSON representation of a single music track sent to the browser.
#[derive(Serialize)]
struct TrackInfo {
    id: String,
    title: String,
    artist: String,
    duration_secs: Option<f64>,
    file_size: Option<u64>,
    audio_url: Option<String>,
    thumb_url: Option<String>,
}

/// Request body for `POST /api/download`.
#[derive(Deserialize)]
struct StartDownloadRequest {
    url: String,
    /// When true, omits `--break-on-existing` so every video is checked
    /// individually — slower but fills gaps in partially-archived channels.
    #[serde(default)]
    full_scan: bool,
    /// Quality selector: "best" (default), "1080p", "720p", "480p", "360p", or "music".
    /// When "music", audio-only mode is used regardless of other settings.
    #[serde(default)]
    quality: String,
    /// Treat the URL as an ongoing live broadcast and record from the start.
    /// Adds `--live-from-start --wait-for-video 30` and timestamps the
    /// output filename so re-recordings don't collide.
    #[serde(default)]
    live: bool,
}

/// Response body for `GET /api/progress`.
#[derive(Serialize)]
struct ProgressResponse {
    jobs: Vec<JobSnapshot>,
    queued: Vec<QueuedSnapshot>,
    max_concurrent: usize,
}

#[derive(Serialize)]
struct QueuedSnapshot {
    label: String,
    url: String,
}

#[derive(Serialize, Deserialize)]
struct SettingsPayload {
    transcode: bool,
    /// URL of the source repository, shown in the footer for AGPL §13 compliance.
    /// Editable via the settings UI; empty string on POST clears it.
    #[serde(default)]
    source_url: Option<String>,
    /// Current binding address and port, sent by server only.
    #[serde(skip_deserializing, default)]
    current_bind: Option<String>,
    /// List of available bind options, sent by server only.
    #[serde(skip_deserializing, default)]
    available_binds: Option<Vec<BindOption>>,
    /// Selected bind mode (localhost, tailscale, lan, all). Clients can send this on POST to change.
    #[serde(skip_deserializing, default)]
    bind_mode: Option<String>,
    /// Whether a password is required for downloads (sent by server only).
    #[serde(skip_deserializing, default)]
    download_password_required: bool,
    /// New plaintext password to set for downloads. Clients send this on POST; server does not return it.
    #[serde(skip_serializing, default)]
    new_download_password: Option<String>,
    /// Plex library path, readable and writable by both client and server.
    #[serde(default)]
    plex_library_path: Option<String>,
    /// Whether the background scheduler is enabled.
    #[serde(default)]
    scheduler_enabled: bool,
    /// Hours between automatic channel checks (1–168). Ignored if 0 on POST.
    #[serde(default)]
    scheduler_interval_hours: u32,
    /// Seconds until the next scheduled check. `None` if scheduler is disabled or last check unknown.
    #[serde(skip_deserializing, default)]
    scheduler_next_check_secs: Option<u64>,
    /// Maximum simultaneous yt-dlp processes. 0 = unlimited. Ignored if 0 on POST.
    #[serde(default)]
    max_concurrent: usize,
    /// If true, invoke the bundled yt-dlp under `~/.local/share/yt-offline/bin/`
    /// instead of the system PATH yt-dlp.
    #[serde(default)]
    use_bundled_ytdlp: bool,
    /// Whether the bundled yt-dlp binary is installed on disk (sent by server only).
    #[serde(skip_deserializing, default)]
    bundled_ytdlp_installed: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BindOption {
    pub id: String,
    pub label: String,
    pub address: String,
}

// Build a `/files/<rel>` URL from an absolute path, percent-encoding each segment.
//
// `library_root` is the parent of channels_root so that paths like
// `<root>/channels/handle/video.mkv` become `/files/channels/handle/video.mkv`
// and `<root>/tiktok/user/clip.mp4` becomes `/files/tiktok/user/clip.mp4`.
fn file_url(library_root: &StdPath, full: &StdPath) -> Option<String> {
    let rel = full.strip_prefix(library_root).ok()?;
    let mut parts: Vec<String> = Vec::new();
    for c in rel.components() {
        if let std::path::Component::Normal(s) = c {
            parts.push(percent_encode_segment(s.to_str()?));
        }
    }
    if parts.is_empty() { return None; }
    Some(format!("/files/{}", parts.join("/")))
}

fn percent_encode_segment(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            // `write!` reuses `out`'s buffer — avoids the intermediate
            // `String` allocation that `format!` would produce per byte.
            _ => { let _ = write!(out, "%{:02X}", b); }
        }
    }
    out
}

fn lang_label(code: &str) -> String {
    let base = code.split('-').next().unwrap_or(code);
    let name = match base {
        "en" => "English",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh" => "Chinese",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "it" => "Italian",
        "ar" => "Arabic",
        "hi" => "Hindi",
        "nl" => "Dutch",
        "pl" => "Polish",
        "tr" => "Turkish",
        "sv" => "Swedish",
        "id" => "Indonesian",
        "vi" => "Vietnamese",
        "th" => "Thai",
        _ => return code.to_string(),
    };
    if code.ends_with("-orig") || code.contains("auto") {
        format!("{name} (auto)")
    } else {
        name.to_string()
    }
}

fn find_video_info_path(library: &[library::Channel], id: &str) -> Option<PathBuf> {
    library::find_video(library, id).and_then(|(v, _)| v.info_path.clone())
}

fn find_video_path(library: &[library::Channel], id: &str) -> Option<PathBuf> {
    library::find_video(library, id).and_then(|(v, _)| v.video_path.clone())
}

fn detect_tailscale_ip() -> Option<String> {
    if std::path::Path::new("/proc/net/if_inet6").exists() || std::path::Path::new("/etc/tailscale").exists() {
        if let Ok(output) = std::process::Command::new("hostname")
            .arg("-I")
            .output() {
            let ip_str = String::from_utf8_lossy(&output.stdout);
            ip_str
                .split_whitespace()
                .find(|ip| ip.starts_with("100."))
                .map(|s| s.to_string())
        } else {
            None
        }
    } else {
        None
    }
}

fn detect_lan_ip() -> Option<String> {
    if let Ok(output) = std::process::Command::new("hostname")
        .arg("-I")
        .output() {
        let ip_str = String::from_utf8_lossy(&output.stdout);
        ip_str
            .split_whitespace()
            .find(|ip| !ip.starts_with("127.") && !ip.starts_with("100."))
            .map(|s| s.to_string())
    } else {
        None
    }
}

pub fn get_available_binds(port: u16) -> Vec<BindOption> {
    let mut opts = vec![
        BindOption {
            id: "localhost".to_string(),
            label: "Localhost only".to_string(),
            address: format!("127.0.0.1:{port}"),
        },
    ];

    if let Some(ts_ip) = detect_tailscale_ip() {
        opts.push(BindOption {
            id: "tailscale".to_string(),
            label: format!("Tailscale ({})", ts_ip),
            address: format!("{ts_ip}:{port}"),
        });
    }

    if let Some(lan_ip) = detect_lan_ip() {
        if lan_ip != "127.0.0.1" {
            opts.push(BindOption {
                id: "lan".to_string(),
                label: format!("LAN ({})", lan_ip),
                address: format!("{lan_ip}:{port}"),
            });
        }
    }

    opts.push(BindOption {
        id: "all".to_string(),
        label: "All interfaces (0.0.0.0)".to_string(),
        address: format!("0.0.0.0:{port}"),
    });

    opts
}

pub fn resolve_bind_mode(mode: &str) -> String {
    match mode {
        "tailscale" => detect_tailscale_ip().unwrap_or_else(|| "127.0.0.1".to_string()),
        "lan" => detect_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string()),
        "all" => "0.0.0.0".to_string(),
        _ => "127.0.0.1".to_string(),
    }
}

/// Infer the bind-mode id (`localhost`/`tailscale`/`lan`/`all`) from a stored bind address.
pub fn bind_mode_of(addr: &str) -> &'static str {
    match addr {
        "127.0.0.1" | "localhost" => "localhost",
        "0.0.0.0" => "all",
        a if a.starts_with("100.") => "tailscale",
        _ => "lan",
    }
}

// ── Cookies ─────────────────────────────────────────────────────────────────────

/// Convert SubRip (SRT) subtitle text to WebVTT.
///
/// The only structural differences are the `WEBVTT` header and the timestamp
/// decimal separator (SRT uses `,`, VTT uses `.`).
fn srt_to_vtt(srt: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for line in srt.lines() {
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Path to the `cookies.txt` yt-dlp reads, resolved against the process working
/// directory (the same place `config.toml` lives and where the downloader's
/// relative `--cookies cookies.txt` resolves).
pub fn cookies_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("cookies.txt")
}

/// Count cookie entries (Netscape lines with 7 tab-separated fields).
fn count_cookies(text: &str) -> usize {
    text.lines().filter(|l| l.split('\t').count() >= 7).count()
}

/// Whether a cookies file exists and how many cookie entries it holds.
pub fn cookies_status() -> (bool, usize) {
    match std::fs::read_to_string(cookies_path()) {
        Ok(s) => (true, count_cookies(&s)),
        Err(_) => (false, 0),
    }
}

/// Validate that `text` looks like a Netscape cookie jar and write it to
/// [`cookies_path`]. Returns the number of cookie entries written, or an error
/// message if the content doesn't look like a cookies.txt.
pub fn write_cookies(text: &str) -> Result<usize, String> {
    if text.trim().is_empty() {
        return Err("no cookies provided".to_string());
    }
    let has_cookie_line = text.lines().any(|l| l.split('\t').count() >= 7);
    let has_header = text.trim_start().starts_with("# Netscape")
        || text.trim_start().starts_with("# HTTP Cookie File");
    if !has_cookie_line && !has_header {
        return Err(
            "doesn't look like a Netscape cookies.txt (expected tab-separated fields)".to_string(),
        );
    }
    let mut content = text.to_string();
    if !content.ends_with('\n') {
        content.push('\n');
    }
    let path = cookies_path();
    std::fs::write(&path, &content).map_err(|e| e.to_string())?;
    // cookies.txt carries live session credentials — tighten the mode so it
    // isn't world-readable on multi-user systems. Best-effort, like the
    // similar guard on yt-offline.db.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(&path, perms);
        }
    }
    Ok(count_cookies(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_to_vtt_replaces_comma_in_timestamps_only() {
        let srt = "1\n00:00:01,500 --> 00:00:03,000\nHello, world\n";
        let vtt = srt_to_vtt(srt);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        // Comma in body preserved; comma in timestamp converted to dot.
        assert!(vtt.contains("00:00:01.500 --> 00:00:03.000"));
        assert!(vtt.contains("Hello, world"));
    }

    #[test]
    fn count_cookies_only_counts_seven_field_lines() {
        let body = "# Netscape HTTP Cookie File\n\
                    .youtube.com\tTRUE\t/\tFALSE\t0\tname\tvalue\n\
                    not a cookie line\n\
                    .example.com\tTRUE\t/\tFALSE\t0\tn2\tv2\n";
        assert_eq!(count_cookies(body), 2);
    }

    #[test]
    fn percent_encode_segment_passes_safe_chars() {
        assert_eq!(percent_encode_segment("abcXYZ0-9._~"), "abcXYZ0-9._~");
    }

    #[test]
    fn percent_encode_segment_escapes_space_and_slash() {
        assert_eq!(percent_encode_segment("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn percent_encode_segment_escapes_non_ascii() {
        // 'é' in UTF-8 is 0xC3 0xA9.
        assert_eq!(percent_encode_segment("é"), "%C3%A9");
    }

    #[test]
    fn bind_mode_of_recognizes_loopback_and_wildcard() {
        assert_eq!(bind_mode_of("127.0.0.1"), "localhost");
        assert_eq!(bind_mode_of("localhost"), "localhost");
        assert_eq!(bind_mode_of("0.0.0.0"), "all");
        assert_eq!(bind_mode_of("100.64.1.2"), "tailscale");
        assert_eq!(bind_mode_of("192.168.1.10"), "lan");
    }

    #[test]
    fn hash_password_verify_roundtrip() {
        let h = hash_password("hunter2").unwrap();
        assert!(verify_password("hunter2", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn lang_label_known_codes() {
        assert_eq!(lang_label("en"), "English");
        assert_eq!(lang_label("ja"), "Japanese");
        assert_eq!(lang_label("en-orig"), "English (auto)");
        // Unknown: returned as-is.
        assert_eq!(lang_label("zz"), "zz");
    }
}

pub fn hash_password(password: &str) -> Option<String> {
    use rand::thread_rng;
    let salt = SaltString::generate(thread_rng());
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .ok()
        .map(|hash| hash.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    if let Ok(parsed_hash) = PasswordHash::new(hash) {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok()
    } else {
        false
    }
}

// ── Session auth ────────────────────────────────────────────────────────────────

/// Generate a 256-bit random session token, hex-encoded.
fn generate_session_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Extract the `session` cookie value from request headers, if present.
fn session_token_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie
        .split(';')
        .filter_map(|p| p.trim().strip_prefix("session="))
        .next()
        .map(|s| s.to_string())
}

/// True if the request carries a valid, non-expired session cookie.
///
/// Expired tokens are removed from the in-memory set as a side effect so the
/// `sessions` map doesn't grow without bound for users who never log out.
fn is_authed(state: &WebState, headers: &HeaderMap) -> bool {
    let Some(token) = session_token_from_headers(headers) else { return false };
    let mut sessions = state.sessions.lock().unwrap();
    let now = std::time::Instant::now();
    // Lazy prune: drop anything older than the TTL.
    sessions.retain(|_, issued| now.duration_since(*issued) < SESSION_TTL);
    sessions.contains_key(&token)
}

/// Whether a download/access password is configured. Backed by an atomic
/// cache to avoid a SQLite hit on every request (especially for static files).
fn password_required(state: &WebState) -> bool {
    state.password_required_cache.load(Ordering::Relaxed)
}

/// Bump the library-version counter. Callers should invoke this after any
/// state change that would alter `/api/library`'s response (watched flip,
/// resume position, rescan, maintenance remove). The counter is consumed
/// as an `ETag` so well-behaved clients can short-circuit unchanged GETs
/// with `If-None-Match`.
fn bump_library_version(state: &WebState) {
    state.library_version.fetch_add(1, Ordering::Relaxed);
}

/// Re-read the password setting from the DB and update the cache. Called
/// after any change that could affect whether a password exists.
fn refresh_password_cache(state: &WebState) {
    let present = state.db
        .get_setting("password_hash").ok().flatten().is_some();
    state.password_required_cache.store(present, Ordering::Relaxed);
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

/// Build the `Set-Cookie` header value for a session token.
///
/// `Secure` is added when the request arrived over HTTPS — detected either
/// by a forwarding proxy (`X-Forwarded-Proto: https`) or by the request URI
/// scheme. Setting `Secure` unconditionally would break logins on plain-HTTP
/// LAN deployments since the browser would refuse to send the cookie back.
fn session_cookie(token: &str, headers: &HeaderMap, max_age_secs: u64) -> String {
    let secure = headers.get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("https"))
        .unwrap_or(false);
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("session={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age_secs}{secure_attr}")
}

/// `POST /api/login` — verify the password and issue a session cookie.
///
/// Rate-limited per source IP: after [`LOGIN_LOCKOUT_AFTER`] failed attempts,
/// further attempts return 429 for [`LOGIN_LOCKOUT_DURATION`]. Successful
/// logins reset the counter for that IP.
async fn post_login(
    State(state): State<Arc<WebState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Response {
    let ip = addr.ip();
    let now = std::time::Instant::now();
    // Check lockout first.
    {
        let mut attempts = state.login_attempts.lock().unwrap();
        // GC entries whose lockout has elapsed.
        attempts.retain(|_, a| a.until.map_or(true, |u| u > now));
        if let Some(a) = attempts.get(&ip) {
            if let Some(until) = a.until {
                if until > now {
                    return (StatusCode::TOO_MANY_REQUESTS, "too many failed attempts — try again shortly").into_response();
                }
            }
        }
    }

    let hash = state.db.get_setting("password_hash").ok().flatten();
    let Some(hash) = hash else {
        // No password configured; nothing to authenticate against.
        return (StatusCode::OK, "no password set").into_response();
    };
    if !verify_password(&body.password, &hash) {
        let mut attempts = state.login_attempts.lock().unwrap();
        let entry = attempts.entry(ip).or_insert(LoginAttempt { failures: 0, until: None });
        entry.failures += 1;
        if entry.failures >= LOGIN_LOCKOUT_AFTER {
            entry.until = Some(now + LOGIN_LOCKOUT_DURATION);
            entry.failures = 0;
        }
        return (StatusCode::UNAUTHORIZED, "invalid password").into_response();
    }
    // Success: reset the failure counter for this IP.
    state.login_attempts.lock().unwrap().remove(&ip);

    let token = generate_session_token();
    state.sessions.lock().unwrap().insert(token.clone(), now);
    let cookie = session_cookie(&token, &headers, SESSION_TTL.as_secs());
    ([(header::SET_COOKIE, cookie)], StatusCode::OK).into_response()
}

/// `POST /api/logout` — invalidate the current session and clear the cookie.
async fn post_logout(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Response {
    if let Some(token) = session_token_from_headers(&headers) {
        state.sessions.lock().unwrap().remove(&token);
    }
    let cookie = session_cookie("", &headers, 0);
    ([(header::SET_COOKIE, cookie)], StatusCode::OK).into_response()
}

/// Middleware that attaches conservative security headers to every response.
///
/// The Content-Security-Policy permits inline JS and styles (the embedded UI
/// is one big inline script tag) but forbids loading code from third-party
/// origins, blocks plugin / object embedding, and prevents the page from
/// being framed. This caps the blast radius of any future XSS slip-up in
/// the embedded UI strings.
async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    // SAFETY: every value here is a fixed compile-time ASCII string.
    let csp = "default-src 'self'; \
               script-src 'self' 'unsafe-inline'; \
               style-src 'self' 'unsafe-inline'; \
               img-src 'self' data: blob: https:; \
               media-src 'self' blob:; \
               connect-src 'self'; \
               font-src 'self'; \
               object-src 'none'; \
               base-uri 'self'; \
               frame-ancestors 'none'";
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    resp
}

/// Middleware gating every route behind a session cookie when a password is set.
/// With no password configured, all requests pass through unchanged (preserves
/// the localhost-only default). `/api/login` is always reachable so users can
/// authenticate; unauthenticated `GET /` is served a login page instead of the app.
async fn auth_middleware(
    State(state): State<Arc<WebState>>,
    req: Request,
    next: Next,
) -> Response {
    if !password_required(&state) {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if path == "/api/login" {
        return next.run(req).await;
    }
    if is_authed(&state, req.headers()) {
        return next.run(req).await;
    }
    if path == "/" {
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            LOGIN_HTML,
        )
            .into_response();
    }
    (StatusCode::UNAUTHORIZED, "authentication required").into_response()
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn get_index() -> impl IntoResponse {
    // No-store on the HTML body: the binary upgrades change the embedded
    // markup without changing the URL, and we don't want a long-lived
    // browser tab serving a months-old UI against a fresh API. The JSON
    // endpoints still ETag their own bodies so library data remains
    // efficiently cached.
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        HTML_UI,
    )
}

async fn get_library(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Response {
    // Conditional GET: short-circuit with 304 when the client's cached
    // ETag matches our current library_version. Saves megabytes for
    // large libraries on every poll.
    let version = state.library_version.load(Ordering::Relaxed);
    let etag = format!("\"{}\"", version);
    if let Some(client_etag) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if client_etag == etag {
            return ([
                (header::ETAG, etag.clone()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ], StatusCode::NOT_MODIFIED).into_response();
        }
    }
    // Body cache: if we already serialized the payload for this version,
    // hand out the cached String. The 304 above catches clients with a
    // matching ETag; the body cache catches clients without one (curl,
    // freshly-opened tabs, mobile WebViews that don't store ETags
    // reliably). Serializing a multi-MB library JSON for every reload
    // burns measurable CPU on large installations.
    {
        let cache = state.library_body_cache.lock().unwrap();
        if let Some((cached_ver, body)) = cache.as_ref() {
            if *cached_ver == version {
                let body = body.clone();
                drop(cache);
                return (
                    [
                        (header::ETAG, etag),
                        (header::CACHE_CONTROL, "no-cache".to_string()),
                        (header::CONTENT_TYPE, "application/json".to_string()),
                    ],
                    body.as_str().to_string(),
                ).into_response();
            }
        }
    }

    let payload = build_library_payload(&state).await;
    // Re-check the version: if the library changed while we were
    // building (a download finished, watched toggled, etc.), the body
    // we just produced is stale for the *new* version. Don't cache
    // anything in that case — we'll serialize again next time.
    let post_version = state.library_version.load(Ordering::Relaxed);
    let body = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    if post_version == version {
        let body_arc = std::sync::Arc::new(body.clone());
        *state.library_body_cache.lock().unwrap() = Some((version, body_arc));
    }
    (
        [
            (header::ETAG, etag),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::CONTENT_TYPE, "application/json".to_string()),
        ],
        body,
    ).into_response()
}

async fn build_library_payload(state: &Arc<WebState>) -> LibraryResponse {
    let lib = state.library.lock().unwrap();
    let watched = state.watched.lock().unwrap();
    let positions = state.positions.lock().unwrap();
    let flags = state.flags.lock().unwrap();
    // file_url() now resolves relative to library_root (= parent of
    // channels_root) so non-YouTube platforms are reachable at
    // `/files/<platform>/<creator>/<video>`.
    let root = state.library_root.as_path();
    let transcode = state.transcode.load(Ordering::Relaxed);

    let to_info = |v: &library::Video, watched: &HashSet<String>| {
        let video_url = v.video_path.as_deref().and_then(|p| {
            if transcode {
                Some(format!("/api/transcode/{}", percent_encode_segment(&v.id)))
            } else {
                file_url(root, p)
            }
        });
        let thumb_url = v.thumb_path.as_deref().and_then(|p| file_url(root, p));
        let subtitles: Vec<SubtitleInfo> = v.subtitles.iter()
            .filter_map(|s| {
                let is_srt = s.path.extension().and_then(|e| e.to_str()) == Some("srt");
                let url = if is_srt {
                    // Route SRT through the on-the-fly conversion endpoint.
                    let rel = s.path.strip_prefix(root).ok()?;
                    Some(format!("/api/sub-vtt/{}", rel.display()))
                } else {
                    file_url(root, &s.path)
                }?;
                Some(SubtitleInfo {
                    lang: s.lang.clone(),
                    label: lang_label(&s.lang),
                    url,
                })
            })
            .collect();
        let resume_pos = positions.get(&v.id).copied().filter(|&p| p > 3.0);
        VideoInfo {
            id: v.id.clone(),
            title: v.title.clone(),
            duration_secs: v.duration_secs,
            file_size: v.file_size,
            upload_date: v.upload_date.clone(),
            mtime_unix: v.mtime_unix,
            has_video: v.video_path.is_some(),
            has_live_chat: v.has_live_chat,
            watched: watched.contains(&v.id),
            bookmark: flags.bookmark.contains(&v.id),
            favourite: flags.favourite.contains(&v.id),
            waiting: flags.waiting.contains(&v.id),
            archive: flags.archive.contains(&v.id),
            video_url,
            thumb_url,
            subtitles,
            has_chapters: v.has_chapters,
            resume_pos,
        }
    };

    let channels = lib.iter().map(|ch| {
        let thumb_url = ch.videos.iter()
            .chain(ch.playlists.iter().flat_map(|p| p.videos.iter()))
            .find_map(|v| v.thumb_path.as_deref().and_then(|p| file_url(root, p)));
        ChannelInfo {
            name: ch.name.clone(),
            platform: ch.platform.dir_name(),
            platform_label: ch.platform.display_name(),
            platform_icon: ch.platform.icon(),
            source_url: ch.source_url.clone(),
            folder_id: ch.folder_id,
            total_videos: ch.total_videos(),
            size_bytes: ch.total_size_cached,
            subscriber_count: ch.meta.as_ref().and_then(|m| m.subscriber_count),
            uploader: ch.meta.as_ref().and_then(|m| m.uploader.clone()),
            channel_url: ch.meta.as_ref().and_then(|m| m.channel_url.clone()),
            thumb_url,
            playlists: ch.playlists.iter().map(|p| PlaylistInfo {
                name: p.name.clone(),
                videos: p.videos.iter().map(|v| to_info(v, &watched)).collect(),
            }).collect(),
            videos: ch.videos.iter().map(|v| to_info(v, &watched)).collect(),
        }
    }).collect();

    let folders = state.db.list_folders().unwrap_or_default();
    LibraryResponse { channels, folders }
}

async fn get_music(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let music_root = {
        let dl = state.downloader.lock().unwrap();
        dl.music_root()
    };
    let tracks = library::scan_music(&music_root);
    let track_infos: Vec<TrackInfo> = tracks.into_iter().map(|t| {
        let audio_url = {
            let rel = t.path.strip_prefix(&music_root).ok()
                .map(|r| format!("/music-files/{}", r.display()));
            rel
        };
        let thumb_url = t.thumb_path.as_deref().and_then(|p| {
            let rel = p.strip_prefix(&music_root).ok()?;
            Some(format!("/music-files/{}", rel.display()))
        });
        TrackInfo {
            id: t.id,
            title: t.title,
            artist: t.artist,
            duration_secs: t.duration_secs,
            file_size: t.file_size,
            audio_url,
            thumb_url,
        }
    }).collect();
    Json(serde_json::json!({ "tracks": track_infos }))
}

/// `GET /ws/progress` — WebSocket upgrade that streams the same
/// [`ProgressResponse`] payload as the polling HTTP endpoint, pushed
/// from the background broadcast task. Clients should treat the JSON
/// payload identically to `/api/progress`.
///
/// On socket close (auth lost, network drop, server restart) the JS
/// client falls back to polling `/api/progress` so users with broken
/// WebSocket setups (some reverse proxies, certain mobile carriers)
/// still see updates.
async fn ws_progress(
    State(state): State<Arc<WebState>>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> axum::response::Response {
    ws.on_upgrade(move |socket| ws_progress_handler(socket, state))
}

async fn ws_progress_handler(
    mut socket: axum::extract::ws::WebSocket,
    state: Arc<WebState>,
) {
    use axum::extract::ws::Message;
    // Subscribe before sending the initial snapshot so we don't miss a
    // broadcast that fires between the snapshot build and the subscribe.
    let mut rx = state.progress_tx.subscribe();
    // Initial snapshot so the UI shows the current state immediately
    // (without waiting up to 5s for the next idle broadcast tick).
    let initial = {
        let mut dl = state.downloader.lock().unwrap();
        dl.poll();
        let queued = dl.pending_snapshots().into_iter()
            .map(|(label, url)| QueuedSnapshot { label, url })
            .collect::<Vec<_>>();
        ProgressResponse {
            jobs: WebState::job_snapshots(&dl),
            queued,
            max_concurrent: dl.max_concurrent,
        }
    };
    if let Ok(json) = serde_json::to_string(&initial) {
        if socket.send(Message::Text(json)).await.is_err() { return; }
    }
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(json) => {
                    if socket.send(Message::Text(json)).await.is_err() { break; }
                }
                // Subscriber lagged. Resync via the next broadcast tick.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            // Drain inbound frames (mostly pongs / client closes) so the
            // socket doesn't stall.
            frame = socket.recv() => match frame {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                _ => {}
            },
        }
    }
}

async fn get_progress(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let mut dl = state.downloader.lock().unwrap();
    dl.poll();
    let queued = dl.pending_snapshots().into_iter()
        .map(|(label, url)| QueuedSnapshot { label, url })
        .collect();
    let max_concurrent = dl.max_concurrent;
    Json(ProgressResponse { jobs: WebState::job_snapshots(&dl), queued, max_concurrent })
}

async fn post_download(
    State(state): State<Arc<WebState>>,
    Json(body): Json<StartDownloadRequest>,
) -> impl IntoResponse {
    // Access control is handled centrally by auth_middleware; reaching here
    // means the request is authenticated (or no password is configured).
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty URL").into_response();
    }
    let mut dl = state.downloader.lock().unwrap();
    if body.quality == "music" {
        dl.start_music(url);
    } else {
        let quality = match body.quality.as_str() {
            "1080p" => DownloadQuality::Res1080,
            "720p"  => DownloadQuality::Res720,
            "480p"  => DownloadQuality::Res480,
            "360p"  => DownloadQuality::Res360,
            _       => DownloadQuality::Best,
        };
        let info = classify_url(&url);
        // Submit from the web download dialog: the user just chose the
        // quality/live/full_scan values. We don't yet know which channel
        // the URL belongs to, so channel options aren't applied here.
        dl.start(url, &info, body.full_scan, quality, body.live, None);
    }
    (StatusCode::ACCEPTED, "ok").into_response()
}

async fn post_watched(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let db = &state.db;
    let mut watched = state.watched.lock().unwrap();
    let now_watched = !watched.contains(&video_id);
    if let Err(e) = db.set_watched(&video_id, now_watched) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    if now_watched { watched.insert(video_id); } else { watched.remove(&video_id); }
    drop(watched);
    bump_library_version(&state);
    (StatusCode::OK, if now_watched { "watched" } else { "unwatched" }).into_response()
}

async fn get_settings(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let cfg = state.config.lock().unwrap();
    let source_url = cfg.web.source_url.clone();
    let port = cfg.web.port;
    let current_bind = cfg.web.bind.clone();
    let available_binds = get_available_binds(port);
    let plex_library_path = cfg.plex.library_path.as_deref()
        .map(|p| p.display().to_string());
    let scheduler_enabled = cfg.scheduler.enabled;
    let scheduler_interval_hours = cfg.scheduler.interval_hours;
    let max_concurrent = cfg.backup.max_concurrent;
    let use_bundled_ytdlp = cfg.backup.use_bundled_ytdlp;
    drop(cfg);

    let scheduler_next_check_secs = if scheduler_enabled {
        let last = *state.last_scheduled_check.lock().unwrap();
        let interval_secs = scheduler_interval_hours as u64 * 3600;
        Some(match last {
            None => 0,
            Some(t) => interval_secs.saturating_sub(t.elapsed().as_secs()),
        })
    } else {
        None
    };

    let download_password_required = state.password_required_cache.load(Ordering::Relaxed);
    Json(SettingsPayload {
        transcode: state.transcode.load(Ordering::Relaxed),
        source_url,
        current_bind: Some(format!("{}:{}", current_bind, port)),
        available_binds: Some(available_binds),
        bind_mode: None,
        download_password_required,
        new_download_password: None,
        plex_library_path,
        scheduler_enabled,
        scheduler_interval_hours,
        scheduler_next_check_secs,
        max_concurrent,
        use_bundled_ytdlp,
        bundled_ytdlp_installed: crate::ytdlp_bin::bundled_installed(),
    })
}

async fn post_settings(
    State(state): State<Arc<WebState>>,
    Json(body): Json<SettingsPayload>,
) -> impl IntoResponse {
    state.transcode.store(body.transcode, Ordering::Relaxed);
    let mut cfg = state.config.lock().unwrap();
    cfg.web.transcode = body.transcode;

    if let Some(new_mode) = &body.bind_mode {
        let new_addr = resolve_bind_mode(new_mode);
        cfg.web.bind = new_addr;
    }

    if let Some(ref p) = body.plex_library_path {
        cfg.plex.library_path = if p.trim().is_empty() { None } else { Some(std::path::PathBuf::from(p.trim())) };
    }

    if let Some(ref u) = body.source_url {
        let trimmed = u.trim();
        cfg.web.source_url = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
    }
    cfg.scheduler.enabled = body.scheduler_enabled;
    if body.scheduler_interval_hours > 0 {
        cfg.scheduler.interval_hours = body.scheduler_interval_hours;
    }
    if body.max_concurrent > 0 {
        cfg.backup.max_concurrent = body.max_concurrent;
    }
    cfg.backup.use_bundled_ytdlp = body.use_bundled_ytdlp;

    if let Err(e) = cfg.save(&state.config_path) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("save failed: {e}")).into_response();
    }
    let source_url = cfg.web.source_url.clone();
    let current_bind = cfg.web.bind.clone();
    let port = cfg.web.port;
    let available_binds = get_available_binds(port);
    let scheduler_enabled = cfg.scheduler.enabled;
    let scheduler_interval_hours = cfg.scheduler.interval_hours;
    let max_concurrent = cfg.backup.max_concurrent;
    let use_bundled_ytdlp = cfg.backup.use_bundled_ytdlp;
    drop(cfg);

    // Apply the new concurrency limit and binary choice to the live downloader.
    {
        let mut dl = state.downloader.lock().unwrap();
        if body.max_concurrent > 0 {
            dl.max_concurrent = body.max_concurrent;
        }
        dl.use_bundled_ytdlp = use_bundled_ytdlp;
    }

    if let Some(new_pwd) = &body.new_download_password {
        let db = &state.db;
        let result = if new_pwd.is_empty() {
            db.set_setting("password_hash", None)
        } else if let Some(hashed) = hash_password(new_pwd) {
            db.set_setting("password_hash", Some(&hashed))
        } else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to hash password").into_response();
        };
        if let Err(e) = result {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")).into_response();
        }
        // Password changed: drop all existing sessions so they must re-authenticate.
        state.sessions.lock().unwrap().clear();
        refresh_password_cache(&state);
    }

    let plex_library_path = state.config.lock().unwrap().plex.library_path.as_deref()
        .map(|p| p.display().to_string());
    let download_password_required = state.password_required_cache.load(Ordering::Relaxed);
    let scheduler_next_check_secs = if scheduler_enabled {
        let last = *state.last_scheduled_check.lock().unwrap();
        let interval_secs = scheduler_interval_hours as u64 * 3600;
        Some(match last {
            None => 0,
            Some(t) => interval_secs.saturating_sub(t.elapsed().as_secs()),
        })
    } else {
        None
    };
    Json(SettingsPayload {
        transcode: body.transcode,
        source_url,
        current_bind: Some(format!("{}:{}", current_bind, port)),
        available_binds: Some(available_binds),
        bind_mode: None,
        download_password_required,
        new_download_password: None,
        plex_library_path,
        scheduler_enabled,
        scheduler_interval_hours,
        scheduler_next_check_secs,
        max_concurrent,
        use_bundled_ytdlp,
        bundled_ytdlp_installed: crate::ytdlp_bin::bundled_installed(),
    }).into_response()
}

/// `GET /api/sub-vtt/*path` — serve an SRT subtitle file as WebVTT.
///
/// The path is relative to the channels root.  The file must be within the
/// channels root (path traversal is rejected with 403).
async fn get_sub_vtt(
    State(state): State<Arc<WebState>>,
    Path(rel): Path<String>,
) -> Response {
    let path = state.library_root.join(&rel);
    // Reject path traversal outside the library.
    let ok = match (state.library_root.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(p)) => p.starts_with(root),
        _ => false,
    };
    if !ok {
        return StatusCode::FORBIDDEN.into_response();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let vtt = srt_to_vtt(&content);
    ([(header::CONTENT_TYPE, "text/vtt; charset=utf-8")], vtt).into_response()
}

async fn get_transcode(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> Response {
    let path = {
        let lib = state.library.lock().unwrap();
        find_video_path(&lib, &id)
    };
    let Some(path) = path else {
        return (StatusCode::NOT_FOUND, "no video").into_response();
    };

    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.arg("-hide_banner")
        .arg("-loglevel").arg("error")
        .arg("-i").arg(&path)
        .arg("-c:v").arg("libx264")
        .arg("-preset").arg("veryfast")
        .arg("-crf").arg("23")
        .arg("-c:a").arg("aac")
        .arg("-b:a").arg("128k")
        .arg("-movflags").arg("frag_keyframe+empty_moov+default_base_moof")
        .arg("-f").arg("mp4")
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("ffmpeg spawn failed: {e}")).into_response(),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "no stdout from ffmpeg").into_response(),
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        let _child_guard = child; // dropped at task end → kills ffmpeg
        use tokio::io::AsyncReadExt;
        let mut stdout = stdout;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stdout.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(Ok(Bytes::copy_from_slice(&buf[..n]))).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    break;
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}

#[derive(serde::Deserialize)]
struct ResumeBody { position: f64 }

async fn post_resume(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
    Json(body): Json<ResumeBody>,
) -> impl IntoResponse {
    // Track whether this video had a non-trivial resume position before/after
    // so we only bump the library ETag on transitions (joins / leaves the
    // "Continue watching" set). Plain position updates fire every few seconds
    // during playback — invalidating the cache on each would defeat the
    // ETag optimisation.
    let mut positions = state.positions.lock().unwrap();
    let was_resumable = positions.get(&video_id).copied().is_some_and(|p| p > 3.0);
    let db = &state.db;
    if body.position > 3.0 {
        let _ = db.set_position(&video_id, body.position);
        positions.insert(video_id.clone(), body.position);
    } else {
        let _ = db.clear_position(&video_id);
        positions.remove(&video_id);
    }
    drop(positions);
    let now_resumable = body.position > 3.0;
    if was_resumable != now_resumable {
        bump_library_version(&state);
    }
    (StatusCode::OK, "ok")
}

#[derive(serde::Deserialize)]
struct PreviewQuery { url: String }

async fn get_preview(
    State(state): State<Arc<WebState>>,
    Query(q): Query<PreviewQuery>,
) -> impl IntoResponse {
    let url = q.url.trim().to_string();
    if url.is_empty() {
        return (StatusCode::BAD_REQUEST, "no url").into_response();
    }
    let use_bundled = state.config.lock().unwrap().backup.use_bundled_ytdlp;
    if use_bundled {
        crate::ytdlp_bin::ensure_bundled_executable();
    }
    let ytdlp_path = crate::ytdlp_bin::ytdlp_invocation(use_bundled);
    let mut cmd = tokio::process::Command::new(ytdlp_path);
    // Extend PATH so yt-dlp finds the bundled deno for JS deciphering.
    let bundled_dir = crate::ytdlp_bin::bundled_dir();
    if bundled_dir.exists() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let new_path = match std::env::var_os("PATH") {
            Some(existing) => format!("{}{}{}", bundled_dir.display(), sep, existing.to_string_lossy()),
            None => bundled_dir.display().to_string(),
        };
        cmd.env("PATH", new_path);
    }
    cmd.arg("--dump-single-json")
        .arg("--flat-playlist")
        .arg("--no-warnings");
    // Mirror Downloader::apply_cookie_flags so the preview honors the same
    // cookies precedence (file > browser fallback).
    let browser = state.config.lock().unwrap().player.browser.clone();
    if std::path::Path::new("cookies.txt").exists() {
        cmd.arg("--cookies").arg("cookies.txt");
    } else if !browser.is_empty() && browser != "none" {
        cmd.arg("--cookies-from-browser").arg(&browser);
    }
    // The bundled venv install ships `curl_cffi`, so --impersonate works
    // in both bundled and system modes. Pick the target per source so
    // TikTok previews use the mobile profile and Twitch skips it entirely.
    let _ = use_bundled;
    if let Some(target) = crate::platform::Platform::from_url(&url).impersonate_target() {
        cmd.arg("--impersonate").arg(target);
    }
    let output = cmd
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    let output = match output {
        Ok(o) if !o.stdout.is_empty() => o,
        Ok(_) => return (StatusCode::BAD_REQUEST, "yt-dlp returned no data").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("yt-dlp failed: {e}")).into_response(),
    };
    let val: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, "could not parse yt-dlp output").into_response(),
    };
    let is_playlist = val.get("_type").and_then(|t| t.as_str()).map_or(false, |t| t == "playlist");
    let entry_count = val.get("entries").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0);
    let preview = serde_json::json!({
        "type": if is_playlist { "channel/playlist" } else { "video" },
        "title": val.get("title").and_then(|t| t.as_str()),
        "channel": val.get("channel").or_else(|| val.get("uploader")).and_then(|t| t.as_str()),
        "thumbnail": val.get("thumbnail").and_then(|t| t.as_str()),
        "duration": val.get("duration").and_then(|d| d.as_f64()),
        "view_count": val.get("view_count").and_then(|v| v.as_u64()),
        "entry_count": entry_count,
    });
    Json(preview).into_response()
}

async fn post_clear_jobs(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    state.downloader.lock().unwrap().clear_finished();
    (StatusCode::OK, "cleared")
}

async fn post_remove_job(
    State(state): State<Arc<WebState>>,
    Path(idx): Path<usize>,
) -> impl IntoResponse {
    state.downloader.lock().unwrap().remove_job(idx);
    (StatusCode::OK, "removed")
}

async fn get_chapters(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let info_path = {
        let lib = state.library.lock().unwrap();
        find_video_info_path(&lib, &video_id)
    };
    let Some(info_path) = info_path else {
        return (StatusCode::NOT_FOUND, "no info.json").into_response();
    };
    let Ok(text) = std::fs::read_to_string(&info_path) else {
        return (StatusCode::OK, Json(serde_json::json!({"chapters": []}))).into_response();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (StatusCode::OK, Json(serde_json::json!({"chapters": []}))).into_response();
    };
    let chapters = val.get("chapters")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter().filter_map(|ch| {
                let title = ch.get("title").and_then(|t| t.as_str())?.to_string();
                let start = ch.get("start_time").and_then(|t| t.as_f64())?;
                let end = ch.get("end_time").and_then(|t| t.as_f64());
                Some(serde_json::json!({"title": title, "start": start, "end": end}))
            }).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (StatusCode::OK, Json(serde_json::json!({"chapters": chapters}))).into_response()
}

/// `GET /api/comments/:id` — extract the `comments` array from a video's
/// info.json sidecar and return it as JSON.
///
/// yt-dlp populates the field when `--write-comments` is in effect (see
/// [`crate::download_options::DownloadOptions::fetch_comments`]); the
/// response is `{"comments": []}` for videos that haven't had comments
/// captured yet. The endpoint filters down to the fields the UI actually
/// renders so a 10k-comment dump on a popular video stays manageable.
async fn get_comments(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let info_path = {
        let lib = state.library.lock().unwrap();
        find_video_info_path(&lib, &video_id)
    };
    let Some(info_path) = info_path else {
        return (StatusCode::NOT_FOUND, "no info.json").into_response();
    };
    let Ok(text) = std::fs::read_to_string(&info_path) else {
        return (StatusCode::OK, Json(serde_json::json!({"comments": []}))).into_response();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (StatusCode::OK, Json(serde_json::json!({"comments": []}))).into_response();
    };
    let comments = val.get("comments")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter().filter_map(|c| {
                let id = c.get("id").and_then(|v| v.as_str())?.to_string();
                let text = c.get("text").and_then(|v| v.as_str())?.to_string();
                let author = c.get("author").and_then(|v| v.as_str()).map(String::from);
                let likes = c.get("like_count").and_then(|v| v.as_i64());
                let parent = c.get("parent").and_then(|v| v.as_str()).map(String::from);
                let time = c.get("_time_text").and_then(|v| v.as_str())
                    .or_else(|| c.get("time_text").and_then(|v| v.as_str()))
                    .map(String::from);
                Some(serde_json::json!({
                    "id": id,
                    "author": author,
                    "text": text,
                    "likes": likes,
                    "parent": parent,
                    "time": time,
                }))
            }).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (StatusCode::OK, Json(serde_json::json!({"comments": comments}))).into_response()
}

async fn get_metadata(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let info_path = {
        let lib = state.library.lock().unwrap();
        find_video_info_path(&lib, &video_id)
    };
    let Some(info_path) = info_path else {
        return (StatusCode::NOT_FOUND, "no info.json").into_response();
    };
    match std::fs::read_to_string(&info_path) {
        Ok(text) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            text,
        ).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_description(
    State(state): State<Arc<WebState>>,
    Path(video_id): Path<String>,
) -> impl IntoResponse {
    let lib = state.library.lock().unwrap();
    if let Some((v, _)) = library::find_video(&lib, &video_id) {
        let text = v.description_path.as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        return (StatusCode::OK, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response();
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn post_rescan(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let mut new_lib = library::scan_channels_with_cache(&state.channels_root, Some(&state.db));
    if let Ok(map) = state.db.get_all_channel_options() {
        library::apply_channel_options(&mut new_lib, &map);
    }
    if let Ok(folder_map) = state.db.get_all_channel_assignments() {
        library::apply_channel_folders(&mut new_lib, &folder_map);
    }
    // Refresh watched from DB after rescan
    if let Ok(w) = state.db.get_watched() {
        *state.watched.lock().unwrap() = w;
    }
    *state.library.lock().unwrap() = new_lib;
    bump_library_version(&state);
    (StatusCode::OK, "rescanned")
}

/// `POST /api/plex/generate` — generate (or refresh) the Plex symlink tree.
async fn post_plex_generate(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let plex_root = {
        let cfg = state.config.lock().unwrap();
        cfg.plex.library_path.clone()
    };
    let Some(plex_root) = plex_root else {
        return (StatusCode::BAD_REQUEST, "no plex.library_path configured").into_response();
    };
    let lib = state.library.lock().unwrap();
    let result = crate::plex::generate(&lib, &plex_root);
    drop(lib);
    Json(serde_json::json!({
        "links_created": result.links_created,
        "errors": result.errors,
    })).into_response()
}

/// `GET /api/maintenance/scan` — report duplicate videos and missing assets.
async fn get_maintenance_scan(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let lib = state.library.lock().unwrap();
    let report = maintenance::scan(&state.library_root, &lib);
    Json(report)
}

#[derive(Deserialize)]
struct RemoveRequest {
    paths: Vec<PathBuf>,
}

/// `POST /api/maintenance/remove` — delete the listed duplicate files.
/// Paths outside the library root are refused.
async fn post_maintenance_remove(
    State(state): State<Arc<WebState>>,
    Json(body): Json<RemoveRequest>,
) -> impl IntoResponse {
    let (removed, errors) = maintenance::remove_files(&state.library_root, &body.paths);
    // Refresh the library so the removed copies disappear from the UI.
    let mut new_lib = library::scan_channels_with_cache(&state.channels_root, Some(&state.db));
    if let Ok(map) = state.db.get_all_channel_options() {
        library::apply_channel_options(&mut new_lib, &map);
    }
    if let Ok(folder_map) = state.db.get_all_channel_assignments() {
        library::apply_channel_folders(&mut new_lib, &folder_map);
    }
    *state.library.lock().unwrap() = new_lib;
    bump_library_version(&state);
    Json(serde_json::json!({ "removed": removed, "errors": errors }))
}

/// `POST /api/maintenance/repair/:id` — re-fetch missing sidecars for one video.
async fn post_maintenance_repair(
    State(state): State<Arc<WebState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let target = {
        let lib = state.library.lock().unwrap();
        maintenance::locate(&lib, &id)
    };
    let Some((dir, stem)) = target else {
        return (StatusCode::NOT_FOUND, "video not found in library").into_response();
    };
    state.downloader.lock().unwrap().repair(&id, &dir, &stem);
    (StatusCode::ACCEPTED, "repair queued").into_response()
}

/// `POST /api/scheduler/run` — trigger an immediate scheduled channel check.
#[derive(Deserialize)]
struct FolderCreateBody { name: String }

#[derive(Deserialize)]
struct FolderRenameBody { name: String }

#[derive(Deserialize)]
struct AssignFolderBody { folder_id: Option<i64> }

/// `POST /api/folders` — create a new folder. Body: `{ "name": "<name>" }`.
/// Returns `{ "id": <new_id> }`.
async fn post_create_folder(
    State(state): State<Arc<WebState>>,
    Json(body): Json<FolderCreateBody>,
) -> impl IntoResponse {
    let name = body.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "folder name empty").into_response();
    }
    match state.db.create_folder(name) {
        Ok(id) => {
            bump_library_version(&state);
            Json(serde_json::json!({"id": id})).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response(),
    }
}

/// `POST /api/folders/:id/rename` — rename an existing folder.
async fn post_rename_folder(
    State(state): State<Arc<WebState>>,
    Path(id): Path<i64>,
    Json(body): Json<FolderRenameBody>,
) -> impl IntoResponse {
    let name = body.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "folder name empty").into_response();
    }
    match state.db.rename_folder(id, name) {
        Ok(()) => {
            bump_library_version(&state);
            (StatusCode::OK, "ok").into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response(),
    }
}

/// `DELETE /api/folders/:id` — drop the folder. Member channels revert
/// to "Unfiled" via the foreign-key cascade.
async fn delete_folder(
    State(state): State<Arc<WebState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match state.db.delete_folder(id) {
        Ok(()) => {
            // Mirror onto the live library snapshot.
            let mut lib = state.library.lock().unwrap();
            for ch in lib.iter_mut() {
                if ch.folder_id == Some(id) {
                    ch.folder_id = None;
                }
            }
            drop(lib);
            bump_library_version(&state);
            (StatusCode::OK, "ok").into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response(),
    }
}

/// `GET /api/backup/db` — stream the SQLite database file back as an
/// attachment so the user can keep an offsite copy of their watched /
/// flag / channel-options / folder state.
///
/// We intentionally don't snapshot config.toml or cookies.txt here:
/// config is short and easy to recreate, cookies.txt contains live
/// session credentials that shouldn't fly over the wire unprompted, and
/// keeping the endpoint to one file means no extra deps for tar/zip.
async fn get_backup_db(State(state): State<Arc<WebState>>) -> Response {
    let db_path = state.channels_root.join("yt-offline.db");
    let Ok(bytes) = std::fs::read(&db_path) else {
        return (StatusCode::NOT_FOUND, "no db file on disk (running in-memory?)").into_response();
    };
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let filename = format!("yt-offline-{now_secs}.db");
    (
        [
            (header::CONTENT_TYPE, "application/x-sqlite3".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    ).into_response()
}

/// `POST /api/restore/db` — accept a SQLite database body and merge its
/// rows into the live DB. Mirrors `GET /api/backup/db` in the opposite
/// direction.
///
/// The body is the raw `yt-offline.db` bytes — same shape as what
/// `get_backup_db` produces. We write it to a sibling temp file, hand it
/// to [`Database::restore_from_backup`] for the actual merge, then
/// refresh the in-memory watched / positions / flags caches so the next
/// `/api/library` response reflects the import.
///
/// Hard-cap the body at 100 MB to keep a malicious or fat-finger upload
/// from filling tmpfs. Realistic backups are a few KB to a few MB for
/// even very large libraries.
async fn post_restore_db(
    State(state): State<Arc<WebState>>,
    body: Bytes,
) -> Response {
    const MAX_BACKUP_BYTES: usize = 100 * 1024 * 1024;
    if body.len() > MAX_BACKUP_BYTES {
        return (StatusCode::PAYLOAD_TOO_LARGE,
                "backup file too large (max 100 MB)").into_response();
    }
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty body — POST the .db bytes").into_response();
    }
    // SQLite magic header check before we even bother writing the file.
    // All valid SQLite 3 databases start with "SQLite format 3\0".
    if !body.starts_with(b"SQLite format 3\0") {
        return (StatusCode::BAD_REQUEST,
                "not a SQLite database (bad magic header)").into_response();
    }

    // Write to a sibling tmp file so it lives on the same filesystem as
    // the live DB — keeps ATTACH happy and avoids cross-device rename
    // issues if we ever extend this to atomic-replace semantics.
    let tmp_path = state.channels_root.join(".yt-offline.restore.tmp");
    if let Err(e) = std::fs::write(&tmp_path, &body) {
        return (StatusCode::INTERNAL_SERVER_ERROR,
                format!("write tmp file: {e}")).into_response();
    }

    let summary = match state.db.restore_from_backup(&tmp_path) {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return (StatusCode::BAD_REQUEST,
                    format!("restore failed: {e}")).into_response();
        }
    };
    let _ = std::fs::remove_file(&tmp_path);

    // Refresh in-memory caches so the next /api/library reflects the new
    // watched / position / flag rows without waiting for an app restart.
    if let Ok(w) = state.db.get_watched() {
        *state.watched.lock().unwrap() = w;
    }
    if let Ok(p) = state.db.get_positions() {
        *state.positions.lock().unwrap() = p;
    }
    if let Ok(f) = state.db.get_video_flags() {
        *state.flags.lock().unwrap() = f;
    }
    // Bump the library version so cached /api/library responses revalidate.
    bump_library_version(&state);

    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_string()),
    ).into_response()
}

/// `POST /api/folders/:id/check` — fire a re-check on every channel in
/// the folder. Mirrors `POST /api/scheduler/run` but scoped to a single
/// folder's members. Each channel's stored download_options + quality
/// override apply as if the user had hit "Check for new videos" by hand.
async fn post_check_folder(
    State(state): State<Arc<WebState>>,
    Path(folder_id): Path<i64>,
) -> impl IntoResponse {
    if state.downloader.lock().unwrap().any_running() {
        return (StatusCode::CONFLICT, "downloads already running").into_response();
    }
    let scheduled: Vec<(String, crate::download_options::DownloadOptions)> =
        state.library.lock().unwrap()
            .iter()
            .filter(|ch| ch.folder_id == Some(folder_id))
            .map(|ch| (crate::downloader::recheck_url(ch), ch.download_options.clone()))
            .collect();
    if scheduled.is_empty() {
        return (StatusCode::OK, "no channels in folder").into_response();
    }
    let count = scheduled.len();
    let mut dl = state.downloader.lock().unwrap();
    for (url, opts) in scheduled {
        let info = classify_url(&url);
        let quality = opts.quality.unwrap_or(DownloadQuality::Best);
        dl.start(url, &info, true, quality, false, Some(&opts));
    }
    (StatusCode::ACCEPTED, format!("started {count} channel checks")).into_response()
}

/// `POST /api/channels/:platform/:handle/folder` — move a channel into a
/// folder, or pass `null` to clear (back to "Unfiled").
async fn post_assign_folder(
    State(state): State<Arc<WebState>>,
    Path((platform, handle)): Path<(String, String)>,
    Json(body): Json<AssignFolderBody>,
) -> impl IntoResponse {
    if let Err(e) = state.db.set_channel_folder(&platform, &handle, body.folder_id) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response();
    }
    {
        let mut lib = state.library.lock().unwrap();
        for ch in lib.iter_mut() {
            if ch.platform.dir_name() == platform && ch.name == handle {
                ch.folder_id = body.folder_id;
                break;
            }
        }
    }
    bump_library_version(&state);
    (StatusCode::OK, "ok").into_response()
}

#[derive(Deserialize)]
struct FlagToggleBody { value: bool }

/// `POST /api/videos/:id/flags/:flag` — set or clear a single per-video
/// flag (`bookmark` / `favourite` / `waiting` / `archive`). The watched
/// flag is handled by the separate `POST /api/watched/:id` endpoint for
/// backward compatibility.
async fn post_video_flag(
    State(state): State<Arc<WebState>>,
    Path((video_id, flag)): Path<(String, String)>,
    Json(body): Json<FlagToggleBody>,
) -> impl IntoResponse {
    let set_ref: &str = match flag.as_str() {
        "bookmark" | "favourite" | "waiting" | "archive" => flag.as_str(),
        _ => return (StatusCode::BAD_REQUEST, "unknown flag").into_response(),
    };
    if let Err(e) = state.db.set_video_flag(&video_id, set_ref, body.value) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    // Mirror into the in-memory bundle so the next GET /api/library reflects
    // the change without waiting for a rescan.
    {
        let mut bundle = state.flags.lock().unwrap();
        let target = match set_ref {
            "bookmark" => &mut bundle.bookmark,
            "favourite" => &mut bundle.favourite,
            "waiting" => &mut bundle.waiting,
            "archive" => &mut bundle.archive,
            _ => unreachable!("validated above"),
        };
        if body.value { target.insert(video_id); } else { target.remove(&video_id); }
    }
    bump_library_version(&state);
    (StatusCode::OK, "ok").into_response()
}

/// `GET /api/channels/:platform/:handle/options` — fetch the stored
/// download-option overrides for a single channel. Returns the default
/// (empty) options when nothing is stored.
async fn get_channel_options(
    State(state): State<Arc<WebState>>,
    Path((platform, handle)): Path<(String, String)>,
) -> impl IntoResponse {
    let opts = match state.db.get_channel_options(&platform, &handle) {
        Ok(Some(json)) => crate::download_options::DownloadOptions::from_json(&json),
        _ => crate::download_options::DownloadOptions::default(),
    };
    Json(opts).into_response()
}

/// `POST /api/channels/:platform/:handle/options` — upsert the overrides.
/// An empty-by-default body is stored as DELETE so we don't keep useless rows.
async fn post_channel_options(
    State(state): State<Arc<WebState>>,
    Path((platform, handle)): Path<(String, String)>,
    Json(body): Json<crate::download_options::DownloadOptions>,
) -> impl IntoResponse {
    let result = if body.is_empty() {
        state.db.delete_channel_options(&platform, &handle)
    } else {
        match serde_json::to_string(&body) {
            Ok(json) => state.db.set_channel_options(&platform, &handle, &json),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("encode: {e}")).into_response(),
        }
    };
    if let Err(e) = result {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response();
    }
    // Push the new options onto the live library snapshot so a re-check
    // triggered before the next rescan still sees them.
    {
        let mut lib = state.library.lock().unwrap();
        for ch in lib.iter_mut() {
            if ch.platform.dir_name() == platform && ch.name == handle {
                ch.download_options = body.clone();
                break;
            }
        }
    }
    bump_library_version(&state);
    (StatusCode::OK, "ok").into_response()
}

/// `DELETE /api/channels/:platform/:handle/options` — clear overrides, returning
/// the channel to global defaults.
async fn delete_channel_options(
    State(state): State<Arc<WebState>>,
    Path((platform, handle)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Err(e) = state.db.delete_channel_options(&platform, &handle) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response();
    }
    {
        let mut lib = state.library.lock().unwrap();
        for ch in lib.iter_mut() {
            if ch.platform.dir_name() == platform && ch.name == handle {
                ch.download_options = Default::default();
                break;
            }
        }
    }
    bump_library_version(&state);
    (StatusCode::OK, "cleared").into_response()
}

async fn post_scheduler_run(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    if state.downloader.lock().unwrap().any_running() {
        return (StatusCode::CONFLICT, "downloads already running").into_response();
    }
    // Snapshot (url, options) pairs so we can iterate without holding the
    // library lock through start().
    let scheduled: Vec<(String, crate::download_options::DownloadOptions)> =
        state.library.lock().unwrap()
            .iter()
            .map(|ch| (crate::downloader::recheck_url(ch), ch.download_options.clone()))
            .collect();
    if scheduled.is_empty() {
        return (StatusCode::OK, "no channels to check").into_response();
    }
    let count = scheduled.len();
    let mut dl = state.downloader.lock().unwrap();
    for (url, opts) in scheduled {
        let info = classify_url(&url);
        let quality = opts.quality.unwrap_or(DownloadQuality::Best);
        dl.start(url, &info, true, quality, false, Some(&opts));
    }
    *state.last_scheduled_check.lock().unwrap() = Some(std::time::Instant::now());
    (StatusCode::ACCEPTED, format!("started {count} channel checks")).into_response()
}

/// `GET /api/stats` — aggregate library statistics (totals, top channels,
/// per-year upload histogram, per-week download activity).
async fn get_stats(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let lib = state.library.lock().unwrap();
    let watched = state.watched.lock().unwrap();
    let positions = state.positions.lock().unwrap();
    let report = crate::stats::build(&lib, &watched, &positions, crate::stats::now_unix());
    Json(report)
}

/// `POST /api/ytdlp/update` — download (or update) the bundled yt-dlp + deno
/// binaries. Streams output through a regular [`Job`] entry.
async fn post_ytdlp_update(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    state.downloader.lock().unwrap().start_ytdlp_update();
    (StatusCode::ACCEPTED, "started bundled yt-dlp update").into_response()
}

/// Delete cookies.txt, removing all stored session cookies.
pub fn clear_cookies() -> Result<(), String> {
    let p = cookies_path();
    if p.exists() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `GET /api/cookies` — report whether a cookies file exists and its entry count.
async fn get_cookies() -> impl IntoResponse {
    let (exists, count) = cookies_status();
    Json(serde_json::json!({ "exists": exists, "cookies": count }))
}

#[derive(Deserialize)]
struct CookiesBody {
    cookies: String,
}

/// `POST /api/cookies` — replace cookies.txt with pasted Netscape-format content.
async fn post_cookies(Json(body): Json<CookiesBody>) -> impl IntoResponse {
    match write_cookies(&body.cookies) {
        Ok(count) => Json(serde_json::json!({ "ok": true, "cookies": count })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// `DELETE /api/cookies` — remove cookies.txt entirely.
async fn delete_cookies() -> impl IntoResponse {
    match clear_cookies() {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run the web server in the foreground until the process is killed.
///
/// The `--web` CLI path. We hand `serve` a sentinel `Receiver` whose `Sender`
/// is leaked, so `recv()` blocks forever and the shutdown branch of the
/// `tokio::select!` inside `serve` never fires. The previous `let _ = tx`
/// dropped the sender on the same line and caused immediate shutdown.
pub fn run(config: Config) -> ! {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::mem::forget(tx); // intentional leak: keeps rx alive forever
    rt.block_on(serve(config, rx));
    // serve() should run until the process exits. If it ever returns,
    // sit in an idle loop instead of panicking — the listener bind failed
    // and was already logged.
    loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
}

pub fn run_with_shutdown(config: Config) -> std::sync::mpsc::Sender<()> {
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(serve(config, shutdown_rx));
    });
    shutdown_tx
}

async fn serve(config: Config, shutdown_rx: std::sync::mpsc::Receiver<()>) {
    let channels_root = config.backup.directory.clone();
    let _ = std::fs::create_dir_all(&channels_root);
    // library_root holds every platform folder side-by-side (channels/,
    // tiktok/, twitch/, …). The implicit anchor is the parent of the
    // user-configured channels dir.
    let library_root = channels_root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| channels_root.clone());
    let _ = std::fs::create_dir_all(&library_root);
    // Pre-create every platform's folder so the static-file mount can serve
    // them without 404ing on first access.
    for &p in crate::platform::Platform::all() {
        let dir = crate::platform::platform_root(&channels_root, p);
        let _ = std::fs::create_dir_all(&dir);
    }
    let db_path = channels_root.join("yt-offline.db");
    let config_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("config.toml");

    // Open the DB first so the scanner can consult info_cache to skip
    // re-parsing unchanged info.json files (the dominant cost on a
    // warm-filesystem rescan of a large library).
    let db = Database::open(&db_path)
        .unwrap_or_else(|_| Database::open_in_memory().expect("in-memory db failed"));
    let mut library = library::scan_channels_with_cache(&channels_root, Some(&db));
    if let Ok(map) = db.get_all_channel_options() {
        library::apply_channel_options(&mut library, &map);
    }
    if let Ok(folder_map) = db.get_all_channel_assignments() {
        library::apply_channel_folders(&mut library, &folder_map);
    }
    let watched = db.get_watched().unwrap_or_default();
    let positions = db.get_positions().unwrap_or_default();

    let downloader = Downloader::new(
        channels_root.clone(),
        config.player.browser.clone(),
        config.backup.max_concurrent,
        config.backup.use_bundled_ytdlp,
    );
    let music_root = downloader.music_root();
    let _ = std::fs::create_dir_all(&music_root);
    let transcode = AtomicBool::new(config.web.transcode);
    let port = config.web.port;
    let bind_addr = config.web.bind.clone();

    let password_required_initial = db.get_setting("password_hash").ok().flatten().is_some();
    let flags = db.get_video_flags().unwrap_or_default();
    // Capacity 16 is plenty — only a small number of browser tabs ever
    // subscribe and the broadcast is lossy by design (subscribers that lag
    // get a Lagged error and just resubscribe).
    let (progress_tx, _initial_rx) = tokio::sync::broadcast::channel::<String>(16);
    let state = Arc::new(WebState {
        library: Mutex::new(library),
        downloader: Mutex::new(downloader),
        watched: Mutex::new(watched),
        positions: Mutex::new(positions),
        flags: Mutex::new(flags),
        db,
        channels_root: channels_root.clone(),
        library_root: library_root.clone(),
        config_path,
        config: Mutex::new(config),
        transcode,
        sessions: Mutex::new(HashMap::new()),
        last_scheduled_check: Mutex::new(None),
        password_required_cache: AtomicBool::new(password_required_initial),
        library_version: AtomicU64::new(1),
        progress_tx,
        login_attempts: Mutex::new(HashMap::new()),
        library_body_cache: Mutex::new(None),
    });

    // Broadcast progress snapshots to WebSocket subscribers. Ticks fast
    // (every 500 ms) while any download is active or queued; slow
    // (every 5 s) when idle, just to keep clients refreshed if state
    // changed via direct flag/option/folder edits.
    let progress_state = state.clone();
    tokio::spawn(async move {
        loop {
            // Pick interval based on whether there's anything to report on.
            let interval_ms = {
                let dl = progress_state.downloader.lock().unwrap();
                if dl.any_running() || dl.pending_count() > 0 { 500 } else { 5_000 }
            };
            tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
            // Skip broadcast when no one's listening — saves the JSON encode.
            if progress_state.progress_tx.receiver_count() == 0 {
                continue;
            }
            // Drive the same poll() that /api/progress does so the snapshot
            // includes the latest stdout/stderr lines.
            let snapshot = {
                let mut dl = progress_state.downloader.lock().unwrap();
                dl.poll();
                let queued = dl.pending_snapshots().into_iter()
                    .map(|(label, url)| QueuedSnapshot { label, url })
                    .collect::<Vec<_>>();
                ProgressResponse {
                    jobs: WebState::job_snapshots(&dl),
                    queued,
                    max_concurrent: dl.max_concurrent,
                }
            };
            if let Ok(json) = serde_json::to_string(&snapshot) {
                let _ = progress_state.progress_tx.send(json);
            }
        }
    });

    // Background scheduler — ticks every 60 s; runs channel checks when due.
    let sched_state = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let (enabled, interval_hours) = {
                let cfg = sched_state.config.lock().unwrap();
                (cfg.scheduler.enabled, cfg.scheduler.interval_hours)
            };
            if !enabled { continue; }
            if sched_state.downloader.lock().unwrap().any_running() { continue; }
            // Clamp interval defensively. A manually edited config.toml with 0
            // (or accidentally tiny value) would otherwise trigger every tick.
            let safe_hours = interval_hours.max(1);
            let interval_dur = std::time::Duration::from_secs(safe_hours as u64 * 3600);
            let due = {
                let last = *sched_state.last_scheduled_check.lock().unwrap();
                last.map_or(true, |t| t.elapsed() >= interval_dur)
            };
            if !due { continue; }
            let scheduled: Vec<(String, crate::download_options::DownloadOptions)> =
                sched_state.library.lock().unwrap()
                    .iter()
                    .map(|ch| (crate::downloader::recheck_url(ch), ch.download_options.clone()))
                    .collect();
            if scheduled.is_empty() { continue; }
            let mut dl = sched_state.downloader.lock().unwrap();
            for (url, opts) in scheduled {
                let info = classify_url(&url);
                let quality = opts.quality.unwrap_or(DownloadQuality::Best);
                dl.start(url, &info, true, quality, false, Some(&opts));
            }
            *sched_state.last_scheduled_check.lock().unwrap() = Some(std::time::Instant::now());
        }
    });

    let app = Router::new()
        .route("/", get(get_index))
        .route("/api/library", get(get_library))
        .route("/api/progress", get(get_progress))
        .route("/ws/progress", get(ws_progress))
        .route("/api/download", post(post_download))
        .route("/api/watched/:id", post(post_watched))
        .route("/api/videos/:id/flags/:flag", post(post_video_flag))
        .route("/api/folders", post(post_create_folder))
        .route("/api/folders/:id/rename", post(post_rename_folder))
        .route("/api/folders/:id/check", post(post_check_folder))
        .route("/api/backup/db", get(get_backup_db))
        .route("/api/restore/db", post(post_restore_db))
        .route("/api/folders/:id", axum::routing::delete(delete_folder))
        .route("/api/channels/:platform/:handle/folder", post(post_assign_folder))
        .route("/api/resume/:id", post(post_resume))
        .route("/api/preview", get(get_preview))
        .route("/api/rescan", post(post_rescan))
        .route("/api/jobs/clear", post(post_clear_jobs))
        .route("/api/jobs/:idx", axum::routing::delete(post_remove_job))
        .route("/api/description/:id", get(get_description))
        .route("/api/chapters/:id", get(get_chapters))
        .route("/api/comments/:id", get(get_comments))
        .route("/api/metadata/:id", get(get_metadata))
        .route("/api/settings", get(get_settings).post(post_settings))
        .route("/api/transcode/:id", get(get_transcode))
        .route("/api/sub-vtt/*path", get(get_sub_vtt))
        .route("/api/plex/generate", post(post_plex_generate))
        .route("/api/maintenance/scan", get(get_maintenance_scan))
        .route("/api/maintenance/remove", post(post_maintenance_remove))
        .route("/api/maintenance/repair/:id", post(post_maintenance_repair))
        .route("/api/cookies", get(get_cookies).post(post_cookies).delete(delete_cookies))
        .route("/api/scheduler/run", post(post_scheduler_run))
        .route(
            "/api/channels/:platform/:handle/options",
            get(get_channel_options).post(post_channel_options).delete(delete_channel_options),
        )
        .route("/api/stats", get(get_stats))
        .route("/api/ytdlp/update", post(post_ytdlp_update))
        .route("/api/music", get(get_music))
        .route("/api/login", post(post_login))
        .route("/api/logout", post(post_logout))
        // Serve from library_root (parent of channels_root) so URLs become
        // `/files/<platform>/<creator>/...` for every platform, not just
        // YouTube. ServeDir rejects `..` and refuses symlink escapes.
        .nest_service("/files", ServeDir::new(&library_root))
        .nest_service("/music-files", ServeDir::new(&music_root))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        // Cap any uploaded body at 4 MiB. cookies.txt and POSTed JSON payloads
        // are tiny in normal use; anything larger is either accidental or
        // malicious. Path-specific overrides aren't needed since we have no
        // legitimate large-upload endpoints.
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        // Compress JSON responses (gzip). `/api/library` in particular can
        // be megabytes for large collections; gzip slices that ~10×.
        // ServeDir output (video bytes) is already-compressed media, so the
        // overhead would be wasted there — tower_http's compression layer
        // skips already-compressed content types automatically.
        .layer(CompressionLayer::new().gzip(true))
        .layer(middleware::from_fn(security_headers))
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(format!("{bind_addr}:{port}")).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Cannot bind to {bind_addr}:{port}: {e}");
            return;
        }
    };
    println!("yt-offline web UI: http://localhost:{port}");

    // `into_make_service_with_connect_info` so handlers can extract the
    // client's `SocketAddr` (used for per-IP rate limiting on /api/login).
    let server = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>());
    tokio::select! {
        _ = server => {},
        _ = tokio::task::spawn_blocking(move || {
            let _ = shutdown_rx.recv();
        }) => {
            println!("Web server shutting down");
        }
    }
}


// ── Embedded UI ───────────────────────────────────────────────────────────────
//
// The HTML/CSS/JS lives in standalone files under `src/web_ui/` so editors
// give it syntax highlighting and the diff churn for UI tweaks stays out of
// this Rust source. `include_str!` bakes them into the binary at compile
// time, so there's no runtime fs lookup and no extra deploy artifacts.

/// Standalone login page served at `GET /` when a password is set and the
/// request is unauthenticated. Posts to `/api/login` and reloads on success.
const LOGIN_HTML: &str = include_str!("web_ui/login.html");

/// Main library UI. Single-page app with embedded styles and JS — see the
/// architecture notes at the top of `web_ui/index.html`.
const HTML_UI: &str = include_str!("web_ui/index.html");
